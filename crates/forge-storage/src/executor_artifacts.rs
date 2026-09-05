//! Durable executor ArtifactSubmission receipt-to-Artifact persistence.

use forge_domain::{Artifact, ArtifactProducer, StageId, TaskId};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::{
    ArtifactLocation, StorageError, StorageTransaction, StoredArtifact, artifact_body_columns,
    database_timestamp, encode_object, encode_snapshot, enum_text, u64_to_i64,
};

/// Exact Run fence and environment epoch that scoped an executor receipt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutorArtifactRunScope {
    run_id: Uuid,
    lease_fencing_token: u64,
    environment_epoch: u64,
}

impl ExecutorArtifactRunScope {
    /// Creates a non-empty, positive Run receipt scope.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidInput`] when an identity is nil or a
    /// fencing field is zero.
    pub fn new(
        run_id: Uuid,
        lease_fencing_token: u64,
        environment_epoch: u64,
    ) -> Result<Self, StorageError> {
        if run_id.is_nil() {
            return Err(StorageError::InvalidInput {
                reason: "executor artifact run_id must not be nil".to_owned(),
            });
        }
        if lease_fencing_token == 0 {
            return Err(StorageError::InvalidInput {
                reason: "executor artifact lease_fencing_token must be positive".to_owned(),
            });
        }
        if environment_epoch == 0 {
            return Err(StorageError::InvalidInput {
                reason: "executor artifact environment_epoch must be positive".to_owned(),
            });
        }
        Ok(Self {
            run_id,
            lease_fencing_token,
            environment_epoch,
        })
    }

    /// Returns the scoped Run identity.
    #[must_use]
    pub fn run_id(self) -> Uuid {
        self.run_id
    }

    /// Returns the Lease fencing token copied into the Run.
    #[must_use]
    pub fn lease_fencing_token(self) -> u64 {
        self.lease_fencing_token
    }

    /// Returns the Run environment epoch.
    #[must_use]
    pub fn environment_epoch(self) -> u64 {
        self.environment_epoch
    }
}

/// One immutable ExecutorSubmission transport receipt used to submit an Artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutorArtifactReceipt {
    message_id: Uuid,
    scope: ExecutorArtifactRunScope,
}

impl ExecutorArtifactReceipt {
    /// Creates a receipt bound to one exact Run fence and environment epoch.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidInput`] when the message identity is nil
    /// or the supplied Run scope is invalid.
    pub fn new(
        message_id: Uuid,
        run_id: Uuid,
        lease_fencing_token: u64,
        environment_epoch: u64,
    ) -> Result<Self, StorageError> {
        if message_id.is_nil() {
            return Err(StorageError::InvalidInput {
                reason: "executor artifact message_id must not be nil".to_owned(),
            });
        }
        Ok(Self {
            message_id,
            scope: ExecutorArtifactRunScope::new(run_id, lease_fencing_token, environment_epoch)?,
        })
    }

    /// Returns the globally unique ExecutorSubmission transport identity.
    #[must_use]
    pub fn message_id(self) -> Uuid {
        self.message_id
    }

    /// Returns the exact Run scope carried by this receipt.
    #[must_use]
    pub fn scope(self) -> ExecutorArtifactRunScope {
        self.scope
    }
}

/// Typed executor evidence write that cannot silently lose its transport receipt.
#[derive(Clone, Debug)]
pub struct ExecutorArtifactWrite {
    location: ArtifactLocation,
    receipt: ExecutorArtifactReceipt,
}

