//! PostgreSQL + actual UDS Gateway. Manual Supervisor, no model simulation or paid calls.
use anyhow::{Context, Result};
use forge_domain::{EmployeeId, LifecycleStatus};
use forge_protocol::wire::CommandName;
use forge_testkit::m0::{M0Harness, single_stage_pipeline};
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, time::Duration};
use uuid::Uuid;

async fn call(
    client: &Client,
    tool: &str,
    args: Value,
    message_id: Uuid,
) -> Result<(StatusCode, Value)> {
    let response = client
        .post("http://localhost/tools")
        .json(&json!({"tool":tool,"arguments":args,"message_id":message_id}))
        .send()
        .await?;
    Ok((response.status(), response.json().await?))
}

#[tokio::test]
#[ignore = "requires local PostgreSQL and NATS; no provider inference"]
async fn task_instructions_gate_acceptance_without_routing_general_inbox_to_writer() -> Result<()> {
    let harness = M0Harness::start().await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    let project = harness.create_project("M2 Inbox acceptance").await?;
    let version = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    harness.create_employee(project, "Bob").await?;
    let task = harness
        .create_task(project, version, "Follow current instructions")
        .await?;
    harness.approve_task(project, task).await?;
    harness.start_project(project).await?;
    supervisor.next_provision_for_task(task).await?;
    let run = harness.wait_for_run_count(task, 1).await?.remove(0);
    let employee: EmployeeId = run.require_employee_id()?;
    let thread: Uuid = harness
        .execute(
            project,
            CommandName::OpenEmployeeThread,
            json!({"employee_id":employee}),
        )
        .await?
        .resource
        .context("thread")?
        .id
        .parse()?;
    let task_state = harness.store.load_task(task).await?.context("task")?.task;
    let target = json!({"kind":"task_execution","context":{"task_id":task,"pipeline_version_id":version,
        "stage_id":"work","stage_visit":task_state.current_stage_visit().context("visit")?.get()}});
    let mut required = vec![];
    for (revision, requirement, body) in [
        (1, "acknowledged", "Use the documented migration path."),
        (
            2,
            "answered",
            "Explain how the old records will be retained.",
        ),
    ] {
        let receipt = harness
            .execute(
                project,
                CommandName::SendEmployeeMessage,
                json!({
            "thread_id":thread,"expected_thread_revision":revision,"target":target,
            "kind":"instruction","requirement":requirement,"body":body}),
            )
            .await?;
        required.push(receipt.resource.context("message")?.id.parse::<Uuid>()?);
    }
    harness
        .execute(
            project,
            CommandName::SendEmployeeMessage,
            json!({"thread_id":thread,
        "expected_thread_revision":3,"target":{"kind":"inbox"},"kind":"question",
        "requirement":"answered","body":"General question for a separate Communication Run."}),
        )
        .await?;
    // Explicit fixture-private retained path; no user source repository is touched.
    let root = std::env::temp_dir().join(format!("forge-inbox-{}", Uuid::now_v7()));
    std::fs::create_dir(&root)?;
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
    let gateway = harness.core.serve_run_gateway(run.id, &root).await?;
    let client = Client::builder()
        .unix_socket(gateway.socket_path())
        .timeout(Duration::from_secs(8))
        .build()?;
    let (status, list) = call(&client, "inbox.list", json!({}), Uuid::now_v7()).await?;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["messages"].as_array().context("messages")?.len(), 2);
    assert_eq!(list["acknowledgement_recorded"], false);
    let artifact_id = Uuid::now_v7();
    let (status, artifact) = call(
        &client,
        "artifact.submit",
        json!({"artifact_kind":"stage_evidence",
        "title":"Work completed","body":{"summary":"Old data retained in the existing table."}}),
        artifact_id,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{artifact}");
    let outcome_id = Uuid::now_v7();
    let outcome = json!({"stage_id":"work","outcome":"completed","artifact_submission_message_ids":[artifact_id]});
    let (status, blocked) = call(&client, "outcome.submit", outcome.clone(), outcome_id).await?;
    assert!(!status.is_success(), "{blocked}");
    assert_eq!(blocked["code"], "instruction_pending");
    assert_eq!(
        harness
            .store
            .load_task(task)
            .await?
            .context("task")?
            .task
            .lifecycle(),
        LifecycleStatus::InProgress
    );
    for id in &required {
        let (status, receipt) = call(
            &client,
            "inbox.acknowledge",
            json!({"target_message_id":id}),
            Uuid::now_v7(),
        )
        .await?;
        assert_eq!(status, StatusCode::OK, "{receipt}");
        let (duplicate_status, _) = call(
            &client,
            "inbox.acknowledge",
            json!({"target_message_id":id}),
            Uuid::now_v7(),
        )
        .await?;
        assert!(
            !duplicate_status.is_success(),
            "a new call must not fabricate a second receipt timestamp"
        );
    }
    // Acknowledging the question is not answering it.
    assert!(
        !call(&client, "outcome.submit", outcome.clone(), outcome_id)
            .await?
            .0
            .is_success()
    );
    let reply_call = Uuid::now_v7();
    let reply_body = json!({"target_message_id":required[1],"body":"Keep the source table until migration is approved."});
    let (status, answer) = call(&client, "inbox.reply", reply_body.clone(), reply_call).await?;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(
        call(&client, "inbox.reply", reply_body, reply_call)
            .await?
            .1,
        answer
    );
    let messages = harness
        .store
        .list_employee_messages(project, thread, 0, 100)
        .await?;
    assert_eq!(messages.len(), 4, "reply retry must not append twice");
    assert_eq!(messages[3].data().reply_to, Some(required[1]));
    let (status, accepted) = call(&client, "outcome.submit", outcome, outcome_id).await?;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    assert_eq!(
        harness
            .store
            .load_task(task)
            .await?
            .context("task")?
            .task
            .lifecycle(),
        LifecycleStatus::Done
    );
    // Closed Task can't acquire a new required instruction retroactively.
    assert!(harness.execute(project,CommandName::SendEmployeeMessage,json!({"thread_id":thread,
        "expected_thread_revision":5,"target":target,"kind":"instruction","requirement":"acknowledged",
        "body":"Late instruction"})).await.is_err());
    gateway.shutdown().await;
    harness.stop_project(project).await?;
    harness.shutdown().await;
    Ok(())
}
