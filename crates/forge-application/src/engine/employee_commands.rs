//! Employee commands change only future eligibility and catalog configuration.
use super::{
    CommandError, CommandTransaction, Engine, RepositoryError,
    event::{event, event_payload},
    pipeline_access::validate_employee_stage_eligibility,
    receipt::{finish_command, resource},
};
use crate::{CommandEnvelope, CommandPayload};
use forge_domain::{AggregateRef, CommandId, DomainEventKind, Project, Timestamp};
use forge_protocol::wire::CommandReceipt;
use serde_json::json;

impl Engine<'_> {
    pub(super) async fn manage_employee(
        &self,
        transaction: &mut impl CommandTransaction,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<CommandReceipt, CommandError> {
        let (employee_id, expected_revision) = match &envelope.payload {
            CommandPayload::AmendEmployee {
                employee_id,
                expected_employee_revision,
                ..
            }
            | CommandPayload::EnableEmployee {
                employee_id,
                expected_employee_revision,
                ..
            }
            | CommandPayload::DisableEmployee {
                employee_id,
                expected_employee_revision,
                ..
            }
            | CommandPayload::RetireEmployee {
                employee_id,
                expected_employee_revision,
                ..
            } => (*employee_id, *expected_employee_revision),
            _ => return Err(CommandError::UnsupportedCommand),
        };
        let mut employee = transaction
            .lock_employee(employee_id)
            .await?
            .filter(|employee| employee.project_id() == project.id())
            .ok_or(CommandError::NotFound {
                aggregate: "employee",
            })?;
        if employee.revision() != expected_revision {
            return Err(RepositoryError::StaleRevision {
                aggregate: "employee",
            }
            .into());
        }
        let (kind, reason) = match &envelope.payload {
            CommandPayload::AmendEmployee { patch, .. } => {
                employee.amend(patch, now)?;
                validate_employee_stage_eligibility(transaction, &project, &employee).await?;
                (DomainEventKind::EmployeeAmended, None)
            }
            CommandPayload::EnableEmployee { reason, .. } => {
                employee.enable(now)?;
                (DomainEventKind::EmployeeEnabled, reason.clone())
            }
            CommandPayload::DisableEmployee { reason, .. } => {
                employee.disable(now)?;
                (DomainEventKind::EmployeeDisabled, reason.clone())
            }
            CommandPayload::RetireEmployee { reason, .. } => {
                employee.retire(now)?;
                (DomainEventKind::EmployeeRetired, reason.clone())
            }
            _ => return Err(CommandError::UnsupportedCommand),
        };
        transaction
            .update_employee(&employee, expected_revision)
            .await?;
        let original_project_revision = project.revision();
        project.record_child_mutation(now)?;
        transaction
            .update_project(&project, original_project_revision)
            .await?;
        let audit = event(
            project.id(),
            AggregateRef::Employee(employee.id()),
            employee.revision(),
            kind,
            self.actors.human,
            command_id,
            reason,
            event_payload([
                ("employee_id", json!(employee.id())),
                ("name", json!(employee.name())),
                ("role", json!(employee.role())),
                ("stage_eligibility", json!(employee.stage_eligibility())),
                ("state", json!(employee.state())),
                ("max_concurrent_runs", json!(employee.max_concurrent_runs())),
            ]),
            now,
        )?;
        finish_command(
            transaction,
            &project,
            envelope,
            request_hash,
            command_id,
            self.actors.human,
            vec![audit],
            Some(resource("employee", employee.id().as_uuid())),
        )
        .await
    }
}
