use std::collections::BTreeMap;

use anyhow::{Context, Result};
use forge_domain::{PipelineVersion, PipelineVersionId, Project, ProjectId, Task, TaskId};
use forge_testkit::reference::{MemoryQueueState, MemorySnapshot};
use serde_json::{Value, json};
use sqlx::PgPool;

/// Assertion data only. No command policy or mutation exists in these readers.
pub struct Snapshot {
    pub projects: BTreeMap<ProjectId, Project>,
    pub tasks: BTreeMap<TaskId, Task>,
    pub versions: BTreeMap<PipelineVersionId, PipelineVersion>,
    pub raw: Value,
}

impl Snapshot {
    pub fn memory(snapshot: MemorySnapshot) -> Result<Self> {
        let tasks = snapshot
            .tasks
            .values()
            .map(|entry| {
                json!({"canonical_snapshot": entry.task,
                "priority_rank": entry.persistence.priority_rank,
                "attempt_count": entry.persistence.attempt_count,
                "task_work_surface_id": entry.persistence.task_work_surface_id})
            })
            .collect::<Vec<_>>();
        let events = snapshot
            .events
            .iter()
            .map(|entry| {
                let event = &entry.event;
                json!({"id": event.id(), "project_id": event.project_id(),
                "project_sequence": entry.project_sequence,
                "event_type": event.kind(), "aggregate": event.aggregate(),
                "aggregate_revision": event.aggregate_revision(),
                "actor": event.actor(), "command_id": event.command_id(),
                "reason": event.reason(), "payload": event.payload(),
                "occurred_at": event.occurred_at()})
            })
            .collect::<Vec<_>>();
        let outbox = snapshot
            .outbox
            .iter()
            .map(|entry| {
                json!({"id":entry.id,"project_id":entry.project_id,"event_id":entry.event_id,
                "subject":entry.subject,"envelope":entry.envelope})
            })
            .collect::<Vec<_>>();
        let receipts = snapshot
            .idempotency
            .values()
            .map(|entry| {
                json!({"project_id":entry.project_id,"idempotency_key":entry.key,
                "command_id":entry.command_id,"command_name":entry.command_name,
                "expected_revision":entry.expected_revision,"request_hash":entry.request_hash,
                "actor":entry.actor,"receipt":entry.receipt,"event_id":entry.event_id,
                "response_revision":entry.response_revision})
            })
            .collect::<Vec<_>>();
        let queue = snapshot.queue.iter().map(|entry| {
            json!({"id":entry.id,"project_id":entry.input.project_id,
                "task_id":entry.input.task_id,"task_sequence":entry.input.task_sequence,
                "pipeline_id":entry.input.pipeline_id,"pipeline_version_id":entry.input.pipeline_version_id,
                "stage_id":entry.input.stage_id,"task_revision":entry.input.task_revision,
                "attempt_number":entry.input.attempt_number,"priority_rank":entry.input.priority_rank,
                "priority_level_id":entry.input.priority_level_id,"resource_profile":entry.input.resource_profile,
                "eligible_at":entry.input.eligible_at,
                "queue_state":match entry.state {MemoryQueueState::Queued=>"queued",MemoryQueueState::Leased=>"leased",MemoryQueueState::Cancelled=>"cancelled"}})
        }).collect::<Vec<_>>();
        let artifacts = snapshot
            .artifacts
            .values()
            .map(|entry| {
                json!({"artifact":entry.artifact,"task_id":entry.location.task_id,
                "run_id":entry.location.run_id,"stage_id":entry.location.stage_id,
                "producer":entry.location.producer,"producer_id":entry.location.producer_id,
                "producer_data":entry.location.producer_data})
            })
            .collect::<Vec<_>>();
        let runs = snapshot.runs.iter().map(|entry| {
            json!({"id":entry.scope.id,"project_id":entry.scope.project_id,"task_id":entry.scope.task_id,
                "lease_fencing_token":entry.scope.lease_fencing_token,"environment_epoch":entry.scope.environment_epoch,
                "queue_entry_id":entry.queue_entry_id,"lease_active":entry.lease_active,
                "reservation_held":entry.reservation_held,"stop_requested":entry.stop_requested})
        }).collect::<Vec<_>>();
        let raw = json!({
            "projects":snapshot.projects,"tasks":tasks,"employees":snapshot.employees,
            "pipelines":snapshot.pipelines,"pipeline_versions":snapshot.pipeline_versions,
            "artifacts":artifacts,"task_dependencies":snapshot.dependencies.values().collect::<Vec<_>>(),
            "event_log":events,"outbox":outbox,"idempotency_keys":receipts,"queue_entries":queue,
            "runs":runs,"recovery_holds":snapshot.recovery_holds
        });
        Ok(Self {
            projects: snapshot.projects,
            tasks: snapshot
                .tasks
                .into_iter()
                .map(|(id, entry)| (id, entry.task))
                .collect(),
            versions: snapshot.pipeline_versions,
            raw,
        })
    }

