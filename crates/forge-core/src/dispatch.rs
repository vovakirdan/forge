//! Atomic M0 queue dispatch from canonical Task state to a fenced Run.

use forge_domain::{
    AggregateRef, ArtifactRequirementScope, CommandId, DomainEventKind, ExecutorKind,
    LifecycleStatus, PipelineStage, ProjectId, Task, Timestamp,
};
use forge_protocol::supervisor::v1::CoreToSupervisor;
use forge_protocol::supervisor::v1::{ProvisionRun, core_to_supervisor};
use forge_storage::LeaseRunRequest;
use serde_json::{Value, json};
use time::Duration;
use tracing::warn;
use uuid::Uuid;

use crate::{
    CoreError, CoreService,
    dispatch_constraint::{EmployeeSelection, persist_constraint_result, select_employee},
    event::{event, event_payload},
    task_support::{
        current_stage_executor, load_scoped_task, persist_task_and_project,
        pinned_pipeline_version, stage_payload,
    },
};

const M0_LEASE_DURATION: Duration = Duration::minutes(5);

impl CoreService {
    /// Makes all currently dispatchable work available to the connected local
    /// Supervisor. Each Run is committed before its desired provision request
    /// crosses the process boundary.
    pub async fn dispatch_available(&self, project_id: ProjectId) -> Result<usize, CoreError> {
        let started = std::time::Instant::now();
        let parent = crate::observability::current_traceparent();
        let result = crate::observability::trace_operation(
            Some(&parent),
            crate::observability::Operation::Scheduler,
            self.dispatch_available_inner(project_id),
        )
        .await;
        self.record_operation(
            crate::observability::Operation::Scheduler,
            result.is_ok(),
            started.elapsed(),
        );
        result
    }

