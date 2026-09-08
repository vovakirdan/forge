//! Revision-bound read-only outcomes and Pipeline-authorized assessment records.
use super::{
    executor::{SubmissionContext, employee_actor},
    git_proposal::invalid,
    validation::ParsedStageOutcomeSubmission,
};
use crate::{
    CoreError, CoreService,
    event::{event, event_payload},
};
use forge_domain::{
    AggregateRef, ArtifactId, CommandId, DomainEventKind,
    candidate_review::CandidateReviewRecord,
    runtime::{RunScope, SandboxRunSpec, SurfaceAccess, SurfaceSpec},
};
use forge_storage::StorageTransaction;
use serde_json::json;
use std::collections::BTreeSet;

impl CoreService {
    pub(super) async fn record_readonly_candidate_outcome(
        &self,
        tx: &mut StorageTransaction<'_>,
        context: &SubmissionContext,
        submission: &ParsedStageOutcomeSubmission,
        artifact_ids: &BTreeSet<ArtifactId>,
    ) -> Result<(), CoreError> {
        let task = &context.stored_task.task;
        let run = &context.run;
        let spec: SandboxRunSpec = serde_json::from_value(run.run_spec.clone())
            .map_err(|_| invalid("invalid read-only RunSpec"))?;
        spec.validate()
            .map_err(|_| invalid("invalid read-only RunSpec"))?;
        let SurfaceSpec::GitCandidateSnapshot { candidate, .. } = &spec.binding.surface else {
            return Err(invalid(
                "read-only Git result requires a pinned commit snapshot",
            ));
        };
        if spec.binding.access != SurfaceAccess::ReadOnly
            || submission.candidate_commit.as_ref() != Some(&candidate.commit)
        {
            return Err(invalid(
                "read-only outcome must cite the exact pinned candidate_commit",
            ));
        }
        let current = tx
            .latest_accepted_git_candidate(context.project.id(), task.id())
            .await?
            .filter(|current| {
                current.candidate == *candidate && current.surface_id == spec.surface_id
            })
            .ok_or_else(|| invalid("assessment revision is stale or unaccepted"))?;
        let stage = context
            .version
            .stage(&submission.stage_id)
            .ok_or_else(|| invalid("stage is absent"))?;
        let Some(policy) = stage.acceptance_policy() else {
            return Ok(());
        };
        policy.validate_for(stage)?;
        if policy.requires_independence()
            && !current.permits_independent_reviewer(run.require_employee_id()?)
        {
            return Err(invalid(
                "a contributing writer cannot supply an independent assessment",
            ));
        }
        let now = crate::canonical_clock::project_mutation_time(&context.project);
        let review = CandidateReviewRecord {
            id: submission.scope.message_id,
            project_id: context.project.id(),
            task_id: task.id(),
            pipeline_version_id: context.version.id(),
            stage_id: submission.stage_id.clone(),
            stage_visit: task
                .current_stage_visit()
                .ok_or_else(|| invalid("missing assessment stage visit"))?
                .get(),
            employee_id: run.require_employee_id()?,
            scope: RunScope {
                run_id: run.id,
                fencing_token: run.lease_fencing_token,
                environment_epoch: run.environment_epoch,
            },
            candidate_proposal_id: current.proposal_id,
            candidate: current.candidate,
            subject_artifact_ids: current.artifact_ids,
            verdict_artifact_ids: artifact_ids.clone(),
            outcome: submission.outcome.clone(),
            verdict: policy
                .verdict(&submission.outcome)
                .ok_or_else(|| invalid("Pipeline has no verdict for this outcome"))?,
            recorded_at: now,
        };
        tx.insert_candidate_review(&review).await?;
        tx.append_event_and_outbox(&event(
            context.project.id(),
            AggregateRef::Task(task.id()),
            task.revision().get(),
            DomainEventKind::CandidateReviewed,
            employee_actor(run)?,
            CommandId::from(review.id),
            None,
            event_payload([("review", json!(review))]),
            now,
        )?)
        .await?;
        Ok(())
    }
}