impl ExecutorArtifactWrite {
    /// Combines an EmployeeRun Artifact location with its exact receipt scope.
    ///
    /// The location must name the same Run, Task, and stage; the SQL write
    /// rechecks that tuple against the current active Run Lease before it
    /// inserts.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidInput`] if the location is not a complete
    /// `EmployeeRun` context for the receipt's Run.
    pub fn new(
        location: ArtifactLocation,
        receipt: ExecutorArtifactReceipt,
    ) -> Result<Self, StorageError> {
        if location.producer != ArtifactProducer::EmployeeRun {
            return Err(StorageError::InvalidInput {
                reason: "executor artifact location must use employee_run producer".to_owned(),
            });
        }
        if location.run_id != Some(receipt.scope().run_id()) {
            return Err(StorageError::InvalidInput {
                reason: "executor artifact location run_id must match receipt run_id".to_owned(),
            });
        }
        if location.task_id.is_none() {
            return Err(StorageError::InvalidInput {
                reason: "executor artifact location requires a task_id".to_owned(),
            });
        }
        if location.stage_id.is_none() {
            return Err(StorageError::InvalidInput {
                reason: "executor artifact location requires a stage_id".to_owned(),
            });
        }
        Ok(Self { location, receipt })
    }

    /// Returns the immutable evidence location that will be persisted.
    #[must_use]
    pub fn location(&self) -> &ArtifactLocation {
        &self.location
    }

    /// Returns the receipt that will be mapped to the inserted Artifact.
    #[must_use]
    pub fn receipt(&self) -> ExecutorArtifactReceipt {
        self.receipt
    }
}

/// A canonical Artifact resolved from one requested ExecutorSubmission receipt.
#[derive(Clone, Debug)]
pub struct ExecutorArtifactMapping {
    /// Requested transport receipt identity.
    pub message_id: Uuid,
    /// Canonical immutable Artifact and its durable location.
    pub artifact: StoredArtifact,
}

impl StorageTransaction<'_> {
    /// Inserts immutable evidence without an executor transport receipt.
    ///
    /// Core uses [`Self::insert_executor_artifact`] for every executor-originated
    /// ArtifactSubmission so an Artifact and its receipt map commit together.
    pub async fn insert_artifact(
        &mut self,
        artifact: &Artifact,
        location: &ArtifactLocation,
    ) -> Result<(), StorageError> {
        let record = ArtifactInsert::new(artifact, location)?;
        insert_artifact_row(&mut self.transaction, record).await
    }

    /// Atomically persists one executor Artifact and its unique receipt mapping.
    ///
    /// The receipt reservation is inserted first with a deferred Artifact FK,
    /// so a duplicate message id cannot leave a second Artifact in the current
    /// transaction. The database verifies the exact active Run fence, epoch,
    /// Project, Task, stage, and active Lease before either record can commit.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::ExecutorArtifactReceiptAlreadyMapped`] if the
    /// message id was previously accepted, or a stale-revision error when the
    /// Run scope no longer matches the active Run.
    pub async fn insert_executor_artifact(
        &mut self,
        artifact: &Artifact,
        write: &ExecutorArtifactWrite,
    ) -> Result<(), StorageError> {
        let record = ArtifactInsert::new(artifact, write.location())?;
        reserve_executor_artifact_receipt(&mut self.transaction, artifact, write).await?;
        insert_artifact_row(&mut self.transaction, record).await
    }

    /// Resolves requested receipt ids in their caller-provided order.
    ///
    /// Only mappings in `scope` are returned. Missing receipt ids are omitted;
    /// Core must compare the returned ids with its requested set before it cites
    /// artifacts in a StageOutcomeSubmission.
    pub async fn lookup_executor_artifacts(
        &mut self,
        scope: ExecutorArtifactRunScope,
        message_ids: &[Uuid],
    ) -> Result<Vec<ExecutorArtifactMapping>, StorageError> {
        if message_ids.is_empty() {
            return Ok(Vec::new());
        }
        if message_ids.iter().any(Uuid::is_nil) {
            return Err(StorageError::InvalidInput {
                reason: "executor artifact lookup message_id must not be nil".to_owned(),
            });
        }
        let rows = sqlx::query(
            "SELECT requested.message_id AS receipt_message_id, a.canonical_snapshot::text AS canonical_snapshot, a.task_id, a.run_id, a.stage_id, a.producer_type, a.producer_id, a.producer::text AS producer FROM unnest($1::uuid[]) WITH ORDINALITY AS requested(message_id, ordinal) JOIN executor_artifact_receipts receipt ON receipt.message_id = requested.message_id AND receipt.run_id = $2 AND receipt.lease_fencing_token = $3 AND receipt.environment_epoch = $4 JOIN artifacts a ON a.id = receipt.artifact_id ORDER BY requested.ordinal ASC",
        )
        .bind(message_ids.to_vec())
        .bind(scope.run_id())
        .bind(u64_to_i64(
            scope.lease_fencing_token(),
            "executor_artifact_receipt.lease_fencing_token",
        )?)
        .bind(u64_to_i64(
            scope.environment_epoch(),
            "executor_artifact_receipt.environment_epoch",
        )?)
        .fetch_all(&mut *self.transaction)
        .await?;

        rows.into_iter()
            .map(|row| {
                let message_id = row.try_get("receipt_message_id")?;
                let artifact = crate::store::stored_artifact_from_row(row)?;
                Ok(ExecutorArtifactMapping {
                    message_id,
                    artifact,
                })
            })
            .collect()
    }
}

