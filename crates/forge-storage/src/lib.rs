//! PostgreSQL schema migration support for Forge canonical storage.
//!
//! The runtime never applies migrations implicitly. Operators invoke the
//! `forge-migrate` binary before starting a Core that requires a newer schema.

#![forbid(unsafe_code)]

mod credentials;
mod error;
pub mod evidence;
mod executor_artifacts;
mod gateway;
mod health;
mod model;
mod outbox;
mod proxy_keys;
mod read;
mod recovery;
mod run_control;
mod run_evidence;
mod run_lifecycle;
mod runtime;
mod scheduler;
mod store;
mod task_write;
mod write;

use sqlx::{PgPool, migrate::Migrator};
use thiserror::Error;

pub use credentials::StoredCredential;
pub use error::StorageError;
pub use executor_artifacts::{
    ExecutorArtifactMapping, ExecutorArtifactReceipt, ExecutorArtifactRunScope,
    ExecutorArtifactWrite,
};
pub use gateway::{GatewaySubmissionRecord, GatewayWriteResult};
pub use health::OperationalSnapshot;
pub use model::{
    ArtifactLocation, IdempotencyRecord, QueueEntry, QueueEntryInput, QueueState, RunDesiredState,
    RunObservedState, RunProjection, StoredArtifact, StoredEmployee, StoredEvent, StoredTask,
    TaskPersistence,
};
pub use outbox::{OutboxClaimRequest, OutboxFailureMark, OutboxLease, OutboxPublishMark};
pub use proxy_keys::StoredProxyKey;
pub use recovery::RunRecoveryState;
pub use run_lifecycle::{ExecutorSubmissionRecord, FencedWrite, ObservedRunUpdate};
pub use runtime::{EnvironmentReport, IncidentKind};
pub use scheduler::{LeaseRunRequest, ProvisionedRun};
pub use store::{PostgresStore, StorageTransaction};

pub(crate) use model::{
    artifact_body_columns, database_timestamp, decode_snapshot, domain_timestamp, encode_object,
    encode_snapshot, encode_value, enum_text, i32_to_u32, i64_to_u64, u32_to_i32, u64_to_i64,
    value_object,
};
pub(crate) use store::{queue_entry_from_row, run_projection_from_row};

/// The schema migrations embedded in this release of Forge.
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Returns when the database contains the canonical M0 tables and indexes.
///
/// This deliberately performs validation only. Applying migrations remains an
/// explicit operator action through [`apply_migrations_and_validate`].
///
/// # Errors
///
/// Returns [`MigrationError`] when a required migration, table, column, or
/// index is absent, or PostgreSQL cannot be queried.
pub async fn validate_schema(pool: &PgPool) -> Result<(), MigrationError> {
    validate_migration_ledger(pool).await?;

    for table in REQUIRED_TABLES {
        let exists: bool = sqlx::query_scalar(
            "\
            SELECT EXISTS (
                SELECT 1
                FROM information_schema.tables
                WHERE table_schema = current_schema()
                  AND table_name = $1
            )",
        )
        .bind(table.name)
        .fetch_one(pool)
        .await?;

        if !exists {
            return Err(MigrationError::MissingTable { table: table.name });
        }
    }

    for column in REQUIRED_COLUMNS {
        let exists: bool = sqlx::query_scalar(
            "\
            SELECT EXISTS (
                SELECT 1
                FROM information_schema.columns
                WHERE table_schema = current_schema()
                  AND table_name = $1
                  AND column_name = $2
            )",
        )
        .bind(column.table)
        .bind(column.name)
        .fetch_one(pool)
        .await?;

        if !exists {
            return Err(MigrationError::MissingColumn {
                table: column.table,
                column: column.name,
            });
        }
    }

    for index in REQUIRED_INDEXES {
        let exists: bool = sqlx::query_scalar(
            "\
            SELECT EXISTS (
                SELECT 1
                FROM pg_catalog.pg_indexes
                WHERE schemaname = current_schema()
                  AND indexname = $1
            )",
        )
        .bind(index.name)
        .fetch_one(pool)
        .await?;

        if !exists {
            return Err(MigrationError::MissingIndex { index: index.name });
        }
    }

    Ok(())
}

