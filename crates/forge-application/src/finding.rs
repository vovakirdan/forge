//! Explicit report/triage intent; execution provenance is supplied by the trusted caller.
use forge_domain::{ArtifactId, TaskId};
use serde::Deserialize;
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportFindingCommand {
    pub source_task_id: TaskId,
    pub description: String,
    pub severity: String,
    #[serde(default)]
    pub evidence: BTreeSet<ArtifactId>,
}
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FindingTriageDecision {
    Attach { task_id: TaskId },
    Ignore,
}
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriageFindingCommand {
    pub finding_id: Uuid,
    pub expected_finding_revision: u64,
    pub reason: String,
    pub decision: FindingTriageDecision,
}
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromoteFindingCommand {
    pub finding_id: Uuid,
    pub expected_finding_revision: u64,
    pub reason: String,
    pub task: crate::CreateTaskCommand,
}
