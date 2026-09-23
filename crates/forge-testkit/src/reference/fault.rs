//! Shared transaction fault injection for memory and PostgreSQL conformance.

use forge_application::{CommandTransaction, RepositoryError, ports::*};
use forge_domain::{
    Actor, Artifact, ArtifactId, DomainEvent, Employee, EventId, Pipeline, PipelineId,
    PipelineVersion, PipelineVersionId, Project, ProjectId, Task, TaskDependency, TaskId,
};
use uuid::Uuid;

/// Fail after a durable operation has staged changes, not before it runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultPoint {
    /// An aggregate was inserted or updated.
    AfterAggregateWrite,
    /// An event and its outbox row were appended together.
    AfterAuditAppend,
    /// The stable receipt was inserted.
    AfterReceiptInsert,
    /// All command operations succeeded; fail before actual commit.
    BeforeCommit,
}

/// Decorates the actual repository port without reproducing command logic.
pub struct FaultTransaction<T> {
    inner: T,
    point: FaultPoint,
    failed: bool,
}

impl<T> FaultTransaction<T> {
    /// Arms a single failure for this transaction only.
    pub fn new(inner: T, point: FaultPoint) -> Self {
        Self {
            inner,
            point,
            failed: false,
        }
    }

    fn after(&mut self, point: Option<FaultPoint>) -> Result<(), RepositoryError> {
        if self.failed || point == Some(self.point) {
            self.failed = true;
            Err(RepositoryError::Unavailable)
        } else {
            Ok(())
        }
    }

    /// Test driver must call this before taking the transaction to commit.
    pub fn before_commit(&mut self) -> Result<(), RepositoryError> {
        self.after(Some(FaultPoint::BeforeCommit))
    }

