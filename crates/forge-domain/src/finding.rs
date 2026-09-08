//! An observation is not backlog intent; promotion requires separate authority.
use crate::{
    Actor, ArtifactId, DomainError, ProjectId, TaskId, Timestamp,
    evidence::{invalid, validate_uuid},
    runtime::RunScope,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum FindingState {
    Open,
    Attached {
        task_id: TaskId,
        reason: String,
        by: Actor,
        at: Timestamp,
    },
    Promoted {
        task_id: TaskId,
        reason: String,
        by: Actor,
        at: Timestamp,
    },
    Ignored {
        reason: String,
        by: Actor,
        at: Timestamp,
    },
}
impl FindingState {
    pub fn key(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Attached { .. } => "attached",
            Self::Promoted { .. } => "promoted",
            Self::Ignored { .. } => "ignored",
        }
    }
    pub fn target_task(&self) -> Option<TaskId> {
        match self {
            Self::Attached { task_id, .. } | Self::Promoted { task_id, .. } => Some(*task_id),
            _ => None,
        }
    }
}

/// Immutable report/provenance; revision and triage state change under a Project gate.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub source_task_id: TaskId,
    pub source_run: Option<RunScope>,
    pub description: String,
    /// Project vocabulary, not a hard-coded severity enum or execution priority.
    pub severity: String,
    pub evidence: BTreeSet<ArtifactId>,
    pub reported_by: Actor,
    pub reported_at: Timestamp,
    pub revision: u64,
    pub state: FindingState,
}
impl Finding {
    pub fn validate_snapshot(&self) -> Result<(), DomainError> {
        validate_uuid(self.id, "finding.id")?;
        self.project_id.validate_v7("finding.project")?;
        self.source_task_id.validate_v7("finding.source_task")?;
        self.reported_by.validate_snapshot()?;
        text(&self.description, 50_000)?;
        text(&self.severity, 64)?;
        if self.revision == 0 || self.evidence.len() > 64 {
            return Err(invalid("finding", "invalid revision or evidence count"));
        }
        for id in &self.evidence {
            id.validate_v7("finding.evidence")?;
        }
        if let Some(scope) = self.source_run {
            validate_uuid(scope.run_id, "finding.run")?;
            if scope.fencing_token == 0 || scope.environment_epoch == 0 {
                return Err(invalid("finding.run", "invalid execution scope"));
            }
        }
        if let Some(task) = self.state.target_task() {
            task.validate_v7("finding.target")?;
        }
        match &self.state {
            FindingState::Open => {
                if self.revision != 1 {
                    return Err(invalid("finding", "open report must be its first revision"));
                }
            }
            FindingState::Attached { reason, by, at, .. }
            | FindingState::Promoted { reason, by, at, .. }
            | FindingState::Ignored { reason, by, at } => {
                text(reason, 10_000)?;
                by.validate_snapshot()?;
                if *at < self.reported_at || self.revision < 2 {
                    return Err(invalid("finding", "invalid triage time or revision"));
                }
            }
        }
        Ok(())
    }
    pub fn triage(
        &mut self,
        expected_revision: u64,
        state: FindingState,
    ) -> Result<(), DomainError> {
        if self.revision != expected_revision
            || matches!(
                self.state,
                FindingState::Promoted { .. } | FindingState::Ignored { .. }
            )
            || matches!(state, FindingState::Open)
        {
            return Err(invalid(
                "finding.triage",
                "stale revision or terminal report",
            ));
        }
        let mut next = self.clone();
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("finding.revision", "exhausted revision"))?;
        next.state = state;
        next.validate_snapshot()?;
        *self = next;
        Ok(())
    }
}
fn text(value: &str, max: usize) -> Result<(), DomainError> {
    if value.trim().is_empty() || value.contains('\0') || value.chars().count() > max {
        return Err(invalid("finding", "invalid bounded report text"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ActorId;
    fn report() -> Finding {
        Finding {
            id: Uuid::now_v7(),
            project_id: ProjectId::new(),
            source_task_id: TaskId::new(),
            source_run: None,
            description: "Potential separate defect".into(),
            severity: "owner-defined".into(),
            evidence: BTreeSet::new(),
            reported_by: Actor::human(ActorId::new()),
            reported_at: Timestamp::now_utc(),
            revision: 1,
            state: FindingState::Open,
        }
    }
    #[test]
    fn triage_is_revisioned_and_promotion_final() {
        let mut finding = report();
        finding.validate_snapshot().unwrap();
        let state = FindingState::Promoted {
            task_id: TaskId::new(),
            reason: "Explicit backlog intent".into(),
            by: finding.reported_by,
            at: finding.reported_at,
        };
        let before = finding.clone();
        assert!(finding.triage(2, state.clone()).is_err());
        assert_eq!(finding, before);
        finding.triage(1, state.clone()).unwrap();
        assert_eq!(finding.state.key(), "promoted");
        assert!(finding.triage(2, state).is_err());
    }
    #[test]
    fn malformed_reports_and_forged_scope_are_rejected() {
        let mut finding = report();
        finding.description = " ".into();
        assert!(finding.validate_snapshot().is_err());
        let mut finding = report();
        finding.source_run = Some(RunScope {
            run_id: Uuid::now_v7(),
            fencing_token: 0,
            environment_epoch: 1,
        });
        assert!(finding.validate_snapshot().is_err());
        let mut value = serde_json::to_value(report()).unwrap();
        value["create_task"] = serde_json::json!(true);
        assert!(serde_json::from_value::<Finding>(value).is_err());
    }
}
