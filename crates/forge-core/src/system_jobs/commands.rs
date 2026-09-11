use super::invalid;
use crate::{
    CoreError, CoreService,
    command::{finish_command, resource},
    event::{event, event_payload},
};
use forge_application::{CommandEnvelope, CommandPayload};
use forge_domain::{
    AggregateRef, CommandId, DomainEventKind, Project, Timestamp, system_job::SystemJobManagement,
};
use forge_protocol::wire::CommandReceipt;
use forge_storage::{StorageTransaction, SystemJobSettings};
use serde_json::json;

impl CoreService {
    pub(crate) async fn manage_system_jobs(
        &self,
        tx: &mut StorageTransaction<'_>,
        mut project: Project,
        envelope: &CommandEnvelope,
        hash: &str,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<CommandReceipt, CoreError> {
        let input = match &envelope.payload {
            CommandPayload::ConfigureSystemJobs(v)
            | CommandPayload::RequestTaskSummary(v)
            | CommandPayload::RequestEmployeeOnboarding(v)
            | CommandPayload::RetrySystemJob(v)
            | CommandPayload::SkipEmployeeOnboarding(v) => v,
            _ => return Err(invalid("unsupported management action")),
        };
        input.validate_project(project.id())?;
        let resource_ref = match input {
            SystemJobManagement::Configure {
                expected_settings_revision,
                policy,
                binding,
            } => {
                let prior = tx.system_job_settings(project.id()).await?;
                if prior.as_ref().map_or(0, |s| s.revision) != *expected_settings_revision {
                    return Err(invalid("settings revision changed"));
                }
                // Validate the selected credential now; no secret bytes enter settings or output.
                if tx
                    .load_credential(
                        project.id(),
                        binding.execution_profile.credential_binding().secret_id,
                    )
                    .await?
                    .is_none()
                {
                    return Err(invalid("credential is not enrolled in this Project"));
                }
                tx.save_system_job_settings(
                    project.id(),
                    &SystemJobSettings {
                        revision: expected_settings_revision + 1,
                        policy: policy.clone(),
                        binding: (**binding).clone(),
                    },
                )
                .await?;
                tx.initialize_system_job_cursor(project.id()).await?;
                if !policy.enabled {
                    for run in tx.lock_active_runs_for_project(project.id()).await? {
                        if run.assignment.system_job().is_some() {
                            tx.request_run_stop(
                                run.id,
                                run.lease_fencing_token,
                                run.environment_epoch,
                                false,
                            )
                            .await?;
                        }
                    }
                }
                None
            }
            SystemJobManagement::RequestSummary { task_id } => {
                let task = tx
                    .lock_task(*task_id)
                    .await?
                    .ok_or_else(|| invalid("Task does not exist"))?;
                if task.task.project_id() != project.id() {
                    return Err(CoreError::Forbidden);
                }
                let settings = tx
                    .system_job_settings(project.id())
                    .await?
                    .ok_or_else(|| invalid("configure SystemJobs first"))?;
                let through = tx.canonical_event_watermark(project.id()).await?;
                let id = tx
                    .enqueue_task_summary(
                        project.id(),
                        *task_id,
                        through,
                        settings.policy.coalesce_seconds,
                    )
                    .await?;
                Some(resource("system_job", id))
            }
            SystemJobManagement::RequestOnboarding { employee_id } => {
                if tx.system_job_settings(project.id()).await?.is_none() {
                    return Err(invalid("configure SystemJobs first"));
                }
                Some(resource(
                    "system_job",
                    tx.request_employee_onboarding(project.id(), *employee_id, true)
                        .await?,
                ))
            }
            SystemJobManagement::Retry { job_id } => {
                let job = tx
                    .lock_system_job(project.id(), *job_id)
                    .await?
                    .ok_or_else(|| invalid("job does not exist"))?;
                if !matches!(job.state.as_str(), "held" | "cancelled")
                    || tx.system_job_has_retained_attempt(job.id).await?
                {
                    return Err(invalid(
                        "retry requires a held job and proven physical quiescence",
                    ));
                }
                if job.kind == forge_domain::system_job::SystemJobKind::Onboarding {
                    let target = job
                        .target_employee_id
                        .ok_or_else(|| invalid("onboarding target missing"))?;
                    tx.request_employee_onboarding(project.id(), target, true)
                        .await?;
                } else {
                    tx.set_system_job_state(job.id, "pending", None, true)
                        .await?;
                }
                Some(resource("system_job", job.id))
            }
            SystemJobManagement::SkipOnboarding {
                employee_id,
                reason,
            } => {
                if let Some(id) = tx.onboarding_job_id(project.id(), *employee_id).await? {
                    if tx.system_job_has_retained_attempt(id).await? {
                        return Err(invalid("stop and reconcile onboarding before skipping it"));
                    }
                    tx.set_system_job_state(id, "cancelled", Some("manager_skipped"), false)
                        .await?;
                }
                tx.set_onboarding_receipt(
                    project.id(),
                    *employee_id,
                    "skipped",
                    json!({
                        "reason": reason, "actor": self.actors.human,
                        "command_id": command_id, "at": now, "completed": false
                    }),
                )
                .await?;
                Some(resource("employee", employee_id.as_uuid()))
            }
        };
        let previous = project.revision();
        project.record_child_mutation(now)?;
        tx.update_project(&project, previous).await?;
        let audit = event(
            project.id(),
            AggregateRef::Project(project.id()),
            project.revision(),
            DomainEventKind::SystemJobChanged,
            self.actors.human,
            command_id,
            None,
            event_payload([
                ("operation", json!(envelope.name)),
                ("resource", json!(resource_ref)),
            ]),
            now,
        )?;
        finish_command(
            tx,
            &project,
            envelope,
            hash,
            command_id,
            self.actors.human,
            vec![audit],
            resource_ref,
        )
        .await
    }
}
