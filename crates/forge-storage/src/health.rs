//! Read-only operational projections. No diagnostics query changes canonical state.

use crate::{PostgresStore, StorageError};
use sqlx::Row;

/// Bounded aggregate values only; no identifiers, payloads or provider messages.
#[derive(Clone, Copy, Debug, Default)]
pub struct OperationalSnapshot {
    pub queued: i64,
    pub active_leases: i64,
    pub active_runs: i64,
    pub unknown_environments: i64,
    pub open_incidents: i64,
    pub pending_outbox: i64,
    pub dead_letter_outbox: i64,
    pub oldest_outbox_seconds: f64,
}

impl PostgresStore {
    /// Local dependency probe; intentionally does not apply migrations.
    pub async fn healthcheck(&self) -> Result<(), StorageError> {
        let _: i32 = sqlx::query_scalar("SELECT 1").fetch_one(&self.pool).await?;
        Ok(())
    }

    /// Aggregates are collected on scrape, independently from command commits.
    pub async fn operational_snapshot(&self) -> Result<OperationalSnapshot, StorageError> {
        let row = sqlx::query(
            "SELECT
                (SELECT count(*) FROM queue_entries WHERE queue_state = 'queued') AS queued,
                (SELECT count(*) FROM leases WHERE lease_state = 'active') AS active_leases,
                (SELECT count(*) FROM runs WHERE observed_state IN ('provisioning','running','stopping')) AS active_runs,
                (SELECT count(*) FROM run_environment_reservations WHERE state = 'unknown' AND released_at IS NULL) AS unknown_environments,
                (SELECT count(*) FROM run_incidents WHERE resolved_at IS NULL) AS open_incidents,
                (SELECT count(*) FROM outbox WHERE delivery_state IN ('pending','leased')) AS pending_outbox,
                (SELECT count(*) FROM outbox WHERE delivery_state = 'dead_letter') AS dead_letter_outbox,
                (SELECT COALESCE(GREATEST(EXTRACT(EPOCH FROM clock_timestamp() - min(created_at)),0),0)::float8 FROM outbox WHERE delivery_state IN ('pending','leased')) AS oldest_outbox_seconds"
        ).fetch_one(&self.pool).await?;
        Ok(OperationalSnapshot {
            queued: row.try_get("queued")?,
            active_leases: row.try_get("active_leases")?,
            active_runs: row.try_get("active_runs")?,
            unknown_environments: row.try_get("unknown_environments")?,
            open_incidents: row.try_get("open_incidents")?,
            pending_outbox: row.try_get("pending_outbox")?,
            dead_letter_outbox: row.try_get("dead_letter_outbox")?,
            oldest_outbox_seconds: row.try_get("oldest_outbox_seconds")?,
        })
    }
}
