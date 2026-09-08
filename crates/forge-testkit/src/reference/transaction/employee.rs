//! In-memory Employee persistence mirrors PostgreSQL revision and uniqueness checks.

use forge_application::RepositoryError;
use forge_domain::Employee;

use super::{MemoryTransaction, invalid};

impl MemoryTransaction {
    pub(super) fn persist_new_employee(
        &mut self,
        employee: &Employee,
    ) -> Result<(), RepositoryError> {
        self.require_project(employee.project_id())?;
        employee
            .validate_snapshot()
            .map_err(|_| invalid("invalid employee snapshot"))?;
        if self.staged.employees.contains_key(&employee.id())
            || self.staged.employees.values().any(|stored| {
                stored.project_id() == employee.project_id() && stored.name() == employee.name()
            })
        {
            return Err(invalid("duplicate employee"));
        }
        self.staged
            .employees
            .insert(employee.id(), employee.clone());
        Ok(())
    }

    pub(super) fn persist_employee_revision(
        &mut self,
        employee: &Employee,
        expected_revision: u64,
    ) -> Result<(), RepositoryError> {
        employee
            .validate_snapshot()
            .map_err(|_| invalid("invalid employee snapshot"))?;
        let current = self
            .staged
            .employees
            .get(&employee.id())
            .filter(|stored| {
                stored.project_id() == employee.project_id()
                    && stored.revision() == expected_revision
            })
            .ok_or(RepositoryError::StaleRevision {
                aggregate: "employee",
            })?;
        if current.revision().checked_add(1) != Some(employee.revision()) {
            return Err(invalid("employee revision must advance exactly once"));
        }
        if self.staged.employees.values().any(|stored| {
            stored.id() != employee.id()
                && stored.project_id() == employee.project_id()
                && stored.name() == employee.name()
        }) {
            return Err(invalid("duplicate employee name"));
        }
        self.staged
            .employees
            .insert(employee.id(), employee.clone());
        Ok(())
    }
}
