//! Real Run-scoped UDS HTTP/MCP tests against the local PostgreSQL/NATS topology.

use anyhow::{Context, Result};
use forge_protocol::supervisor::v1::{AcknowledgementDisposition, RunEventKind};
use forge_testkit::m0::{M0Harness, single_stage_pipeline};
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};
use uuid::Uuid;

async fn call(
    client: &Client,
    tool: &str,
    args: Value,
    message_id: Uuid,
) -> Result<reqwest::Response> {
    Ok(client
        .post("http://localhost/tools")
        .json(&json!({"message_id":message_id,"tool":tool,"arguments":args}))
        .send()
        .await?)
}

struct SocketRoot(PathBuf);
impl SocketRoot {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!("forge-gw-{}", Uuid::now_v7()));
        std::fs::create_dir(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
        Ok(Self(path))
    }
}
impl Drop for SocketRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
#[ignore = "requires local PostgreSQL and NATS; run just test-integration"]
async fn gateway_human_question_requires_canonical_resolution_not_generic_resume() -> Result<()> {
    let harness = M0Harness::start().await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    let project = harness.create_project("Gateway human escalation").await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    harness
        .create_employee(project, "Questioning worker")
        .await?;
    let task_id = harness
        .create_task(project, pipeline, "Needs a human")
        .await?;
    harness.approve_task(project, task_id).await?;
    harness.start_project(project).await?;
    supervisor.next_provision_for_task(task_id).await?;
    let run = harness.wait_for_run_count(task_id, 1).await?.remove(0);
    let root = SocketRoot::new()?;
    let gateway = harness.core.serve_run_gateway(run.id, &root.0).await?;
    let client = Client::builder()
        .unix_socket(gateway.socket_path())
        .timeout(Duration::from_secs(8))
        .build()?;
    let task_before = harness
        .store
        .load_task(task_id)
        .await?
        .context("Task")?
        .task;
    let progress_id = Uuid::now_v7();
    let progress = json!({"note":"Inspection complete; an operator choice is required."});
    assert_eq!(
        call(&client, "progress.report", progress.clone(), progress_id)
            .await?
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        call(&client, "progress.report", progress, progress_id)
            .await?
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        harness
            .store
            .load_task(task_id)
            .await?
            .context("Task")?
            .task
            .revision(),
        task_before.revision()
    );
    let progress_records:i64=sqlx::query_scalar("SELECT count(*) FROM event_log WHERE command_id=$1 AND payload->>'category'='progress_reported'")
        .bind(progress_id).fetch_one(&harness.pool).await?;
    assert_eq!(
        progress_records, 1,
        "signal replay must not duplicate canonical progress"
    );
    let question_id = Uuid::now_v7();
    let response = call(
        &client,
        "human.request",
        json!({"question":"May I proceed with this destructive migration?"}),
        question_id,
    )
    .await?;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "{}",
        response.text().await?
    );
    let mut tx = harness.store.begin().await?;
    let canonical = tx
        .load_escalation_for_wait(
            project,
            task_id,
            harness
                .store
                .load_task(task_id)
                .await?
                .context("Task")?
                .task
                .wait_conditions()
                .next()
                .context("wait")?
                .id(),
        )
        .await?
        .context("canonical question")?;
    let forge_domain::resolution::EscalationState::Assigned { assignment_id } = canonical.state
    else {
        anyhow::bail!("Human assignment expected");
    };
    tx.commit().await?;
    let stop = supervisor.next_stop_for_run(&run.id.to_string()).await?;
    assert_eq!(
        stop.mode,
        forge_protocol::supervisor::v1::StopMode::Graceful as i32
    );
    let waiting = harness
        .store
        .load_task(task_id)
        .await?
        .context("Task")?
        .task;
    assert_eq!(waiting.lifecycle(), forge_domain::LifecycleStatus::Waiting);
    assert_eq!(waiting.current_stage_id(), task_before.current_stage_id());
    let conditions = waiting.wait_conditions().collect::<Vec<_>>();
    assert_eq!(conditions.len(), 1);
    assert_eq!(
        conditions[0].kind(),
        &forge_domain::TaskWaitKind::EscalationPending
    );
    assert_eq!(
        conditions[0].source_stage_id(),
        None,
        "not a Pipeline-owned DecisionRequired wait"
    );
    let wait_id = conditions[0].id();
    let (lease_state,queue_state):(String,String)=sqlx::query_as("SELECT l.lease_state,q.queue_state FROM runs r JOIN leases l ON l.id=r.lease_id JOIN queue_entries q ON q.id=r.queue_entry_id WHERE r.id=$1")
        .bind(run.id).fetch_one(&harness.pool).await?;
    assert_eq!(
        lease_state, "active",
        "physical stop must precede lease release"
    );
    assert_eq!(queue_state, "cancelled");
    assert_eq!(
        harness
            .store
            .load_run(run.id)
            .await?
            .context("Run")?
            .last_sequence,
        0
    );
    assert_eq!(
        call(&client, "board.list", json!({}), Uuid::now_v7())
            .await?
            .status(),
        StatusCode::FORBIDDEN
    );
    let ack = supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Stopped)
        .await?;
    assert_eq!(ack.disposition, AcknowledgementDisposition::Accepted as i32);
    let still_waiting = harness
        .store
        .load_task(task_id)
        .await?
        .context("Task")?
        .task;
    assert_eq!(
        still_waiting.lifecycle(),
        forge_domain::LifecycleStatus::Waiting
    );
    let envelope=forge_application::CommandEnvelope::parse(forge_protocol::wire::CommandName::ResumeTask,forge_protocol::wire::CommandRequest {
        project_id:project.as_uuid().to_string(),expected_revision:harness.store.load_project(project).await?.context("Project")?.revision(),
        payload:json!({"task_id":task_id,"expected_task_revision":still_waiting.revision().get(),"wait_condition_id":wait_id}).as_object().context("object")?.clone(),
    },Uuid::now_v7().to_string())?;
    assert!(
        harness.core.execute_command(envelope).await.is_err(),
        "a generic resume cannot bypass the canonical question"
    );
    harness.execute(project, forge_protocol::wire::CommandName::SubmitHumanResolution,
        json!({"escalation_id":canonical.id,"expected_escalation_revision":canonical.revision,
            "assignment_id":assignment_id,"lease_generation":canonical.generation,
            "answer":{"disposition":"continue_stage","summary":"Proceed only within the documented scope"}})
    ).await?;
    assert!(
        harness
            .store
            .load_task(task_id)
            .await?
            .context("Task")?
            .task
            .wait_conditions()
            .all(|wait| wait.id() != wait_id)
    );
    let next = supervisor.next_provision_for_task(task_id).await?;
    assert_ne!(next.run_id, run.id.to_string());
    harness.stop_project(project).await?;
    let next_run = harness
        .store
        .load_run(Uuid::parse_str(&next.run_id)?)
        .await?
        .context("next Run")?;
    supervisor
        .send_observation_kind(
            &next_run,
            next_run.lease_fencing_token,
            1,
            RunEventKind::Stopped,
        )
        .await?;
    gateway.shutdown().await;
    harness.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL and NATS; run just test-integration"]