struct ArtifactInsert<'a> {
    artifact: &'a Artifact,
    location: &'a ArtifactLocation,
    body_storage: String,
    body_json: Option<String>,
    object_ref: Option<String>,
    producer: String,
    producer_data: String,
    metadata: String,
    snapshot: String,
}

impl<'a> ArtifactInsert<'a> {
    fn new(artifact: &'a Artifact, location: &'a ArtifactLocation) -> Result<Self, StorageError> {
        let (body_storage, body_json, object_ref) = artifact_body_columns(artifact)?;
        Ok(Self {
            artifact,
            location,
            body_storage,
            body_json,
            object_ref,
            producer: enum_text(&location.producer, "artifact.producer_type")?,
            producer_data: encode_object(&location.producer_data, "artifact.producer")?,
            metadata: encode_object(artifact.metadata(), "artifact.metadata")?,
            snapshot: encode_snapshot(artifact, "artifact")?,
        })
    }
}

async fn reserve_executor_artifact_receipt(
    transaction: &mut Transaction<'_, Postgres>,
    artifact: &Artifact,
    write: &ExecutorArtifactWrite,
) -> Result<(), StorageError> {
    let task_id = required_task_id(write.location())?;
    let stage_id = required_stage_id(write.location())?;
    let receipt = write.receipt();
    let scope = receipt.scope();
    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO executor_artifact_receipts (message_id, run_id, lease_fencing_token, environment_epoch, artifact_id) SELECT $1, r.id, $3, $4, $5 FROM runs AS r JOIN leases AS l ON l.id = r.lease_id AND l.project_id = r.project_id AND l.task_id = r.task_id AND l.employee_id = r.employee_id AND l.fencing_token = r.lease_fencing_token AND l.environment_epoch = r.environment_epoch AND l.lease_state = 'active' WHERE r.id = $2 AND r.lease_fencing_token = $3 AND r.environment_epoch = $4 AND r.project_id = $6 AND r.task_id = $7 AND r.stage_id = $8 ON CONFLICT (message_id) DO NOTHING RETURNING message_id",
    )
    .bind(receipt.message_id())
    .bind(scope.run_id())
    .bind(u64_to_i64(
        scope.lease_fencing_token(),
        "executor_artifact_receipt.lease_fencing_token",
    )?)
    .bind(u64_to_i64(
        scope.environment_epoch(),
        "executor_artifact_receipt.environment_epoch",
    )?)
    .bind(artifact.id().as_uuid())
    .bind(artifact.project_id().as_uuid())
    .bind(task_id.as_uuid())
    .bind(stage_id.as_str())
    .fetch_optional(&mut **transaction)
    .await?;
    if inserted.is_some() {
        return Ok(());
    }

    let known_receipt: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM executor_artifact_receipts WHERE message_id = $1)",
    )
    .bind(receipt.message_id())
    .fetch_one(&mut **transaction)
    .await?;
    if known_receipt {
        Err(StorageError::ExecutorArtifactReceiptAlreadyMapped)
    } else {
        Err(StorageError::StaleRevision {
            aggregate: "run artifact receipt fence",
        })
    }
}