    pub async fn postgres(pool: &PgPool) -> Result<Self> {
        let mut raw = json!({});
        // This fixed allowlist reads only the synthetic fixture's private schema.
        for table in [
            "projects",
            "tasks",
            "employees",
            "pipelines",
            "pipeline_versions",
            "artifacts",
            "task_dependencies",
            "event_log",
            "outbox",
            "idempotency_keys",
            "queue_entries",
            "leases",
            "runs",
            "run_environment_reservations",
            "project_recovery_settings",
        ] {
            let text: String = sqlx::query_scalar(&format!(
                "SELECT COALESCE(jsonb_agg(to_jsonb(r) ORDER BY to_jsonb(r)::text), '[]'::jsonb)::text FROM {table} r"
            )).fetch_one(pool).await?;
            raw[table] = serde_json::from_str(&text)?;
        }
        raw["event_log"]
            .as_array_mut()
            .context("event list")?
            .sort_by_key(|event| {
                (
                    event["project_id"].as_str().unwrap_or_default().to_owned(),
                    event["project_sequence"].as_u64(),
                )
            });
        let mut projects = BTreeMap::new();
        for row in raw["projects"].as_array().context("projects")? {
            let project: Project = serde_json::from_value(row["canonical_snapshot"].clone())?;
            projects.insert(project.id(), project);
        }
        let mut tasks = BTreeMap::new();
        for row in raw["tasks"].as_array().context("tasks")? {
            let task: Task = serde_json::from_value(row["canonical_snapshot"].clone())?;
            tasks.insert(task.id(), task);
        }
        let mut versions = BTreeMap::new();
        for row in raw["pipeline_versions"].as_array().context("versions")? {
            let version: PipelineVersion = serde_json::from_value(row["definition"].clone())?;
            versions.insert(version.id(), version);
        }
        Ok(Self {
            projects,
            tasks,
            versions,
            raw,
        })
    }

    pub fn rows(&self, table: &str) -> &[Value] {
        self.raw[table].as_array().expect("snapshot table")
    }

    pub fn assert_audit_atomic(&self) {
        let events = self.rows("event_log");
        let outbox = self.rows("outbox");
        assert_eq!(events.len(), outbox.len(), "one intent per Event");
        let mut next_sequence = BTreeMap::new();
        for event in events {
            let sequence = next_sequence
                .entry(event["project_id"].to_string())
                .or_insert(0_u64);
            *sequence += 1;
            assert_eq!(event["project_sequence"], json!(sequence));
            let deliveries = outbox
                .iter()
                .filter(|entry| entry["event_id"] == event["id"])
                .collect::<Vec<_>>();
            assert_eq!(deliveries.len(), 1);
            assert_eq!(deliveries[0]["envelope"]["event_id"], event["id"]);
            assert_eq!(
                deliveries[0]["envelope"]["project_sequence"],
                event["project_sequence"]
            );
            assert_eq!(deliveries[0]["envelope"]["payload"], event["payload"]);
        }
        for record in self.rows("idempotency_keys") {
            let receipt = &record["receipt"];
            assert_eq!(receipt["command_id"], record["command_id"]);
            for id in receipt["event_ids"].as_array().expect("receipt events") {
                assert!(
                    events
                        .iter()
                        .any(|event| event["id"] == *id
                            && event["command_id"] == record["command_id"])
                );
            }
        }
    }
}