async fn gateway_keeps_sequences_separate_replays_receipts_and_revokes_immediately() -> Result<()> {
    let harness = M0Harness::start().await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    let project = harness.create_project("Gateway scope acceptance").await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    harness.create_employee(project, "Gateway worker").await?;
    let task = harness
        .create_task(project, pipeline, "Gateway task")
        .await?;
    harness.approve_task(project, task).await?;
    harness.start_project(project).await?;
    supervisor.next_provision_for_task(task).await?;
    let run = harness.wait_for_run_count(task, 1).await?.remove(0);
    let mut transaction = harness.store.begin().await?;
    for scope in [
        forge_domain::runtime::RunScope {
            run_id: run.id,
            fencing_token: run.lease_fencing_token + 1,
            environment_epoch: run.environment_epoch,
        },
        forge_domain::runtime::RunScope {
            run_id: run.id,
            fencing_token: run.lease_fencing_token,
            environment_epoch: run.environment_epoch + 1,
        },
    ] {
        assert!(!transaction.gateway_scope_is_active(scope).await?);
    }
    transaction.commit().await?;
    let root = SocketRoot::new()?;
    let gateway = harness.core.serve_run_gateway(run.id, &root.0).await?;
    assert_eq!(
        std::fs::metadata(gateway.socket_path())?
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(
        harness
            .core
            .serve_run_gateway(run.id, &root.0)
            .await
            .is_err(),
        "a live scoped socket cannot be replaced"
    );
    let client = Client::builder()
        .unix_socket(gateway.socket_path())
        .timeout(Duration::from_secs(8))
        .build()?;
    let board = call(&client, "board.list", json!({}), Uuid::now_v7()).await?;
    assert_eq!(board.status(), StatusCode::OK);
    let board: Value = board.json().await?;
    assert_eq!(board["tasks"][0]["task_id"], json!(task));
    for key in [
        "run_spec",
        "context_manifest",
        "environment",
        "credentials",
        "work_surface",
    ] {
        assert!(!board.to_string().contains(key));
    }

    let artifact_id = Uuid::now_v7();
    let artifact = json!({"artifact_kind":"stage_evidence","title":"Explicit evidence","metadata":{},"body":{"result":"done"}});
    let response = call(&client, "artifact.submit", artifact.clone(), artifact_id).await?;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "{}",
        response.text().await?
    );
    assert_eq!(
        harness
            .store
            .load_run(run.id)
            .await?
            .context("Run")?
            .last_sequence,
        0
    );
    assert_eq!(
        call(&client, "artifact.submit", artifact.clone(), artifact_id)
            .await?
            .status(),
        StatusCode::OK
    );
    assert_eq!(harness.store.list_artifacts_for_task(task).await?.len(), 1);
    let mut changed = artifact.clone();
    changed["title"] = json!("Changed");
    assert_eq!(
        call(&client, "artifact.submit", changed, artifact_id)
            .await?
            .status(),
        StatusCode::CONFLICT
    );
    let ack = supervisor
        .send_observation(&run, run.lease_fencing_token, 1)
        .await?;
    assert_eq!(ack.disposition, AcknowledgementDisposition::Accepted as i32);
    assert_eq!(
        harness
            .store
            .load_run(run.id)
            .await?
            .context("Run")?
            .last_sequence,
        1
    );

    let mut injected = artifact;
    injected["run_id"] = json!(Uuid::now_v7());
    assert!(
        !call(&client, "artifact.submit", injected, Uuid::now_v7())
            .await?
            .status()
            .is_success()
    );
    assert!(
        !call(&client, "task.cancel", json!({}), Uuid::now_v7())
            .await?
            .status()
            .is_success()
    );
    let other_project = harness.create_project("Other Gateway project").await?;
    let other_pipeline = harness
        .create_pipeline(other_project, single_stage_pipeline())
        .await?;
    let other_task = harness
        .create_task(other_project, other_pipeline, "not visible")
        .await?;
    assert!(
        !call(
            &client,
            "task.read",
            json!({"task_id":other_task}),
            Uuid::now_v7()
        )
        .await?
        .status()
        .is_success()
    );

    let outcome_id = Uuid::now_v7();
    let outcome = json!({"stage_id":"work","outcome":"completed","artifact_submission_message_ids":[artifact_id]});
    let response = call(&client, "outcome.submit", outcome.clone(), outcome_id).await?;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "{}",
        response.text().await?
    );
    assert_eq!(
        call(&client, "outcome.submit", outcome, outcome_id)
            .await?
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        harness
            .store
            .load_run(run.id)
            .await?
            .context("Run")?
            .last_sequence,
        1
    );

    harness.stop_project(project).await?;
    assert_eq!(
        call(&client, "board.list", json!({}), Uuid::now_v7())
            .await?
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&client, "artifact.submit", json!({}), Uuid::now_v7())
            .await?
            .status(),
        StatusCode::FORBIDDEN
    );
    supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 2, RunEventKind::Stopped)
        .await?;
    gateway.shutdown().await;
    harness.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL and NATS; run just test-integration"]
