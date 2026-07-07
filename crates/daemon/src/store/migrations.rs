use anyhow::Result;
use diesel::sqlite::SqliteConnection;
use diesel_migrations::{embed_migrations, EmbeddedMigration, MigrationHarness};

/// Path relative to the daemon crate root → workspace root migrations/
pub const MIGRATIONS: EmbeddedMigration = embed_migrations!("../../migrations");

/// Run all pending migrations. Called once at startup.
pub fn run_migrations(conn: &mut SqliteConnection) -> Result<()> {
    conn.run_pending_migrations(MIGRATIONS)
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("migration failed: {}", e))?;
    Ok(())
}
