//! Operator configuration only; identities and timestamps belong to Core.
use forge_domain::{
    DomainError, ProjectHookVersion, ProjectId, TaskKind, Timestamp, runtime::ResourceLimits,
};
use serde::Deserialize;
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigureProjectHookInput {
    pub name: String,
    pub image: String,
    pub command: Vec<String>,
    pub workdir: String,
    pub limits: ResourceLimits,
    pub max_output_bytes: u64,
    #[serde(default)]
    pub applicable_task_kinds: BTreeSet<TaskKind>,
    pub required: bool,
}
impl ConfigureProjectHookInput {
    pub fn materialize(
        &self,
        id: Uuid,
        project_id: ProjectId,
        created_at: Timestamp,
    ) -> Result<ProjectHookVersion, DomainError> {
        let hook = ProjectHookVersion {
            id,
            project_id,
            created_at,
            name: self.name.clone(),
            image: self.image.clone(),
            command: self.command.clone(),
            workdir: self.workdir.clone(),
            limits: self.limits.clone(),
            max_output_bytes: self.max_output_bytes,
            applicable_task_kinds: self.applicable_task_kinds.clone(),
            required: self.required,
        };
        hook.validate_snapshot()?;
        Ok(hook)
    }
}
