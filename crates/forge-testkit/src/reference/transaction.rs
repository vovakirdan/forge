//! Mechanical persistence; deliberately contains no command dispatch or Task policy.

mod command_handoffs;
mod dispatch_constraints;
mod employee;
mod finding;
mod pipeline;
mod resolution;
mod task_resume;

use forge_application::{CommandTransaction, RepositoryError, ports::*};
use forge_domain::{
    Actor, Artifact, ArtifactId, DomainEvent, Employee, EventId, Pipeline, PipelineId,
    PipelineVersion, PipelineVersionId, Project, ProjectId, Task, TaskDependency, TaskId,
};
use uuid::Uuid;

use super::{MemoryQueueEntry, MemoryQueueState, MemoryTransaction};

fn invalid(reason: &str) -> RepositoryError {
    RepositoryError::InvalidInput {
        reason: reason.to_owned(),
    }
}

impl MemoryTransaction {
    fn validate_git_binding(
        &self,
        task: &Task,
        persistence: TaskPersistence,
    ) -> Result<(), RepositoryError> {
        if let forge_domain::TaskWorkSurface::Git(binding) = task.work_surface() {
            let registered = self
                .staged
                .project_repositories
                .get(&binding.repository_id)
                .filter(|repository| {
                    repository.project_id == task.project_id()
                        && binding.matches_repository(repository)
                });
            if registered.is_none()
                || persistence.task_work_surface_id != Some(binding.surface_id)
                || self.staged.tasks.values().any(|other| {
                    other.task.id() != task.id()
                        && other.persistence.task_work_surface_id == Some(binding.surface_id)
                })
            {
                return Err(invalid(
                    "Task Git binding violates repository or surface scope",
                ));
            }
        }
        if let Some(stored) = self.staged.tasks.get(&task.id())
            && stored.task.work_surface() != &forge_domain::TaskWorkSurface::None
            && stored.task.work_surface() != task.work_surface()
        {
            return Err(invalid("Task Git binding is immutable"));
        }
        Ok(())
    }
    fn require_project(&self, project: ProjectId) -> Result<(), RepositoryError> {
        if self.staged.projects.contains_key(&project) {
            Ok(())
        } else {
            Err(RepositoryError::NotFound {
                aggregate: "project",
            })
        }
    }

    fn require_task_scope(&self, project: ProjectId, task: TaskId) -> Result<(), RepositoryError> {
        match self.staged.tasks.get(&task) {
            Some(stored) if stored.task.project_id() == project => Ok(()),
            _ => Err(RepositoryError::NotFound { aggregate: "task" }),
        }
    }
}

