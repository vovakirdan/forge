//! Policy audit stores fixed catalog names and safe categories, never tool input.

use super::{RunGateway, catalog};
use crate::{
    CoreError,
    event::{event, event_payload},
};
use forge_domain::{AggregateRef, CommandId, DomainEventKind, Timestamp};
use serde_json::json;
use uuid::Uuid;

impl RunGateway {
    pub(super) async fn audit_decision(
        &self,
        tool: &str,
        message_id: Option<Uuid>,
        allowed: bool,
        category: &'static str,
    ) -> Result<(), CoreError> {
        let mut transaction = self.core.store.begin().await?;
        let project =
            transaction
                .lock_project(self.project_id)
                .await?
                .ok_or(CoreError::NotFound {
                    aggregate: "project",
                })?;
        if allowed && !transaction.gateway_scope_is_active(self.scope).await? {
            return Err(super::invalid(
                "Run scope was revoked before authorization commit",
            ));
        }
        // The closure was bound to a canonical Run; even a now-revoked request
        // is attributed to that original Task, never to a worker-supplied id.
        let (aggregate, revision) = if let Some(owner) = self.assignment.task_stage() {
            let task = transaction
                .lock_task(owner.task_id)
                .await?
                .ok_or(CoreError::NotFound { aggregate: "task" })?
                .task;
            if task.project_id() != project.id() {
                return Err(super::invalid("audit scope mismatch"));
            }
            (AggregateRef::Task(task.id()), task.revision().get())
        } else {
            (AggregateRef::Project(project.id()), project.revision())
        };
        let audit = event(
            project.id(),
            aggregate,
            revision,
            if allowed {
                DomainEventKind::ToolGatewayCallAllowed
            } else {
                DomainEventKind::ToolGatewayCallDenied
            },
            self.core.actors.core,
            CommandId::new(),
            None,
            event_payload([
                (
                    "tool",
                    json!(catalog::logical_tool(tool).unwrap_or("unknown")),
                ),
                ("run_id", json!(self.scope.run_id)),
                ("message_id", json!(message_id)),
                ("fencing_token", json!(self.scope.fencing_token)),
                ("environment_epoch", json!(self.scope.environment_epoch)),
                ("category", json!(category)),
            ]),
            Timestamp::now_utc(),
        )?;
        transaction.append_event_and_outbox(&audit).await?;
        transaction.commit().await?;
        Ok(())
    }
}
