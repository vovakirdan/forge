//! PostgreSQL adapter for the shared borrowed command transaction.
//!
//! Existing SQL owns locks, constraints and serialization. Core still owns
//! commit and rollback; the application engine cannot publish a partial command.

use forge_application::{CommandTransaction, RepositoryError, ports::*};
use forge_domain::{
    Actor, Artifact, ArtifactId, DomainEvent, Employee, EmployeeId, EventId, Pipeline, PipelineId,
    PipelineVersion, PipelineVersionId, Project, ProjectId, Task, TaskDependency, TaskId,
};
use uuid::Uuid;

use crate::{FencedWrite, RunProjection, StorageError, StorageTransaction};

impl From<StorageError> for RepositoryError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::NotFound { aggregate } => Self::NotFound { aggregate },
            StorageError::StaleRevision { aggregate } => Self::StaleRevision { aggregate },
            StorageError::InvalidInput { reason } => Self::InvalidInput { reason },
            _ => Self::Unavailable,
        }
    }
}

impl From<&RunProjection> for ActiveRun {
    fn from(run: &RunProjection) -> Self {
        Self {
            id: run.id,
            project_id: run.project_id,
            assignment: run.assignment.clone(),
            stage_visit: serde_json::from_value::<forge_domain::ContextSnapshot>(
                run.context_manifest.clone(),
            )
            .ok()
            .filter(|context| {
                run.assignment.task_stage().is_some_and(|owner| {
                    let data = context.data();
                    data.run_id == run.id
                        && data.project_id == run.project_id
                        && Some(data.employee_id) == run.employee_id
                        && data.task_id == owner.task_id
                        && data.stage_id == owner.stage_id
                })
            })
            .and_then(|context| context.data().stage_visit)
            .and_then(|visit| serde_json::from_value(serde_json::json!(visit)).ok()),
            employee_id: run.employee_id,
            lease_fencing_token: run.lease_fencing_token,
            environment_epoch: run.environment_epoch,
        }
    }
}

