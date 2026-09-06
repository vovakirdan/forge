//! Test-only schema ownership. Production Core still performs validation only.

use anyhow::{Context, Result};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

pub(super) async fn isolated_pool(database_url: &str) -> Result<PgPool> {
    // The identifier is entirely generated, never interpolated from a user or
    // provider. Each Core fixture gets its own queue, boot ledger and snapshots.
    let schema = format!("forge_test_{}", Uuid::now_v7().simple());
    let bootstrap = PgPoolOptions::new()
        .max_connections(1)
        .connect(database_url)
        .await
        .context("connect PostgreSQL for isolated test schema")?;
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&bootstrap)
        .await
        .with_context(|| format!("create test-owned PostgreSQL schema {schema}"))?;
    bootstrap.close().await;
    let connection_schema = schema.clone();
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .after_connect(move |connection, _metadata| {
            let schema = connection_schema.clone();
            Box::pin(async move {
                // No public fallback: SQLx must create a private migration
                // ledger instead of finding the shared development tables.
                sqlx::query("SELECT set_config('search_path', $1, false)")
                    .bind(schema)
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(database_url)
        .await
        .context("connect isolated Forge test pool")?;
    forge_storage::apply_migrations_and_validate(&pool)
        .await
        .with_context(|| format!("migrate isolated test schema {schema}"))?;
    // Retain successful and failed fixture schemas for explicit diagnosis. No
    // drop-on-failure, cascading deletion, or mutation of old shared data.
    Ok(pool)
}
