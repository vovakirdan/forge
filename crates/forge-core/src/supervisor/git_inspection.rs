//! Post-quiescence observation and Pipeline acceptance are separate transactions.
use super::{
    SupervisorIdentity,
    bridge::InboundResult,
    git_proposal::invalid,
    validation::{ParsedStageOutcomeSubmission, SubmissionIngress, SubmissionScope},
};
use crate::{
    CoreError, CoreService,
    event::{event, event_payload},
    task_support::load_scoped_task,
};
use forge_domain::{
    AggregateRef, CommandId, DomainEventKind, Project, ProjectExecutionGate,
    communication::TaskMessageContext,
    git::{GitCandidate, GitObjectId},
    git_delivery::{GitProposalState, GitStageProposal, TaskGitCandidate},
};
use forge_protocol::supervisor::v1::{
    CoreToSupervisor, GitCandidateInspectionCode, GitCandidateInspectionResult,
    InspectGitCandidate, core_to_supervisor,
};
use forge_storage::{GitInspectionRequest, RunObservedState, StorageTransaction};
use serde_json::json;
use uuid::Uuid;

impl CoreService {
    /// Bounded durable-intent reconciliation. Retries do not launch model work.
    pub(crate) async fn reconcile_git_proposals(&self) -> Result<(), CoreError> {
        let Some(identity) = self.supervisor.reconciled_identity().await else {
            return Ok(());
        };
        for run_id in self.store.pending_git_proposal_runs().await? {
            let mut tx = self.store.begin().await?;
            let Some(run) = tx.load_run(run_id).await? else {
                continue;
            };
            let Some(mut project) = tx.lock_project(run.project_id).await? else {
                continue;
            };
            let Some(stored) = tx.load_git_proposal(run_id).await? else {
                continue;
            };
            let proposal = &stored.proposal;
            if !matches!(
                stored.state,
                GitProposalState::AwaitingQuiescence | GitProposalState::Inspecting
            ) {
                continue;
            }
            let task = load_scoped_task(&mut tx, &project, proposal.task_id)
                .await?
                .task;
            if !proposal_matches_task(proposal, &task) {
                self.hold_git_proposal(
                    &mut tx,
                    &mut project,
                    proposal,
                    "task_changed_after_submission",
                )
                .await?;
                tx.set_git_proposal_state(proposal.id, GitProposalState::Superseded)
                    .await?;
                tx.commit().await?;
                continue;
            }
            if !tx.git_writer_quiescent(proposal).await? {
                // Lost/failed is not positive physical stop. Runtime watchdog owns
                // containment; never inspect a potentially live writer.
                if matches!(
                    run.observed_state,
                    RunObservedState::Failed | RunObservedState::Lost
                ) {
                    self.hold_git_proposal(
                        &mut tx,
                        &mut project,
                        proposal,
                        "writer_quiescence_unknown",
                    )
                    .await?;
                    tx.commit().await?;
                }
                continue;
            }
            let request = if let Some(request) = stored.inspection {
                if tx.git_inspection_expired(proposal.id).await?
                    || request.host_id != identity.host_id
                    || request.boot_id != identity.boot_id
                {
                    self.hold_git_proposal(
                        &mut tx,
                        &mut project,
                        proposal,
                        "inspection_unavailable_or_host_changed",
                    )
                    .await?;
                    tx.commit().await?;
                    continue;
                }
                request
            } else {
                let request = GitInspectionRequest {
                    command_id: Uuid::now_v7(),
                    host_id: identity.host_id.clone(),
                    boot_id: identity.boot_id.clone(),
                };
                tx.begin_git_inspection(proposal.id, &request).await?;
                request
            };
            tx.commit().await?;
            self.supervisor
                .send(CoreToSupervisor {
                    message: Some(core_to_supervisor::Message::InspectGitCandidate(
                        InspectGitCandidate {
                            command_id: request.command_id.to_string(),
                            run_id: proposal.scope.run_id.to_string(),
                            lease_fencing_token: proposal.scope.fencing_token,
                            environment_epoch: proposal.scope.environment_epoch,
                            project_id: proposal.project_id.to_string(),
                            surface_id: proposal.surface_id.to_string(),
                            expected_commit: proposal.expected_commit.as_str().into(),
                            host_id: request.host_id,
                            boot_id: request.boot_id,
                        },
                    )),
                })
                .await?;
        }
        Ok(())
    }