impl CommandTransaction for StorageTransaction<'_> {
    async fn insert_command_handoff(
        &mut self,
        handoff: &forge_domain::TaskHandoff,
    ) -> Result<(), RepositoryError> {
        Ok(StorageTransaction::insert_command_handoff(self, handoff).await?)
    }
    async fn required_hooks_satisfied(
        &mut self,
        task: &Task,
        version: &PipelineVersion,
        candidate_override: Option<Uuid>,
    ) -> Result<bool, RepositoryError> {
        Ok(
            StorageTransaction::required_hooks_satisfied(self, task, version, candidate_override)
                .await?,
        )
    }
    async fn communication_escalation_is_current(
        &mut self,
        escalation: &forge_domain::resolution::Escalation,
    ) -> Result<bool, RepositoryError> {
        StorageTransaction::communication_escalation_is_current(self, escalation)
            .await
            .map_err(Into::into)
    }
    async fn load_escalation_for_wait(
        &mut self,
        project: ProjectId,
        task: TaskId,
        wait: forge_domain::WaitConditionId,
    ) -> Result<Option<forge_domain::resolution::Escalation>, RepositoryError> {
        StorageTransaction::load_escalation_for_wait(self, project, task, wait)
            .await
            .map_err(Into::into)
    }
    async fn load_resolver_route(
        &mut self,
        project: ProjectId,
        key: &str,
    ) -> Result<Option<forge_domain::resolution::ResolverRoute>, RepositoryError> {
        StorageTransaction::load_resolver_route(self, project, key)
            .await
            .map_err(Into::into)
    }
    async fn save_resolver_route(
        &mut self,
        route: &forge_domain::resolution::ResolverRoute,
        expected: Option<u64>,
    ) -> Result<(), RepositoryError> {
        StorageTransaction::save_resolver_route(self, route, expected)
            .await
            .map_err(Into::into)
    }
    async fn load_escalation(
        &mut self,
        id: Uuid,
    ) -> Result<Option<forge_domain::resolution::Escalation>, RepositoryError> {
        StorageTransaction::load_escalation(self, id)
            .await
            .map_err(Into::into)
    }
    async fn save_escalation(
        &mut self,
        value: &forge_domain::resolution::Escalation,
        expected: Option<u64>,
    ) -> Result<(), RepositoryError> {
        StorageTransaction::save_escalation(self, value, expected)
            .await
            .map_err(Into::into)
    }
    async fn load_resolution_assignment(
        &mut self,
        id: Uuid,
    ) -> Result<Option<forge_domain::resolution::ResolutionAssignment>, RepositoryError> {
        StorageTransaction::load_resolution_assignment(self, id)
            .await
            .map_err(Into::into)
    }
    async fn insert_resolution_assignment(
        &mut self,
        value: &forge_domain::resolution::ResolutionAssignment,
    ) -> Result<(), RepositoryError> {
        StorageTransaction::insert_resolution_assignment(self, value)
            .await
            .map_err(Into::into)
    }
    async fn update_resolution_assignment(
        &mut self,
        value: &forge_domain::resolution::ResolutionAssignment,
    ) -> Result<(), RepositoryError> {
        StorageTransaction::update_resolution_assignment(self, value)
            .await
            .map_err(Into::into)
    }
    async fn lock_finding(
        &mut self,
        id: Uuid,
    ) -> Result<Option<forge_domain::finding::Finding>, RepositoryError> {
        Ok(StorageTransaction::lock_finding(self, id).await?)
    }
    async fn insert_finding(
        &mut self,
        finding: &forge_domain::finding::Finding,
    ) -> Result<(), RepositoryError> {
        Ok(StorageTransaction::insert_finding(self, finding).await?)
    }
    async fn update_finding(
        &mut self,
        finding: &forge_domain::finding::Finding,
        expected: u64,
    ) -> Result<(), RepositoryError> {
        Ok(StorageTransaction::update_finding(self, finding, expected).await?)
    }
    async fn lock_task_resume_schedule(
        &mut self,
        id: Uuid,
    ) -> Result<Option<forge_domain::TaskResumeSchedule>, RepositoryError> {
        StorageTransaction::lock_task_resume_schedule(self, id)
            .await
            .map_err(Into::into)
    }
    async fn insert_task_resume_schedule(
        &mut self,
        schedule: &forge_domain::TaskResumeSchedule,
    ) -> Result<(), RepositoryError> {
        StorageTransaction::insert_task_resume_schedule(self, schedule)
            .await
            .map_err(Into::into)
    }
    async fn update_task_resume_schedule(
        &mut self,
        schedule: &forge_domain::TaskResumeSchedule,
    ) -> Result<(), RepositoryError> {
        StorageTransaction::update_task_resume_schedule(self, schedule)
            .await
            .map_err(Into::into)
    }
    async fn load_task_dispatch_constraint(
        &mut self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<Option<forge_domain::NextRunEmployeeConstraint>, RepositoryError> {
        StorageTransaction::load_task_dispatch_constraint(self, project, task)
            .await
            .map_err(Into::into)
    }
    async fn insert_task_dispatch_constraint(
        &mut self,
        constraint: &forge_domain::NextRunEmployeeConstraint,
    ) -> Result<(), RepositoryError> {
        StorageTransaction::insert_task_dispatch_constraint(self, constraint)
            .await
            .map_err(Into::into)
    }
    async fn update_task_dispatch_constraint(
        &mut self,
        constraint: &forge_domain::NextRunEmployeeConstraint,
        expected: &forge_domain::NextRunConstraintState,
    ) -> Result<(), RepositoryError> {
        StorageTransaction::update_task_dispatch_constraint(self, constraint, expected)
            .await
            .map_err(Into::into)
    }
    async fn lock_active_runs_for_employee(
        &mut self,
        project: ProjectId,
        employee: EmployeeId,
    ) -> Result<Vec<ActiveRun>, RepositoryError> {
        StorageTransaction::lock_active_runs_for_employee(self, project, employee)
            .await
            .map(|runs| runs.iter().map(ActiveRun::from).collect())
            .map_err(Into::into)
    }
    async fn revoke_run_lease(
        &mut self,
        run: Uuid,
        fence: u64,
        epoch: u64,
    ) -> Result<bool, RepositoryError> {
        StorageTransaction::revoke_run_lease(self, run, fence, epoch)
            .await
            .map_err(Into::into)
    }
    async fn insert_message_requirement_waiver(
        &mut self,
        waiver: &forge_domain::communication::MessageRequirementWaiver,
    ) -> Result<(), RepositoryError> {
        StorageTransaction::insert_message_requirement_waiver(self, waiver)
            .await
            .map_err(Into::into)
    }
    async fn load_project_repository(
        &mut self,
        id: Uuid,
    ) -> Result<Option<forge_domain::ProjectRepository>, RepositoryError> {
        StorageTransaction::load_project_repository(self, id)
            .await
            .map_err(Into::into)
    }
    async fn insert_project_repository(
        &mut self,
        repository: &forge_domain::ProjectRepository,
    ) -> Result<(), RepositoryError> {
        StorageTransaction::insert_project_repository(self, repository)
            .await
            .map_err(Into::into)
    }
    async fn task_git_source_setting(
        &mut self,
        project_id: forge_domain::ProjectId,
        task_id: forge_domain::TaskId,
    ) -> Result<forge_domain::git::TaskGitSourceSetting, RepositoryError> {
        StorageTransaction::task_git_source_setting(self, project_id, task_id)
            .await
            .map_err(Into::into)
    }
    async fn insert_task_git_source_setting(
        &mut self,
        project_id: forge_domain::ProjectId,
        task_id: forge_domain::TaskId,
        setting: &forge_domain::git::TaskGitSourceSetting,
    ) -> Result<(), RepositoryError> {
        StorageTransaction::insert_task_git_source_setting(self, project_id, task_id, setting)
            .await
            .map_err(Into::into)
    }
    async fn lock_employee_thread(
        &mut self,
        id: Uuid,
    ) -> Result<Option<forge_domain::communication::EmployeeThread>, RepositoryError> {
        StorageTransaction::lock_employee_thread(self, id)
            .await
            .map_err(Into::into)
    }
    async fn insert_employee_thread(
        &mut self,
        thread: &forge_domain::communication::EmployeeThread,
    ) -> Result<(), RepositoryError> {
        StorageTransaction::insert_employee_thread(self, thread)
            .await
            .map_err(Into::into)
    }
    async fn update_employee_thread(
        &mut self,
        thread: &forge_domain::communication::EmployeeThread,
        expected_revision: u64,
    ) -> Result<(), RepositoryError> {
        StorageTransaction::update_employee_thread(self, thread, expected_revision)
            .await
            .map_err(Into::into)
    }
    async fn load_employee_message(
        &mut self,
        id: Uuid,
    ) -> Result<Option<forge_domain::communication::EmployeeMessage>, RepositoryError> {
        StorageTransaction::load_employee_message(self, id)
            .await
            .map_err(Into::into)
    }
    async fn insert_employee_message(
        &mut self,
        message: &forge_domain::communication::EmployeeMessage,
    ) -> Result<(), RepositoryError> {
        StorageTransaction::insert_employee_message(self, message)
            .await
            .map_err(Into::into)
    }
    async fn lock_project_creation(&mut self, id: ProjectId) -> Result<(), RepositoryError> {
        StorageTransaction::lock_project_creation(self, id)
            .await
            .map_err(Into::into)
    }

    async fn lock_project(&mut self, id: ProjectId) -> Result<Option<Project>, RepositoryError> {
        StorageTransaction::lock_project(self, id)
            .await
            .map_err(Into::into)
    }

    async fn insert_project(&mut self, project: &Project) -> Result<(), RepositoryError> {
        StorageTransaction::insert_project(self, project)
            .await
            .map_err(Into::into)
    }

    async fn update_project(
        &mut self,
        project: &Project,
        expected_revision: u64,
    ) -> Result<(), RepositoryError> {
        StorageTransaction::update_project(self, project, expected_revision)
            .await
            .map_err(Into::into)
    }

    async fn lock_pipeline(&mut self, id: PipelineId) -> Result<Option<Pipeline>, RepositoryError> {
        StorageTransaction::lock_pipeline(self, id)
            .await
            .map_err(Into::into)
    }

    async fn lock_pipeline_version(
        &mut self,
        id: PipelineVersionId,
    ) -> Result<Option<PipelineVersion>, RepositoryError> {
        StorageTransaction::lock_pipeline_version(self, id)
            .await
            .map_err(Into::into)
    }

    async fn insert_pipeline(&mut self, pipeline: &Pipeline) -> Result<(), RepositoryError> {
        StorageTransaction::insert_pipeline(self, pipeline)
            .await
            .map_err(Into::into)
    }

    async fn insert_pipeline_version(
        &mut self,
        version: &PipelineVersion,
    ) -> Result<(), RepositoryError> {
        StorageTransaction::insert_pipeline_version(self, version)
            .await
            .map_err(Into::into)
    }

    async fn update_pipeline(
        &mut self,
        pipeline: &Pipeline,
        expected_revision: u64,
    ) -> Result<(), RepositoryError> {
        StorageTransaction::update_pipeline(self, pipeline, expected_revision)
            .await
            .map_err(Into::into)
    }

    async fn insert_employee(&mut self, employee: &Employee) -> Result<(), RepositoryError> {
        StorageTransaction::insert_employee(self, employee)
            .await
            .map_err(Into::into)
    }

    async fn lock_employee(&mut self, id: EmployeeId) -> Result<Option<Employee>, RepositoryError> {
        StorageTransaction::lock_employee(self, id)
            .await
            .map(|stored| stored.map(|stored| stored.employee))
            .map_err(Into::into)
    }

    async fn update_employee(
        &mut self,
        employee: &Employee,
        expected_revision: u64,
    ) -> Result<(), RepositoryError> {
        StorageTransaction::update_employee(self, employee, expected_revision)
            .await
            .map_err(Into::into)
    }

    async fn lock_task(&mut self, id: TaskId) -> Result<Option<StoredTask>, RepositoryError> {
        StorageTransaction::lock_task(self, id)
            .await
            .map_err(Into::into)
    }

    async fn insert_task(
        &mut self,
        task: &Task,
        persistence: TaskPersistence,
    ) -> Result<(), RepositoryError> {
        StorageTransaction::insert_task(self, task, persistence)
            .await
            .map_err(Into::into)
    }

    async fn update_task(
        &mut self,
        task: &Task,
        persistence: TaskPersistence,
        expected_revision: u64,
    ) -> Result<(), RepositoryError> {
        StorageTransaction::update_task(self, task, persistence, expected_revision)
            .await
            .map_err(Into::into)
    }

    async fn list_dependencies(
        &mut self,
        project: ProjectId,
    ) -> Result<Vec<TaskDependency>, RepositoryError> {
        StorageTransaction::list_dependencies(self, project)
            .await
            .map_err(Into::into)
    }

    async fn insert_dependency(
        &mut self,
        project: ProjectId,
        dependency: TaskDependency,
        actor: Actor,
    ) -> Result<bool, RepositoryError> {
        StorageTransaction::insert_dependency(self, project, dependency, actor)
            .await
            .map_err(Into::into)
    }

    async fn remove_dependency(
        &mut self,
        project: ProjectId,
        blocker: TaskId,
        blocked: TaskId,
    ) -> Result<bool, RepositoryError> {
        StorageTransaction::remove_dependency(self, project, blocker, blocked)
            .await
            .map_err(Into::into)
    }

    async fn lock_artifact(
        &mut self,
        id: ArtifactId,
    ) -> Result<Option<StoredArtifact>, RepositoryError> {
        StorageTransaction::lock_artifact(self, id)
            .await
            .map_err(Into::into)
    }

    async fn insert_artifact(
        &mut self,
        artifact: &Artifact,
        location: &ArtifactLocation,
    ) -> Result<(), RepositoryError> {
        StorageTransaction::insert_artifact(self, artifact, location)
            .await
            .map_err(Into::into)
    }

    async fn enqueue(&mut self, input: &QueueEntryInput) -> Result<bool, RepositoryError> {
        StorageTransaction::enqueue(self, input)
            .await
            .map(|entry| entry.is_some())
            .map_err(Into::into)
    }

    async fn invalidate_queued_entries_for_task(
        &mut self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<u64, RepositoryError> {
        StorageTransaction::invalidate_queued_entries_for_task(self, project, task)
            .await
            .map_err(Into::into)
    }

    async fn lock_active_runs_for_project(
        &mut self,
        project: ProjectId,
    ) -> Result<Vec<ActiveRun>, RepositoryError> {
        StorageTransaction::lock_active_runs_for_project(self, project)
            .await
            .map(|runs| runs.iter().map(ActiveRun::from).collect())
            .map_err(Into::into)
    }

    async fn lock_active_runs_for_task(
        &mut self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<Vec<ActiveRun>, RepositoryError> {
        StorageTransaction::lock_active_runs_for_task(self, project, task)
            .await
            .map(|runs| runs.iter().map(ActiveRun::from).collect())
            .map_err(Into::into)
    }

    async fn request_run_stop(
        &mut self,
        run: Uuid,
        fence: u64,
        epoch: u64,
        force: bool,
    ) -> Result<bool, RepositoryError> {
        StorageTransaction::request_run_stop(self, run, fence, epoch, force)
            .await
            .map(|write| write == FencedWrite::Applied)
            .map_err(Into::into)
    }

    async fn cancel_leased_queue_for_fenced_run(
        &mut self,
        run: Uuid,
        fence: u64,
        epoch: u64,
    ) -> Result<bool, RepositoryError> {
        StorageTransaction::cancel_leased_queue_for_fenced_run(self, run, fence, epoch)
            .await
            .map(|write| write == FencedWrite::Applied)
            .map_err(Into::into)
    }

    async fn set_recovery_hold(
        &mut self,
        project: ProjectId,
        hold: bool,
    ) -> Result<(), RepositoryError> {
        StorageTransaction::set_recovery_hold(self, project, hold)
            .await
            .map_err(Into::into)
    }

    async fn lookup_idempotency(
        &mut self,
        project: ProjectId,
        key: &str,
    ) -> Result<Option<IdempotencyRecord>, RepositoryError> {
        StorageTransaction::lookup_idempotency(self, project, key)
            .await
            .map_err(Into::into)
    }

    async fn insert_idempotency(
        &mut self,
        record: &IdempotencyRecord,
    ) -> Result<(), RepositoryError> {
        StorageTransaction::insert_idempotency(self, record)
            .await
            .map_err(Into::into)
    }

    async fn append_event_and_outbox(
        &mut self,
        event: &DomainEvent,
    ) -> Result<EventId, RepositoryError> {
        StorageTransaction::append_event_and_outbox(self, event)
            .await
            .map(|event| event.id)
            .map_err(Into::into)
    }
}
