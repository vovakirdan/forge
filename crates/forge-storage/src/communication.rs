//! Mechanical Inbox persistence; authorization and ordered mutation belong to Core.
use forge_domain::{
    EmployeeId, ProjectId,
    communication::{EmployeeMessage, EmployeeThread, MessageTarget},
};
use uuid::Uuid;

use crate::{
    PostgresStore, StorageError, StorageTransaction, database_timestamp, decode_snapshot,
    encode_snapshot, u64_to_i64,
};

impl crate::model::SnapshotValidatable for EmployeeThread {
    fn validate_storage_snapshot(&self) -> Result<(), forge_domain::DomainError> {
        EmployeeThread::new(self.data().clone()).map(|_| ())
    }
}
impl crate::model::SnapshotValidatable for EmployeeMessage {
    fn validate_storage_snapshot(&self) -> Result<(), forge_domain::DomainError> {
        EmployeeMessage::new(self.data().clone()).map(|_| ())
    }
}

impl PostgresStore {
    /// One message-sequence page of persisted delivery facts. Receipts are reduced in SQL,
    /// so repeated attempts cannot expand a bounded browser response without limit.
    pub async fn list_message_delivery(
        &self,
        project: ProjectId,
        thread: Uuid,
        after: u64,
        limit: u32,
    ) -> Result<Vec<serde_json::Value>, StorageError> {
        let rows = sqlx::query_scalar(
            r#"SELECT jsonb_build_object(
                'message_id', m.id, 'sequence', m.sequence,
                'assignment', CASE WHEN a.id IS NULL THEN NULL ELSE jsonb_build_object(
                    'state', a.state, 'attempt_number', a.attempt_number,
                    'run_id', (SELECT r.id FROM runs r WHERE r.project_id=m.project_id
                        AND r.communication_assignment_id=a.id ORDER BY r.attempt_number DESC LIMIT 1),
                    'retry_ready', EXISTS (
                        SELECT 1 FROM runs r
                        JOIN leases l ON forge_lease_owns_run(l,r)
                        JOIN run_environment_reservations e ON e.run_id=r.id
                        WHERE r.project_id=m.project_id AND r.communication_assignment_id=a.id
                          AND r.purpose='communication' AND r.attempt_number=a.attempt_number
                          AND a.state='held' AND l.lease_state<>'active'
                          AND e.state='quiescent' AND e.released_at IS NOT NULL
                          AND NOT EXISTS (SELECT 1 FROM run_environment_reservations other
                              WHERE other.communication_assignment_id=a.id AND other.released_at IS NULL)
                          AND NOT EXISTS (SELECT 1 FROM escalations question
                              WHERE question.source_run_id=r.id AND question.escalation_state<>'resolved')
                    )) END,
                'runtime_accepted_at', receipts.runtime_accepted_at,
                'acknowledged_at', receipts.acknowledged_at,
                'answered_at', receipts.answered_at,
                'answered_reply_id', (SELECT r.reply_id FROM employee_message_receipts r
                    WHERE r.project_id=m.project_id AND r.message_id=m.id AND r.kind='answered'
                    ORDER BY r.created_at DESC, r.run_id DESC LIMIT 1),
                'waiver', w.canonical_snapshot
            )
            FROM employee_messages m
            LEFT JOIN communication_assignments a ON a.project_id=m.project_id AND a.source_message_id=m.id
            LEFT JOIN employee_message_waivers w ON w.project_id=m.project_id AND w.message_id=m.id
            LEFT JOIN LATERAL (
                SELECT max(r.created_at) FILTER (WHERE r.kind='runtime_accepted') AS runtime_accepted_at,
                       max(r.created_at) FILTER (WHERE r.kind='acknowledged') AS acknowledged_at,
                       max(r.created_at) FILTER (WHERE r.kind='answered') AS answered_at
                FROM employee_message_receipts r
                WHERE r.project_id=m.project_id AND r.message_id=m.id
            ) receipts ON TRUE
            WHERE m.project_id=$1 AND m.thread_id=$2 AND m.sequence>$3
            ORDER BY m.sequence LIMIT $4"#,
        )
        .bind(project.as_uuid())
        .bind(thread)
        .bind(u64_to_i64(after, "delivery.after")?)
        .bind(i64::from(limit.clamp(1, 101)))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn list_employee_threads(
        &self,
        project: ProjectId,
        employee: EmployeeId,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<EmployeeThread>, StorageError> {
        let values: Vec<String> = sqlx::query_scalar(
            "SELECT canonical_snapshot::text FROM employee_threads WHERE project_id=$1 AND employee_id=$2 AND ($3::uuid IS NULL OR id>$3) ORDER BY id LIMIT $4")
            .bind(project.as_uuid()).bind(employee.as_uuid()).bind(after).bind(i64::from(limit.clamp(1,101)))
            .fetch_all(&self.pool).await?;
        values
            .into_iter()
            .map(|s| decode_snapshot(&s, "employee_thread"))
            .collect()
    }

    pub async fn list_employee_messages(
        &self,
        project: ProjectId,
        thread: Uuid,
        after: u64,
        limit: u32,
    ) -> Result<Vec<EmployeeMessage>, StorageError> {
        let values: Vec<String> = sqlx::query_scalar(
            "SELECT canonical_snapshot::text FROM employee_messages WHERE project_id=$1 AND thread_id=$2 AND sequence>$3 ORDER BY sequence LIMIT $4")
            .bind(project.as_uuid()).bind(thread).bind(u64_to_i64(after,"message.sequence")?)
            .bind(i64::from(limit.clamp(1,101))).fetch_all(&self.pool).await?;
        values
            .into_iter()
            .map(|s| decode_snapshot(&s, "employee_message"))
            .collect()
    }

    pub async fn load_employee_thread(
        &self,
        project: ProjectId,
        thread: Uuid,
    ) -> Result<Option<EmployeeThread>, StorageError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT canonical_snapshot::text FROM employee_threads WHERE project_id=$1 AND id=$2",
        )
        .bind(project.as_uuid())
        .bind(thread)
        .fetch_optional(&self.pool)
        .await?;
        value
            .map(|s| decode_snapshot(&s, "employee_thread"))
            .transpose()
    }
}

