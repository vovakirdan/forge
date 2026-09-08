//! Canonical executor ArtifactSubmission handling and shared Run scope checks.

use forge_domain::{
    Actor, ActorId, AggregateRef, Artifact, ArtifactBody, ArtifactId, ArtifactProducer, CommandId,
    DomainEventKind, ExecutorKind, LifecycleStatus, NewArtifact, PipelineVersion, Project,
};
use forge_storage::{
    ArtifactLocation, ExecutorArtifactReceipt, ExecutorArtifactWrite, ExecutorSubmissionRecord,
    FencedWrite, RunProjection, StorageTransaction, StoredTask,
};
use serde_json::json;

use crate::{
    CoreError, CoreService,
    event::{event, event_payload},
    task_support::{
        current_stage_executor, load_scoped_task, persist_task_and_project,
        pinned_pipeline_version, retained_persistence, task_event,
    },
};

use super::{
    SupervisorIdentity,
    bridge::{InboundResult, refused_fenced_write},
    validation::{ParsedArtifactSubmission, SubmissionIngress, SubmissionScope},
};

pub(super) struct SubmissionContext {
    pub(super) project: Project,
    pub(super) stored_task: StoredTask,
    pub(super) run: RunProjection,
    pub(super) version: PipelineVersion,
}

impl CoreService {
    pub(super) async fn record_executor_artifact(
        &self,
        identity: Option<&SupervisorIdentity>,
        submission: ParsedArtifactSubmission,
    ) -> Result<InboundResult, CoreError> {
        let mut transaction = self.store.begin().await?;
        if submission.scope.is_gateway()
            && let Some(refused) =
                reserve_submission(&mut transaction, &submission.scope, "artifact_submission")
                    .await?
        {
            return Ok(refused);
        }
        let mut context = self
            .load_employee_submission_context(&mut transaction, &submission.scope)
            .await?;
        let now = crate::canonical_clock::project_mutation_time(&context.project);
        if !submission.scope.is_gateway()
            && let Some(refused) =
                reserve_submission(&mut transaction, &submission.scope, "artifact_submission")
                    .await?
        {
            return Ok(refused);
        }

        let actor = employee_actor(&context.run)?;
        let artifact = Artifact::new(
            ArtifactId::new(),
            NewArtifact {
                project_id: context.project.id(),
                kind: submission.kind,
                title: submission.title,
                body: ArtifactBody::inline_json(submission.body),
                metadata: submission.metadata,
                created_by: actor,
                created_at: now,
            },
        )?;
        let previous_revision = context.stored_task.task.revision().get();
        let task = &mut context.stored_task.task;
        task.attach_artifact(
            &artifact,
            ArtifactProducer::EmployeeRun,
            actor,
            Some(context.run.require_employee_id()?),
            now,
        )?;
        let stage_id = task
            .current_stage_id()
            .cloned()
            .ok_or(CoreError::InvalidTransport {
                field: "task.current_stage_id",
                reason: "is absent for an employee artifact submission".to_owned(),
            })?;
        let location = ArtifactLocation {
            task_id: Some(task.id()),
            run_id: Some(context.run.id),
            stage_id: Some(stage_id),
            producer: ArtifactProducer::EmployeeRun,
            producer_id: Some(context.run.require_employee_id()?.as_uuid()),
            producer_data: identity.map_or_else(
                || json!({"ingress": "gateway"}),
                |identity| json!({"supervisor_instance_id": identity.instance_id()}),
            ),
        };
        let receipt = ExecutorArtifactReceipt::new(
            submission.scope.message_id,
            submission.scope.run_id,
            submission.scope.lease_fencing_token,
            submission.scope.environment_epoch,
        )?;
        transaction
            .insert_executor_artifact(&artifact, &ExecutorArtifactWrite::new(location, receipt)?)
            .await?;
        let persistence =
            retained_persistence(&context.project, task, context.stored_task.persistence)?;
        persist_task_and_project(
            &mut transaction,
            &mut context.project,
            task,
            persistence,
            previous_revision,
            now,
        )
        .await?;
        let command_id = CommandId::from(submission.scope.message_id);
        let artifact_event = event(
            context.project.id(),
            AggregateRef::Artifact(artifact.id()),
            1,
            DomainEventKind::ArtifactCreated,
            actor,
            command_id,
            None,
            event_payload([("kind", json!(artifact.kind().as_str()))]),
            now,
        )?;
        let attached_event = task_event(
            &context.project,
            task,
            DomainEventKind::TaskArtifactAttached,
            actor,
            command_id,
            now,
            event_payload([
                ("artifact_id", json!(artifact.id().as_uuid())),
                ("run_id", json!(context.run.id)),
            ]),
        )?;
        transaction.append_event_and_outbox(&artifact_event).await?;
        transaction.append_event_and_outbox(&attached_event).await?;
        transaction.commit().await?;
        Ok(InboundResult::Accepted("Executor artifact recorded"))
    }