/// Explicitly applies the embedded migrations and then validates their shape.
///
/// Core must call [`validate_schema`] at startup instead of this function so a
/// normal process start can never modify canonical state.
///
/// # Errors
///
/// Returns [`MigrationError`] when SQLx cannot apply a migration or the
/// resulting schema does not meet Forge's storage contract.
pub async fn apply_migrations_and_validate(pool: &PgPool) -> Result<(), MigrationError> {
    MIGRATOR.run(pool).await?;
    validate_schema(pool).await
}

/// Errors returned while applying or validating the canonical schema.
#[derive(Debug, Error)]
pub enum MigrationError {
    /// SQLx could not apply or verify a migration.
    #[error("failed to apply or verify a Forge migration")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    /// PostgreSQL could not be inspected while validating the schema.
    #[error("failed to inspect the Forge PostgreSQL schema")]
    Database(#[from] sqlx::Error),

    /// The migration ledger does not contain the M0 canonical schema version.
    #[error("required Forge migration version {version} has not been applied")]
    MissingMigration { version: i64 },

    /// A canonical table is absent.
    #[error("required Forge table `{table}` is missing")]
    MissingTable {
        /// Canonical table name.
        table: &'static str,
    },

    /// A canonical column is absent.
    #[error("required Forge column `{table}.{column}` is missing")]
    MissingColumn {
        /// Canonical table name.
        table: &'static str,
        /// Canonical column name.
        column: &'static str,
    },

    /// A scheduler or outbox index is absent.
    #[error("required Forge index `{index}` is missing")]
    MissingIndex {
        /// Canonical index name.
        index: &'static str,
    },
}

const M0_CANONICAL_SCHEMA_VERSION: i64 = 14;

struct NamedContractItem {
    name: &'static str,
}

struct ColumnContractItem {
    table: &'static str,
    name: &'static str,
}

const REQUIRED_TABLES: &[NamedContractItem] = &[
    NamedContractItem { name: "projects" },
    NamedContractItem { name: "employees" },
    NamedContractItem { name: "pipelines" },
    NamedContractItem {
        name: "pipeline_versions",
    },
    NamedContractItem { name: "tasks" },
    NamedContractItem {
        name: "task_dependencies",
    },
    NamedContractItem {
        name: "queue_entries",
    },
    NamedContractItem { name: "leases" },
    NamedContractItem { name: "runs" },
    NamedContractItem { name: "artifacts" },
    NamedContractItem { name: "event_log" },
    NamedContractItem { name: "outbox" },
    NamedContractItem {
        name: "idempotency_keys",
    },
    NamedContractItem {
        name: "consumer_dedupe",
    },
    NamedContractItem {
        name: "executor_artifact_receipts",
    },
];

const REQUIRED_COLUMNS: &[ColumnContractItem] = &[
    ColumnContractItem {
        table: "projects",
        name: "revision",
    },
    ColumnContractItem {
        table: "projects",
        name: "next_task_sequence",
    },
    ColumnContractItem {
        table: "projects",
        name: "canonical_snapshot",
    },
    ColumnContractItem {
        table: "pipeline_versions",
        name: "definition",
    },
    ColumnContractItem {
        table: "tasks",
        name: "task_key",
    },
    ColumnContractItem {
        table: "tasks",
        name: "pipeline_id",
    },
    ColumnContractItem {
        table: "tasks",
        name: "pipeline_version_id",
    },
    ColumnContractItem {
        table: "tasks",
        name: "current_stage_id",
    },
    ColumnContractItem {
        table: "tasks",
        name: "lifecycle",
    },
    ColumnContractItem {
        table: "tasks",
        name: "revision",
    },
    ColumnContractItem {
        table: "tasks",
        name: "resume_to_lifecycle",
    },
    ColumnContractItem {
        table: "tasks",
        name: "canonical_snapshot",
    },
    ColumnContractItem {
        table: "artifacts",
        name: "body_json",
    },
    ColumnContractItem {
        table: "artifacts",
        name: "producer",
    },
    ColumnContractItem {
        table: "artifacts",
        name: "canonical_snapshot",
    },
    ColumnContractItem {
        table: "runs",
        name: "employee_id",
    },
    ColumnContractItem {
        table: "runs",
        name: "lease_fencing_token",
    },
    ColumnContractItem {
        table: "runs",
        name: "environment_epoch",
    },
    ColumnContractItem {
        table: "runs",
        name: "last_sequence",
    },
    ColumnContractItem {
        table: "runs",
        name: "desired_state",
    },
    ColumnContractItem {
        table: "runs",
        name: "observed_state",
    },
    ColumnContractItem {
        table: "event_log",
        name: "reason",
    },
    ColumnContractItem {
        table: "executor_artifact_receipts",
        name: "message_id",
    },
    ColumnContractItem {
        table: "executor_artifact_receipts",
        name: "run_id",
    },
    ColumnContractItem {
        table: "executor_artifact_receipts",
        name: "lease_fencing_token",
    },
    ColumnContractItem {
        table: "executor_artifact_receipts",
        name: "environment_epoch",
    },
    ColumnContractItem {
        table: "executor_artifact_receipts",
        name: "artifact_id",
    },
];

const REQUIRED_INDEXES: &[NamedContractItem] = &[
    NamedContractItem {
        name: "queue_entries_runnable_order_idx",
    },
    NamedContractItem {
        name: "outbox_ready_idx",
    },
    NamedContractItem {
        name: "outbox_leased_reclaim_idx",
    },
    NamedContractItem {
        name: "leases_active_task_idx",
    },
    NamedContractItem {
        name: "executor_artifact_receipts_scope_message_idx",
    },
];

async fn validate_migration_ledger(pool: &PgPool) -> Result<(), MigrationError> {
    let ledger_exists: bool =
        sqlx::query_scalar("SELECT to_regclass('_sqlx_migrations') IS NOT NULL")
            .fetch_one(pool)
            .await?;

    if !ledger_exists {
        return Err(MigrationError::MissingMigration {
            version: M0_CANONICAL_SCHEMA_VERSION,
        });
    }

    let applied: bool = sqlx::query_scalar(
        "\
        SELECT EXISTS (
            SELECT 1
            FROM _sqlx_migrations
            WHERE version = $1
              AND success = TRUE
        )",
    )
    .bind(M0_CANONICAL_SCHEMA_VERSION)
    .fetch_one(pool)
    .await?;

