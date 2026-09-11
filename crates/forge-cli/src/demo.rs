//! One end-to-end M0 scenario expressed entirely through the public API.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use forge_protocol::wire::{CommandReceipt, CommandRequest};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::time::{Instant, sleep};
use uuid::Uuid;

use forge_cli::LocalClient;

const COMPLETION_TIMEOUT: Duration = Duration::from_secs(20);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Public, credential-free outcome of the deterministic M0 smoke scenario.
#[derive(Serialize)]
pub(crate) struct DemoReport {
    project_id: String,
    task_id: String,
    lifecycle: String,
    artifact_count: usize,
    event_count: usize,
    run_count: usize,
}

/// Creates an isolated Project, dispatches one deterministic Employee Run, and
/// checks the completed Task through public HTTP/SSE projections.
pub(crate) async fn run_m0(client: &LocalClient) -> Result<DemoReport> {
    let project_id = Uuid::now_v7().to_string();
    let project = command(
        client,
        "create_project",
        &project_id,
        0,
        json!({"name": "M0 demo"}),
    )
    .await?;
    let pipeline = command(
        client,
        "create_pipeline",
        &project_id,
        project.project_revision,
        demo_pipeline_payload(),
    )
    .await?;
    let pipeline_version_id = created_pipeline_version_id(client, &project_id, &pipeline).await?;
    let employee = command(
        client,
        "create_employee",
        &project_id,
        pipeline.project_revision,
        json!({
            "name": "M0 executor",
            "role": "m0_executor",
            "stage_eligibility": {"mode": "any"}
        }),
    )
    .await?;
    // This explicit deterministic demo exercises M0, not semantic onboarding.
    let oriented = command(
        client,
        "skip_employee_onboarding",
        &project_id,
        employee.project_revision,
        json!({
            "employee_id": resource_id(&employee, "employee")?,
            "reason": "Explicit operator setup for the M0 deterministic demo"
        }),
    )
    .await?;
    let task = command(
        client,
        "create_task",
        &project_id,
        oriented.project_revision,
        json!({
            "title": "Complete one deterministic M0 task",
            "description": "A public API smoke scenario.",
            "definition_of_done": "The fake Employee records required evidence and completes.",
            "kind": "delivery",
            "pipeline_version_id": pipeline_version_id,
            "priority": "normal"
        }),
    )
    .await?;
    let task_id = resource_id(&task, "task")?;
    let draft = get_task(client, &project_id, &task_id).await?;
    let approved = command(
        client,
        "approve_task",
        &project_id,
        task.project_revision,
        json!({
            "task_id": task_id,
            "expected_task_revision": number(&draft, "revision")?
        }),
    )
    .await?;
    command(
        client,
        "start_project_execution",
        &project_id,
        approved.project_revision,
        json!({"reason": "run deterministic M0 acceptance"}),
    )
    .await?;

    let completed = wait_for_completed_task(client, &project_id, &task_id).await?;
    let artifacts = completed
        .get("artifacts")
        .and_then(Value::as_array)
        .context("completed Task response is missing artifacts")?;
    if artifacts.is_empty() {
        bail!("completed M0 Task did not retain an Artifact");
    }
    let events = client
        .watch_events(&project_id, None)
        .await
        .context("read M0 canonical event replay")?;
    require_event(&events, "artifact_created")?;
    require_event(&events, "task_completed")?;
    let runs = client
        .get_json(&format!("/v1/projects/{project_id}/runs"))
        .await
        .context("read M0 Run projection")?;
    let run_count = runs
        .get("items")
        .and_then(Value::as_array)
        .context("Run list response is missing items")?
        .len();
    if run_count == 0 {
        bail!("completed M0 Task has no Run projection");
    }

    Ok(DemoReport {
        project_id,
        task_id,
        lifecycle: required_string(&completed, "lifecycle")?.to_owned(),
        artifact_count: artifacts.len(),
        event_count: events.len(),
        run_count,
    })
}