    pub(super) async fn record_git_candidate_inspection(
        &self,
        identity: &SupervisorIdentity,
        result: GitCandidateInspectionResult,
    ) -> Result<InboundResult, CoreError> {
        let parse = |s: &str| {
            Uuid::parse_str(s)
                .ok()
                .filter(|id| id.get_version_num() == 7)
                .ok_or_else(|| invalid("invalid inspection identity"))
        };
        let _message_id = parse(&result.message_id)?;
        let run_id = parse(&result.run_id)?;
        let command_id = parse(&result.request_command_id)?;
        let code = GitCandidateInspectionCode::try_from(result.result_code)
            .map_err(|_| invalid("unknown inspection code"))?;
        if code == GitCandidateInspectionCode::Unspecified {
            return Err(invalid("missing inspection code"));
        }
        let candidate = if code == GitCandidateInspectionCode::Verified {
            Some(GitCandidate {
                commit: GitObjectId::new(result.commit.clone())
                    .map_err(|_| invalid("invalid commit"))?,
                tree: GitObjectId::new(result.tree.clone()).map_err(|_| invalid("invalid tree"))?,
            })
        } else {
            if !result.commit.is_empty() || !result.tree.is_empty() {
                return Err(invalid("refusal cannot claim a candidate"));
            }
            None
        };
        let mut tx = self.store.begin().await?;
        let run = tx
            .load_run(run_id)
            .await?
            .ok_or(CoreError::NotFound { aggregate: "Run" })?;
        let mut project = tx
            .lock_project(run.project_id)
            .await?
            .ok_or(CoreError::NotFound {
                aggregate: "Project",
            })?;
        let stored = tx
            .load_git_proposal(run_id)
            .await?
            .ok_or(CoreError::NotFound {
                aggregate: "Git proposal",
            })?;
        let proposal = &stored.proposal;
        if result.host_id != identity.host_id
            || result.boot_id != identity.boot_id
            || result.lease_fencing_token != proposal.scope.fencing_token
            || result.environment_epoch != proposal.scope.environment_epoch
        {
            return Err(invalid("inspection sender or fence mismatch"));
        }
        let safe_result = json!({"message_id":result.message_id,"command_id":command_id,"code":code.as_str_name(),
            "run_id":run_id,"fencing_token":result.lease_fencing_token,"environment_epoch":result.environment_epoch,
            "commit":result.commit,"tree":result.tree,"host_id":result.host_id,"boot_id":result.boot_id});
        if let Some(previous) = tx.git_inspection_receipt(proposal.id, command_id).await? {
            if previous != safe_result {
                return Err(CoreError::IdempotencyConflict);
            }
            return Ok(InboundResult::Accepted("Git inspection already recorded"));
        }
        let request = stored
            .inspection
            .as_ref()
            .ok_or_else(|| invalid("inspection was not requested"))?;
        if request.command_id != command_id
            || request.host_id != identity.host_id
            || request.boot_id != identity.boot_id
            || result.host_id != identity.host_id
            || result.boot_id != identity.boot_id
            || result.lease_fencing_token != proposal.scope.fencing_token
            || result.environment_epoch != proposal.scope.environment_epoch
        {
            return Err(invalid(
                "inspection does not match the requested exact scope",
            ));
        }
        if stored.state != GitProposalState::Inspecting {
            return Ok(InboundResult::Ignored("git_proposal_no_longer_pending"));
        }
        tx.record_git_inspection(proposal.id, &safe_result).await?;
        let now = crate::canonical_clock::project_mutation_time(&project);
        tx.append_event_and_outbox(&event(
            project.id(),
            AggregateRef::Project(project.id()),
            project.revision(),
            DomainEventKind::GitCandidateInspected,
            identity.actor(),
            CommandId::from(command_id),
            None,
            event_payload([("proposal_id", json!(proposal.id)), ("result", safe_result)]),
            now,
        )?)
        .await?;
        let Some(candidate) = candidate else {
            if matches!(
                code,
                GitCandidateInspectionCode::Busy | GitCandidateInspectionCode::Unavailable
            ) && tx.retry_busy_git_inspection(proposal.id).await?
            {
                tx.commit().await?;
                return Ok(InboundResult::Accepted(
                    "Transient Git inspection refusal retained; bounded retry pending",
                ));
            }
            self.hold_git_proposal(&mut tx, &mut project, proposal, code.as_str_name())
                .await?;
            tx.commit().await?;
            return Ok(InboundResult::Accepted(
                "Git inspection refusal retained; manager action required",
            ));
        };
        proposal.validate_observed_candidate(&candidate)?;
        if !tx.git_writer_quiescent(proposal).await? {
            return Err(invalid("writer is not positively quiescent"));
        }
        let mut contributors = tx.git_task_contributors(proposal.task_id).await?;
        contributors.insert(proposal.employee_id);
        tx.insert_task_git_candidate(&TaskGitCandidate {
            proposal_id: proposal.id,
            project_id: proposal.project_id,
            task_id: proposal.task_id,
            surface_id: proposal.surface_id,
            candidate,
            contributors,
            artifact_ids: proposal.artifact_ids.clone(),
            verified_at: now,
        })
        .await?;
        let task = load_scoped_task(&mut tx, &project, proposal.task_id)
            .await?
            .task;
        if !proposal_matches_task(proposal, &task) {
            self.hold_git_proposal(
                &mut tx,
                &mut project,
                proposal,
                "task_changed_after_submission",
            )
            .await?;
            tx.set_git_proposal_state(proposal.id, GitProposalState::Superseded)
                .await?;
            tx.commit().await?;
            return Ok(InboundResult::Accepted(
                "Verified candidate retained; Task has changed",
            ));
        }
        let message_context = TaskMessageContext {
            task_id: proposal.task_id,
            pipeline_version_id: proposal.pipeline_version_id,
            stage_id: proposal.stage_id.clone(),
            stage_visit: proposal.stage_visit,
        };
        if project.execution_gate() != ProjectExecutionGate::Open
            || !tx
                .pending_task_messages(project.id(), &message_context)
                .await?
                .is_empty()
        {
            self.hold_git_proposal(
                &mut tx,
                &mut project,
                proposal,
                "acceptance_blocked_by_project_or_instruction",
            )
            .await?;
            tx.commit().await?;
            return Ok(InboundResult::Accepted(
                "Candidate retained; final acceptance requires manager action",
            ));
        }
        let scope = SubmissionScope {
            message_id: proposal.id,
            run_id: proposal.scope.run_id,
            lease_fencing_token: proposal.scope.fencing_token,
            environment_epoch: proposal.scope.environment_epoch,
            sequence: 0,
            ingress: SubmissionIngress::Supervisor,
        };
        let context = self
            .load_employee_submission_context(&mut tx, &scope)
            .await?;
        let submission = ParsedStageOutcomeSubmission {
            scope,
            stage_id: proposal.stage_id.clone(),
            outcome: proposal.outcome.clone(),
            artifact_ids: proposal.artifact_ids.clone(),
            artifact_submission_message_ids: Vec::new(),
            note: proposal.note.clone(),
            cancellation_reason_id: proposal.cancellation_reason_id.clone(),
            candidate_commit: Some(proposal.expected_commit.clone()),
        };
        self.apply_resolved_employee_outcome(
            tx,
            context,
            submission,
            proposal.artifact_ids.clone(),
            Some(proposal.clone()),
        )
        .await
    }

