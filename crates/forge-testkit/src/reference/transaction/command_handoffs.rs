use super::*;

impl MemoryTransaction {
    pub(super) fn record_command_handoff(
        &mut self,
        handoff: &forge_domain::TaskHandoff,
    ) -> Result<(), RepositoryError> {
        let data = handoff.data();
        let forge_domain::HandoffProducer::Command { command_id } = data.producer else {
            return Err(invalid("command producer required"));
        };
        let receipt = self
            .staged
            .idempotency
            .values()
            .find(|record| record.command_id == command_id)
            .filter(|record| {
                record.project_id == data.project_id
                    && record.command_name == "submit_external_stage_outcome"
                    && record.receipt["resource"]["kind"] == "task"
                    && record.receipt["resource"]["id"] == data.task_id.to_string()
                    && serde_json::to_value(data.actor).ok().as_ref() == Some(&record.actor)
            });
        if receipt.is_none()
            || self.staged.command_handoffs.contains_key(&command_id)
            || self
                .staged
                .tasks
                .get(&data.task_id)
                .is_none_or(|task| task.task.project_id() != data.project_id)
        {
            return Err(invalid("handoff must match immutable command scope"));
        }
        self.staged
            .command_handoffs
            .insert(command_id, handoff.clone());
        Ok(())
    }
}
