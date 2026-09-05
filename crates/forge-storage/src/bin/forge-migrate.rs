//! Explicit operator entry point for Forge PostgreSQL migrations.

#![forbid(unsafe_code)]

use anyhow::{Context, Result};
use clap::Parser;
use forge_storage::apply_migrations_and_validate;
use sqlx::postgres::PgPoolOptions;
use std::env;
use std::time::Duration;

/// Applies and validates the Forge canonical PostgreSQL schema.
#[derive(Debug, Parser)]
#[command(name = "forge-migrate", version, about)]
struct Arguments {
    /// PostgreSQL connection URL. Prefer `FORGE_DATABASE_URL` so process listings
    /// and Cargo command echoes cannot expose local credentials.
    #[arg(long)]
    database_url: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let arguments = Arguments::parse();
    let database_url = arguments
        .database_url
        .or_else(|| env::var("FORGE_DATABASE_URL").ok())
        .context("set FORGE_DATABASE_URL or pass --database-url")?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&database_url)
        .await
        .context("could not connect to PostgreSQL for Forge migration")?;

    apply_migrations_and_validate(&pool)
        .await
        .context("Forge migration or schema validation failed")?;
    pool.close().await;

    println!("Forge schema is up to date.");
    Ok(())
}
