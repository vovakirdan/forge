//! Retained runtime output -> scoped spool -> MinIO, never a Pipeline verdict.

use crate::{
    CoreError, CoreService,
    credentials::credential_error,
    event::{event, event_payload},
};
use forge_domain::{
    AggregateRef, CommandId, DomainEventKind, EvidenceScope, EvidenceStream, Timestamp,
    runtime::RecoveryAssessment,
};
use forge_provider_common::PrivateMaterialization;
use forge_storage::{
    IncidentKind,
    evidence::{
        EvidenceRedaction, EvidenceSpool, ObjectEvidenceStore, S3EvidenceConfig, SpoolLimits,
    },
};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;

pub(crate) struct EvidenceRuntime {
    pub spool: EvidenceSpool,
    pub object_store: Option<ObjectEvidenceStore>,
    pub serial: Mutex<()>,
}
impl EvidenceRuntime {
    pub fn new(
        root: &std::path::Path,
        object_store: Option<ObjectEvidenceStore>,
    ) -> Result<Self, CoreError> {
        Ok(Self {
            spool: EvidenceSpool::open(root, SpoolLimits::default())
                .map_err(|_| credential_error())?,
            object_store,
            serial: Mutex::new(()),
        })
    }
}

impl CoreService {
    /// Optional remote durable body storage. Without it, bounded local receipts
    /// remain explicitly PendingUpload, rather than pretending MinIO is healthy.
    pub fn with_evidence_store(mut self, config: S3EvidenceConfig) -> Result<Self, CoreError> {
        let object_store = ObjectEvidenceStore::s3(config).map_err(|_| credential_error())?;
        let runtime = Arc::get_mut(self.evidence.as_mut().ok_or_else(credential_error)?)
            .ok_or_else(credential_error)?;
        runtime.object_store = Some(object_store);
        Ok(self)
    }

    /// Reads an explicitly selected private JSON configuration, without logging it.
    pub fn with_evidence_config_file(self, path: &std::path::Path) -> Result<Self, CoreError> {
        let bytes = PrivateMaterialization::open(path)
            .and_then(|file| file.read(16 * 1024))
            .map_err(|_| credential_error())?;
        let config: S3EvidenceConfig =
            serde_json::from_slice(bytes.expose()).map_err(|_| credential_error())?;
        self.with_evidence_store(config)
    }

    pub async fn evidence_tick(&self) -> Result<usize, CoreError> {
        self.collect_evidence(None).await
    }

    /// Bounded operator/test collection for one already quiescent Run.
    pub async fn collect_run_evidence(&self, run_id: uuid::Uuid) -> Result<usize, CoreError> {
        self.collect_evidence(Some(run_id)).await
    }