    /// Returns the transaction; on any error callers must drop or roll it back.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

macro_rules! forward {
    ($name:ident($($arg:ident: $ty:ty),*) -> $output:ty, $point:expr) => {
        async fn $name(&mut self, $($arg: $ty),*) -> Result<$output, RepositoryError> {
            self.after(None)?;
            let value = self.inner.$name($($arg),*).await?;
            self.after($point)?;
            Ok(value)
        }
    };
}

impl<T: CommandTransaction> CommandTransaction for FaultTransaction<T> {
    forward!(insert_command_handoff(handoff:&forge_domain::TaskHandoff)->(),Some(FaultPoint::AfterAggregateWrite));
    forward!(required_hooks_satisfied(task:&Task,version:&PipelineVersion,candidate_override:Option<Uuid>)->bool,None);
    forward!(communication_escalation_is_current(escalation:&forge_domain::resolution::Escalation)->bool,None);
    forward!(load_escalation_for_wait(project:ProjectId,task:TaskId,wait:forge_domain::WaitConditionId)->Option<forge_domain::resolution::Escalation>,None);
    forward!(load_resolver_route(project: ProjectId, key: &str) -> Option<forge_domain::resolution::ResolverRoute>, None);
    forward!(save_resolver_route(route: &forge_domain::resolution::ResolverRoute, expected: Option<u64>) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(load_escalation(id: Uuid) -> Option<forge_domain::resolution::Escalation>, None);
    forward!(save_escalation(value: &forge_domain::resolution::Escalation, expected: Option<u64>) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(load_resolution_assignment(id: Uuid) -> Option<forge_domain::resolution::ResolutionAssignment>, None);
    forward!(insert_resolution_assignment(value: &forge_domain::resolution::ResolutionAssignment) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(update_resolution_assignment(value: &forge_domain::resolution::ResolutionAssignment) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(lock_finding(id: Uuid) -> Option<forge_domain::finding::Finding>, None);
    forward!(insert_finding(finding: &forge_domain::finding::Finding) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(update_finding(finding: &forge_domain::finding::Finding, expected: u64) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(lock_task_resume_schedule(id:Uuid)->Option<forge_domain::TaskResumeSchedule>,None);
    forward!(insert_task_resume_schedule(schedule:&forge_domain::TaskResumeSchedule)->(),Some(FaultPoint::AfterAggregateWrite));
    forward!(update_task_resume_schedule(schedule:&forge_domain::TaskResumeSchedule)->(),Some(FaultPoint::AfterAggregateWrite));
    forward!(load_task_dispatch_constraint(project: ProjectId, task: TaskId) -> Option<forge_domain::NextRunEmployeeConstraint>, None);
    forward!(insert_task_dispatch_constraint(constraint: &forge_domain::NextRunEmployeeConstraint) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(update_task_dispatch_constraint(constraint: &forge_domain::NextRunEmployeeConstraint, expected: &forge_domain::NextRunConstraintState) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(insert_message_requirement_waiver(waiver: &forge_domain::communication::MessageRequirementWaiver) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(load_project_repository(id: Uuid) -> Option<forge_domain::ProjectRepository>, None);
    forward!(task_git_source_setting(project_id: ProjectId, task_id: TaskId) -> forge_domain::git::TaskGitSourceSetting, None);
    forward!(insert_task_git_source_setting(project_id: ProjectId, task_id: TaskId, setting: &forge_domain::git::TaskGitSourceSetting) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(insert_project_repository(repository: &forge_domain::ProjectRepository) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(lock_employee_thread(id: Uuid) -> Option<forge_domain::communication::EmployeeThread>, None);
    forward!(insert_employee_thread(thread: &forge_domain::communication::EmployeeThread) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(update_employee_thread(thread: &forge_domain::communication::EmployeeThread, expected_revision: u64) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(load_employee_message(id: Uuid) -> Option<forge_domain::communication::EmployeeMessage>, None);
    forward!(insert_employee_message(message: &forge_domain::communication::EmployeeMessage) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(lock_project_creation(id: ProjectId) -> (), None);
    forward!(lock_project(id: ProjectId) -> Option<Project>, None);
    forward!(insert_project(project: &Project) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(update_project(project: &Project, expected_revision: u64) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(project_has_tasks(project_id: ProjectId) -> bool, None);
    forward!(lock_pipeline(id: PipelineId) -> Option<Pipeline>, None);
    forward!(lock_pipeline_version(id: PipelineVersionId) -> Option<PipelineVersion>, None);
    forward!(insert_pipeline(pipeline: &Pipeline) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(update_pipeline(pipeline: &Pipeline, expected_revision: u64) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(insert_pipeline_version(version: &PipelineVersion) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(insert_employee(employee: &Employee) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(lock_employee(id: forge_domain::EmployeeId) -> Option<Employee>, None);
    forward!(update_employee(employee: &Employee, expected_revision: u64) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(lock_task(id: TaskId) -> Option<StoredTask>, None);
    forward!(insert_task(task: &Task, persistence: TaskPersistence) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(update_task(task: &Task, persistence: TaskPersistence, expected_revision: u64) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(list_dependencies(project: ProjectId) -> Vec<TaskDependency>, None);
    forward!(insert_dependency(project: ProjectId, dependency: TaskDependency, actor: Actor) -> bool, None);
    forward!(remove_dependency(project: ProjectId, blocker: TaskId, blocked: TaskId) -> bool, None);
    forward!(lock_artifact(id: ArtifactId) -> Option<StoredArtifact>, None);
    forward!(insert_artifact(artifact: &Artifact, location: &ArtifactLocation) -> (), Some(FaultPoint::AfterAggregateWrite));
    forward!(enqueue(input: &QueueEntryInput) -> bool, None);
    forward!(invalidate_queued_entries_for_task(project: ProjectId, task: TaskId) -> u64, None);
    forward!(lock_active_runs_for_project(project: ProjectId) -> Vec<ActiveRun>, None);
    forward!(lock_active_runs_for_employee(project: ProjectId, employee: forge_domain::EmployeeId) -> Vec<ActiveRun>, None);
    forward!(lock_active_runs_for_task(project: ProjectId, task: TaskId) -> Vec<ActiveRun>, None);
    forward!(request_run_stop(run: Uuid, fence: u64, epoch: u64, force: bool) -> bool, None);
    forward!(revoke_run_lease(run: Uuid, fence: u64, epoch: u64) -> bool, Some(FaultPoint::AfterAggregateWrite));
    forward!(cancel_leased_queue_for_fenced_run(run: Uuid, fence: u64, epoch: u64) -> bool, None);
    forward!(set_recovery_hold(project: ProjectId, hold: bool) -> (), None);
    forward!(lookup_idempotency(project: ProjectId, key: &str) -> Option<IdempotencyRecord>, None);
    forward!(insert_idempotency(record: &IdempotencyRecord) -> (), Some(FaultPoint::AfterReceiptInsert));
    forward!(append_event_and_outbox(event: &DomainEvent) -> EventId, Some(FaultPoint::AfterAuditAppend));
}
