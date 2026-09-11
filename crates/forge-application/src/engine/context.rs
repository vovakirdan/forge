//! Trusted invocation authority and injectable operational time.
use super::CommandError;
use forge_domain::{Actor, ActorKind, ProjectId, Timestamp};
use forge_protocol::wire::CommandName;

/// Operational time source; command preparation reads it only after Project locking.
pub trait Clock: Send + Sync {
    /// Returns wall time without applying the canonical Project mutation floor.
    fn now(&self) -> Timestamp;
}

/// Production UTC wall clock; reference tests may supply a manually controlled clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now_utc()
    }
}

/// Built by the authenticated composition boundary, never decoded from HTTP JSON.
#[derive(Clone, Debug)]
pub struct CommandContext {
    /// Exact Project scope granted by the trusted composition boundary.
    pub project_id: ProjectId,
    /// Authenticated/delegated requester used unchanged for hashing and audit.
    pub actor: Actor,
    /// Core identity used for command consequences caused by the orchestration engine.
    pub core_actor: Actor,
    /// Explicit trusted grants, never accepted from caller-provided JSON.
    pub capabilities: Vec<CommandName>,
}

impl CommandContext {
    /// Current local-operator policy: grants every command to Human, none to other kinds.
    /// Trusted callers can construct explicit grants independently of actor kind.
    #[must_use]
    pub fn local_human(project_id: ProjectId, actor: Actor, core_actor: Actor) -> Self {
        use CommandName::*;
        Self {
            project_id,
            actor,
            core_actor,
            capabilities: if actor.kind() == ActorKind::Human {
                vec![
                    ReportFinding,
                    TriageFinding,
                    PromoteFinding,
                    RegisterProjectRepository,
                    BindTaskGitRepository,
                    SetTaskGitSourcePolicy,
                    CreateProject,
                    CreatePipeline,
                    PublishPipelineVersion,
                    SetPipelineDefaultVersion,
                    DeletePipeline,
                    CreateEmployee,
                    OpenEmployeeThread,
                    SendEmployeeMessage,
                    WaiveMessageRequirement,
                    AmendEmployee,
                    EnableEmployee,
                    DisableEmployee,
                    RetireEmployee,
                    StopEmployee,
                    SetNextRunEmployee,
                    ClearNextRunEmployee,
                    ScheduleTaskResume,
                    ConfigureResolverRoute,
                    RaiseEscalation,
                    SubmitHumanResolution,
                    RerouteEscalation,
                    CancelTaskResume,
                    PauseTask,
                    CreateTask,
                    AmendDraft,
                    ApproveTask,
                    CancelTask,
                    ResumeTask,
                    SetTaskPriority,
                    CreateDependency,
                    RemoveDependency,
                    StartProjectExecution,
                    StopProjectExecution,
                    SubmitExternalStageOutcome,
                    ConfigureEmployeeRuntime,
                    ConfigureProjectHook,
                    ImportTaskFileSnapshot,
                    CaptureTaskFileSnapshot,
                    AttachTaskFileInput,
                    EnrollCredential,
                    ConfigureBootRecoveryPolicy,
                    AcceptRunRecoveryAssessment,
                    RetryCommunication,
                    RetryGitIntegration,
                    AcceptGitIntegrationResult,
                ]
            } else {
                Vec::new()
            },
        }
    }

    /// Verifies scope, trusted grants, and typed/raw request agreement before repository access.
    pub fn authorize(&self, envelope: &crate::CommandEnvelope) -> Result<(), CommandError> {
        if self.project_id != envelope.project_id
            || self.core_actor.kind() != ActorKind::Core
            || !self.capabilities.contains(&envelope.name)
        {
            return Err(CommandError::Forbidden);
        }
        envelope.validate_integrity()?;
        Ok(())
    }
}
