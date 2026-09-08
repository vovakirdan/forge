//! Handoff identity is bound to the accepted command receipt in the same commit.
use crate::{StorageError, StorageTransaction, encode_snapshot};
use forge_domain::{HandoffProducer, TaskHandoff};

impl StorageTransaction<'_> {
    pub async fn insert_command_handoff(
        &mut self,
        handoff: &TaskHandoff,
    ) -> Result<(), StorageError> {
        let data = handoff.data();
        let HandoffProducer::Command { command_id } = data.producer else {
            return Err(StorageError::InvalidInput {
                reason: "command handoff requires command provenance".into(),
            });
        };
        sqlx::query("INSERT INTO task_handoffs(id,project_id,task_id,kind,body,command_id) VALUES($1,$2,$3,'accepted',$4::jsonb,$5)")
            .bind(data.id).bind(data.project_id.as_uuid()).bind(data.task_id.as_uuid())
            .bind(encode_snapshot(handoff,"command.handoff")?).bind(command_id.as_uuid())
            .execute(&mut *self.transaction).await?;
        Ok(())
    }
}
