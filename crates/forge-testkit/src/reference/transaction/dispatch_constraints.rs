use super::{MemoryTransaction, invalid};
use forge_application::RepositoryError;
use forge_domain::{NextRunConstraintState, NextRunEmployeeConstraint};

impl MemoryTransaction {
    pub(super) fn insert_dispatch_constraint(
        &mut self,
        constraint: &NextRunEmployeeConstraint,
    ) -> Result<(), RepositoryError> {
        constraint
            .validate_snapshot()
            .map_err(|_| invalid("invalid constraint"))?;
        self.require_task_scope(constraint.project_id, constraint.task_id)?;
        if constraint.state != NextRunConstraintState::Pending
            || self
                .staged
                .dispatch_constraints
                .contains_key(&constraint.id)
            || self
                .staged
                .dispatch_constraints
                .values()
                .any(|stored| stored.task_id == constraint.task_id && stored.state.is_active())
            || self
                .staged
                .employees
                .get(&constraint.employee_id)
                .is_none_or(|employee| employee.project_id() != constraint.project_id)
            || !self
                .staged
                .pipeline_versions
                .contains_key(&constraint.pipeline_version_id)
        {
            return Err(invalid("duplicate constraint or invalid scope"));
        }
        self.staged
            .dispatch_constraints
            .insert(constraint.id, constraint.clone());
        Ok(())
    }

    pub(super) fn update_dispatch_constraint(
        &mut self,
        constraint: &NextRunEmployeeConstraint,
        expected: &NextRunConstraintState,
    ) -> Result<(), RepositoryError> {
        let stored = self.staged.dispatch_constraints.get(&constraint.id).ok_or(
            RepositoryError::NotFound {
                aggregate: "dispatch constraint",
            },
        )?;
        if &stored.state != expected {
            return Err(RepositoryError::StaleRevision {
                aggregate: "dispatch constraint",
            });
        }
        let mut permitted = stored.clone();
        permitted
            .transition(constraint.state.clone())
            .map_err(|_| invalid("invalid constraint transition"))?;
        if &permitted != constraint {
            return Err(invalid("constraint identity is immutable"));
        }
        if let NextRunConstraintState::Consumed { run_id } = constraint.state
            && !self.staged.runs.iter().any(|run| {
                run.scope.id == run_id
                    && run.scope.project_id == constraint.project_id
                    && run.scope.employee_id == Some(constraint.employee_id)
                    && run.scope.stage_visit == Some(constraint.stage_visit)
                    && run.scope.assignment.task_stage().is_some_and(|owner| {
                        owner.task_id == constraint.task_id && owner.stage_id == constraint.stage_id
                    })
                    && self.staged.queue.iter().any(|queue| {
                        queue.id == run.queue_entry_id
                            && queue.input.pipeline_version_id == constraint.pipeline_version_id
                    })
            })
        {
            return Err(invalid(
                "consuming Run does not own the constrained Task visit",
            ));
        }
        self.staged
            .dispatch_constraints
            .insert(constraint.id, constraint.clone());
        Ok(())
    }
}
