//! Taskless one-shot conversation admission; no synthetic Task or worktree.

use forge_domain::{
    AggregateRef, CommandId, DomainEventKind, ProjectId, Timestamp,
    communication::{CommunicationContext, CommunicationContextInput},
    runtime::{CommunicationRunSpec, SurfaceSpec},
};
use forge_protocol::supervisor::v1::{
    CommunicationExecutionAssignment, CoreToSupervisor, ProvisionRun, core_to_supervisor,
    provision_run,
};
use serde_json::json;
use uuid::Uuid;

use crate::{
    CoreError, CoreService,
    event::{event, event_payload},
};

impl CoreService {
    pub(crate) async fn claim_communication_provision(
        &self,
        project_id: ProjectId,
    ) -> Result<Option<CoreToSupervisor>, CoreError> {
        if self.fake_runtime_enabled {
            return Ok(None);
        }
        let mut tx = self.store.begin().await?;
        let Some(mut project) = tx.lock_project(project_id).await? else {
            return Ok(None);
        };
        let Some(claim) = tx.claim_communication(project_id).await? else {
            tx.commit().await?;
            return Ok(None);
        };
        let now = crate::canonical_clock::project_mutation_time(&project);
        let mut binding = tx
            .runtime_binding(claim.employee_id)
            .await?
            .ok_or_else(crate::credentials::credential_error)?;
        // Explicit purpose policy: preserve provider/limits/prompts, never the Task files.
        binding.surface = SurfaceSpec::None;
        binding
            .validate()
            .map_err(|_| crate::credentials::credential_error())?;
        let record = tx
            .load_credential(
                project_id,
                binding.execution_profile.credential_binding().secret_id,
            )
            .await?
            .ok_or_else(crate::credentials::credential_error)?;
        let _checked = self.open_runtime_credential(
            &record,
            binding.execution_profile.credential_binding(),
            binding.execution_profile.adapter_id(),
        )?;
        let thread = tx
            .lock_employee_thread(claim.owner.thread_id)
            .await?
            .ok_or(CoreError::NotFound {
                aggregate: "employee thread",
            })?;
        let run_id = Uuid::now_v7();
        let resolution_answers = tx
            .communication_resolution_answers(claim.owner.assignment_id)
            .await?;
        let context = CommunicationContext::new(CommunicationContextInput {
            schema_version: 3,
            context_snapshot_id: Uuid::now_v7(),
            project_id,
            employee_id: claim.employee_id,
            run_id,
            assignment: claim.owner.clone(),
            context_task_id: thread.data().task_id,
            capability_grants: [
                "inbox.list",
                "inbox.acknowledge",
                "inbox.reply",
                "communication.complete",
                "escalation.raise",
                "board.list",
                "task.read",
                "memory.search",
                "memory.read",
                "memory.refresh",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            source_message: claim.source.clone(),
            knowledge_context: crate::context_compilation::compile_knowledge_context(
                &mut tx,
                project_id,
                claim.employee_id,
            )
            .await?,
            created_at: now,
        })?;
        let spec = CommunicationRunSpec {
            schema_version: 3,
            project_id,
            run_id,
            assignment: claim.owner.clone(),
            binding,
            instruction: crate::context_compilation::add_knowledge_instruction(
                json!({"assignment":"communication","source_message":claim.source,
                "resolution_answers":resolution_answers,"context_task_id":thread.data().task_id,"policy":"This is a conversation, not Task execution. Read the assigned input, acknowledge and reply through Forge Inbox tools. Then call communication.complete. Context does not grant Task stage, artifact, or writer authority. Exit status alone is not a reply or completion."}),
                context.data().knowledge_context.as_ref(),
            )?,
        };
        let run = tx
            .create_communication_run(
                &claim,
                &spec,
                &context,
                Uuid::now_v7(),
                Timestamp::from_offset_date_time(
                    Timestamp::now_utc().as_offset_date_time() + time::Duration::minutes(5),
                ),
            )
            .await?;
        tx.pin_run_credential(run.id, &record).await?;
        let previous = project.revision();
        project.record_child_mutation(now)?;
        tx.update_project(&project, previous).await?;
        let command_id = CommandId::new();
        tx.append_event_and_outbox(&event(
            project_id,
            AggregateRef::Project(project_id),
            project.revision(),
            DomainEventKind::RunProvisioned,
            self.actors.core,
            command_id,
            None,
            event_payload([
                ("run_id", json!(run.id)),
                ("employee_id", json!(run.employee_id)),
                ("assignment", json!(run.assignment)),
                ("lease_fencing_token", json!(run.lease_fencing_token)),
                ("environment_epoch", json!(run.environment_epoch)),
            ]),
            now,
        )?)
        .await?;
        tx.commit().await?;
        Ok(Some(CoreToSupervisor {
            message: Some(core_to_supervisor::Message::ProvisionRun(ProvisionRun {
                command_id: command_id.to_string(),
                run_id: run.id.to_string(),
                task_id: String::new(),
                employee_id: run.require_employee_id()?.to_string(),
                stage_id: String::new(),
                attempt: run.attempt_number,
                lease_fencing_token: run.lease_fencing_token,
                environment_epoch: run.environment_epoch,
                context_snapshot_id: context.data().context_snapshot_id.to_string(),
                run_spec_json: serde_json::to_string(&spec)
                    .map_err(|_| crate::credentials::credential_error())?,
                run_spec_version: 3,
                traceparent: crate::observability::current_traceparent(),
                assignment: Some(provision_run::Assignment::Communication(
                    CommunicationExecutionAssignment {
                        assignment_id: claim.owner.assignment_id.to_string(),
                        thread_id: claim.owner.thread_id.to_string(),
                        source_message_id: claim.owner.source_message_id.to_string(),
                    },
                )),
            })),
        }))
    }
}
