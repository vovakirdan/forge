//! Borrowed atomic command repository. Its owner alone commits or rolls back.
use super::RepositoryError;
pub use super::records::*;
use forge_domain::{
    Actor, Artifact, ArtifactId, DomainEvent, Employee, EmployeeId, EventId, Pipeline, PipelineId,
    PipelineVersion, PipelineVersionId, Project, ProjectId, Task, TaskDependency, TaskId,
};
use std::future::Future;
use uuid::Uuid;

/// Every future is Send so the production command path remains usable by Axum.
/// Reads after writes see staged data. The owner must roll back on any command error;
/// dropping an uncommitted transaction publishes nothing. No method commits or dispatches.
/// Project locks serialize all child writes; adapters must not reimplement command decisions.
pub trait CommandTransaction: Send {
    fn insert_command_handoff(
        &mut self,
        handoff: &forge_domain::TaskHandoff,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    fn required_hooks_satisfied(
        &mut self,
        task: &Task,
        version: &PipelineVersion,
        candidate_override: Option<Uuid>,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;
    fn communication_escalation_is_current(
        &mut self,
        escalation: &forge_domain::resolution::Escalation,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;
    fn load_escalation_for_wait(
        &mut self,
        project: ProjectId,
        task: TaskId,
        wait: forge_domain::WaitConditionId,
    ) -> impl Future<Output = Result<Option<forge_domain::resolution::Escalation>, RepositoryError>> + Send;
    fn load_resolver_route(
        &mut self,
        project: ProjectId,
        key: &str,
    ) -> impl Future<
        Output = Result<Option<forge_domain::resolution::ResolverRoute>, RepositoryError>,
    > + Send;
    fn save_resolver_route(
        &mut self,
        route: &forge_domain::resolution::ResolverRoute,
        expected: Option<u64>,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    fn load_escalation(
        &mut self,
        id: Uuid,
    ) -> impl Future<Output = Result<Option<forge_domain::resolution::Escalation>, RepositoryError>> + Send;
    fn save_escalation(
        &mut self,
        value: &forge_domain::resolution::Escalation,
        expected: Option<u64>,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    fn load_resolution_assignment(
        &mut self,
        id: Uuid,
    ) -> impl Future<
        Output = Result<Option<forge_domain::resolution::ResolutionAssignment>, RepositoryError>,
    > + Send;
    fn insert_resolution_assignment(
        &mut self,
        value: &forge_domain::resolution::ResolutionAssignment,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    fn update_resolution_assignment(
        &mut self,
        value: &forge_domain::resolution::ResolutionAssignment,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    fn lock_finding(
        &mut self,
        id: Uuid,
    ) -> impl Future<Output = Result<Option<forge_domain::finding::Finding>, RepositoryError>> + Send;
    fn insert_finding(
        &mut self,
        finding: &forge_domain::finding::Finding,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    fn update_finding(
        &mut self,
        finding: &forge_domain::finding::Finding,
        expected: u64,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    /// Locks one durable alarm after its Project gate is held.
    fn lock_task_resume_schedule(
        &mut self,
        id: Uuid,
    ) -> impl Future<Output = Result<Option<forge_domain::TaskResumeSchedule>, RepositoryError>> + Send;
    /// Stages a single pending alarm for an exact wait.
    fn insert_task_resume_schedule(
        &mut self,
        schedule: &forge_domain::TaskResumeSchedule,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    /// Advances pending to exactly one immutable terminal result.
    fn update_task_resume_schedule(
        &mut self,
        schedule: &forge_domain::TaskResumeSchedule,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    /// Load the current retained one-shot constraint under the Project lock.
    fn load_task_dispatch_constraint(
        &mut self,
        project: ProjectId,
        task: TaskId,
    ) -> impl Future<
        Output = Result<Option<forge_domain::NextRunEmployeeConstraint>, RepositoryError>,
    > + Send;
    fn insert_task_dispatch_constraint(
        &mut self,
        constraint: &forge_domain::NextRunEmployeeConstraint,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    fn update_task_dispatch_constraint(
        &mut self,
        constraint: &forge_domain::NextRunEmployeeConstraint,
        expected: &forge_domain::NextRunConstraintState,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    /// Persists an explicit operator waiver without modifying the original message.
    fn insert_message_requirement_waiver(
        &mut self,
        waiver: &forge_domain::communication::MessageRequirementWaiver,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    /// Reads an immutable operator-registered source under the Project lock.
    fn load_project_repository(
        &mut self,
        id: Uuid,
    ) -> impl Future<Output = Result<Option<forge_domain::ProjectRepository>, RepositoryError>> + Send;
    /// Registers a new immutable source; duplicate Project names are rejected.
    fn insert_project_repository(
        &mut self,
        repository: &forge_domain::ProjectRepository,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    /// Reads the latest future-only source policy, or its revision-one default.
    fn task_git_source_setting(
        &mut self,
        project_id: ProjectId,
        task_id: TaskId,
    ) -> impl Future<Output = Result<forge_domain::git::TaskGitSourceSetting, RepositoryError>> + Send;
    /// Appends an immutable policy revision under the Project lock.
    fn insert_task_git_source_setting(
        &mut self,
        project_id: ProjectId,
        task_id: TaskId,
        setting: &forge_domain::git::TaskGitSourceSetting,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    /// Locks a conversation under its already-locked Project.
    fn lock_employee_thread(
        &mut self,
        id: Uuid,
    ) -> impl Future<
        Output = Result<Option<forge_domain::communication::EmployeeThread>, RepositoryError>,
    > + Send;
    /// Stages a new independently addressed conversation.
    fn insert_employee_thread(
        &mut self,
        thread: &forge_domain::communication::EmployeeThread,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    /// Advances the conversation sequence with optimistic revision protection.
    fn update_employee_thread(
        &mut self,
        thread: &forge_domain::communication::EmployeeThread,
        expected_revision: u64,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    /// Reads a prior immutable message for reply-scope validation.
    fn load_employee_message(
        &mut self,
        id: Uuid,
    ) -> impl Future<
        Output = Result<Option<forge_domain::communication::EmployeeMessage>, RepositoryError>,
    > + Send;
    /// Stages one immutable message; duplicate IDs/sequences are rejected.
    fn insert_employee_message(
        &mut self,
        message: &forge_domain::communication::EmployeeMessage,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
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
    /// Stages exactly the next mutable catalog revision, retaining every graph.
    fn update_pipeline(
        &mut self,
        pipeline: &Pipeline,
        expected_revision: u64,
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
    /// Locks an Employee catalog row without acquiring authority over its Runs.
    fn lock_employee(
        &mut self,
        id: EmployeeId,
    ) -> impl Future<Output = Result<Option<Employee>, RepositoryError>> + Send;
    /// Stages the next Employee revision only when the observed revision still matches.
    fn update_employee(
        &mut self,
        employee: &Employee,
        expected_revision: u64,
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
    /// Locks every Employee-owned execution whose logical or physical ownership remains.
    fn lock_active_runs_for_employee(
        &mut self,
        project: ProjectId,
        employee: forge_domain::EmployeeId,
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
    /// Revokes authority only after a matching forced stop; physical reservations remain held.
    fn revoke_run_lease(
        &mut self,
        run: Uuid,
        fence: u64,
        epoch: u64,
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