    async fn dispatch_available_inner(&self, project_id: ProjectId) -> Result<usize, CoreError> {
        if !self.supervisor.is_connected().await {
            return Ok(0);
        }
        if !self.fake_runtime_enabled && !self.supervisor.is_reconciled().await {
            return Ok(0);
        }

        let mut delivered = 0_usize;
        loop {
            let next = match self.claim_and_provision(project_id).await? {
                Some(provision) => Some(provision),
                None => match self.claim_hook_provision(project_id).await? {
                    Some(provision) => Some(provision),
                    None => match self.claim_resolution_provision(project_id).await? {
                        Some(provision) => Some(provision),
                        None => self.claim_communication_provision(project_id).await?,
                    },
                },
            };
            let Some(mut provision) = next else {
                break;
            };
            if !self.fake_runtime_enabled
                && let Some(core_to_supervisor::Message::ProvisionRun(request)) =
                    &mut provision.message
                && request.run_spec_version != 5
            {
                let run_id = Uuid::parse_str(&request.run_id)
                    .map_err(|_| crate::credentials::credential_error())?;
                if let Err(error) = self.prepare_run(run_id).await {
                    self.fail_preparation(run_id).await?;
                    warn!(error=%error,"Run preparation failed before Supervisor delivery");
                    continue;
                }
                request.traceparent =
                    self.prepared_traceparent(run_id, request.environment_epoch)?;
            }
            // Serialize the final delivery with Project stop. Preparation can
            // take time, so its earlier authorization is not a delivery permit.
            let mut delivery = self.store.begin().await?;
            if !self.fake_runtime_enabled
                && let Some(core_to_supervisor::Message::ProvisionRun(request)) = &provision.message
            {
                let scope = forge_domain::runtime::RunScope {
                    run_id: Uuid::parse_str(&request.run_id)
                        .map_err(|_| crate::credentials::credential_error())?,
                    fencing_token: request.lease_fencing_token,
                    environment_epoch: request.environment_epoch,
                };
                let active = if request.run_spec_version == 5 {
                    delivery
                        .hook_scope_is_active(scope, self.instance_id)
                        .await?
                } else {
                    delivery.gateway_scope_is_active(scope).await?
                };
                if !active {
                    delivery.commit().await?;
                    self.fail_preparation(scope.run_id).await?;
                    continue;
                }
            }
            let sent = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                self.supervisor.send(provision),
            )
            .await;
            delivery.commit().await?;
            match sent {
                Ok(Ok(())) => delivered = delivered.saturating_add(1),
                error => {
                    // The Run remains durably provision-requested. Recovery will
                    // reconcile it after a Supervisor reconnect; never undo a
                    // committed fence merely because local delivery raced a drop.
                    warn!(error = ?error, "could not deliver committed Run provision to Supervisor");
                    break;
                }
            }
        }
        Ok(delivered)
    }

    async fn claim_and_provision(
        &self,
        project_id: ProjectId,
    ) -> Result<Option<CoreToSupervisor>, CoreError> {
        loop {
            let mut transaction = self.store.begin().await?;
            let Some(mut project) = transaction.lock_project(project_id).await? else {
                return Ok(None);
            };
            let observed_wall = Timestamp::now_utc();
            let now = crate::canonical_clock::project_mutation_time_at(&project, observed_wall);
            let Some(queue_entry) = transaction.claim_next(project_id).await? else {
                transaction.commit().await?;
                return Ok(None);
            };
            let scoped =
                load_scoped_task(&mut transaction, &project, queue_entry.input.task_id).await?;
            let expected_task_revision = scoped.task.revision().get();
            if expected_task_revision != queue_entry.input.task_revision {
                transaction.release_queue_claim(queue_entry.id).await?;
                transaction.commit().await?;
                return Ok(None);
            }
            let version = pinned_pipeline_version(&mut transaction, &project, &scoped.task).await?;
            let executor = current_stage_executor(&version, &scoped.task)?;
            if executor != ExecutorKind::Employee {
                transaction.release_queue_claim(queue_entry.id).await?;
                transaction.commit().await?;
                return Ok(None);
            }
            let (employee, dispatch_constraint) =
                match select_employee(&mut transaction, &queue_entry, &scoped.task).await? {
                    EmployeeSelection::Ready {
                        employee,
                        constraint,
                    } => (employee, constraint),
                    EmployeeSelection::Unavailable => {
                        transaction.release_queue_claim(queue_entry.id).await?;
                        transaction.commit().await?;
                        return Ok(None);
                    }
                    EmployeeSelection::ChangeConstraint { constraint, result } => {
                        persist_constraint_result(
                            &mut transaction,
                            &mut project,
                            &scoped.task,
                            constraint,
                            result,
                            self.actors.core,
                            CommandId::new(),
                            now,
                        )
                        .await?;
                        transaction.release_queue_claim(queue_entry.id).await?;
                        transaction.commit().await?;
                        // A held/expired constraint changes only its own admission result;
                        // continue this pass so another Task is not stranded behind it.
                        continue;
                    }
                };
            let stage =
                version
                    .stage(&queue_entry.input.stage_id)
                    .ok_or(CoreError::InvalidTransport {
                        field: "queue.stage_id",
                        reason: "is absent from the task pinned pipeline version".to_owned(),
                    })?;
            let surface_id = scoped
                .persistence
                .task_work_surface_id
                .unwrap_or_else(Uuid::now_v7);
            if let forge_domain::TaskWorkSurface::Git(git) = scoped.task.work_surface() {
                if self.fake_runtime_enabled
                    || scoped.persistence.task_work_surface_id != Some(git.surface_id)
                {
                    return Err(CoreError::InvalidTransport {
                        field: "task.work_surface",
                        reason: "Git-bound Tasks require their pinned real execution surface"
                            .into(),
                    });
                }
                let repository = transaction
                    .load_project_repository(git.repository_id)
                    .await?
                    .filter(|repository| {
                        repository.project_id == project_id && git.matches_repository(repository)
                    })
                    .ok_or(CoreError::InvalidTransport {
                        field: "task.work_surface",
                        reason: "Task source does not match its registered Project repository"
                            .into(),
                    })?;
                repository.validate_snapshot()?;
            }
            let mut pinned_credential = None;
            let (run_spec_version, run_spec) = if self.fake_runtime_enabled {
                (1_u16, fake_run_spec(stage, &scoped.task)?)
            } else {
                let mut source_request = None;
                let mut binding = transaction
                    .runtime_binding(employee.id())
                    .await?
                    .ok_or(CoreError::InvalidTransport {
                    field: "employee.runtime_binding",
                    reason:
                        "an explicit sandbox runtime profile is required; fake fallback is disabled"
                            .into(),
                })?;
                if let forge_domain::TaskWorkSurface::Git(git) = scoped.task.work_surface() {
                    let repository = git.source.as_path().to_string_lossy().into_owned();
                    binding.surface = if binding.access
                        == forge_domain::runtime::SurfaceAccess::ReadOnly
                    {
                        let accepted = transaction.latest_accepted_git_candidate(project_id, scoped.task.id()).await?
                        .filter(|candidate| candidate.surface_id == git.surface_id)
                        .ok_or(CoreError::InvalidTransport {
                            field: "task.work_surface",
                            reason: "read-only Git execution requires an accepted candidate for this Task surface".into(),
                        })?;
                        match &git.initial_base {
                            forge_domain::git::GitInitialRevision::Commit(base) => {
                                forge_domain::runtime::SurfaceSpec::GitCandidateSnapshot {
                                    repository,
                                    base_ref: base.as_str().to_owned(),
                                    candidate: accepted.candidate,
                                }
                            }
                            forge_domain::git::GitInitialRevision::Unborn { object_format } => {
                                forge_domain::runtime::SurfaceSpec::GitUnbornCandidateSnapshot {
                                    repository,
                                    object_format: *object_format,
                                    candidate: accepted.candidate,
                                }
                            }
                        }
                    } else {
                        let setting = transaction
                            .task_git_source_setting(project_id, scoped.task.id())
                            .await?;
                        source_request = Some(forge_domain::git::GitSourceRequest {
                            repository_id: git.repository_id,
                            source: git.source.clone(),
                            target_ref: git.target_ref.clone(),
                            initial_revision: git.initial_base.clone(),
                            policy_revision: setting.revision,
                            policy: setting.policy,
                        });
                        match &git.initial_base {
                            forge_domain::git::GitInitialRevision::Commit(base) => {
                                forge_domain::runtime::SurfaceSpec::GitWorktree {
                                    repository,
                                    base_ref: base.as_str().to_owned(),
                                }
                            }
                            forge_domain::git::GitInitialRevision::Unborn { object_format } => {
                                forge_domain::runtime::SurfaceSpec::GitUnborn {
                                    repository,
                                    object_format: *object_format,
                                }
                            }
                        }
                    };
                }
                binding
                    .validate()
                    .map_err(|error| CoreError::InvalidTransport {
                        field: "runtime_binding",
                        reason: error.to_string(),
                    })?;
                if stage.workspace().is_some_and(|requirement| {
                    !requirement.is_compatible(&binding.surface, binding.access)
                }) {
                    return Err(CoreError::InvalidTransport {
                    field: "stage.workspace",
                    reason: "the assigned runtime does not satisfy the pinned stage workspace requirement".into(),
                });
                }
                if self.execution.is_none() {
                    return Err(crate::credentials::credential_error());
                }
                let record = transaction
                    .load_credential(
                        project_id,
                        binding.execution_profile.credential_binding().secret_id,
                    )
                    .await?
                    .ok_or_else(crate::credentials::credential_error)?;
                let _checked = self.open_runtime_credential(
                    &record,
                    binding.execution_profile.credential_binding(),
                    binding.execution_profile.adapter_id(),
                )?;
                pinned_credential = Some(record);
                let instruction = serde_json::to_string(&json!({
                "task": scoped.task.spec(), "stage": stage,
                "previous_handoff": transaction.latest_handoff(scoped.task.id()).await?,
                "git_source_input": source_request.as_ref().map(|_| "Read /run/forge-source/descriptor.json for this Run's immutable source selection. A committed selection includes /run/forge-source/source.bundle; fetch its selected_revision into your own Git refs and explicitly merge/rebase when needed. Forge preserves retained files and never resets your branch or performs a semantic merge. An unborn selection has no bundle or existing commit. Do not access another Task's host directory."),
                "submission_policy": if matches!(scoped.task.work_surface(), forge_domain::TaskWorkSurface::Git(_))
                    && binding.access == forge_domain::runtime::SurfaceAccess::ReadWrite {
                    "Use Forge Gateway to attach artifacts and explicitly submit a permitted stage outcome with candidate_commit set to the full committed HEAD SHA. Commit the intended changes yourself and leave tracked/untracked work clean before submitting; Forge never automatically adds, commits, or discards your files. Submission is a proposal: final acceptance follows physical stop and exact clean-HEAD inspection. Exit status alone does not complete the Task."
                } else {
                    "Use Forge Gateway to attach artifacts and explicitly submit one of the permitted stage outcomes. Exit status alone does not complete the task."
                },
            })).map_err(|_| CoreError::InvalidTransport { field: "context", reason: "cannot serialize task contract".into() })?;
                let spec = forge_domain::runtime::TaskRunSpecV6 {
                    schema_version: forge_domain::runtime::TASK_RUN_SPEC_VERSION,
                    project_id,
                    surface_id,
                    binding,
                    instruction,
                    source_request,
                    file_inputs: transaction
                        .task_file_inputs(project_id, scoped.task.id())
                        .await?,
                };
                spec.validate()
                    .map_err(|error| CoreError::InvalidTransport {
                        field: "run_spec",
                        reason: error.to_string(),
                    })?;
                (
                    spec.schema_version,
                    serde_json::to_value(spec).map_err(|_| CoreError::InvalidTransport {
                        field: "run_spec",
                        reason: "cannot serialize runtime intent".into(),
                    })?,
                )
            };
            let context_snapshot_id = Uuid::now_v7();
            let run_id = Uuid::now_v7();
            let previous_handoff = transaction
                .latest_handoff(scoped.task.id())
                .await?
                .map(serde_json::from_value::<forge_domain::TaskHandoff>)
                .transpose()
                .map_err(|_| CoreError::InvalidTransport {
                    field: "handoff",
                    reason: "stored handoff violates its domain contract".into(),
                })?;
            let profile_revision = run_spec
                .get("binding")
                .and_then(|value| value.get("execution_profile"))
                .map(|value| {
                    format!(
                        "{}:{}",
                        value["id"].as_str().unwrap_or("unknown"),
                        value["revision"]
                    )
                })
                .unwrap_or_else(|| "fake_m0:1".into());
            let context = forge_domain::ContextSnapshot::new(forge_domain::ContextSnapshotInput {
                context_snapshot_id,
                project_id,
                task_id: scoped.task.id(),
                run_id,
                employee_id: employee.id(),
                pipeline_version_id: queue_entry.input.pipeline_version_id,
                stage_id: queue_entry.input.stage_id.clone(),
                stage_visit: scoped.task.current_stage_visit().map(|visit| visit.get()),
                task_revision_before_dispatch: expected_task_revision,
                task_spec: scoped.task.spec().clone(),
                system_policy_revision: profile_revision.clone(),
                employee_prompt_revision: profile_revision,
                capability_grants: [
                    "board.list",
                    "task.read",
                    "artifact.submit",
                    "outcome.submit",
                    "progress.report",
                    "human.request",
                    "escalation.raise",
                    "finding.report",
                    "inbox.list",
                    "inbox.acknowledge",
                    "inbox.reply",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect(),
                tool_catalog_revision: "forge_task_tools_v2".into(),
                run_spec_id: run_id,
                prior_handoff: previous_handoff,
                artifacts: vec![],
                control_instruction: None,
                created_at: now,
            })?;
            let lease_request = LeaseRunRequest {
                queue_entry: queue_entry.clone(),
                employee_id: employee.id(),
                lease_id: Uuid::now_v7(),
                run_id,
                lease_expires_at: Timestamp::from_offset_date_time(
                    observed_wall.as_offset_date_time() + M0_LEASE_DURATION,
                ),
                task_work_surface_id: (!self.fake_runtime_enabled).then_some(surface_id),
                lease_scope: json!({"simulator": self.fake_runtime_enabled}),
                resource_reservation: json!({}),
                run_spec_version,
                run_spec: run_spec.clone(),
                context_manifest: serde_json::to_value(context).map_err(|_| {
                    CoreError::InvalidTransport {
                        field: "context",
                        reason: "cannot serialize context snapshot".into(),
                    }
                })?,
            };
            // Storage validates the claimed queue snapshot against the Task's old
            // revision. Starting the Task before this call would invalidate the
            // exact queue item that authorizes the Lease.
            let provisioned = transaction.create_lease_and_run(&lease_request).await?;
            if !self.fake_runtime_enabled {
                transaction
                    .reserve_environment(provisioned.run.id, Some(surface_id))
                    .await?;
                if let Some(record) = pinned_credential {
                    transaction
                        .pin_run_credential(provisioned.run.id, &record)
                        .await?;
                }
            }

            let mut task = scoped.task;
            let starts_task = task.lifecycle() == LifecycleStatus::Ready;
            if starts_task {
                version.start_entry_employee_stage(&mut task, now)?;
            }
            task.record_run_attempt(now)?;
            let next_attempt_count = scoped.persistence.attempt_count.checked_add(1).ok_or(
                CoreError::InvalidTransport {
                    field: "task.attempt_count",
                    reason: "cannot exceed u32::MAX".to_owned(),
                },
            )?;
            let mut persistence =
                crate::scheduler::task_persistence(&project, &task, next_attempt_count)?;
            persistence.task_work_surface_id = lease_request.task_work_surface_id;
            persist_task_and_project(
                &mut transaction,
                &mut project,
                &task,
                persistence,
                expected_task_revision,
                now,
            )
            .await?;

            let command_id = CommandId::new();
            if let Some(constraint) = dispatch_constraint {
                persist_constraint_result(
                    &mut transaction,
                    &mut project,
                    &task,
                    constraint,
                    forge_domain::NextRunConstraintState::Consumed {
                        run_id: provisioned.run.id,
                    },
                    self.actors.core,
                    command_id,
                    now,
                )
                .await?;
            }
            if starts_task {
                let started = event(
                    project.id(),
                    AggregateRef::Task(task.id()),
                    task.revision().get(),
                    DomainEventKind::TaskStarted,
                    self.actors.core,
                    command_id,
                    None,
                    stage_payload(&task),
                    now,
                )?;
                transaction.append_event_and_outbox(&started).await?;
            }
            let provisioned_event = event(
                project.id(),
                AggregateRef::Task(task.id()),
                task.revision().get(),
                DomainEventKind::RunProvisioned,
                self.actors.core,
                command_id,
                None,
                event_payload([
                    ("run_id", json!(provisioned.run.id)),
                    ("employee_id", json!(employee.id().as_uuid())),
                    (
                        "stage_id",
                        json!(&provisioned.run.require_task_stage()?.stage_id),
                    ),
                    ("attempt", json!(provisioned.run.attempt_number)),
                    (
                        "lease_fencing_token",
                        json!(provisioned.lease_fencing_token),
                    ),
                    ("environment_epoch", json!(provisioned.environment_epoch)),
                ]),
                now,
            )?;
            transaction
                .append_event_and_outbox(&provisioned_event)
                .await?;
            transaction.commit().await?;

            let run_spec_json =
                serde_json::to_string(&run_spec).map_err(|error| CoreError::InvalidTransport {
                    field: "m0_run_spec",
                    reason: error.to_string(),
                })?;
            return Ok(Some(CoreToSupervisor {
                message: Some(core_to_supervisor::Message::ProvisionRun(ProvisionRun {
                    command_id: command_id.as_uuid().to_string(),
                    run_id: provisioned.run.id.to_string(),
                    task_id: task.id().as_uuid().to_string(),
                    employee_id: employee.id().as_uuid().to_string(),
                    stage_id: provisioned.run.require_task_stage()?.stage_id.to_string(),
                    attempt: provisioned.run.attempt_number,
                    lease_fencing_token: provisioned.lease_fencing_token,
                    environment_epoch: provisioned.environment_epoch,
                    context_snapshot_id: context_snapshot_id.to_string(),
                    run_spec_json,
                    run_spec_version: u32::from(run_spec_version),
                    traceparent: crate::observability::current_traceparent(),
                    assignment: (!self.fake_runtime_enabled).then(|| {
                        forge_protocol::supervisor::v1::provision_run::Assignment::TaskStage(
                            forge_protocol::supervisor::v1::TaskStageExecutionAssignment {
                                task_id: task.id().to_string(),
                                stage_id: queue_entry.input.stage_id.to_string(),
                                queue_entry_id: queue_entry.id.to_string(),
                            },
                        )
                    }),
                })),
            }));
        }
    }
}