    if applied {
        Ok(())
    } else {
        Err(MigrationError::MissingMigration {
            version: M0_CANONICAL_SCHEMA_VERSION,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{REQUIRED_COLUMNS, REQUIRED_INDEXES, REQUIRED_TABLES};
    use std::collections::BTreeSet;

    #[test]
    fn schema_contract_has_unique_required_table_names() {
        let names = REQUIRED_TABLES
            .iter()
            .map(|item| item.name)
            .collect::<BTreeSet<_>>();

        assert_eq!(names.len(), REQUIRED_TABLES.len());
    }

    #[test]
    fn schema_contract_names_scheduler_and_outbox_indexes() {
        let names = REQUIRED_INDEXES
            .iter()
            .map(|item| item.name)
            .collect::<BTreeSet<_>>();

        assert!(names.contains("queue_entries_runnable_order_idx"));
        assert!(names.contains("outbox_ready_idx"));
        assert!(names.contains("outbox_leased_reclaim_idx"));
        assert!(names.contains("executor_artifact_receipts_scope_message_idx"));
    }

    #[test]
    fn schema_contract_covers_task_artifact_and_run_columns() {
        let names = REQUIRED_COLUMNS
            .iter()
            .map(|item| (item.table, item.name))
            .collect::<BTreeSet<_>>();

        assert!(names.contains(&("tasks", "lifecycle")));
        assert!(names.contains(&("artifacts", "body_json")));
        assert!(names.contains(&("runs", "lease_fencing_token")));
        assert!(names.contains(&("executor_artifact_receipts", "environment_epoch")));
    }
}
