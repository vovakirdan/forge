//! Owner-configured System stages. Core schedules evidence; it never discovers tests.
mod acceptance;
mod completion;
mod handoff;
use crate::{
    CoreError, CoreService,
    event::{event, event_payload},
    task_support::{
        load_scoped_task, persist_task_and_project, pinned_pipeline_version, retained_persistence,
        task_event,
    },
};
use completion::HookCompletion;
use forge_domain::{
    AggregateRef, CommandId, DomainEventKind, HookAssignmentRef, LifecycleStatus, Project,
    ProjectId, Task, TaskWaitCondition, TaskWaitKind, TaskWorkSurface, WaitConditionId,
    git_integration::SystemStageAction, runtime::HookRunSpec,
};
use forge_protocol::supervisor::v1::{
    CoreToSupervisor, HookExecutionAssignment, ProvisionRun, core_to_supervisor, provision_run,
};
use forge_storage::{StorageTransaction, StoredHookInvocation};
use serde_json::json;
use uuid::Uuid;

impl CoreService {
    pub(crate) async fn claim_hook_provision(
        &self,
        project_id: ProjectId,
    ) -> Result<Option<CoreToSupervisor>, CoreError> {
        if self.fake_runtime_enabled {
            return Ok(None);
        }
        for task_id in self.store.hook_work(project_id).await? {
            let mut tx = self.store.begin().await?;
            let Some(mut project) = tx.lock_project(project_id).await? else {
                continue;
            };
            let task = load_scoped_task(&mut tx, &project, task_id).await?.task;
            if task.lifecycle() != LifecycleStatus::InProgress
                || !tx.hook_dispatch_open(project_id, task_id, None).await?
            {
                continue;
            }
            let version = pinned_pipeline_version(&mut tx, &project, &task).await?;
            let Some(stage) = task.current_stage_id().and_then(|id| version.stage(id)) else {
                continue;
            };
            let Some(SystemStageAction::ProjectHook {
                hook_version_id, ..
            }) = stage.system_action()
            else {
                continue;
            };
            let Some(hook) = tx
                .load_project_hook_version(project_id, *hook_version_id)
                .await?
            else {
                self.hold_hook_task(
                    &mut tx,
                    &mut project,
                    &task,
                    "Pinned project Hook version is missing",
                )
                .await?;
                tx.commit().await?;
                continue;
            };
            if stage
                .system_action()
                .ok_or_else(|| invalid("Hook action absent"))?
                .validate_hook_version(stage, &version, &hook)
                .is_err()
            {
                self.hold_hook_task(&mut tx,&mut project,&task,
                    "Pinned Hook policy does not match this Pipeline; management must select a compatible version").await?;
                tx.commit().await?;
                continue;
            }
            if !hook.applies_to(task.kind()) {
                let skip = forge_storage::HookSkipSpec {
                    schema_version: 1,
                    invocation_id: Uuid::now_v7(),
                    project_id,
                    task_id,
                    pipeline_version_id: version.id(),
                    stage_id: stage.id().clone(),
                    stage_visit: task
                        .current_stage_visit()
                        .ok_or_else(|| invalid("stage visit absent"))?
                        .get(),
                    hook,
                };
                tx.insert_hook_skip(&skip, self.instance_id, task.revision().get())
                    .await?;
                self.apply_hook_result(
                    &mut tx,
                    &mut project,
                    &HookCompletion::from(&skip),
                    json!({"verdict":"skipped","reason":"task_kind_not_applicable"}),
                )
                .await?;
                tx.commit().await?;
                continue;
            }
            let TaskWorkSurface::Git(binding) = task.work_surface() else {
                self.hold_hook_task(
                    &mut tx,
                    &mut project,
                    &task,
                    "Project Hook requires a Task-owned Git candidate",
                )
                .await?;
                tx.commit().await?;
                continue;
            };
            let Some(candidate) = tx
                .latest_accepted_git_candidate(project_id, task_id)
                .await?
            else {
                self.hold_hook_task(
                    &mut tx,
                    &mut project,
                    &task,
                    "No accepted Git candidate is available for this Hook",
                )
                .await?;
                tx.commit().await?;
                continue;
            };
            if candidate.surface_id != binding.surface_id {
                return Err(invalid("candidate surface mismatch"));
            }
            let repository = tx
                .load_project_repository(binding.repository_id)
                .await?
                .ok_or_else(|| invalid("repository absent"))?;
            if repository.project_id != project_id || !binding.matches_repository(&repository) {
                return Err(invalid("repository scope changed"));
            }
            let spec = HookRunSpec {
                schema_version: 5,
                project_id,
                run_id: Uuid::now_v7(),
                assignment: HookAssignmentRef {
                    invocation_id: Uuid::now_v7(),
                    task_id,
                    pipeline_version_id: version.id(),
                    stage_id: stage.id().clone(),
                    stage_visit: task
                        .current_stage_visit()
                        .ok_or_else(|| invalid("stage visit absent"))?
                        .get(),
                    candidate_proposal_id: candidate.proposal_id,
                },
                hook,
                binding: binding.clone(),
                candidate: candidate.candidate,
            };
            spec.validate().map_err(|_| invalid("invalid Hook spec"))?;
            let stored = StoredHookInvocation {
                run_id: Some(spec.run_id),
                state: "running".into(),
                core_instance: self.instance_id,
                expected_task_revision: task.revision().get(),
                result: None,
                spec,
            };
            if !tx.lock_run_admission(project_id, None).await? {
                return Ok(None);
            }
            tx.insert_hook_invocation(&stored).await?;
            let run = tx.create_hook_run(&stored).await?;
            let prior = project.revision();
            let now = crate::canonical_clock::project_mutation_time(&project);
            project.record_child_mutation(now)?;
            tx.update_project(&project, prior).await?;
            tx.append_event_and_outbox(&event(
                project_id,
                AggregateRef::Project(project_id),
                project.revision(),
                DomainEventKind::RunProvisioned,
                self.actors.core,
                CommandId::from(stored.spec.assignment.invocation_id),
                None,
                event_payload([
                    ("run_id", json!(run.id)),
                    ("assignment", json!(run.assignment)),
                    ("employee_id", json!(null)),
                    ("lease_fencing_token", json!(run.lease_fencing_token)),
                    ("environment_epoch", json!(run.environment_epoch)),
                ]),
                now,
            )?)
            .await?;
            tx.commit().await?;
            let spec = &stored.spec;
            let owner = &spec.assignment;
            return Ok(Some(CoreToSupervisor {
                message: Some(core_to_supervisor::Message::ProvisionRun(ProvisionRun {
                    command_id: Uuid::now_v7().to_string(),
                    run_id: run.id.to_string(),
                    employee_id: String::new(),
                    task_id: String::new(),
                    stage_id: String::new(),
                    attempt: 1,
                    lease_fencing_token: run.lease_fencing_token,
                    environment_epoch: run.environment_epoch,
                    context_snapshot_id: owner.invocation_id.to_string(),
                    run_spec_json: serde_json::to_string(spec)
                        .map_err(|_| invalid("spec encoding failed"))?,
                    run_spec_version: 5,
                    traceparent: crate::observability::current_traceparent(),
                    assignment: Some(provision_run::Assignment::Hook(HookExecutionAssignment {
                        invocation_id: owner.invocation_id.to_string(),
                        task_id: owner.task_id.to_string(),
                        pipeline_version_id: owner.pipeline_version_id.to_string(),
                        stage_id: owner.stage_id.to_string(),
                        stage_visit: owner.stage_visit,
                        candidate_proposal_id: owner.candidate_proposal_id.to_string(),
                    })),
                })),
            }));
        }
        Ok(None)
    }

