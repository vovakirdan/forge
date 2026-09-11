use super::{audit, digest, invalid};
use crate::{CoreError, CoreService};
use forge_domain::{
    DomainEventKind, ProjectId,
    runtime::{SurfaceSpec, SystemJobRunSpec},
    system_job::{SystemJobAssignmentRef, SystemJobInput, SystemJobKind},
};
use forge_protocol::supervisor::v1::{
    CoreToSupervisor, ProvisionRun, SystemJobExecutionAssignment, core_to_supervisor, provision_run,
};
use serde_json::json;
use uuid::Uuid;

impl CoreService {
    pub(crate) async fn claim_system_job_provision(
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
        let Some(settings) = tx.system_job_settings(project_id).await? else {
            return Ok(None);
        };
        let Some(job) = tx.pending_system_job(project_id).await? else {
            return Ok(None);
        };
        if let Some(reason) = tx.system_job_allowance(&job, &settings).await? {
            if reason == "attempt_limit" {
                tx.set_system_job_state(job.id, "held", Some(reason), false)
                    .await?;
            }
            tx.commit().await?;
            return Ok(None);
        }
        let mut binding = settings.binding.clone();
        binding.surface = SurfaceSpec::None;
        binding.limits.wall_seconds = binding
            .limits
            .wall_seconds
            .min(settings.policy.wall_seconds);
        binding.limits.stop_grace_seconds = binding
            .limits
            .stop_grace_seconds
            .min(binding.limits.wall_seconds);
        let context = match tx.system_job_source_context(&job).await {
            Ok(context) => context,
            Err(error) => {
                tx.set_system_job_state(job.id, "held", Some("source_unavailable"), false)
                    .await?;
                tx.commit().await?;
                tracing::warn!(job_id=%job.id,error=%error,"SystemJob input requires management");
                return Ok(None);
            }
        };
        let bytes = serde_json::to_vec(&context).map_err(|_| invalid("invalid input"))?;
        if bytes.len() > settings.policy.max_input_bytes as usize {
            tx.set_system_job_state(job.id, "held", Some("required_context_overflow"), false)
                .await?;
            tx.commit().await?;
            return Ok(None);
        }
        let input = SystemJobInput {
            source_task_id: job.source_task_id,
            target_employee_id: job.target_employee_id,
            covered_sequence: job.covered_sequence,
            source_digest: digest(&bytes),
            context,
        };
        let purpose = match job.kind {
            SystemJobKind::Summarization => {
                "Summarize only the supplied canonical evidence. Preserve who reported each claim; do not verify its truth, invent timestamps, approve work, change a Task, or create authority. Produce one task_summary and optionally project_knowledge_entry or employee_memory_entry for listed allowed employees. Do not copy personal information into project output. Cite exact source_refs from the input. Empty/truncated evidence is not a license to invent facts."
            }
            SystemJobKind::Onboarding => {
                "Read the pinned employee role, prompts, tool capabilities and published knowledge. Produce exactly one employee_memory_entry for the target employee, with task_id null. Record a concise orientation note and cite supplied source_refs. This records familiarity with these versions, not demonstrated comprehension or a resumed provider session."
            }
        };
        let instruction = serde_json::to_string(&json!({
            "purpose": job.kind,
            "policy": purpose,
            "submission": "Call forge_submit_system_job_result (system_job.submit_result) exactly once with {source_digest, entries:[{subject:{kind:task_summary,task_id:...} OR {kind:employee_memory_entry,employee_id:...,task_id:...} OR {kind:project_knowledge_entry,task_id:...},markdown:...,source_refs:[...]}]}. Use actual snake_case kinds and UUIDs. Do not provide dates, hashes, entry IDs or revisions; Core owns them. Successful process exit is not result acceptance. You have no Task assignment or Task management tools.",
            "max_result_bytes": settings.policy.max_result_bytes,
            "input": input
        })).map_err(|_| invalid("invalid instruction"))?;
        let owner = SystemJobAssignmentRef {
            job_id: job.id,
            attempt_id: Uuid::now_v7(),
            generation: job.generation,
            kind: job.kind,
        };
        let run_id = Uuid::now_v7();
        let spec = SystemJobRunSpec {
            schema_version: 7,
            project_id,
            run_id,
            assignment: owner.clone(),
            input,
            max_result_bytes: settings.policy.max_result_bytes,
            binding,
            instruction,
        };
        if spec.binding.system_prompt.len()
            + spec.binding.employee_prompt.len()
            + spec.instruction.len()
            + 4
            > settings.policy.max_input_bytes as usize
        {
            tx.set_system_job_state(job.id, "held", Some("required_context_overflow"), false)
                .await?;
            tx.commit().await?;
            return Ok(None);
        }
        let Some(record) = tx
            .load_credential(
                project_id,
                spec.binding
                    .execution_profile
                    .credential_binding()
                    .secret_id,
            )
            .await?
        else {
            tx.set_system_job_state(job.id, "held", Some("credential_unavailable"), false)
                .await?;
            tx.commit().await?;
            return Ok(None);
        };
        if self
            .open_runtime_credential(
                &record,
                spec.binding.execution_profile.credential_binding(),
                spec.binding.execution_profile.adapter_id(),
            )
            .is_err()
        {
            tx.set_system_job_state(job.id, "held", Some("credential_unavailable"), false)
                .await?;
            tx.commit().await?;
            return Ok(None);
        }
        let run = tx
            .create_system_job_run(&job, &spec, self.instance_id, Uuid::now_v7())
            .await?;
        tx.pin_run_credential(run_id, &record).await?;
        audit(
            &mut tx,
            &mut project,
            self.actors.core,
            DomainEventKind::RunProvisioned,
            json!({"run_id": run_id, "assignment": run.assignment,
                "source_digest": spec.input.source_digest}),
        )
        .await?;
        tx.commit().await?;
        Ok(Some(CoreToSupervisor {
            message: Some(core_to_supervisor::Message::ProvisionRun(ProvisionRun {
                command_id: Uuid::now_v7().to_string(),
                run_id: run_id.to_string(),
                task_id: String::new(),
                employee_id: String::new(),
                stage_id: String::new(),
                attempt: 1,
                lease_fencing_token: run.lease_fencing_token,
                environment_epoch: run.environment_epoch,
                context_snapshot_id: owner.attempt_id.to_string(),
                run_spec_json: serde_json::to_string(&spec).map_err(|_| invalid("invalid spec"))?,
                run_spec_version: 7,
                traceparent: crate::observability::current_traceparent(),
                assignment: Some(provision_run::Assignment::SystemJob(
                    SystemJobExecutionAssignment {
                        job_id: owner.job_id.to_string(),
                        attempt_id: owner.attempt_id.to_string(),
                        generation: owner.generation,
                        kind: serde_json::to_value(owner.kind)
                            .map_err(|_| invalid("invalid kind"))?
                            .as_str()
                            .ok_or_else(|| invalid("invalid kind"))?
                            .to_owned(),
                    },
                )),
            })),
        }))
    }
}
