//! Borrowed atomic command repository. Its owner alone commits or rolls back.
use super::RepositoryError;
pub use super::records::*;
use forge_domain::{
    Actor, Artifact, ArtifactId, DomainEvent, Employee, EventId, Pipeline, PipelineId,
    PipelineVersion, PipelineVersionId, Project, ProjectId, Task, TaskDependency, TaskId,
};
use std::future::Future;
use uuid::Uuid;

/// Every future is Send so the production command path remains usable by Axum.
/// Reads after writes see staged data. The owner must roll back on any command error;
/// dropping an uncommitted transaction publishes nothing. No method commits or dispatches.
/// Project locks serialize all child writes; adapters must not reimplement command decisions.
pub trait CommandTransaction: Send {
    /// Serializes the reserved Project identity, including before its row exists.
    fn lock_project_creation(
        &mut self,
        id: ProjectId,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    /// Loads and exclusively locks the Project until the owner finishes this transaction.
    fn lock_project(
        &mut self,
        id: ProjectId,
    ) -> impl Future<Output = Result<Option<Project>, RepositoryError>> + Send;
    /// Stages a newly reserved Project; a conflicting identity is rejected.
    fn insert_project(
        &mut self,
        project: &Project,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    /// Stages the next Project snapshot only if its stored revision matches.
    fn update_project(
        &mut self,
        project: &Project,
        expected_revision: u64,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    /// Loads the immutable catalog binding under the transaction's Project lock.
    fn lock_pipeline(
        &mut self,
        id: PipelineId,
    ) -> impl Future<Output = Result<Option<Pipeline>, RepositoryError>> + Send;
    /// Loads the immutable graph version used to validate Task stage transitions.
    fn lock_pipeline_version(
        &mut self,
        id: PipelineVersionId,
    ) -> impl Future<Output = Result<Option<PipelineVersion>, RepositoryError>> + Send;
    /// Stages a new named Pipeline belonging to its Project.
    fn insert_pipeline(
        &mut self,
        pipeline: &Pipeline,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    /// Stages a new immutable Pipeline graph version.
    fn insert_pipeline_version(
        &mut self,
        version: &PipelineVersion,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    /// Stages a new Employee identity without executing any runtime.
    fn insert_employee(
        &mut self,
        employee: &Employee,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    /// Loads and locks the Task and its retained scheduler fields.
    fn lock_task(
        &mut self,
        id: TaskId,
    ) -> impl Future<Output = Result<Option<StoredTask>, RepositoryError>> + Send;
    /// Stages a new Task and its initial scheduler projection.
    fn insert_task(
        &mut self,
        task: &Task,
        persistence: TaskPersistence,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    /// Stages Task and scheduler fields atomically, requiring the stored Task revision.
    fn update_task(
        &mut self,
        task: &Task,
        persistence: TaskPersistence,
        expected_revision: u64,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    /// Lists this Project's dependency edges from the current staged state.
    fn list_dependencies(
        &mut self,
        project: ProjectId,
    ) -> impl Future<Output = Result<Vec<TaskDependency>, RepositoryError>> + Send;
    /// Inserts one directed edge; returns false when that exact edge already exists.
    fn insert_dependency(
        &mut self,
        project: ProjectId,
        dependency: TaskDependency,
        actor: Actor,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;
    /// Removes the exact directed edge; returns whether an edge existed.
    fn remove_dependency(
        &mut self,
        project: ProjectId,
        blocker: TaskId,
        blocked: TaskId,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;
    /// Loads immutable evidence and its attachment context.
    fn lock_artifact(
        &mut self,
        id: ArtifactId,
    ) -> impl Future<Output = Result<Option<StoredArtifact>, RepositoryError>> + Send;
    /// Stages immutable evidence and its Task/Run/stage attachment together.
    fn insert_artifact(
        &mut self,
        artifact: &Artifact,
        location: &ArtifactLocation,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    /// Stages an eligible snapshot; false means its active snapshot identity already exists.
    fn enqueue(
        &mut self,
        input: &QueueEntryInput,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;
    /// Cancels queued snapshots only; leased work and reservations are retained.
    fn invalidate_queued_entries_for_task(
        &mut self,
        project: ProjectId,
        task: TaskId,
    ) -> impl Future<Output = Result<u64, RepositoryError>> + Send;
    /// Locks nonterminal Runs in the Project and returns only their fencing identities.
    fn lock_active_runs_for_project(
        &mut self,
        project: ProjectId,
    ) -> impl Future<Output = Result<Vec<ActiveRun>, RepositoryError>> + Send;
    /// Locks nonterminal Runs for the exact Project-scoped Task.
    fn lock_active_runs_for_task(
        &mut self,
        project: ProjectId,
        task: TaskId,
    ) -> impl Future<Output = Result<Vec<ActiveRun>, RepositoryError>> + Send;
    /// Stages stopping only for the matching fence and epoch; never releases its reservation.
    fn request_run_stop(
        &mut self,
        run: Uuid,
        fence: u64,
        epoch: u64,
        force: bool,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;
    /// Cancels a fenced Run's leased queue entry, retaining Lease and reservation until confirmed exit.
    fn cancel_leased_queue_for_fenced_run(
        &mut self,
        run: Uuid,
        fence: u64,
        epoch: u64,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;
    /// Sets the durable Project recovery gate; start clears it without runtime work here.
    fn set_recovery_hold(
        &mut self,
        project: ProjectId,
        hold: bool,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    /// Reads the Project-scoped cached receipt before optimistic revision validation.
    fn lookup_idempotency(
        &mut self,
        project: ProjectId,
        key: &str,
    ) -> impl Future<Output = Result<Option<IdempotencyRecord>, RepositoryError>> + Send;
    /// Stages a unique Project/key receipt in the same transaction as all command effects.
    fn insert_idempotency(
        &mut self,
        record: &IdempotencyRecord,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    /// Stages one canonical Event and its outbox delivery atomically; returns the Event identity.
    fn append_event_and_outbox(
        &mut self,
        event: &DomainEvent,
    ) -> impl Future<Output = Result<EventId, RepositoryError>> + Send;
}