async fn command(
    client: &LocalClient,
    name: &str,
    project_id: &str,
    expected_revision: u64,
    payload: Value,
) -> Result<CommandReceipt> {
    let payload = payload
        .as_object()
        .cloned()
        .context("internal M0 command payload must be a JSON object")?;
    client
        .execute_command(
            name,
            &CommandRequest {
                project_id: project_id.to_owned(),
                expected_revision,
                payload,
            },
            &Uuid::now_v7().to_string(),
        )
        .await
        .with_context(|| format!("execute M0 command {name}"))
}

async fn created_pipeline_version_id(
    client: &LocalClient,
    project_id: &str,
    receipt: &CommandReceipt,
) -> Result<String> {
    let pipeline_id = resource_id(receipt, "pipeline")?;
    let response = client
        .get_json(&format!("/v1/projects/{project_id}/pipelines"))
        .await
        .context("read M0 PipelineVersion projection")?;
    response
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|item| item.get("pipeline_id").and_then(Value::as_str) == Some(pipeline_id.as_str()))
        .and_then(|item| item.get("id").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .context("created M0 PipelineVersion is absent from its read projection")
}

async fn get_task(client: &LocalClient, project_id: &str, task_id: &str) -> Result<Value> {
    client
        .get_json(&format!("/v1/projects/{project_id}/tasks/{task_id}"))
        .await
        .context("read M0 Task projection")
}

async fn wait_for_completed_task(
    client: &LocalClient,
    project_id: &str,
    task_id: &str,
) -> Result<Value> {
    let deadline = Instant::now() + COMPLETION_TIMEOUT;
    loop {
        let task = get_task(client, project_id, task_id).await?;
        if task_is_complete(&task) {
            return Ok(task);
        }
        if Instant::now() >= deadline {
            bail!(
                "M0 Task did not complete within {} seconds",
                COMPLETION_TIMEOUT.as_secs()
            );
        }
        sleep(POLL_INTERVAL).await;
    }
}

fn demo_pipeline_payload() -> Value {
    json!({
        "name": "M0 deterministic delivery",
        "task_kinds": ["delivery"],
        "entry_stage_id": "work",
        "stages": [{
            "id": "work",
            "name": "Work",
            "executor_kind": "employee",
            "outcomes": ["implemented"]
        }],
        "transitions": [{
            "from_stage_id": "work",
            "outcome": "implemented",
            "target": {"kind": "done"},
            "artifact_requirements": [{
                "kind": "change_set",
                "minimum_count": 1,
                "scope": "current_stage"
            }]
        }]
    })
}

fn resource_id(receipt: &CommandReceipt, expected_kind: &str) -> Result<String> {
    let resource = receipt
        .resource
        .as_ref()
        .context("M0 command receipt did not return a primary resource")?;
    if resource.kind != expected_kind {
        bail!(
            "M0 command receipt returned {} instead of {expected_kind}",
            resource.kind
        );
    }
    Ok(resource.id.clone())
}

fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .with_context(|| format!("response is missing string field {field}"))
}

fn number(value: &Value, field: &str) -> Result<u64> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .with_context(|| format!("response is missing integer field {field}"))
}

fn require_event(events: &[forge_protocol::wire::EventEnvelope], event_type: &str) -> Result<()> {
    if events.iter().any(|event| event.event_type == event_type) {
        Ok(())
    } else {
        bail!("M0 event replay is missing {event_type}");
    }
}

fn task_is_complete(task: &Value) -> bool {
    task.get("lifecycle").and_then(Value::as_str) == Some("done")
        && task
            .get("artifacts")
            .and_then(Value::as_array)
            .is_some_and(|artifacts| !artifacts.is_empty())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{demo_pipeline_payload, task_is_complete};

    #[test]
    fn completion_requires_both_a_terminal_task_and_evidence() {
        assert!(!task_is_complete(
            &json!({"lifecycle": "done", "artifacts": []})
        ));
        assert!(!task_is_complete(
            &json!({"lifecycle": "in_progress", "artifacts": [{}]})
        ));
        assert!(task_is_complete(
            &json!({"lifecycle": "done", "artifacts": [{}]})
        ));
    }

    #[test]
    fn m0_pipeline_requires_current_stage_evidence() {
        let transition = &demo_pipeline_payload()["transitions"][0];

        assert_eq!(transition["target"]["kind"], "done");
        assert_eq!(
            transition["artifact_requirements"][0]["scope"],
            "current_stage"
        );
    }
}
