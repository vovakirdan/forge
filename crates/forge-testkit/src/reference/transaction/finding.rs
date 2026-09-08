//! Mechanical report identity and same-Project foreign keys.
use super::{MemoryTransaction, RepositoryError, invalid};
use forge_domain::finding::Finding;

impl MemoryTransaction {
    fn validate_finding(&self, finding: &Finding) -> Result<(), RepositoryError> {
        finding
            .validate_snapshot()
            .map_err(|_| invalid("invalid Finding"))?;
        self.require_task_scope(finding.project_id, finding.source_task_id)?;
        if let Some(task) = finding.state.target_task() {
            self.require_task_scope(finding.project_id, task)?;
        }
        if let Some(scope) = finding.source_run {
            let valid =
                self.staged
                    .runs
                    .iter()
                    .find(|run| run.scope.id == scope.run_id)
                    .is_some_and(|run| {
                        run.scope.project_id == finding.project_id
                            && run
                                .scope
                                .assignment
                                .task_stage()
                                .is_some_and(|owner| owner.task_id == finding.source_task_id)
                            && run.scope.lease_fencing_token == scope.fencing_token
                            && run.scope.environment_epoch == scope.environment_epoch
                            && run.scope.employee_id.is_some_and(|id| {
                                id.as_uuid() == finding.reported_by.id().as_uuid()
                            })
                    });
            if !valid || finding.reported_by.kind() != forge_domain::ActorKind::Employee {
                return Err(invalid("invalid Finding Run"));
            }
        }
        Ok(())
    }
    pub(super) fn insert_finding_record(
        &mut self,
        finding: &Finding,
    ) -> Result<(), RepositoryError> {
        self.validate_finding(finding)?;
        if finding.revision != 1
            || finding.state.key() != "open"
            || self.staged.findings.contains_key(&finding.id)
        {
            return Err(invalid("duplicate or noninitial Finding"));
        }
        self.staged.findings.insert(finding.id, finding.clone());
        Ok(())
    }
    pub(super) fn update_finding_record(
        &mut self,
        finding: &Finding,
        expected: u64,
    ) -> Result<(), RepositoryError> {
        self.validate_finding(finding)?;
        let stored = self
            .staged
            .findings
            .get(&finding.id)
            .ok_or(RepositoryError::NotFound {
                aggregate: "finding",
            })?;
        if stored.revision != expected {
            return Err(RepositoryError::StaleRevision {
                aggregate: "finding",
            });
        }
        let mut next = stored.clone();
        next.triage(expected, finding.state.clone())
            .map_err(|_| invalid("invalid Finding update"))?;
        if &next != finding {
            return Err(invalid("immutable Finding changed"));
        }
        self.staged.findings.insert(finding.id, finding.clone());
        Ok(())
    }
}
