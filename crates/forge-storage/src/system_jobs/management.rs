use super::*;

impl StorageTransaction<'_> {
    pub async fn canonical_event_watermark(
        &mut self,
        project: ProjectId,
    ) -> Result<u64, StorageError> {
        let n: i64 = sqlx::query_scalar(
            "SELECT coalesce(max(project_sequence),0) FROM event_log WHERE project_id=$1",
        )
        .bind(project.as_uuid())
        .fetch_one(&mut *self.transaction)
        .await?;
        u64::try_from(n).map_err(|_| invalid())
    }
    pub async fn request_employee_onboarding(
        &mut self,
        project: ProjectId,
        employee: EmployeeId,
        explicit: bool,
    ) -> Result<Uuid, StorageError> {
        let state:Option<String>=sqlx::query_scalar("SELECT state FROM employee_onboarding WHERE project_id=$1 AND employee_id=$2 FOR UPDATE")
            .bind(project.as_uuid()).bind(employee.as_uuid()).fetch_optional(&mut *self.transaction).await?;
        if state.is_none() || (!explicit && state.as_deref() != Some("pending")) {
            return Err(invalid());
        }
        let existing:Option<Uuid>=sqlx::query_scalar("SELECT id FROM system_jobs WHERE project_id=$1 AND kind='onboarding' AND target_employee_id=$2")
            .bind(project.as_uuid()).bind(employee.as_uuid()).fetch_optional(&mut *self.transaction).await?;
        let id = if let Some(id) = existing {
            if explicit {
                if self.system_job_has_retained_attempt(id).await? {
                    return Err(invalid());
                }
                self.set_system_job_state(id, "pending", None, true).await?;
            }
            id
        } else {
            let id = Uuid::now_v7();
            sqlx::query("INSERT INTO system_jobs(id,project_id,kind,target_employee_id,state,input) VALUES($1,$2,'onboarding',$3,'pending','{}')")
                .bind(id).bind(project.as_uuid()).bind(employee.as_uuid()).execute(&mut *self.transaction).await?;
            id
        };
        sqlx::query("UPDATE employee_onboarding SET state='pending',job_id=$3,receipt=NULL,revision=revision+1,updated_at=clock_timestamp() WHERE project_id=$1 AND employee_id=$2")
            .bind(project.as_uuid()).bind(employee.as_uuid()).bind(id).execute(&mut *self.transaction).await?;
        Ok(id)
    }
    pub async fn enqueue_pending_onboarding(
        &mut self,
        project: ProjectId,
    ) -> Result<usize, StorageError> {
        let ids:Vec<Uuid>=sqlx::query_scalar("SELECT o.employee_id FROM employee_onboarding o JOIN employee_runtime_bindings b ON b.employee_id=o.employee_id JOIN employees e ON e.id=o.employee_id WHERE o.project_id=$1 AND o.state='pending' AND o.job_id IS NULL AND e.employee_state='active' ORDER BY o.employee_id LIMIT 32")
            .bind(project.as_uuid()).fetch_all(&mut *self.transaction).await?;
        for id in &ids {
            self.request_employee_onboarding(project, EmployeeId::from(*id), false)
                .await?;
        }
        Ok(ids.len())
    }
    pub async fn system_job_has_retained_attempt(
        &mut self,
        id: Uuid,
    ) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM system_job_attempts a WHERE a.job_id=$1 AND (a.state='running' OR EXISTS(SELECT 1 FROM run_environment_reservations e WHERE e.run_id=a.run_id AND e.released_at IS NULL)))")
            .bind(id).fetch_one(&mut *self.transaction).await?)
    }
    pub async fn set_onboarding_receipt(
        &mut self,
        project: ProjectId,
        employee: EmployeeId,
        state: &str,
        receipt: Value,
    ) -> Result<(), StorageError> {
        if !matches!(state, "completed" | "skipped") {
            return Err(invalid());
        }
        let n=sqlx::query("UPDATE employee_onboarding SET state=$3,receipt=$4,revision=revision+1,updated_at=clock_timestamp() WHERE project_id=$1 AND employee_id=$2")
            .bind(project.as_uuid()).bind(employee.as_uuid()).bind(state).bind(receipt).execute(&mut *self.transaction).await?.rows_affected();
        if n != 1 {
            return Err(invalid());
        }
        Ok(())
    }
    pub async fn onboarding_job_id(
        &mut self,
        project: ProjectId,
        employee: EmployeeId,
    ) -> Result<Option<Uuid>, StorageError> {
        Ok(sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT job_id FROM employee_onboarding WHERE project_id=$1 AND employee_id=$2",
        )
        .bind(project.as_uuid())
        .bind(employee.as_uuid())
        .fetch_optional(&mut *self.transaction)
        .await?
        .flatten())
    }
}
