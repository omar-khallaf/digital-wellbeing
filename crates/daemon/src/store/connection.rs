use std::error::Error;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::Result;
use diesel::sqlite::SqliteConnection;
use diesel::{Connection, RunQueryDsl, sql_query};
use diesel_async::pooled_connection::deadpool::{Object, Pool};
use diesel_async::pooled_connection::{
    AsyncDieselConnectionManager, ManagerConfig, RecyclingMethod,
};
use diesel_async::sync_connection_wrapper::{SpawnBlocking, SyncConnectionWrapper};
use futures::future::{BoxFuture, FutureExt};

/// A [`SpawnBlocking`] wrapper that stores an explicit [`tokio::runtime::Handle`]
/// passed from the creator, rather than calling [`tokio::runtime::Handle::try_current`].
///
/// Unlike [`diesel_async::sync_connection_wrapper::implementation::Tokio`],
/// this type NEVER creates a nested `tokio::runtime::Runtime`. This prevents a
/// panic during runtime shutdown where dropping a `SyncConnectionWrapper` on a
/// worker thread would drop the nested Runtime's `BlockingPool` from an
/// async context.
///
/// It also avoids `Handle::try_current` which can fail on threads that aren't
/// tokio worker threads (e.g., `zbus::Connection executor` threads).
///
/// Use [`SyncConnectionWrapper::with_runtime`] instead of [`SyncConnectionWrapper::new`]
/// to provide the handle explicitly.
pub struct StoredHandle(tokio::runtime::Handle);

impl SpawnBlocking for StoredHandle {
    fn spawn_blocking<'a, R>(
        &mut self,
        task: impl FnOnce() -> R + Send + 'static,
    ) -> BoxFuture<'a, Result<R, Box<dyn Error + Send + Sync + 'static>>>
    where
        R: Send + 'static,
    {
        let handle = self.0.clone();
        let join = handle.spawn_blocking(task);
        WaitOnDrop { handle, join }.boxed()
    }

    /// Panics — use [`SyncConnectionWrapper::with_runtime`] instead.
    fn get_runtime() -> Self {
        panic!(
            "StoredHandle::get_runtime() should not be called. \
             Use SyncConnectionWrapper::with_runtime() with an explicit handle instead."
        )
    }
}

// ── WaitOnDrop future wrapper ─────────────────────────────────────────────
//
// diesel-async's SyncConnectionWrapper::spawn_blocking (line 379 of mod.rs)
// CLONES the `Arc<Mutex<C>>` and sends the clone into the blocking task.
// If the returned future is dropped before the blocking task completes
// (e.g., the D-Bus handler gets cancelled), the Clone keeps the Arc refcount
// elevated for the task's duration.
//
// When the connection is later returned to the pool, deadpool calls
// `is_broken()` → `exclusive_connection()` → `Arc::get_mut()` which requires
// refcount == 1. If the blocking task is still running with its clone,
// `Arc::get_mut` returns None and **panics** with "Cannot access shared
// transaction state".
//
// This wrapper waits for the spawned blocking task to complete on Drop,
// mirroring diesel-async's own `CancelWrapper` for its built-in `Tokio`
// spawn-blocking type. Without this, query cancellation from zbus executor
// threads causes a cascading panic + task cancellation storm.

struct WaitOnDrop<R: Send + 'static> {
    handle: tokio::runtime::Handle,
    join: tokio::task::JoinHandle<R>,
}

impl<R: Send + 'static> Future for WaitOnDrop<R> {
    type Output = Result<R, Box<dyn Error + Send + Sync + 'static>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        Pin::new(&mut this.join)
            .poll(cx)
            .map(|r| r.map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync + 'static>))
    }
}

impl<R: Send + 'static> Drop for WaitOnDrop<R> {
    fn drop(&mut self) {
        if !self.join.is_finished() {
            // The blocking task is still running with a clone of the Arc.
            // Wait for it to complete before the connection is recycled.
            tokio::task::block_in_place(|| {
                let _ = self.handle.block_on(&mut self.join);
            });
        }
    }
}

pub type SqliteConn = SyncConnectionWrapper<SqliteConnection, StoredHandle>;
pub type DbConn = Object<SqliteConn>;

#[derive(Clone)]
pub struct DbPool {
    inner: Pool<SqliteConn>,
}

impl DbPool {
    pub async fn get(&self) -> Result<DbConn> {
        self.inner
            .get()
            .await
            .map_err(|e| anyhow::anyhow!("pool error: {}", e))
    }
}

pub struct StoreBuilder {
    db_path: PathBuf,
}

impl StoreBuilder {
    pub fn new(db_path: PathBuf) -> Self {
        Self { db_path }
    }

    pub async fn build(self) -> Result<DbPool> {
        let db_path_str = self.db_path.to_string_lossy().to_string();

        // Run migrations synchronously first (diesel_migrations requires sync conn)
        {
            let mut conn = SqliteConnection::establish(&db_path_str)?;
            sql_query("PRAGMA journal_mode=WAL;").execute(&mut conn)?;
            sql_query("PRAGMA synchronous=NORMAL;").execute(&mut conn)?;
            sql_query("PRAGMA busy_timeout = 5000;").execute(&mut conn)?;
            crate::store::migrations::run_migrations(&mut conn)?;
        }

        let mut config = ManagerConfig::default();
        config.recycling_method = RecyclingMethod::Fast;

        // Capture the tokio handle while we KNOW we are inside the runtime.
        let handle = tokio::runtime::Handle::current();

        config.custom_setup = Box::new(move |url| {
            let url = url.to_string();
            let handle = handle.clone();
            async move {
                let mut conn = SqliteConnection::establish(&url)?;
                sql_query("PRAGMA journal_mode=WAL;")
                    .execute(&mut conn)
                    .map_err(diesel::ConnectionError::CouldntSetupConfiguration)?;
                sql_query("PRAGMA synchronous=NORMAL;")
                    .execute(&mut conn)
                    .map_err(diesel::ConnectionError::CouldntSetupConfiguration)?;
                sql_query("PRAGMA busy_timeout = 5000;")
                    .execute(&mut conn)
                    .map_err(diesel::ConnectionError::CouldntSetupConfiguration)?;
                Ok(SqliteConn::with_runtime(conn, StoredHandle(handle)))
            }
            .boxed()
        });
        let mgr = AsyncDieselConnectionManager::<SqliteConn>::new_with_config(
            db_path_str.clone(),
            config,
        );
        let pool = Pool::builder(mgr)
            .max_size(4)
            .build()
            .map_err(|e| anyhow::anyhow!("failed to build pool: {}", e))?;

        Ok(DbPool { inner: pool })
    }
}
