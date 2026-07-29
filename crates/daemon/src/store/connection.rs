//! Turso-backed database connection layer.
//!
//! Turso-native async connection layer. `DbConn` is turso's `Connection`
//! (has query/execute/tx). `DbPool` holds the `Database` factory and
//! produces fresh connections. All access is fully async — no blocking I/O.

use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use turso::Connection;

pub type DbConn = Connection;

#[derive(Clone)]
pub struct DbPool {
    db: turso::Database,
}

impl DbPool {
    /// Get a fresh connection from the database handle.
    pub async fn client(&self) -> Result<DbConn> {
        self.db
            .connect()
            .map_err(|e| anyhow::anyhow!("connection error: {}", e))
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
        let db_path = self.db_path.to_string_lossy().to_string();
        let db = turso::Builder::new_local(&db_path)
            .experimental_generated_columns(true)
            .experimental_multiprocess_wal(true)
            .experimental_vacuum(false)
            .experimental_mvcc_passive_checkpoint(true)
            .build()
            .await
            .context("failed to create turso database")?;

        let pool = DbPool { db };
        let conn = pool.client().await?;

        // Run migrations
        crate::store::migrations::run_migrations(&conn).await?;

        Ok(pool)
    }
}
