use crate::{
    CoreError, CoreService,
    command::{finish_command, resource},
    event::{event, event_payload},
    task_support::{load_scoped_task, persist_task_and_project, retained_persistence, task_event},
};
use forge_application::{CommandEnvelope, CommandPayload};
use forge_domain::{
    AggregateRef, CommandId, DomainEventKind, ExecutorKind, LifecycleStatus, Project, Timestamp,
    runtime::{BootRecoveryPolicy, RecoveryAssessment},
};
use forge_protocol::wire::CommandReceipt;
use forge_storage::StorageTransaction;
use serde_json::json;

impl CoreService {
    pub(crate) async fn apply_recovery_command(
        &self,
        transaction: &mut StorageTransaction<'_>,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<CommandReceipt, CoreError> {
        let mut events = Vec::new();
        let (kind, payload, target) = match envelope.payload {
            CommandPayload::ConfigureBootRecoveryPolicy { policy } => {
                transaction
                    .configure_boot_recovery_policy(project.id(), policy)
                    .await?;
                (
                    DomainEventKind::BootRecoveryPolicyConfigured,
                    event_payload([("policy", json!(policy))]),
                    resource("project", project.id().as_uuid()),
                )
            }
            CommandPayload::AcceptRunRecoveryAssessment { run_id, assessment } => {
                let Some(run) = transaction.load_run(run_id).await? else {
                    return Err(CoreError::NotFound { aggregate: "run" });
                };
                if run.project_id != project.id() {
                    return Err(CoreError::NotFound { aggregate: "run" });
                }
                let state =
                    transaction
                        .run_recovery_state(run_id)
                        .await?
                        .ok_or(CoreError::NotFound {
                            aggregate: "Run recovery state",
                        })?;
                if state.lease_active {
                    return Err(recovery_refusal("execution is not retired"));
                }
                let policy = transaction.boot_recovery_policy(project.id()).await?;
                let mut recovery_queue = None;
                if assessment == RecoveryAssessment::NotStartedConfirmed {
                    if state.reserved {
                        return Err(recovery_refusal(
                            "physical quiescence is required before accepting safe re-execution",
                        ));
                    }
                    if transaction.run_has_result_evidence(run_id).await? {
                        return Err(recovery_refusal(
                            "accepted result evidence contradicts not_started_confirmed",
                        ));
                    }
                    let stored = load_scoped_task(transaction, &project, run.task_id).await?;
                    let mut task = stored.task;
                    if task.lifecycle().is_terminal()
                        || task.current_stage_id().map(|stage| stage.as_str())
                            != Some(run.stage_id.as_str())
                    {
                        return Err(recovery_refusal(
                            "Task no longer belongs to the interrupted stage",
                        ));
                    }
                    if policy != BootRecoveryPolicy::ManualHold {
                        let detail = format!("recovery_run={run_id}");
                        let wait = task
                            .wait_conditions()
                            .find(|wait| wait.detail() == Some(detail.as_str()))
                            .map(|wait| wait.id())
                            .ok_or_else(|| {
                                recovery_refusal("the Run recovery wait must still be active")
                            })?;
                        let previous = task.revision().get();
                        task.resolve_wait_condition(wait, now)?;
                        let persistence =
                            retained_persistence(&project, &task, stored.persistence)?;
                        persist_task_and_project(
                            transaction,
                            &mut project,
                            &task,
                            persistence,
                            previous,
                            now,
                        )
                        .await?;
                        if matches!(
                            task.lifecycle(),
                            LifecycleStatus::Ready | LifecycleStatus::InProgress
                        ) {
                            let version = transaction
                                .lock_pipeline_version(task.pipeline().pipeline_version_id())
                                .await?
                                .ok_or(CoreError::NotFound {
                                    aggregate: "pipeline version",
                                })?;
                            let stage = task
                                .current_stage_id()
                                .and_then(|stage| version.stage(stage))
                                .ok_or_else(|| recovery_refusal("current stage is absent"))?;
                            if stage.executor_kind() != ExecutorKind::Employee {
                                return Err(recovery_refusal(
                                    "only Employee stages can re-execute",
                                ));
                            }
                            recovery_queue = transaction
                                .enqueue(&crate::scheduler::queue_input(
                                    &project,
                                    &task,
                                    &persistence,
                                    Timestamp::now_utc(),
                                )?)
                                .await?
                                .map(|queue| queue.id);
                        }
                        events.push(task_event(
                            &project,
                            &task,
                            DomainEventKind::TaskResumed,
                            self.actors.human,
                            command_id,
                            now,
                            event_payload([
                                ("run_id", json!(run_id)),
                                ("reason_code", json!("accepted_not_started_confirmed")),
                            ]),
                        )?);
                    }
                }
                if !transaction
                    .accept_recovery_assessment(run_id, command_id, assessment, recovery_queue)
                    .await?
                {
                    return Err(recovery_refusal(
                        "an assessment was already accepted for this Run",
                    ));
                }
                (
                    DomainEventKind::RunRecoveryAssessmentAccepted,
                    event_payload([
                        ("run_id", json!(run_id)),
                        ("assessment", json!(assessment)),
                        ("recovery_queue_entry_id", json!(recovery_queue)),
                    ]),
                    resource("run", run_id),
                )
            }
            _ => return Err(recovery_refusal("unsupported recovery command")),
        };
        let previous = project.revision();
        project.record_child_mutation(now)?;
        transaction.update_project(&project, previous).await?;
        events.push(event(
            project.id(),
            AggregateRef::Project(project.id()),
            project.revision(),
            kind,
            self.actors.human,
            command_id,
            None,
            payload,
            now,
        )?);
        finish_command(
            transaction,
            &project,
            envelope,
            request_hash,
            command_id,
            self.actors.human,
            events,
            Some(target),
        )
        .await
    }
}

fn recovery_refusal(reason: &str) -> CoreError {
    CoreError::InvalidTransport {
        field: "recovery",
        reason: reason.into(),
    }
}
