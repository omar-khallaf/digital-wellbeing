//! Raw SQL migration runner.
//!
//! Embeds migration SQL at compile time so migrations are always available
//! from the binary, without depending on runtime filesystem paths.
//!
//! Tracks applied migrations via a `__schema_migrations` table. On startup,
//! creates the tracking table if missing, then runs only unapplied migrations
//! in sorted order, each wrapped in a transaction.

use crate::store::connection::DbConn;

const MIGRATION_INITIAL_SCHEMA: &str =
    include_str!("../../../../migrations/2026-07-18-000001_initial_schema/up.sql");

#[derive(Debug, Clone, Copy)]
struct EmbeddedMigration {
    name: &'static str,
    sql: &'static str,
}

fn embedded_migrations() -> &'static [EmbeddedMigration] {
    &[EmbeddedMigration {
        name: "2026-07-18-000001_initial_schema",
        sql: MIGRATION_INITIAL_SCHEMA,
    }]
}

/// Run all pending embedded migrations. Called once at startup.
pub async fn run_migrations(conn: &DbConn) -> anyhow::Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS __schema_migrations (
            version    TEXT PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
        )",
        (),
    )
    .await
    .map_err(|e| anyhow::anyhow!("failed to create __schema_migrations table: {}", e))?;

    let last_applied: Option<String> = {
        let mut rows = conn
            .query(
                "SELECT version FROM __schema_migrations ORDER BY version DESC LIMIT 1",
                (),
            )
            .await
            .map_err(|e| anyhow::anyhow!("failed to query last migration: {}", e))?;
        match rows.next().await? {
            Some(row) => Some(row.get::<String>(0)?),
            None => None,
        }
    };

    let mut migrations: Vec<&EmbeddedMigration> = embedded_migrations().iter().collect();
    migrations.sort_by(|a, b| a.name.cmp(b.name));

    let pending: Vec<&&EmbeddedMigration> = match &last_applied {
        Some(last) => migrations
            .iter()
            .filter(|m| m.name > last.as_str())
            .collect(),
        None => migrations.iter().collect(),
    };

    if pending.is_empty() {
        tracing::info!(
            "migration runner: all {} embedded migrations already applied",
            migrations.len()
        );
        return Ok(());
    }

    for migration in pending {
        let statements: Vec<&str> = migration
            .sql
            .split(';')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        tracing::info!(
            "migration runner: applying {} ({} statements)",
            migration.name,
            statements.len()
        );

        conn.execute("BEGIN", ())
            .await
            .map_err(|e| anyhow::anyhow!("failed to BEGIN transaction: {}", e))?;

        if let Err(e) = apply_statements(conn, migration.name, &statements).await {
            tracing::error!(
                "migration runner: rollback due to error on {}: {}",
                migration.name,
                e
            );
            let _ = conn.execute("ROLLBACK", ()).await;
            return Err(e);
        }

        conn.execute("COMMIT", ())
            .await
            .map_err(|e| anyhow::anyhow!("failed to COMMIT migration {}: {}", migration.name, e))?;

        conn.execute(
            "INSERT INTO __schema_migrations (version) VALUES (?1)",
            (migration.name,),
        )
        .await
        .map_err(|e| anyhow::anyhow!("failed to record migration {}: {}", migration.name, e))?;

        tracing::info!("migration runner: {} applied successfully", migration.name);
    }

    tracing::info!("migration runner: all pending migrations completed successfully");
    Ok(())
}

async fn apply_statements(conn: &DbConn, name: &str, statements: &[&str]) -> anyhow::Result<()> {
    for (idx, statement) in statements.iter().enumerate() {
        tracing::debug!(
            "migration runner: executing statement {}/{} for {}",
            idx + 1,
            statements.len(),
            name,
        );
        conn.execute(statement, ()).await.map_err(|e| {
            anyhow::anyhow!(
                "migration failed on {} statement {}/{}: {}",
                name,
                idx + 1,
                statements.len(),
                e,
            )
        })?;
    }
    Ok(())
}
