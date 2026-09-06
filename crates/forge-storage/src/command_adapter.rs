//! PostgreSQL adapter for the shared borrowed command transaction.
//!
//! Existing SQL owns locks, constraints and serialization. Core still owns
//! commit and rollback; the application engine cannot publish a partial command.

use forge_application::{CommandTransaction, RepositoryError, ports::*};
use forge_domain::{
    Actor, Artifact, ArtifactId, DomainEvent, Employee, EventId, Pipeline, PipelineId,
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
            task_id: run.task_id,
            lease_fencing_token: run.lease_fencing_token,
            environment_epoch: run.environment_epoch,
        }
    }
}

impl CommandTransaction for StorageTransaction<'_> {
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

    async fn insert_employee(&mut self, employee: &Employee) -> Result<(), RepositoryError> {
        StorageTransaction::insert_employee(self, employee)
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
