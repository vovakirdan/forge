//! Capturing a Git result does not promote the Task or release its writer.
use super::{
    bridge::InboundResult,
    executor::{SubmissionContext, employee_actor},
    outcome::next_stage_wait,
    validation::ParsedStageOutcomeSubmission,
};
use crate::{
    CoreError, CoreService,
    event::{event, event_payload},
};
use forge_domain::{
    ArtifactId, CommandId, DomainError, DomainEventKind, TaskWorkSurface,
    git_delivery::{GitProposalState, GitStageProposal},
    runtime::{RunScope, SandboxRunSpec, SurfaceAccess},
};
use forge_storage::{FencedWrite, StorageTransaction};
use serde_json::json;
use std::collections::BTreeSet;

impl CoreService {
    pub(super) async fn capture_git_proposal(
        &self,
        mut tx: StorageTransaction<'_>,
        mut context: SubmissionContext,
        submission: ParsedStageOutcomeSubmission,
        artifact_ids: BTreeSet<ArtifactId>,
    ) -> Result<InboundResult, CoreError> {
        let task = &context.stored_task.task;
        let TaskWorkSurface::Git(binding) = task.work_surface() else {
            return Err(invalid("Task is not Git-bound"));
        };
        let commit = submission
            .candidate_commit
            .clone()
            .ok_or_else(|| invalid("Git writer must submit an explicit full candidate_commit"))?;
        let spec: SandboxRunSpec = serde_json::from_value(context.run.run_spec.clone())
            .map_err(|_| invalid("Git writer RunSpec is invalid"))?;
        spec.validate()
            .map_err(|_| invalid("Git writer RunSpec is invalid"))?;
        if context.run.run_spec_version != 2
            || spec.surface_id != binding.surface_id
            || spec.binding.access != SurfaceAccess::ReadWrite
        {
            return Err(invalid("outcome does not belong to a Task Git writer"));
        }
        let now = crate::canonical_clock::project_mutation_time(&context.project);
        let wait = next_stage_wait(
            &context.version,
            task,
            &submission.outcome,
            self.actors.core,
            now,
        )?;
        // Reject structurally invalid Pipeline evidence while the writer can still
        // repair its submission. The clone is not persisted or marked complete.
        match context.version.apply_outcome(
            &mut task.clone(),
            forge_domain::StageOutcomeSubmission {
                outcome: submission.outcome.clone(),
                outcome_artifact_ids: artifact_ids.clone(),
                submitted_by: employee_actor(&context.run)?,
                submitted_at: now,
                cancellation_reason_id: submission.cancellation_reason_id.clone(),
                cancellation_note: submission.note.clone(),
            },
            context.project.cancellation_reasons(),
            None,
            wait,
        ) {
            Ok(_) | Err(DomainError::StageVisitLimitExceeded { .. }) => {}
            Err(error) => return Err(error.into()),
        }
        let proposal = GitStageProposal {
            id: submission.scope.message_id,
            project_id: context.project.id(),
            task_id: task.id(),
            pipeline_version_id: task.pipeline().pipeline_version_id(),
            stage_id: submission.stage_id,
            stage_visit: task
                .current_stage_visit()
                .ok_or_else(|| invalid("missing stage visit"))?
                .get(),
            task_revision: task.revision().get(),
            employee_id: context.run.require_employee_id()?,
            scope: RunScope {
                run_id: context.run.id,
                fencing_token: submission.scope.lease_fencing_token,
                environment_epoch: submission.scope.environment_epoch,
            },
            surface_id: binding.surface_id,
            outcome: submission.outcome,
            artifact_ids,
            note: submission.note,
            cancellation_reason_id: submission.cancellation_reason_id,
            expected_commit: commit,
            submitted_at: now,
        };
        tx.insert_git_proposal(&proposal).await?;
        if tx
            .request_run_stop(
                context.run.id,
                proposal.scope.fencing_token,
                proposal.scope.environment_epoch,
                false,
            )
            .await?
            != FencedWrite::Applied
        {
            return Err(invalid("writer no longer owns a stoppable Run"));
        }
        let previous = context.project.revision();
        context.project.record_child_mutation(now)?;
        tx.update_project(&context.project, previous).await?;
        tx.append_event_and_outbox(&event(
            context.project.id(),
            forge_domain::AggregateRef::Project(context.project.id()),
            context.project.revision(),
            DomainEventKind::GitStageProposed,
            employee_actor(&context.run)?,
            CommandId::from(proposal.id),
            None,
            event_payload([
                ("proposal_id", json!(proposal.id)),
                ("task_id", json!(proposal.task_id)),
                ("run_id", json!(proposal.scope.run_id)),
                ("candidate_commit", json!(proposal.expected_commit)),
                ("state", json!(GitProposalState::AwaitingQuiescence)),
            ]),
            now,
        )?)
        .await?;
        tx.commit().await?;
        // Durable desired state is authoritative; watchdog retries a lost send.
        let _ = self
            .deliver_pending_stop_requests(context.project.id())
            .await;
        Ok(InboundResult::ProposalSaved)
    }
}
pub(super) fn invalid(reason: &str) -> CoreError {
    CoreError::InvalidTransport {
        field: "git_candidate",
        reason: reason.into(),
    }
}