fn fake_run_spec(stage: &PipelineStage, task: &Task) -> Result<Value, CoreError> {
    let transition = stage
        .transitions()
        .find(|candidate| {
            candidate
                .artifact_requirements()
                .iter()
                .filter(|requirement| requirement.scope() == ArtifactRequirementScope::TaskHistory)
                .all(|requirement| {
                    task.artifact_links()
                        .iter()
                        .filter(|link| link.kind() == requirement.kind())
                        .count()
                        >= usize::from(requirement.minimum_count())
                })
        })
        .ok_or(CoreError::InvalidTransport {
            field: "pipeline.stage.transitions",
            reason: "has no M0-fake-compatible outcome with the Task history evidence currently available"
                .to_owned(),
        })?;
    let mut artifacts = Vec::new();
    let mut task_history_artifact_ids = Vec::new();
    for requirement in transition.artifact_requirements() {
        match requirement.scope() {
            ArtifactRequirementScope::CurrentStage => {
                for ordinal in 1..=requirement.minimum_count() {
                    artifacts.push(json!({
                        "kind": requirement.kind().as_str(),
                        "title": format!("M0 {} evidence {ordinal}", requirement.kind().as_str()),
                        "metadata": {
                            "generated_by": "forge_core_m0",
                            "stage_id": stage.id().as_str(),
                        },
                        "body": {
                            "stage_id": stage.id().as_str(),
                            "outcome": transition.outcome().as_str(),
                            "ordinal": ordinal,
                        },
                    }));
                }
            }
            ArtifactRequirementScope::TaskHistory => {
                task_history_artifact_ids.extend(
                    task.artifact_links()
                        .iter()
                        .filter(|link| link.kind() == requirement.kind())
                        .take(usize::from(requirement.minimum_count()))
                        .map(|link| link.artifact_id().as_uuid().to_string()),
                );
            }
        }
    }
    task_history_artifact_ids.sort_unstable();
    task_history_artifact_ids.dedup();
    let cancellation_reason_key = matches!(
        transition.target(),
        forge_domain::PipelineTransitionTarget::Cancelled
    )
    .then_some("unspecified");
    Ok(json!({
        "artifacts": artifacts,
        "stage_outcome": {
            "outcome": transition.outcome().as_str(),
            "note": "deterministic_m0_simulator",
            "artifact_ids": task_history_artifact_ids,
            "cancellation_reason_key": cancellation_reason_key,
        },
        "delay_ms": 0,
    }))
}

#[cfg(test)]
#[path = "dispatch/tests.rs"]
mod tests;
