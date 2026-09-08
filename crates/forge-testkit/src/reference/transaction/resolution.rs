use super::{MemoryTransaction, invalid};
use forge_application::RepositoryError;
use forge_domain::resolution::*;

impl MemoryTransaction {
    pub(super) fn save_route_record(
        &mut self,
        value: &ResolverRoute,
        expected: Option<u64>,
    ) -> Result<(), RepositoryError> {
        value
            .validate_snapshot()
            .map_err(|_| invalid("invalid route"))?;
        self.require_project(value.project_id)?;
        let key = (value.project_id, value.key.clone());
        let prior = self.staged.resolver_routes.get(&key);
        if prior.map(|prior| prior.revision) != expected
            || value.revision
                != expected
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or_else(|| invalid("revision exhausted"))?
        {
            return Err(invalid("route revision mismatch"));
        }
        self.staged.resolver_routes.insert(key, value.clone());
        Ok(())
    }
    pub(super) fn save_escalation_record(
        &mut self,
        value: &Escalation,
        expected: Option<u64>,
    ) -> Result<(), RepositoryError> {
        value
            .validate_snapshot()
            .map_err(|_| invalid("invalid escalation"))?;
        if let Some(source) = value.source.task() {
            self.require_task_scope(value.project_id, source.task_id)?;
        } else {
            self.require_project(value.project_id)?;
            let source = value
                .source
                .communication()
                .ok_or_else(|| invalid("source absent"))?;
            if value.created_by.kind() != forge_domain::ActorKind::Employee
                || !self.staged.runs.iter().any(|run| {
                    run.scope.id == source.run_id
                        && run.scope.project_id == value.project_id
                        && run
                            .scope
                            .employee_id
                            .is_some_and(|id| id.as_uuid() == value.created_by.id().as_uuid())
                        && run.scope.lease_fencing_token == source.fencing_token
                        && run.scope.environment_epoch == source.environment_epoch
                        && run.scope.assignment.communication() == Some(&source.assignment)
                })
            {
                return Err(invalid("Communication source differs from exact Run owner"));
            }
        }
        match (self.staged.escalations.get(&value.id), expected) {
            (None, None) => {
                if self.staged.escalations.values().any(|other| {
                    other.project_id == value.project_id && other.source == value.source
                }) {
                    return Err(invalid("duplicate escalation wait"));
                }
            }
            (Some(prior), Some(expected)) if prior.revision == expected => {
                if value.generation < prior.generation || value.generation > prior.generation + 1 {
                    return Err(invalid("generation mismatch"));
                }
                let mut allowed = prior.clone();
                allowed.generation = value.generation;
                allowed.next_candidate = value.next_candidate;
                allowed
                    .advance(value.state.clone())
                    .map_err(|_| invalid("invalid escalation transition"))?;
                if &allowed != value {
                    return Err(invalid("immutable escalation source"));
                }
            }
            _ => return Err(invalid("escalation revision mismatch")),
        }
        self.staged.escalations.insert(value.id, value.clone());
        Ok(())
    }
    pub(super) fn insert_resolution_record(
        &mut self,
        value: &ResolutionAssignment,
    ) -> Result<(), RepositoryError> {
        value
            .validate_snapshot()
            .map_err(|_| invalid("invalid resolution assignment"))?;
        let escalation = self
            .staged
            .escalations
            .get(&value.escalation_id)
            .ok_or_else(|| invalid("escalation absent"))?;
        value
            .validate_owner(escalation)
            .map_err(|_| invalid("assignment differs from current escalation"))?;
        if escalation.project_id != value.project_id
            || value.state != ResolutionAssignmentState::Active
            || self.staged.resolution_assignments.contains_key(&value.id)
            || self.staged.resolution_assignments.values().any(|prior| {
                prior.escalation_id == value.escalation_id
                    && (prior.lease.generation == value.lease.generation
                        || prior.state == ResolutionAssignmentState::Active)
            })
        {
            return Err(invalid("duplicate assignment or wrong source"));
        }
        if let Resolver::Employee { employee_id } = value.resolver
            && self
                .staged
                .employees
                .get(&employee_id)
                .is_none_or(|employee| employee.project_id() != value.project_id)
        {
            return Err(invalid("resolver Employee absent"));
        }
        self.staged
            .resolution_assignments
            .insert(value.id, value.clone());
        Ok(())
    }
    pub(super) fn update_resolution_record(
        &mut self,
        value: &ResolutionAssignment,
    ) -> Result<(), RepositoryError> {
        let prior = self
            .staged
            .resolution_assignments
            .get(&value.id)
            .ok_or_else(|| invalid("assignment absent"))?;
        let mut allowed = prior.clone();
        allowed
            .finish(value.state.clone())
            .map_err(|_| invalid("invalid assignment terminal state"))?;
        if &allowed != value {
            return Err(invalid("immutable assignment scope"));
        }
        self.staged
            .resolution_assignments
            .insert(value.id, value.clone());
        Ok(())
    }
}