async fn insert_artifact_row(
    transaction: &mut Transaction<'_, Postgres>,
    record: ArtifactInsert<'_>,
) -> Result<(), StorageError> {
    let result = sqlx::query(
        "INSERT INTO artifacts (id, project_id, task_id, run_id, stage_id, kind, title, producer_type, producer_id, producer, metadata, body_storage, body_json, object_ref, canonical_snapshot, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::jsonb, $11::jsonb, $12, $13::jsonb, $14::jsonb, $15::jsonb, $16)",
    )
    .bind(record.artifact.id().as_uuid())
    .bind(record.artifact.project_id().as_uuid())
    .bind(record.location.task_id.map(|id| id.as_uuid()))
    .bind(record.location.run_id)
    .bind(record.location.stage_id.as_ref().map(|id| id.as_str()))
    .bind(record.artifact.kind().as_str())
    .bind(record.artifact.title())
    .bind(record.producer)
    .bind(record.location.producer_id)
    .bind(record.producer_data)
    .bind(record.metadata)
    .bind(record.body_storage)
    .bind(record.body_json)
    .bind(record.object_ref)
    .bind(record.snapshot)
    .bind(database_timestamp(record.artifact.created_at()))
    .execute(&mut **transaction)
    .await?;
    if result.rows_affected() == 1 {
        Ok(())
    } else {
        Err(StorageError::StaleRevision {
            aggregate: "artifact",
        })
    }
}

fn required_task_id(location: &ArtifactLocation) -> Result<TaskId, StorageError> {
    location.task_id.ok_or_else(|| StorageError::InvalidInput {
        reason: "executor artifact location requires a task_id".to_owned(),
    })
}

fn required_stage_id(location: &ArtifactLocation) -> Result<&StageId, StorageError> {
    location
        .stage_id
        .as_ref()
        .ok_or_else(|| StorageError::InvalidInput {
            reason: "executor artifact location requires a stage_id".to_owned(),
        })
}

#[cfg(test)]
mod tests {
    use forge_domain::{ArtifactProducer, StageId, TaskId};
    use serde_json::json;
    use uuid::Uuid;

    use crate::{ArtifactLocation, StorageError};

    use super::{ExecutorArtifactReceipt, ExecutorArtifactRunScope, ExecutorArtifactWrite};

    fn employee_run_location(run_id: Uuid) -> ArtifactLocation {
        ArtifactLocation {
            task_id: Some(TaskId::new()),
            run_id: Some(run_id),
            stage_id: Some(StageId::new("work").expect("stage id")),
            producer: ArtifactProducer::EmployeeRun,
            producer_id: None,
            producer_data: json!({}),
        }
    }

    #[test]
    fn run_scope_rejects_zero_fencing_token() {
        let result = ExecutorArtifactRunScope::new(Uuid::now_v7(), 0, 1);

        assert!(matches!(result, Err(StorageError::InvalidInput { .. })));
    }

    #[test]
    fn receipt_rejects_nil_message_id() {
        let result = ExecutorArtifactReceipt::new(Uuid::nil(), Uuid::now_v7(), 1, 1);

        assert!(matches!(result, Err(StorageError::InvalidInput { .. })));
    }

    #[test]
    fn executor_write_rejects_non_employee_run_location() {
        let run_id = Uuid::now_v7();
        let mut location = employee_run_location(run_id);
        location.producer = ArtifactProducer::Human;
        let receipt = ExecutorArtifactReceipt::new(Uuid::now_v7(), run_id, 1, 1).expect("receipt");

        let result = ExecutorArtifactWrite::new(location, receipt);

        assert!(matches!(result, Err(StorageError::InvalidInput { .. })));
    }

    #[test]
    fn executor_write_rejects_a_different_run_than_its_receipt() {
        let location = employee_run_location(Uuid::now_v7());
        let receipt =
            ExecutorArtifactReceipt::new(Uuid::now_v7(), Uuid::now_v7(), 1, 1).expect("receipt");

        let result = ExecutorArtifactWrite::new(location, receipt);

        assert!(matches!(result, Err(StorageError::InvalidInput { .. })));
    }
}
