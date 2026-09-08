//! Capacity is admission under an Employee row lock, not a lifetime mutex.
use crate::{StorageError, StorageTransaction};
use forge_domain::{EmployeeId, EmployeeState, ProjectId};

impl StorageTransaction<'_> {
    /// Locks Employee admission and rechecks capacity from a fresh SQL snapshot.
    ///
    /// The caller holds the Project lock and must create the new Lease/reservation
    /// before committing. Active Lease and physical reservation for the same Run
    /// consume one slot; uncertain physical ownership still consumes that slot.
    pub async fn lock_employee_capacity(
        &mut self,
        project_id: ProjectId,
        employee_id: EmployeeId,
    ) -> Result<bool, StorageError> {
        let Some(stored) = self.lock_employee(employee_id).await? else {
            return Ok(false);
        };
        let employee = stored.employee;
        if employee.project_id() != project_id || employee.state() != EmployeeState::Enabled {
            return Ok(false);
        }
        // Kept separate from SELECT FOR UPDATE: after waiting for a competing
        // admission, READ COMMITTED must see its newly committed reservation.
        let occupied: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM (SELECT id FROM leases WHERE employee_id=$1 AND lease_state='active' UNION SELECT r.lease_id FROM runs r JOIN run_environment_reservations e ON e.run_id=r.id WHERE e.employee_id=$1 AND e.released_at IS NULL) held",
        ).bind(employee_id.as_uuid()).fetch_one(&mut *self.transaction).await?;
        Ok(occupied < i64::from(employee.max_concurrent_runs()))
    }
}