    async fn hold_git_proposal(
        &self,
        tx: &mut StorageTransaction<'_>,
        project: &mut Project,
        proposal: &GitStageProposal,
        reason: &str,
    ) -> Result<(), CoreError> {
        tx.set_git_proposal_state(proposal.id, GitProposalState::NeedsAttention)
            .await?;
        tx.retire_git_proposal_queue(proposal).await?;
        let stored = load_scoped_task(tx, project, proposal.task_id).await?;
        let now = crate::canonical_clock::project_mutation_time(project);
        if proposal_owns_visit(proposal, &stored.task)
            && !tx.git_proposal_has_replacement_run(proposal).await?
        {
            let previous_revision = stored.task.revision().get();
            let mut task = stored.task;
            task.add_wait_condition(
                forge_domain::TaskWaitCondition::new(
                    forge_domain::WaitConditionId::new(),
                    forge_domain::TaskWaitKind::ManualPause,
                    Some(format!("git_proposal={}; {reason}", proposal.id)),
                    self.actors.core,
                    now,
                )?,
                now,
            )?;
            let persistence =
                crate::task_support::retained_persistence(project, &task, stored.persistence)?;
            crate::task_support::persist_task_and_project(
                tx,
                project,
                &task,
                persistence,
                previous_revision,
                now,
            )
            .await?;
            tx.append_event_and_outbox(&crate::task_support::task_event(
                project,
                &task,
                DomainEventKind::TaskWaiting,
                self.actors.core,
                CommandId::from(proposal.id),
                now,
                event_payload([
                    ("reason", json!(reason)),
                    ("proposal_id", json!(proposal.id)),
                ]),
            )?)
            .await?;
        }
        tx.append_event_and_outbox(&event(
            project.id(),
            AggregateRef::Project(project.id()),
            project.revision(),
            DomainEventKind::GitProposalNeedsAttention,
            self.actors.core,
            CommandId::from(proposal.id),
            None,
            event_payload([
                ("proposal_id", json!(proposal.id)),
                ("reason", json!(reason)),
            ]),
            now,
        )?)
        .await?;
        Ok(())
    }
}
fn proposal_matches_task(proposal: &GitStageProposal, task: &forge_domain::Task) -> bool {
    task.revision().get() == proposal.task_revision && proposal_owns_visit(proposal, task)
}
fn proposal_owns_visit(proposal: &GitStageProposal, task: &forge_domain::Task) -> bool {
    task.lifecycle() == forge_domain::LifecycleStatus::InProgress
        && task.pipeline().pipeline_version_id() == proposal.pipeline_version_id
        && task.current_stage_id() == Some(&proposal.stage_id)
        && task.current_stage_visit().map(|v| v.get()) == Some(proposal.stage_visit)
}
