//! Mechanical persistence of explicit Employee admission intent.
use forge_domain::{NextRunConstraintState, NextRunEmployeeConstraint, ProjectId, TaskId};

use crate::{StorageError, StorageTransaction, database_timestamp, encode_snapshot, u64_to_i64};

impl StorageTransaction<'_> {
    pub async fn load_task_dispatch_constraint(
        &mut self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<Option<NextRunEmployeeConstraint>, StorageError> {
        let value: Option<String> = sqlx::query_scalar("SELECT canonical_snapshot::text FROM task_next_run_constraints WHERE project_id=$1 AND task_id=$2 AND constraint_state IN ('pending','blocked') FOR UPDATE")
            .bind(project.as_uuid()).bind(task.as_uuid()).fetch_optional(&mut *self.transaction).await?;
        value
            .map(|value| {
                let constraint: NextRunEmployeeConstraint =
                    serde_json::from_str(&value).map_err(|source| StorageError::Snapshot {
                        aggregate: "dispatch_constraint",
                        source,
                    })?;
                validate(&constraint)?;
                Ok(constraint)
            })
            .transpose()
    }

    pub async fn insert_task_dispatch_constraint(
        &mut self,
        constraint: &NextRunEmployeeConstraint,
    ) -> Result<(), StorageError> {
        validate(constraint)?;
        if constraint.state != NextRunConstraintState::Pending {
            return Err(invalid());
        }
        sqlx::query("INSERT INTO task_next_run_constraints(id,project_id,task_id,employee_id,pipeline_version_id,stage_id,stage_visit,constraint_state,canonical_snapshot,created_at) VALUES($1,$2,$3,$4,$5,$6,$7,'pending',$8::jsonb,$9)")
            .bind(constraint.id).bind(constraint.project_id.as_uuid()).bind(constraint.task_id.as_uuid())
            .bind(constraint.employee_id.as_uuid()).bind(constraint.pipeline_version_id.as_uuid())
            .bind(constraint.stage_id.as_str()).bind(u64_to_i64(constraint.stage_visit.get(), "constraint.stage_visit")?)
            .bind(encode_snapshot(constraint,"dispatch_constraint")?).bind(database_timestamp(constraint.created_at))
            .execute(&mut *self.transaction).await?;
        Ok(())
    }

    pub async fn update_task_dispatch_constraint(
        &mut self,
        constraint: &NextRunEmployeeConstraint,
        expected: &NextRunConstraintState,
    ) -> Result<(), StorageError> {
        validate(constraint)?;
        let consumed_run = match constraint.state {
            NextRunConstraintState::Consumed { run_id } => Some(run_id),
            _ => None,
        };
        let result = sqlx::query("UPDATE task_next_run_constraints SET constraint_state=$2,canonical_snapshot=$3::jsonb,consumed_run_id=$6 WHERE id=$1 AND project_id=$4 AND canonical_snapshot->'state'=$5::jsonb")
            .bind(constraint.id).bind(constraint.state.key()).bind(encode_snapshot(constraint,"dispatch_constraint")?)
            .bind(constraint.project_id.as_uuid()).bind(encode_snapshot(expected,"dispatch_constraint.state")?)
            .bind(consumed_run)
            .execute(&mut *self.transaction).await?;
        if result.rows_affected() != 1 {
            return Err(StorageError::StaleRevision {
                aggregate: "dispatch constraint",
            });
        }
        Ok(())
    }
}

fn validate(constraint: &NextRunEmployeeConstraint) -> Result<(), StorageError> {
    constraint.validate_snapshot().map_err(|_| invalid())
}

fn invalid() -> StorageError {
    StorageError::InvalidInput {
        reason: "invalid dispatch constraint".into(),
    }
}
