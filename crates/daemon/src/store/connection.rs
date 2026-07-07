use anyhow::Result;
use diesel::sqlite::SqliteConnection;
use diesel::{Connection, sql_query};
use diesel_async::pooled_connection::deadpool::{Pool, Object};
use diesel_async::pooled_connection::AsyncDieselConnectionManager;
use diesel_async::AsyncSqliteConnection;
use std::path::PathBuf;

/// Type alias for a pooled async SQLite connection.
pub type DbConn = Object<AsyncSqliteConnection>;

/// Thread-safe connection pool to the daemon's SQLite database.
#[derive(Clone)]
pub struct DbPool {
    inner: Pool<AsyncSqliteConnection>,
}

impl DbPool {
    pub async fn get(&self) -> Result<DbConn> {
        self.inner.get().await.map_err(|e| anyhow::anyhow!("pool error: {}", e))
    }
}

/// Builder for DbPool. Runs migrations before handing out connections.
pub struct StoreBuilder {
    db_path: PathBuf,
}

impl StoreBuilder {
    pub fn new(db_path: PathBuf) -> Self {
        Self { db_path }
    }

    /// Connect, run pending migrations, return ready-to-use pool.
    pub async fn build(self) -> Result<DbPool> {
        let db_path_str = self.db_path.to_string_lossy().to_string();

        // Run migrations synchronously first (diesel_migrations requires sync conn)
        {
            let mut conn = SqliteConnection::establish(&db_path_str)?;
            sql_query("PRAGMA journal_mode=WAL;").execute(&mut conn)?;
            sql_query("PRAGMA synchronous=NORMAL;").execute(&mut conn)?;
            crate::store::migrations::run_migrations(&mut conn)?;
        }

        // Build async pool via AsyncDieselConnectionManager
        let mgr = AsyncDieselConnectionManager::<AsyncSqliteConnection>::new(db_path_str);
        let pool = Pool::builder(mgr)
            .max_size(4)
            .build()
            .map_err(|e| anyhow::anyhow!("failed to build pool: {}", e))?;

        Ok(DbPool { inner: pool })
    }
}
