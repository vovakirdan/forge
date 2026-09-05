//! Atomic M0 queue dispatch from canonical Task state to a fenced Run.

use forge_domain::{
    AggregateRef, ArtifactRequirementScope, CommandId, DomainEventKind, ExecutorKind,
    LifecycleStatus, PipelineStage, ProjectId, Task, Timestamp,
};
use forge_protocol::supervisor::v1::{ProvisionRun, core_to_supervisor};
use forge_protocol::{supervisor::v1::CoreToSupervisor, wire::M0_RUN_SPEC_VERSION};
use forge_storage::{LeaseRunRequest, QueueEntry, StorageTransaction};
use serde_json::{Value, json};
use time::Duration;
use tracing::warn;
use uuid::Uuid;

use crate::{
    CoreError, CoreService,
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
        if !self.supervisor.is_connected().await {
            return Ok(0);
        }

        let mut delivered = 0_usize;
        while let Some(provision) = self.claim_and_provision(project_id).await? {
            match self.supervisor.send(provision).await {
                Ok(()) => delivered = delivered.saturating_add(1),
                Err(error) => {
                    // The Run remains durably provision-requested. Recovery will
                    // reconcile it after a Supervisor reconnect; never undo a
                    // committed fence merely because local delivery raced a drop.
                    warn!(error = %error, "could not deliver committed Run provision to Supervisor");
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
        let now = Timestamp::now_utc();
        let mut transaction = self.store.begin().await?;
        let Some(mut project) = transaction.lock_project(project_id).await? else {
            return Ok(None);
        };
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
        let Some(employee) = select_employee(&mut transaction, &queue_entry).await? else {
            transaction.release_queue_claim(queue_entry.id).await?;
            transaction.commit().await?;
            return Ok(None);
        };
        let stage =
            version
                .stage(&queue_entry.input.stage_id)
                .ok_or(CoreError::InvalidTransport {
                    field: "queue.stage_id",
                    reason: "is absent from the task pinned pipeline version".to_owned(),
                })?;
        let run_spec = fake_run_spec(stage, &scoped.task)?;
        let context_snapshot_id = Uuid::now_v7();
        let lease_request = LeaseRunRequest {
            queue_entry: queue_entry.clone(),
            employee_id: employee.id(),
            lease_id: Uuid::now_v7(),
            run_id: Uuid::now_v7(),
            lease_expires_at: Timestamp::from_offset_date_time(
                now.as_offset_date_time() + M0_LEASE_DURATION,
            ),
            task_work_surface_id: scoped.persistence.task_work_surface_id,
            lease_scope: json!({"m0": true}),
            resource_reservation: json!({}),
            run_spec_version: u16::try_from(M0_RUN_SPEC_VERSION).map_err(|_| {
                CoreError::InvalidTransport {
                    field: "m0_run_spec_version",
                    reason: "does not fit the storage contract".to_owned(),
                }
            })?,
            run_spec: run_spec.clone(),
            context_manifest: json!({
                "context_snapshot_id": context_snapshot_id,
                "task_revision_before_dispatch": expected_task_revision,
            }),
        };
        // Storage validates the claimed queue snapshot against the Task's old
        // revision. Starting the Task before this call would invalidate the
        // exact queue item that authorizes the Lease.
        let provisioned = transaction.create_lease_and_run(&lease_request).await?;

        let mut task = scoped.task;
        let starts_task = task.lifecycle() == LifecycleStatus::Ready;
        if starts_task {
            version.start_entry_employee_stage(&mut task, now)?;
        }
        task.record_run_attempt(now)?;
        let next_attempt_count =
            scoped
                .persistence
                .attempt_count
                .checked_add(1)
                .ok_or(CoreError::InvalidTransport {
                    field: "task.attempt_count",
                    reason: "cannot exceed u32::MAX".to_owned(),
                })?;
        let mut persistence =
            crate::scheduler::task_persistence(&project, &task, next_attempt_count)?;
        persistence.task_work_surface_id = scoped.persistence.task_work_surface_id;
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
                ("stage_id", json!(&provisioned.run.stage_id)),
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
        Ok(Some(CoreToSupervisor {
            message: Some(core_to_supervisor::Message::ProvisionRun(ProvisionRun {
                command_id: command_id.as_uuid().to_string(),
                run_id: provisioned.run.id.to_string(),
                task_id: task.id().as_uuid().to_string(),
                employee_id: employee.id().as_uuid().to_string(),
                stage_id: provisioned.run.stage_id,
                attempt: provisioned.run.attempt_number,
                lease_fencing_token: provisioned.lease_fencing_token,
                environment_epoch: provisioned.environment_epoch,
                context_snapshot_id: context_snapshot_id.to_string(),
                run_spec_json,
                run_spec_version: M0_RUN_SPEC_VERSION,
            })),
        }))
    }
}

async fn select_employee(
    transaction: &mut StorageTransaction<'_>,
    queue_entry: &QueueEntry,
) -> Result<Option<forge_domain::Employee>, CoreError> {
    let employees = transaction
        .lock_available_employees(queue_entry.input.project_id)
        .await?;
    Ok(employees.into_iter().find_map(|stored| {
        stored
            .employee
            .is_eligible_for(
                queue_entry.input.pipeline_version_id,
                &queue_entry.input.stage_id,
            )
            .then_some(stored.employee)
    }))
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
mod tests {
    use forge_domain::{
        Actor, ActorId, ArtifactKind, ArtifactRequirement, ArtifactRequirementScope, ExecutorKind,
        NewTask, OutcomeKey, PipelineId, PipelineStage, PipelineTransition,
        PipelineTransitionTarget, PipelineVersionId, PriorityScheme, ProjectId,
        ProjectLocalSequence, StageId, Task, TaskId, TaskKey, TaskKind, TaskPipelineBinding,
        TaskProperties, TaskPropertySchema, TaskScope, TaskSource, TaskSpec, TaskSpecInput,
        Timestamp,
    };
    use serde_json::json;

    use super::fake_run_spec;

    fn task_with_history_artifact(kind: &str) -> Task {
        let priority_scheme = PriorityScheme::default_three_levels().expect("priority scheme");
        let now = Timestamp::now_utc();
        let mut task = Task::new_draft(
            NewTask {
                id: TaskId::new(),
                project_id: ProjectId::new(),
                key: TaskKey::new(ProjectLocalSequence::new(1).expect("sequence")),
                kind: TaskKind::Delivery,
                spec: TaskSpec::new(TaskSpecInput {
                    title: "M0 fake test".to_owned(),
                    description: String::new(),
                    rationale: None,
                    definition_of_done: Some("Historical evidence is available".to_owned()),
                    scope: TaskScope::default(),
                    properties: TaskProperties::empty(),
                    labels: Default::default(),
                })
                .expect("task spec"),
                priority_level_id: priority_scheme.default_level_id().clone(),
                pipeline: TaskPipelineBinding::new(
                    PipelineId::new(),
                    PipelineVersionId::new(),
                    StageId::new("work").expect("stage"),
                ),
                source: TaskSource::Human,
                created_by: Actor::human(ActorId::new()),
                created_at: now,
            },
            &priority_scheme,
        )
        .expect("draft task");
        task.approve(&TaskPropertySchema::new([]).expect("schema"), now)
            .expect("approve task");
        let artifact = forge_domain::Artifact::new(
            forge_domain::ArtifactId::new(),
            forge_domain::NewArtifact {
                project_id: task.project_id(),
                kind: ArtifactKind::new(kind).expect("artifact kind"),
                title: "Historical evidence".to_owned(),
                body: forge_domain::ArtifactBody::inline_json(json!({})),
                metadata: json!({}),
                created_by: Actor::human(ActorId::new()),
                created_at: now,
            },
        )
        .expect("artifact");
        task.attach_artifact(
            &artifact,
            forge_domain::ArtifactProducer::Human,
            Actor::human(ActorId::new()),
            None,
            now,
        )
        .expect("attach artifact");
        task
    }

    #[test]
    fn fake_spec_supplies_each_current_stage_requirement() {
        let stage = PipelineStage::new(
            StageId::new("work").expect("stable stage key"),
            "Work",
            ExecutorKind::Employee,
            [PipelineTransition::new(
                OutcomeKey::new("implemented").expect("stable outcome key"),
                PipelineTransitionTarget::Done,
                vec![
                    ArtifactRequirement::new(
                        ArtifactKind::new("change_set").expect("stable artifact key"),
                        2,
                    )
                    .expect("positive requirement"),
                ],
            )
            .expect("valid transition")],
        )
        .expect("valid stage");

        let task = task_with_history_artifact("unrelated");
        let spec = fake_run_spec(&stage, &task).expect("compatible M0 stage");

        assert_eq!(spec["artifacts"].as_array().map(Vec::len), Some(2));
        assert_eq!(spec["stage_outcome"]["outcome"], "implemented");
    }

    #[test]
    fn fake_spec_cites_satisfied_task_history_evidence() {
        let requirement = ArtifactRequirement::new_with_scope(
            ArtifactKind::new("change_set").expect("artifact key"),
            1,
            ArtifactRequirementScope::TaskHistory,
        )
        .expect("positive requirement");
        let stage = PipelineStage::new(
            StageId::new("review").expect("stable stage key"),
            "Review",
            ExecutorKind::Employee,
            [PipelineTransition::new(
                OutcomeKey::new("approved").expect("stable outcome key"),
                PipelineTransitionTarget::Done,
                vec![requirement],
            )
            .expect("valid transition")],
        )
        .expect("valid stage");
        let task = task_with_history_artifact("change_set");

        let spec = fake_run_spec(&stage, &task).expect("history evidence is available");

        assert!(spec["artifacts"].as_array().is_some_and(Vec::is_empty));
        assert_eq!(
            spec["stage_outcome"]["artifact_ids"]
                .as_array()
                .map(Vec::len),
            Some(1)
        );
    }
}