    pub(crate) async fn reconcile_hook_results(&self) -> Result<(), CoreError> {
        for run_id in self.store.unfinished_hook_runs().await? {
            let mut tx = self.store.begin().await?;
            let Some(run) = tx.load_run(run_id).await? else {
                continue;
            };
            let Some(mut project) = tx.lock_project(run.project_id).await? else {
                continue;
            };
            self.record_hook_terminal(
                &mut tx,
                &mut project,
                &run,
                run.observed_state,
                &run.observed_details,
            )
            .await?;
            tx.commit().await?;
        }
        for project in self.store.hook_projects().await? {
            self.dispatch_available(project).await?;
        }
        Ok(())
    }
    pub(super) async fn hold_hook_task(
        &self,
        tx: &mut StorageTransaction<'_>,
        project: &mut Project,
        task: &Task,
        reason: &str,
    ) -> Result<(), CoreError> {
        if matches!(
            task.lifecycle(),
            LifecycleStatus::Done | LifecycleStatus::Cancelled | LifecycleStatus::Waiting
        ) {
            return Ok(());
        }
        let scoped = load_scoped_task(tx, project, task.id()).await?;
        let mut task = scoped.task;
        let previous = task.revision().get();
        let now = crate::canonical_clock::project_mutation_time(project);
        task.add_wait_condition(
            TaskWaitCondition::new(
                WaitConditionId::new(),
                TaskWaitKind::Interrupted,
                Some(reason.into()),
                self.actors.core,
                now,
            )?,
            now,
        )?;
        let persistence = retained_persistence(project, &task, scoped.persistence)?;
        persist_task_and_project(tx, project, &task, persistence, previous, now).await?;
        tx.append_event_and_outbox(&task_event(
            project,
            &task,
            DomainEventKind::TaskWaiting,
            self.actors.core,
            CommandId::new(),
            now,
            event_payload([
                ("reason", json!(reason)),
                ("wait_kind", json!("interrupted")),
            ]),
        )?)
        .await?;
        Ok(())
    }
}
fn invalid(reason: &str) -> CoreError {
    CoreError::InvalidTransport {
        field: "project_hook",
        reason: reason.into(),
    }
}