    async fn collect_evidence(&self, only_run: Option<uuid::Uuid>) -> Result<usize, CoreError> {
        let (Some(runtime), Some(execution)) = (&self.evidence, &self.execution) else {
            return Ok(0);
        };
        let Ok(_serial) = runtime.serial.try_lock() else {
            return Ok(0);
        };
        for run_id in self.store.evidence_import_candidates(only_run).await? {
            let run = self
                .store
                .load_run(run_id)
                .await?
                .ok_or_else(credential_error)?;
            self.collect_runtime_report(&run).await?;
            for (stream, name, file) in [
                (EvidenceStream::Stdout, "stdout", "stdout.jsonl"),
                (EvidenceStream::Stderr, "stderr", "stderr.log"),
            ] {
                let mut transaction = self.store.begin().await?;
                let project = transaction
                    .lock_project(run.project_id)
                    .await?
                    .ok_or_else(credential_error)?;
                if transaction.evidence_stream_imported(run.id, name).await? {
                    continue;
                }
                let (mut secrets, unsafe_auth) =
                    match self.run_redaction_secrets(&mut transaction, &run).await {
                        Ok(secrets) => (secrets, false),
                        Err(_) => (zeroize::Zeroizing::new(Vec::new()), true),
                    };
                let path = execution
                    .root
                    .join("evidence")
                    .join(run.id.to_string())
                    .join(run.environment_epoch.to_string())
                    .join(file);
                let spec: forge_domain::runtime::SandboxLaunchSpec =
                    serde_json::from_value(run.run_spec.clone()).map_err(|_| credential_error())?;
                let limit = spec.max_output_bytes();
                let bytes = if !unsafe_auth && path.try_exists().map_err(|_| credential_error())? {
                    let trusted_root = execution.root.clone();
                    Some(
                        tokio::task::spawn_blocking(move || {
                            PrivateMaterialization::read_beneath(
                                &trusted_root,
                                path.strip_prefix(&trusted_root).map_err(|_| {
                                    forge_provider_common::SecretStoreError::UnsafePermissions
                                })?,
                                limit,
                            )
                        })
                        .await
                        .map_err(|_| credential_error())?,
                    )
                } else {
                    None
                };
                let mut incomplete = unsafe_auth
                    || bytes.is_none()
                        && transaction
                            .run_recovery_state(run.id)
                            .await?
                            .is_some_and(|state| state.started_at.is_some());
                // Reuse durable orphan chunks after a crash before canonical commit.
                let mut chunks: Vec<_> = runtime
                    .spool
                    .pending()
                    .map_err(|_| credential_error())?
                    .into_iter()
                    .filter(|pending| {
                        !unsafe_auth
                            && pending.receipt().data().scope.run_id == run.id
                            && pending.receipt().data().stream == stream
                    })
                    .collect();
                // A persisted prefix does not prove collector.finish committed.
                // Retain it as partial evidence after a crash, never as a full
                // stream or permission to remove its redaction material.
                incomplete |= !chunks.is_empty();
                if chunks.is_empty()
                    && let Some(bytes) = bytes
                {
                    match bytes {
                        Ok(bytes) => {
                            let redaction = EvidenceRedaction::new(
                                "forge-known-credentials-v1".into(),
                                std::mem::take(&mut *secrets),
                            )
                            .map_err(|_| credential_error())?;
                            let mut collector = runtime
                                .spool
                                .start_stream(
                                    EvidenceScope {
                                        project_id: run.project_id,
                                        task_id: run.task_id(),
                                        run_id: run.id,
                                    },
                                    stream,
                                    redaction,
                                )
                                .map_err(|_| credential_error())?;
                            let result = collector.push(bytes.expose());
                            incomplete |= result.evidence_incomplete;
                            chunks.extend(result.chunks);
                            let result = collector.finish();
                            incomplete |= result.evidence_incomplete;
                            chunks.extend(result.chunks);
                        }
                        Err(_) => incomplete = true,
                    }
                }
                let now = Timestamp::now_utc();
                for chunk in &chunks {
                    transaction
                        .record_evidence_object(chunk.receipt(), false)
                        .await?;
                }
                transaction
                    .record_evidence_stream(run.id, name, incomplete)
                    .await?;
                if incomplete {
                    transaction
                        .record_run_incident(
                            project.id(),
                            run.id,
                            IncidentKind::EvidenceIncomplete,
                            RecoveryAssessment::Unknown,
                        )
                        .await?;
                }
                transaction
                    .append_event_and_outbox(&event(
                        project.id(),
                        AggregateRef::Project(project.id()),
                        project.revision(),
                        DomainEventKind::RunEvidenceRecorded,
                        self.actors.core,
                        CommandId::new(),
                        None,
                        event_payload([
                            ("run_id", json!(run.id)),
                            ("stream", json!(name)),
                            ("chunk_count", json!(chunks.len())),
                            ("incomplete", json!(incomplete)),
                            ("location", json!("pending_upload")),
                        ]),
                        now,
                    )?)
                    .await?;
                transaction.commit().await?;
            }
        }
        self.cleanup_retired_run_secrets(only_run).await?;
        let Some(objects) = &runtime.object_store else {
            return Ok(0);
        };
        let mut uploaded = 0;
        for pending in runtime
            .spool
            .pending()
            .map_err(|_| credential_error())?
            .into_iter()
            .take(32)
        {
            let mut transaction = self.store.begin().await?;
            let Some(stored) = transaction
                .evidence_object_stored(pending.receipt().data().id)
                .await?
            else {
                continue;
            };
            transaction.commit().await?;
            let receipt = if stored {
                pending.receipt().stored()?
            } else {
                objects
                    .upload(&pending)
                    .await
                    .map_err(|_| CoreError::InvalidTransport {
                        field: "evidence",
                        reason: "object store unavailable; local evidence retained".into(),
                    })?
            };
            let mut transaction = self.store.begin().await?;
            let scope = receipt.data().scope;
            let project = transaction
                .lock_project(scope.project_id)
                .await?
                .ok_or_else(credential_error)?;
            if transaction.record_evidence_object(&receipt, true).await? {
                transaction
                    .append_event_and_outbox(&event(
                        project.id(),
                        AggregateRef::Project(project.id()),
                        project.revision(),
                        DomainEventKind::RunEvidenceRecorded,
                        self.actors.core,
                        CommandId::new(),
                        None,
                        event_payload([
                            ("run_id", json!(scope.run_id)),
                            ("evidence_id", json!(receipt.data().id)),
                            ("location", json!("stored")),
                        ]),
                        Timestamp::now_utc(),
                    )?)
                    .await?;
            }
            transaction.commit().await?;
            runtime
                .spool
                .release_uploaded(&pending, &receipt)
                .map_err(|_| credential_error())?;
            uploaded += 1;
        }
        Ok(uploaded)
    }
}
