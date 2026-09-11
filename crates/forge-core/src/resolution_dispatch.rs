//! Bounded routing to a real Employee or the durable Human fallback.
use crate::{
    CoreError, CoreService,
    resolution_queue::{assign, audit, invalid, source_is_current},
};
use forge_domain::{
    DomainEventKind, ProjectId, ResolutionAssignmentRef,
    resolution::*,
    runtime::{ResolutionRunSpec, SurfaceSpec},
};
use forge_protocol::supervisor::v1::{
    CoreToSupervisor, ProvisionRun, ResolutionExecutionAssignment, core_to_supervisor,
    provision_run,
};
use serde_json::json;
use uuid::Uuid;

impl CoreService {
    pub(crate) async fn claim_resolution_provision(
        &self,
        project_id: ProjectId,
    ) -> Result<Option<CoreToSupervisor>, CoreError> {
        if self.fake_runtime_enabled {
            return Ok(None);
        }
        // Account-busy routes remain queued. Advance the bounded page even when
        // none can run, so later Human or other-account routes remain reachable.
        // Each Project has its own cursor; another Project cannot reset it.
        let ids = {
            let mut cursors = self.resolution_admission_cursors.lock().await;
            let ids = self
                .store
                .resolution_queue_ids(Some(project_id), cursors.get(&project_id).copied())
                .await?;
            if let Some(last) = ids.last() {
                cursors.insert(project_id, *last);
            } else {
                cursors.remove(&project_id);
            }
            ids
        };
        for id in ids {
            let mut tx = self.store.begin().await?;
            let Some(mut project) = tx.lock_project(project_id).await? else {
                continue;
            };
            if !tx.resolution_admission_open(project_id).await? {
                return Ok(None);
            }
            let Some(mut value) = tx.load_escalation(id).await? else {
                continue;
            };
            if value.state != EscalationState::Queued
                || !source_is_current(&mut tx, &value).await?
                || tx.resolution_has_live_execution(id).await?
            {
                continue;
            }
            let prior = value.revision;
            let mut candidate = None;
            let mut capacity_blocked = false;
            // A busy/unavailable supervisor must not block later route entries.
            while !value.requires_human() {
                let route = value.route.as_ref().ok_or_else(invalid)?;
                let employee = route.employee_ids[value.next_candidate as usize];
                value.next_candidate += 1;
                if !tx.onboarding_allowed(project_id, employee).await? {
                    continue;
                }
                if !tx.lock_employee_capacity(project_id, employee).await? {
                    continue;
                }
                let Some(mut binding) = tx.runtime_binding(employee).await? else {
                    continue;
                };
                binding.surface = SurfaceSpec::None;
                if binding.validate().is_err() {
                    continue;
                }
                if !tx
                    .lock_run_admission(project_id, Some(&binding.execution_profile))
                    .await?
                {
                    capacity_blocked = true;
                    continue;
                }
                let Some(record) = tx
                    .load_credential(
                        project_id,
                        binding.execution_profile.credential_binding().secret_id,
                    )
                    .await?
                else {
                    continue;
                };
                if self
                    .open_runtime_credential(
                        &record,
                        binding.execution_profile.credential_binding(),
                        binding.execution_profile.adapter_id(),
                    )
                    .is_err()
                {
                    continue;
                }
                candidate = Some((employee, binding, record));
                break;
            }
            // A full shared account is temporary, not a reason for Human fallback.
            // Other candidates were tried; retain this question's original route.
            if candidate.is_none() && capacity_blocked {
                continue;
            }
            let now = crate::canonical_clock::project_mutation_time(&project);
            let resolver = candidate
                .as_ref()
                .map_or(Resolver::Human, |(employee, _, _)| Resolver::Employee {
                    employee_id: *employee,
                });
            let assignment = assign(&mut value, resolver, self.actors.core, now)?;
            tx.save_escalation(&value, Some(prior)).await?;
            tx.insert_resolution_assignment(&assignment).await?;
            let Some((employee, binding, record)) = candidate else {
                audit(&mut tx,&mut project,self.actors.core,DomainEventKind::ResolutionAssigned,json!({"escalation_id":id,"assignment":assignment,"fallback":"route_exhausted_or_requires_human"})).await?;
                tx.commit().await?;
                continue;
            };
            let run_id = Uuid::now_v7();
            let owner = ResolutionAssignmentRef {
                assignment_id: assignment.id,
                escalation_id: id,
                lease_generation: assignment.lease.generation,
            };
            let context = ResolutionContext::new(ResolutionContextInput {
                schema_version: 4,
                context_snapshot_id: Uuid::now_v7(),
                project_id,
                employee_id: employee,
                run_id,
                assignment: assignment.clone(),
                escalation: value.clone(),
                capability_grants: [
                    "resolution.read",
                    "resolution.submit",
                    "resolution.decline",
                    "board.list",
                    "task.read",
                    "memory.search",
                    "memory.read",
                    "memory.refresh",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect(),
                created_at: now,
                knowledge_context: crate::context_compilation::compile_knowledge_context(
                    &mut tx, project_id, employee,
                )
                .await?,
            })?;
            let spec = ResolutionRunSpec {
                schema_version: 4,
                project_id,
                run_id,
                assignment: owner.clone(),
                binding,
                instruction: crate::context_compilation::add_knowledge_instruction(
                    json!({"assignment":"resolution","question":value,"resolution_assignment":assignment,
                    "policy":"Answer this question through resolution.submit, or decline. You are not executing the contextual Task. A recommended outcome must be in allowed_outcomes but never changes the Pipeline. ContinueStage only answers the question; action approval requires a Human. Do not modify Task files or claim stage completion. This is one turn; use Forge tools before exiting."}),
                    context.data().knowledge_context.as_ref(),
                )?,
            };
            let run = tx
                .create_resolution_run(&value, &assignment, &spec, &context, Uuid::now_v7())
                .await?;
            tx.pin_run_credential(run_id, &record).await?;
            audit(
                &mut tx,
                &mut project,
                self.actors.core,
                DomainEventKind::ResolutionAssigned,
                json!({"escalation_id":id,"assignment":assignment}),
            )
            .await?;
            audit(&mut tx,&mut project,self.actors.core,DomainEventKind::RunProvisioned,json!({"run_id":run_id,"assignment":run.assignment,"employee_id":employee,"lease_fencing_token":run.lease_fencing_token,"environment_epoch":run.environment_epoch})).await?;
            tx.commit().await?;
            return Ok(Some(CoreToSupervisor {
                message: Some(core_to_supervisor::Message::ProvisionRun(ProvisionRun {
                    command_id: Uuid::now_v7().to_string(),
                    run_id: run_id.to_string(),
                    task_id: String::new(),
                    employee_id: employee.to_string(),
                    stage_id: String::new(),
                    attempt: 1,
                    lease_fencing_token: run.lease_fencing_token,
                    environment_epoch: run.environment_epoch,
                    context_snapshot_id: context.data().context_snapshot_id.to_string(),
                    run_spec_json: serde_json::to_string(&spec).map_err(|_| invalid())?,
                    run_spec_version: 4,
                    traceparent: crate::observability::current_traceparent(),
                    assignment: Some(provision_run::Assignment::Resolution(
                        ResolutionExecutionAssignment {
                            assignment_id: owner.assignment_id.to_string(),
                            escalation_id: owner.escalation_id.to_string(),
                            lease_generation: owner.lease_generation,
                        },
                    )),
                })),
            }));
        }
        Ok(None)
    }
}
