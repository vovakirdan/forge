//! Report creation and explicit triage share one atomic command transaction.
use super::{
    CommandError, CommandTransaction, Engine,
    event::{event, event_payload},
    receipt::{finish_command, resource},
    task_support::load_scoped_task,
};
use crate::{CommandEnvelope, CommandPayload, FindingTriageDecision, ReportFindingCommand};
use forge_domain::{
    AggregateRef, CommandId, DomainEvent, DomainEventKind, Project, TaskSource, Timestamp,
    finding::{Finding, FindingState},
    runtime::RunScope,
};
use forge_protocol::wire::CommandReceipt;
use serde_json::json;
use uuid::Uuid;

impl Engine<'_> {
    /// Caller authenticates the reporting actor, locks Project, and commits or rolls back.
    /// The optional execution scope must come from an active fenced Gateway, never JSON.
    pub async fn record_finding(
        &self,
        tx: &mut impl CommandTransaction,
        project: &mut Project,
        input: &ReportFindingCommand,
        source: Option<RunScope>,
        command: CommandId,
        now: Timestamp,
    ) -> Result<(Finding, DomainEvent), CommandError> {
        let _ = load_scoped_task(tx, project, input.source_task_id).await?;
        if let Some(scope) = source {
            let valid = tx
                .lock_active_runs_for_task(project.id(), input.source_task_id)
                .await?
                .iter()
                .any(|run| {
                    run.id == scope.run_id
                        && run.lease_fencing_token == scope.fencing_token
                        && run.environment_epoch == scope.environment_epoch
                        && run
                            .employee_id
                            .is_some_and(|id| id.as_uuid() == self.actors.human.id().as_uuid())
                });
            if !valid || self.actors.human.kind() != forge_domain::ActorKind::Employee {
                return Err(CommandError::Forbidden);
            }
        } else if self.actors.human.kind() == forge_domain::ActorKind::Employee {
            return Err(CommandError::Forbidden);
        }
        for id in &input.evidence {
            let valid = tx.lock_artifact(*id).await?.is_some_and(|stored| {
                stored.artifact.project_id() == project.id()
                    && stored.location.task_id == Some(input.source_task_id)
            });
            if !valid {
                return Err(CommandError::NotFound {
                    aggregate: "finding evidence",
                });
            }
        }
        let finding = Finding {
            id: Uuid::now_v7(),
            project_id: project.id(),
            source_task_id: input.source_task_id,
            source_run: source,
            description: input.description.clone(),
            severity: input.severity.clone(),
            evidence: input.evidence.clone(),
            reported_by: self.actors.human,
            reported_at: now,
            revision: 1,
            state: FindingState::Open,
        };
        finding.validate_snapshot()?;
        tx.insert_finding(&finding).await?;
        let previous = project.revision();
        project.record_child_mutation(now)?;
        tx.update_project(project, previous).await?;
        let audit = event(
            project.id(),
            AggregateRef::Project(project.id()),
            project.revision(),
            DomainEventKind::FindingReported,
            self.actors.human,
            command,
            None,
            event_payload([("finding", json!(finding))]),
            now,
        )?;
        Ok((finding, audit))
    }
    pub(super) async fn manage_finding(
        &self,
        tx: &mut impl CommandTransaction,
        mut project: Project,
        envelope: &CommandEnvelope,
        hash: &str,
        command: CommandId,
        now: Timestamp,
    ) -> Result<CommandReceipt, CommandError> {
        let mut events = Vec::new();
        let primary = match &envelope.payload {
            CommandPayload::ReportFinding(input) => {
                let (finding, audit) = self
                    .record_finding(tx, &mut project, input, None, command, now)
                    .await?;
                events.push(audit);
                resource("finding", finding.id)
            }
            CommandPayload::TriageFinding(input) => {
                let mut finding = scoped(tx, project.id(), input.finding_id).await?;
                let state = match input.decision {
                    FindingTriageDecision::Attach { task_id } => {
                        let _ = load_scoped_task(tx, &project, task_id).await?;
                        FindingState::Attached {
                            task_id,
                            reason: input.reason.clone(),
                            by: self.actors.human,
                            at: now,
                        }
                    }
                    FindingTriageDecision::Ignore => FindingState::Ignored {
                        reason: input.reason.clone(),
                        by: self.actors.human,
                        at: now,
                    },
                };
                finding.triage(input.expected_finding_revision, state)?;
                tx.update_finding(&finding, input.expected_finding_revision)
                    .await?;
                events.push(
                    self.finding_triage_event(
                        tx,
                        &mut project,
                        &finding,
                        DomainEventKind::FindingTriaged,
                        command,
                        now,
                    )
                    .await?,
                );
                resource("finding", finding.id)
            }
            CommandPayload::PromoteFinding(input) => {
                let mut finding = scoped(tx, project.id(), input.finding_id).await?;
                // Prove triage eligibility before allocating a Task key. All writes still roll back together.
                finding.clone().triage(
                    input.expected_finding_revision,
                    FindingState::Promoted {
                        task_id: forge_domain::TaskId::new(),
                        reason: input.reason.clone(),
                        by: self.actors.human,
                        at: now,
                    },
                )?;
                let (task, audit) = self
                    .create_draft(
                        tx,
                        &mut project,
                        &input.task,
                        TaskSource::PromotedFinding,
                        command,
                        now,
                    )
                    .await?;
                events.push(audit);
                finding.triage(
                    input.expected_finding_revision,
                    FindingState::Promoted {
                        task_id: task.id(),
                        reason: input.reason.clone(),
                        by: self.actors.human,
                        at: now,
                    },
                )?;
                tx.update_finding(&finding, input.expected_finding_revision)
                    .await?;
                events.push(
                    self.finding_triage_event(
                        tx,
                        &mut project,
                        &finding,
                        DomainEventKind::FindingPromoted,
                        command,
                        now,
                    )
                    .await?,
                );
                resource("task", task.id().as_uuid())
            }
            _ => return Err(CommandError::UnsupportedCommand),
        };
        finish_command(
            tx,
            &project,
            envelope,
            hash,
            command,
            self.actors.human,
            events,
            Some(primary),
        )
        .await
    }
    async fn finding_triage_event(
        &self,
        tx: &mut impl CommandTransaction,
        project: &mut Project,
        finding: &Finding,
        kind: DomainEventKind,
        command: CommandId,
        now: Timestamp,
    ) -> Result<DomainEvent, CommandError> {
        let previous = project.revision();
        project.record_child_mutation(now)?;
        tx.update_project(project, previous).await?;
        Ok(event(
            project.id(),
            AggregateRef::Project(project.id()),
            project.revision(),
            kind,
            self.actors.human,
            command,
            None,
            event_payload([("finding", json!(finding))]),
            now,
        )?)
    }
}
async fn scoped(
    tx: &mut impl CommandTransaction,
    project: forge_domain::ProjectId,
    id: Uuid,
) -> Result<Finding, CommandError> {
    tx.lock_finding(id)
        .await?
        .filter(|finding| finding.project_id == project)
        .ok_or(CommandError::NotFound {
            aggregate: "finding",
        })
}