    pub(super) async fn load_employee_submission_context(
        &self,
        transaction: &mut StorageTransaction<'_>,
        scope: &SubmissionScope,
    ) -> Result<SubmissionContext, CoreError> {
        let run = transaction
            .load_run(scope.run_id)
            .await?
            .ok_or(CoreError::NotFound { aggregate: "run" })?;
        let project =
            transaction
                .lock_project(run.project_id)
                .await?
                .ok_or(CoreError::NotFound {
                    aggregate: "project",
                })?;
        let stored_task = load_scoped_task(transaction, &project, run.require_task_id()?).await?;
        if run.project_id != project.id() || run.require_task_id()? != stored_task.task.id() {
            return Err(CoreError::InvalidTransport {
                field: "executor_submission.run_id",
                reason: "does not identify the expected Project Task".to_owned(),
            });
        }
        if stored_task.task.lifecycle() != LifecycleStatus::InProgress {
            return Err(CoreError::InvalidTransport {
                field: "executor_submission.run_id",
                reason: "does not own an in-progress employee stage".to_owned(),
            });
        }
        let version = pinned_pipeline_version(transaction, &project, &stored_task.task).await?;
        let task_stage =
            stored_task
                .task
                .current_stage_id()
                .ok_or(CoreError::InvalidTransport {
                    field: "task.current_stage_id",
                    reason: "is absent for an executor submission".to_owned(),
                })?;
        if task_stage.as_str() != run.require_task_stage()?.stage_id.as_str()
            || current_stage_executor(&version, &stored_task.task)? != ExecutorKind::Employee
        {
            return Err(CoreError::InvalidTransport {
                field: "executor_submission.run_id",
                reason: "does not own the current employee stage".to_owned(),
            });
        }
        let context: forge_domain::ContextSnapshot =
            serde_json::from_value(run.context_manifest.clone()).map_err(|_| {
                CoreError::InvalidTransport {
                    field: "executor_submission.context",
                    reason: "invalid frozen Task context".into(),
                }
            })?;
        if context.data().stage_visit.is_some_and(|visit| {
            Some(visit)
                != stored_task
                    .task
                    .current_stage_visit()
                    .map(|visit| visit.get())
        }) {
            return Err(CoreError::InvalidTransport {
                field: "executor_submission.context",
                reason: "Run does not own the current stage visit".into(),
            });
        }
        if let Some(policy) = version
            .stage(task_stage)
            .and_then(|stage| stage.acceptance_policy())
        {
            policy.validate_task_surface(stored_task.task.work_surface())?;
        }
        Ok(SubmissionContext {
            project,
            stored_task,
            run,
            version,
        })
    }
}

pub(super) async fn reserve_submission(
    transaction: &mut StorageTransaction<'_>,
    scope: &SubmissionScope,
    kind: &'static str,
) -> Result<Option<InboundResult>, CoreError> {
    if let SubmissionIngress::Gateway { payload_hash } = scope.ingress {
        let result = transaction
            .record_gateway_submission(&forge_storage::GatewaySubmissionRecord {
                scope: forge_domain::runtime::RunScope {
                    run_id: scope.run_id,
                    fencing_token: scope.lease_fencing_token,
                    environment_epoch: scope.environment_epoch,
                },
                message_id: scope.message_id,
                payload_hash: payload_hash
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect(),
                receipt: json!({"status":if kind=="git_stage_proposal" {"proposal_saved"} else {"accepted"}, "message_id":scope.message_id, "kind":kind}),
            })
            .await?;
        return match result {
            forge_storage::GatewayWriteResult::Applied => Ok(None),
            forge_storage::GatewayWriteResult::Replayed(receipt)
                if receipt["status"] == "proposal_saved" =>
            {
                Ok(Some(InboundResult::ProposalSaved))
            }
            forge_storage::GatewayWriteResult::Replayed(_) => Ok(Some(InboundResult::Accepted(
                "Gateway submission already recorded",
            ))),
            forge_storage::GatewayWriteResult::Denied => Ok(Some(InboundResult::Ignored(
                "Gateway scope is no longer active",
            ))),
            forge_storage::GatewayWriteResult::Conflict => Err(CoreError::IdempotencyConflict),
        };
    }
    let result = transaction
        .record_executor_submission(&ExecutorSubmissionRecord {
            message_id: scope.message_id,
            run_id: scope.run_id,
            lease_fencing_token: scope.lease_fencing_token,
            environment_epoch: scope.environment_epoch,
            sequence: scope.sequence,
            receipt: json!({
                "kind": kind,
                "run_id": scope.run_id,
                "sequence": scope.sequence,
            }),
            source_event_id: None,
        })
        .await?;
    Ok((result != FencedWrite::Applied).then(|| refused_fenced_write(result)))
}

pub(super) fn employee_actor(run: &RunProjection) -> Result<Actor, CoreError> {
    Ok(Actor::employee(ActorId::from(
        run.require_employee_id()?.as_uuid(),
    )))
}