impl StorageTransaction<'_> {
    pub async fn lock_employee_thread(
        &mut self,
        id: Uuid,
    ) -> Result<Option<EmployeeThread>, StorageError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT canonical_snapshot::text FROM employee_threads WHERE id=$1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *self.transaction)
        .await?;
        value
            .map(|s| decode_snapshot(&s, "employee_thread"))
            .transpose()
    }

    pub async fn insert_employee_thread(
        &mut self,
        thread: &EmployeeThread,
    ) -> Result<(), StorageError> {
        let data = thread.data();
        sqlx::query("INSERT INTO employee_threads(id,project_id,employee_id,task_id,revision,last_sequence,canonical_snapshot,created_at,updated_at) VALUES($1,$2,$3,$4,$5,$6,$7::jsonb,$8,$9)")
            .bind(data.id).bind(data.project_id.as_uuid()).bind(data.employee_id.as_uuid())
            .bind(data.task_id.map(|id| id.as_uuid())).bind(u64_to_i64(data.revision,"thread.revision")?)
            .bind(u64_to_i64(data.last_sequence,"thread.sequence")?).bind(encode_snapshot(thread,"employee_thread")?)
            .bind(database_timestamp(data.created_at)).bind(database_timestamp(data.updated_at))
            .execute(&mut *self.transaction).await?;
        Ok(())
    }

    pub async fn update_employee_thread(
        &mut self,
        thread: &EmployeeThread,
        expected_revision: u64,
    ) -> Result<(), StorageError> {
        let data = thread.data();
        let result = sqlx::query("UPDATE employee_threads SET revision=$1,last_sequence=$2,canonical_snapshot=$3::jsonb,updated_at=$4 WHERE id=$5 AND project_id=$6 AND revision=$7")
            .bind(u64_to_i64(data.revision,"thread.revision")?)
            .bind(u64_to_i64(data.last_sequence,"thread.sequence")?).bind(encode_snapshot(thread,"employee_thread")?)
            .bind(database_timestamp(data.updated_at)).bind(data.id).bind(data.project_id.as_uuid())
            .bind(u64_to_i64(expected_revision,"thread.expected_revision")?)
            .execute(&mut *self.transaction).await?;
        if result.rows_affected() != 1 {
            return Err(StorageError::StaleRevision {
                aggregate: "employee thread",
            });
        }
        Ok(())
    }

    pub async fn load_employee_message(
        &mut self,
        id: Uuid,
    ) -> Result<Option<EmployeeMessage>, StorageError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT canonical_snapshot::text FROM employee_messages WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(&mut *self.transaction)
        .await?;
        value
            .map(|s| decode_snapshot(&s, "employee_message"))
            .transpose()
    }

    pub async fn insert_employee_message(
        &mut self,
        message: &EmployeeMessage,
    ) -> Result<(), StorageError> {
        let data = message.data();
        let target_task = data
            .target
            .task_context()
            .map(|context| context.task_id.as_uuid());
        let target_run = match data.target {
            MessageTarget::ExactRun { run_id, .. } => Some(run_id),
            _ => None,
        };
        let requirement = crate::enum_text(&data.requirement, "message.requirement")?;
        sqlx::query("INSERT INTO employee_messages(id,project_id,employee_id,thread_id,sequence,target_task_id,target_run_id,requirement,reply_to,canonical_snapshot,created_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10::jsonb,$11)")
            .bind(data.id).bind(data.project_id.as_uuid()).bind(data.employee_id.as_uuid()).bind(data.thread_id)
            .bind(u64_to_i64(data.sequence,"message.sequence")?).bind(target_task).bind(target_run)
            .bind(requirement).bind(data.reply_to).bind(encode_snapshot(message,"employee_message")?)
            .bind(database_timestamp(data.created_at)).execute(&mut *self.transaction).await?;
        Ok(())
    }
}