async fn gateway_mcp_negotiates_catalog_calls_and_bounded_untrusted_requests() -> Result<()> {
    let harness = M0Harness::start().await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    let project = harness.create_project("Gateway MCP").await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    harness.create_employee(project, "MCP worker").await?;
    let task = harness.create_task(project, pipeline, "MCP task").await?;
    harness.approve_task(project, task).await?;
    harness.start_project(project).await?;
    supervisor.next_provision_for_task(task).await?;
    let run = harness.wait_for_run_count(task, 1).await?.remove(0);
    let root = SocketRoot::new()?;
    let gateway = harness.core.serve_run_gateway(run.id, &root.0).await?;
    let client = Client::builder()
        .unix_socket(gateway.socket_path())
        .timeout(Duration::from_secs(8))
        .build()?;
    let request = |body: Value| {
        client
            .post("http://localhost/mcp")
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-11-25")
            .json(&body)
    };
    let initialized=request(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}})).send().await?;
    assert_eq!(initialized.status(), StatusCode::OK);
    let body: Value = initialized.json().await?;
    assert_eq!(body["result"]["protocolVersion"], "2025-11-25");
    let listed = request(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}))
        .send()
        .await?;
    assert_eq!(listed.status(), StatusCode::OK, "{}", listed.text().await?);
    let listed: Value = request(json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}))
        .send()
        .await?
        .json()
        .await?;
    let tools = listed["result"]["tools"]
        .as_array()
        .context("MCP tool list")?;
    assert_eq!(tools.len(), 14);
    for name in [
        "forge_search_memory",
        "forge_read_memory",
        "forge_refresh_memory",
    ] {
        assert!(tools.iter().any(|tool| tool["name"] == name));
    }
    assert!(
        !tools
            .iter()
            .any(|tool| tool["name"] == "forge_submit_system_job_result")
    );
    let read:Value=request(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"forge_read_task","arguments":{"task_id":task},"_meta":{"run_id":Uuid::now_v7()}}})).send().await?.json().await?;
    assert_eq!(read["result"]["structuredContent"]["task_id"], json!(task));
    let huge = "x".repeat(256 * 1024 + 1);
    let oversized=request(json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"forge_report_progress","arguments":{"message_id":Uuid::now_v7(),"note":huge}}})).send().await?;
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    // Chunked upload omits Content-Length: the SDK must count bytes while
    // reading, not trust an absent or forged length header.
    let chunks = tokio_stream::iter([
        Ok::<_, std::io::Error>(vec![b' '; 128 * 1024]),
        Ok(vec![b' '; 128 * 1024 + 1]),
    ]);
    let streamed = client
        .post("http://localhost/mcp")
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .header("MCP-Protocol-Version", "2025-11-25")
        .body(reqwest::Body::wrap_stream(chunks))
        .send()
        .await?;
    assert_eq!(streamed.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let foreign_origin = request(json!({"jsonrpc":"2.0","id":6,"method":"tools/list"}))
        .header("Origin", "https://untrusted.invalid")
        .send()
        .await?;
    assert_eq!(foreign_origin.status(), StatusCode::FORBIDDEN);
    harness.stop_project(project).await?;
    let stopped = request(json!({"jsonrpc":"2.0","id":7,"method":"tools/list"}))
        .send()
        .await?;
    assert_eq!(stopped.status(), StatusCode::OK);
    let stopped: Value = stopped.json().await?;
    assert_eq!(stopped["error"]["code"], -32600);
    assert!(
        stopped["result"].is_null(),
        "revoked MCP calls return no catalog or Task data"
    );
    supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Stopped)
        .await?;
    gateway.shutdown().await;
    harness.shutdown().await;
    Ok(())
}