impl CommandTransaction for MemoryTransaction {
    async fn insert_command_handoff(
        &mut self,
        handoff: &forge_domain::TaskHandoff,
    ) -> Result<(), RepositoryError> {
        self.record_command_handoff(handoff)
    }
    async fn required_hooks_satisfied(
        &mut self,
        task: &Task,
        version: &PipelineVersion,
        _candidate_override: Option<Uuid>,
    ) -> Result<bool, RepositoryError> {
        // Reference fixtures have no configured Hook registry/runtime. Never
        // fabricate a pass for a stage whose required/applicable policy is unknown.
        Ok(task.project_id() == version.project_id()
            && task.pipeline().pipeline_version_id() == version.id()
            && !version.stages().any(|stage| {
                matches!(
                    stage.system_action(),
                    Some(forge_domain::git_integration::SystemStageAction::ProjectHook { .. })
                )
            }))
    }
    async fn communication_escalation_is_current(
        &mut self,
        escalation: &forge_domain::resolution::Escalation,
    ) -> Result<bool, RepositoryError> {
        Ok(escalation.source.communication().is_some_and(|source| {
            self.staged.runs.iter().any(|run| {
                run.scope.id == source.run_id
                    && run.scope.project_id == escalation.project_id
                    && run.scope.assignment.communication() == Some(&source.assignment)
                    && run.scope.lease_fencing_token == source.fencing_token
                    && run.scope.environment_epoch == source.environment_epoch
                    && run.stop_requested.is_some()
            })
        }))
    }
    async fn load_escalation_for_wait(
        &mut self,
        project: ProjectId,
        task: TaskId,
        wait: forge_domain::WaitConditionId,
    ) -> Result<Option<forge_domain::resolution::Escalation>, RepositoryError> {
        Ok(self
            .staged
            .escalations
            .values()
            .find(|value| {
                value.project_id == project
                    && value.source.task().is_some_and(|source| {
                        source.task_id == task && source.wait_condition_id == wait
                    })
            })
            .cloned())
    }
    async fn load_resolver_route(
        &mut self,
        project: ProjectId,
        key: &str,
    ) -> Result<Option<forge_domain::resolution::ResolverRoute>, RepositoryError> {
        Ok(self
            .staged
            .resolver_routes
            .get(&(project, key.to_owned()))
            .cloned())
    }
    async fn save_resolver_route(
        &mut self,
        route: &forge_domain::resolution::ResolverRoute,
        expected: Option<u64>,
    ) -> Result<(), RepositoryError> {
        self.save_route_record(route, expected)
    }
    async fn load_escalation(
        &mut self,
        id: Uuid,
    ) -> Result<Option<forge_domain::resolution::Escalation>, RepositoryError> {
        Ok(self.staged.escalations.get(&id).cloned())
    }
    async fn save_escalation(
        &mut self,
        value: &forge_domain::resolution::Escalation,
        expected: Option<u64>,
    ) -> Result<(), RepositoryError> {
        self.save_escalation_record(value, expected)
    }
    async fn load_resolution_assignment(
        &mut self,
        id: Uuid,
    ) -> Result<Option<forge_domain::resolution::ResolutionAssignment>, RepositoryError> {
        Ok(self.staged.resolution_assignments.get(&id).cloned())
    }
    async fn insert_resolution_assignment(
        &mut self,
        value: &forge_domain::resolution::ResolutionAssignment,
    ) -> Result<(), RepositoryError> {
        self.insert_resolution_record(value)
    }
    async fn update_resolution_assignment(
        &mut self,
        value: &forge_domain::resolution::ResolutionAssignment,
    ) -> Result<(), RepositoryError> {
        self.update_resolution_record(value)
    }
    async fn lock_finding(
        &mut self,
        id: Uuid,
    ) -> Result<Option<forge_domain::finding::Finding>, RepositoryError> {
        Ok(self.staged.findings.get(&id).cloned())
    }
    async fn insert_finding(
        &mut self,
        finding: &forge_domain::finding::Finding,
    ) -> Result<(), RepositoryError> {
        self.insert_finding_record(finding)
    }
    async fn update_finding(
        &mut self,
        finding: &forge_domain::finding::Finding,
        expected: u64,
    ) -> Result<(), RepositoryError> {
        self.update_finding_record(finding, expected)
    }
    async fn lock_task_resume_schedule(
        &mut self,
        id: Uuid,
    ) -> Result<Option<forge_domain::TaskResumeSchedule>, RepositoryError> {
        Ok(self.staged.resume_schedules.get(&id).cloned())
    }
    async fn insert_task_resume_schedule(
        &mut self,
        schedule: &forge_domain::TaskResumeSchedule,
    ) -> Result<(), RepositoryError> {
        self.insert_resume(schedule)
    }
    async fn update_task_resume_schedule(
        &mut self,
        schedule: &forge_domain::TaskResumeSchedule,
    ) -> Result<(), RepositoryError> {
        self.update_resume(schedule)
    }
    async fn load_task_dispatch_constraint(
        &mut self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<Option<forge_domain::NextRunEmployeeConstraint>, RepositoryError> {
        Ok(self
            .staged
            .dispatch_constraints
            .values()
            .find(|constraint| {
                constraint.project_id == project
                    && constraint.task_id == task
                    && constraint.state.is_active()
            })
            .cloned())
    }
    async fn insert_task_dispatch_constraint(
        &mut self,
        constraint: &forge_domain::NextRunEmployeeConstraint,
    ) -> Result<(), RepositoryError> {
        self.insert_dispatch_constraint(constraint)
    }
    async fn update_task_dispatch_constraint(
        &mut self,
        constraint: &forge_domain::NextRunEmployeeConstraint,
        expected: &forge_domain::NextRunConstraintState,
    ) -> Result<(), RepositoryError> {
        self.update_dispatch_constraint(constraint, expected)
    }
    async fn lock_active_runs_for_employee(
        &mut self,
        project: ProjectId,
        employee: forge_domain::EmployeeId,
    ) -> Result<Vec<ActiveRun>, RepositoryError> {
        Ok(self
            .staged
            .runs
            .iter()
            .filter(|run| {
                run.scope.project_id == project
                    && run.scope.employee_id == Some(employee)
                    && (run.lease_active || run.reservation_held)
            })
            .map(|run| run.scope.clone())
            .collect())
    }
    async fn revoke_run_lease(
        &mut self,
        run: Uuid,
        fence: u64,
        epoch: u64,
    ) -> Result<bool, RepositoryError> {
        let Some(run) = self.staged.runs.iter_mut().find(|item| {
            item.scope.id == run
                && item.scope.lease_fencing_token == fence
                && item.scope.environment_epoch == epoch
                && item.lease_active
                && item.stop_requested == Some(true)
        }) else {
            return Ok(false);
        };
        run.lease_active = false;
        Ok(true)
    }
    async fn insert_message_requirement_waiver(
        &mut self,
        waiver: &forge_domain::communication::MessageRequirementWaiver,
    ) -> Result<(), RepositoryError> {
        waiver
            .validate_snapshot()
            .map_err(|_| invalid("invalid waiver"))?;
        if self.staged.message_waivers.contains_key(&waiver.message_id)
            || self
                .staged
                .employee_messages
                .get(&waiver.message_id)
                .is_none_or(|m| m.data().project_id != waiver.project_id)
        {
            return Err(invalid("duplicate waiver or invalid message scope"));
        }
        self.staged
            .message_waivers
            .insert(waiver.message_id, waiver.clone());
        Ok(())
    }
    async fn load_project_repository(
        &mut self,
        id: Uuid,
    ) -> Result<Option<forge_domain::ProjectRepository>, RepositoryError> {
        Ok(self.staged.project_repositories.get(&id).cloned())
    }
    async fn task_git_source_setting(
        &mut self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<forge_domain::git::TaskGitSourceSetting, RepositoryError> {
        Ok(self
            .staged
            .git_source_policies
            .iter()
            .rev()
            .find(|((p, t, _), _)| *p == project && *t == task)
            .map(|(_, setting)| setting.clone())
            .unwrap_or_default())
    }
    async fn insert_task_git_source_setting(
        &mut self,
        project: ProjectId,
        task: TaskId,
        setting: &forge_domain::git::TaskGitSourceSetting,
    ) -> Result<(), RepositoryError> {
        self.require_task_scope(project, task)?;
        let current = self.task_git_source_setting(project, task).await?;
        if current.revision.checked_add(1) != Some(setting.revision)
            || !matches!(
                self.staged.tasks.get(&task).map(|s| s.task.work_surface()),
                Some(forge_domain::TaskWorkSurface::Git(_))
            )
        {
            return Err(invalid("stale or out-of-scope Task Git source policy"));
        }
        self.staged
            .git_source_policies
            .insert((project, task, setting.revision), setting.clone());
        Ok(())
    }
    async fn insert_project_repository(
        &mut self,
        repository: &forge_domain::ProjectRepository,
    ) -> Result<(), RepositoryError> {
        repository
            .validate_snapshot()
            .map_err(|_| invalid("invalid project repository"))?;
        self.require_project(repository.project_id)?;
        if self
            .staged
            .project_repositories
            .contains_key(&repository.id)
            || self.staged.project_repositories.values().any(|stored| {
                stored.project_id == repository.project_id && stored.name == repository.name
            })
        {
            return Err(invalid("duplicate project repository"));
        }
        self.staged
            .project_repositories
            .insert(repository.id, repository.clone());
        Ok(())
    }
    async fn lock_employee_thread(
        &mut self,
        id: Uuid,
    ) -> Result<Option<forge_domain::communication::EmployeeThread>, RepositoryError> {
        Ok(self.staged.employee_threads.get(&id).cloned())
    }
    async fn insert_employee_thread(
        &mut self,
        thread: &forge_domain::communication::EmployeeThread,
    ) -> Result<(), RepositoryError> {
        let data = thread.data();
        self.require_project(data.project_id)?;
        if self.staged.employee_threads.contains_key(&data.id)
            || self
                .staged
                .employees
                .get(&data.employee_id)
                .is_none_or(|e| e.project_id() != data.project_id)
        {
            return Err(invalid("duplicate thread or invalid Employee scope"));
        }
        if let Some(task) = data.task_id {
            self.require_task_scope(data.project_id, task)?;
        }
        self.staged.employee_threads.insert(data.id, thread.clone());
        Ok(())
    }
    async fn update_employee_thread(
        &mut self,
        thread: &forge_domain::communication::EmployeeThread,
        expected_revision: u64,
    ) -> Result<(), RepositoryError> {
        let data = thread.data();
        if self.staged.employee_threads.get(&data.id).is_none_or(|t| {
            t.data().project_id != data.project_id || t.data().revision != expected_revision
        }) {
            return Err(RepositoryError::StaleRevision {
                aggregate: "employee thread",
            });
        }
        self.staged.employee_threads.insert(data.id, thread.clone());
        Ok(())
    }
    async fn load_employee_message(
        &mut self,
        id: Uuid,
    ) -> Result<Option<forge_domain::communication::EmployeeMessage>, RepositoryError> {
        Ok(self.staged.employee_messages.get(&id).cloned())
    }
    async fn insert_employee_message(
        &mut self,
        message: &forge_domain::communication::EmployeeMessage,
    ) -> Result<(), RepositoryError> {
        let data = message.data();
        if self.staged.employee_messages.contains_key(&data.id)
            || self
                .staged
                .employee_threads
                .get(&data.thread_id)
                .is_none_or(|t| {
                    t.data().project_id != data.project_id
                        || t.data().employee_id != data.employee_id
                })
            || self
                .staged
                .employee_messages
                .values()
                .any(|m| m.data().thread_id == data.thread_id && m.data().sequence == data.sequence)
        {
            return Err(invalid("duplicate message or invalid thread scope"));
        }
        if let Some(reply) = data.reply_to
            && self
                .staged
                .employee_messages
                .get(&reply)
                .is_none_or(|m| m.data().thread_id != data.thread_id)
        {
            return Err(invalid("reply does not belong to thread"));
        }
        if let Some(context) = data.target.task_context() {
            self.require_task_scope(data.project_id, context.task_id)?;
        }
        if let forge_domain::communication::MessageTarget::ExactRun { run_id, .. } = data.target
            && !self
                .staged
                .runs
                .iter()
                .any(|run| run.scope.id == run_id && run.scope.project_id == data.project_id)
        {
            return Err(invalid("Run does not belong to Project"));
        }
        self.staged
            .employee_messages
            .insert(data.id, message.clone());
        Ok(())
    }
    async fn lock_project_creation(&mut self, _id: ProjectId) -> Result<(), RepositoryError> {
        // The transaction already holds the global writer lock, including absent rows.
        Ok(())
    }

    async fn lock_project(&mut self, id: ProjectId) -> Result<Option<Project>, RepositoryError> {
        Ok(self.staged.projects.get(&id).cloned())
    }

    async fn insert_project(&mut self, project: &Project) -> Result<(), RepositoryError> {
        project
            .validate_snapshot()
            .map_err(|_| invalid("invalid project snapshot"))?;
        if self.staged.projects.contains_key(&project.id()) {
            return Err(invalid("duplicate project"));
        }
        self.staged.projects.insert(project.id(), project.clone());
        Ok(())
    }

    async fn update_project(
        &mut self,
        project: &Project,
        expected_revision: u64,
    ) -> Result<(), RepositoryError> {
        if project.revision() <= expected_revision {
            return Err(invalid("project revision must advance"));
        }
        if self
            .staged
            .projects
            .get(&project.id())
            .is_none_or(|p| p.revision() != expected_revision)
        {
            return Err(RepositoryError::StaleRevision {
                aggregate: "project",
            });
        }
        project
            .validate_snapshot()
            .map_err(|_| invalid("invalid project snapshot"))?;
        self.staged.projects.insert(project.id(), project.clone());
        Ok(())
    }

    async fn lock_pipeline(&mut self, id: PipelineId) -> Result<Option<Pipeline>, RepositoryError> {
        Ok(self.staged.pipelines.get(&id).cloned())
    }

    async fn lock_pipeline_version(
        &mut self,
        id: PipelineVersionId,
    ) -> Result<Option<PipelineVersion>, RepositoryError> {
        Ok(self.staged.pipeline_versions.get(&id).cloned())
    }

    async fn insert_pipeline(&mut self, pipeline: &Pipeline) -> Result<(), RepositoryError> {
        self.require_project(pipeline.project_id())?;
        pipeline
            .validate_snapshot()
            .map_err(|_| invalid("invalid pipeline snapshot"))?;
        if self.staged.pipelines.contains_key(&pipeline.id())
            || (pipeline.deleted_at().is_none()
                && self.staged.pipelines.values().any(|stored| {
                    stored.project_id() == pipeline.project_id()
                        && stored.name() == pipeline.name()
                        && stored.deleted_at().is_none()
                }))
        {
            return Err(invalid("duplicate pipeline"));
        }
        self.staged
            .pipelines
            .insert(pipeline.id(), pipeline.clone());
        Ok(())
    }

    async fn insert_pipeline_version(
        &mut self,
        version: &PipelineVersion,
    ) -> Result<(), RepositoryError> {
        version
            .validate_snapshot()
            .map_err(|_| invalid("invalid pipeline version"))?;
        if !self.staged.pipelines.contains_key(&version.pipeline_id()) {
            return Err(invalid("missing pipeline"));
        }
        if self.staged.pipeline_versions.contains_key(&version.id())
            || self.staged.pipeline_versions.values().any(|v| {
                v.pipeline_id() == version.pipeline_id() && v.version() == version.version()
            })
        {
            return Err(invalid("duplicate pipeline version"));
        }
        self.staged
            .pipeline_versions
            .insert(version.id(), version.clone());
        Ok(())
    }

    async fn update_pipeline(
        &mut self,
        pipeline: &Pipeline,
        expected_revision: u64,
    ) -> Result<(), RepositoryError> {
        self.persist_pipeline_revision(pipeline, expected_revision)
    }

    async fn insert_employee(&mut self, employee: &Employee) -> Result<(), RepositoryError> {
        self.persist_new_employee(employee)
    }

    async fn lock_employee(
        &mut self,
        id: forge_domain::EmployeeId,
    ) -> Result<Option<Employee>, RepositoryError> {
        Ok(self.staged.employees.get(&id).cloned())
    }

    async fn update_employee(
        &mut self,
        employee: &Employee,
        expected_revision: u64,
    ) -> Result<(), RepositoryError> {
        self.persist_employee_revision(employee, expected_revision)
    }

    async fn lock_task(&mut self, id: TaskId) -> Result<Option<StoredTask>, RepositoryError> {
        Ok(self.staged.tasks.get(&id).cloned())
    }

    async fn insert_task(
        &mut self,
        task: &Task,
        persistence: TaskPersistence,
    ) -> Result<(), RepositoryError> {
        self.validate_git_binding(task, persistence)?;
        self.require_project(task.project_id())?;
        task.validate_snapshot()
            .map_err(|_| invalid("invalid task snapshot"))?;
        if self.staged.tasks.contains_key(&task.id())
            || self
                .staged
                .tasks
                .values()
                .any(|s| s.task.project_id() == task.project_id() && s.task.key() == task.key())
        {
            return Err(invalid("duplicate task identity or sequence"));
        }
        self.staged.tasks.insert(
            task.id(),
            StoredTask {
                task: task.clone(),
                persistence,
            },
        );
        Ok(())
    }

    async fn update_task(
        &mut self,
        task: &Task,
        persistence: TaskPersistence,
        expected_revision: u64,
    ) -> Result<(), RepositoryError> {
        self.validate_git_binding(task, persistence)?;
        if task.revision().get() <= expected_revision {
            return Err(invalid("task revision must advance"));
        }
        if self
            .staged
            .tasks
            .get(&task.id())
            .is_none_or(|s| s.task.revision().get() != expected_revision)
        {
            return Err(RepositoryError::StaleRevision { aggregate: "task" });
        }
        task.validate_snapshot()
            .map_err(|_| invalid("invalid task snapshot"))?;
        self.staged.tasks.insert(
            task.id(),
            StoredTask {
                task: task.clone(),
                persistence,
            },
        );
        Ok(())
    }

    async fn list_dependencies(
        &mut self,
        project: ProjectId,
    ) -> Result<Vec<TaskDependency>, RepositoryError> {
        Ok(self
            .staged
            .dependencies
            .iter()
            .filter(|((p, _, _), _)| *p == project)
            .map(|(_, (edge, _))| *edge)
            .collect())
    }

    async fn insert_dependency(
        &mut self,
        project: ProjectId,
        dependency: TaskDependency,
        actor: Actor,
    ) -> Result<bool, RepositoryError> {
        self.require_task_scope(project, dependency.blocker_task_id())?;
        self.require_task_scope(project, dependency.blocked_task_id())?;
        let key = (
            project,
            dependency.blocker_task_id(),
            dependency.blocked_task_id(),
        );
        if self.staged.dependencies.contains_key(&key) {
            return Ok(false);
        }
        self.staged.dependencies.insert(key, (dependency, actor));
        Ok(true)
    }

    async fn remove_dependency(
        &mut self,
        project: ProjectId,
        blocker: TaskId,
        blocked: TaskId,
    ) -> Result<bool, RepositoryError> {
        Ok(self
            .staged
            .dependencies
            .remove(&(project, blocker, blocked))
            .is_some())
    }

    async fn lock_artifact(
        &mut self,
        id: ArtifactId,
    ) -> Result<Option<StoredArtifact>, RepositoryError> {
        Ok(self.staged.artifacts.get(&id).cloned())
    }

    async fn insert_artifact(
        &mut self,
        artifact: &Artifact,
        location: &ArtifactLocation,
    ) -> Result<(), RepositoryError> {
        self.require_project(artifact.project_id())?;
        artifact
            .validate_snapshot()
            .map_err(|_| invalid("invalid artifact snapshot"))?;
        if let Some(task) = location.task_id {
            self.require_task_scope(artifact.project_id(), task)?;
        }
        if self.staged.artifacts.contains_key(&artifact.id()) {
            return Err(invalid("duplicate immutable artifact"));
        }
        self.staged.artifacts.insert(
            artifact.id(),
            StoredArtifact {
                artifact: artifact.clone(),
                location: location.clone(),
            },
        );
        Ok(())
    }

    async fn enqueue(&mut self, input: &QueueEntryInput) -> Result<bool, RepositoryError> {
        self.require_task_scope(input.project_id, input.task_id)?;
        let same_task = |entry: &&MemoryQueueEntry| {
            entry.input.project_id == input.project_id && entry.input.task_id == input.task_id
        };
        if self.staged.queue.iter().filter(same_task).any(|e| {
            e.input.stage_id == input.stage_id && e.input.task_revision == input.task_revision
        }) {
            return Ok(false);
        }
        if self
            .staged
            .queue
            .iter()
            .filter(same_task)
            .any(|e| matches!(e.state, MemoryQueueState::Queued | MemoryQueueState::Leased))
        {
            return Err(invalid("task already has active queue intent"));
        }
        self.staged.queue.push(MemoryQueueEntry {
            id: Uuid::now_v7(),
            input: input.clone(),
            state: MemoryQueueState::Queued,
        });
        Ok(true)
    }

    async fn invalidate_queued_entries_for_task(
        &mut self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<u64, RepositoryError> {
        let mut affected = 0;
        for entry in &mut self.staged.queue {
            if entry.input.project_id == project
                && entry.input.task_id == task
                && entry.state == MemoryQueueState::Queued
            {
                entry.state = MemoryQueueState::Cancelled;
                affected += 1;
            }
        }
        Ok(affected)
    }

    async fn lock_active_runs_for_project(
        &mut self,
        project: ProjectId,
    ) -> Result<Vec<ActiveRun>, RepositoryError> {
        Ok(self
            .staged
            .runs
            .iter()
            .filter(|r| r.scope.project_id == project && (r.lease_active || r.reservation_held))
            .map(|r| r.scope.clone())
            .collect())
    }

    async fn lock_active_runs_for_task(
        &mut self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<Vec<ActiveRun>, RepositoryError> {
        Ok(self
            .staged
            .runs
            .iter()
            .filter(|r| {
                r.scope.project_id == project
                    && (r
                        .scope
                        .assignment
                        .task_stage()
                        .is_some_and(|owner| owner.task_id == task)
                        || r.scope
                            .assignment
                            .hook()
                            .is_some_and(|owner| owner.task_id == task))
                    && (r.lease_active || r.reservation_held)
            })
            .map(|r| r.scope.clone())
            .collect())
    }

    async fn request_run_stop(
        &mut self,
        run: Uuid,
        fence: u64,
        epoch: u64,
        force: bool,
    ) -> Result<bool, RepositoryError> {
        let Some(run) = self.staged.runs.iter_mut().find(|r| {
            r.scope.id == run
                && r.scope.lease_fencing_token == fence
                && r.scope.environment_epoch == epoch
        }) else {
            return Ok(false);
        };
        if run
            .stop_requested
            .is_some_and(|already_force| already_force || !force)
        {
            return Ok(false);
        }
        run.stop_requested = Some(force);
        Ok(true)
    }

    async fn cancel_leased_queue_for_fenced_run(
        &mut self,
        run: Uuid,
        fence: u64,
        epoch: u64,
    ) -> Result<bool, RepositoryError> {
        let Some(run) = self.staged.runs.iter().find(|r| {
            r.scope.id == run
                && r.scope.lease_fencing_token == fence
                && r.scope.environment_epoch == epoch
                && r.lease_active
        }) else {
            return Ok(false);
        };
        let Some(queue) = self
            .staged
            .queue
            .iter_mut()
            .find(|q| q.id == run.queue_entry_id && q.state == MemoryQueueState::Leased)
        else {
            return Ok(false);
        };
        queue.state = MemoryQueueState::Cancelled;
        Ok(true)
    }

    async fn set_recovery_hold(
        &mut self,
        project: ProjectId,
        hold: bool,
    ) -> Result<(), RepositoryError> {
        self.require_project(project)?;
        if hold {
            self.staged.recovery_holds.insert(project);
        } else {
            self.staged.recovery_holds.remove(&project);
        }
        Ok(())
    }

    async fn lookup_idempotency(
        &mut self,
        project: ProjectId,
        key: &str,
    ) -> Result<Option<IdempotencyRecord>, RepositoryError> {
        Ok(self
            .staged
            .idempotency
            .get(&(project, key.to_owned()))
            .cloned())
    }

    async fn insert_idempotency(
        &mut self,
        record: &IdempotencyRecord,
    ) -> Result<(), RepositoryError> {
        self.require_project(record.project_id)?;
        let key = (record.project_id, record.key.clone());
        if self.staged.idempotency.contains_key(&key)
            || self
                .staged
                .idempotency
                .values()
                .any(|existing| existing.command_id == record.command_id)
        {
            return Err(invalid("duplicate idempotency key or command identity"));
        }
        if record.key.trim().is_empty()
            || record.command_name.trim().is_empty()
            || record.request_hash.trim().is_empty()
            || !record.actor.is_object()
            || !record.receipt.is_object()
        {
            return Err(invalid("invalid idempotency record"));
        }
        self.staged.idempotency.insert(key, record.clone());
        Ok(())
    }

    async fn append_event_and_outbox(
        &mut self,
        event: &DomainEvent,
    ) -> Result<EventId, RepositoryError> {
        self.require_project(event.project_id())?;
        self.append_audit(event)
    }
}
