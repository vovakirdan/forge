//! Real PostgreSQL, Core, and UDS Gateway; manual Supervisor, no paid inference.

#[path = "m2_communication_runs/native_input.rs"]
mod native_input;
#[path = "m2_communication_runs/shared_admission.rs"]
mod shared_admission;
#[path = "m2_communication_runs/support.rs"]
mod support;

use anyhow::{Context, Result};
use forge_domain::LifecycleStatus;
use forge_protocol::{
    supervisor::v1::{AcknowledgementDisposition, RunEventKind},
    wire::CommandName,
};
use forge_testkit::m0::single_stage_pipeline;
use reqwest::StatusCode;
use serde_json::json;
use support::{call, client, fixture, send_question};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; synthetic credential, no provider inference"]
async fn taskless_conversation_shares_capacity_without_acquiring_task_authority() -> Result<()> {
    let (harness, project, employee, root) = fixture().await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    let first = harness.create_task(project, pipeline, "Writer one").await?;
    let second = harness.create_task(project, pipeline, "Writer two").await?;
    harness.approve_task(project, first).await?;
    harness.approve_task(project, second).await?;
    harness.start_project(project).await?;
    supervisor.next_provision_for_task(first).await?;
    let source = send_question(&harness, project, employee, first).await?;
    harness.core.dispatch_available(project).await?;
    assert_eq!(
        harness.store.list_runs_for_project(project).await?.len(),
        1,
        "default capacity remains one across purposes"
    );
    let current = harness
        .store
        .list_employees(project)
        .await?
        .remove(0)
        .employee;
    harness.execute(project,CommandName::AmendEmployee,json!({"employee_id":employee,"expected_employee_revision":current.revision(),"patch":{"max_concurrent_runs":3}})).await?;
    harness.core.dispatch_available(project).await?;
    supervisor.next_provision_for_task(second).await?;
    let provision = supervisor.next_communication_provision(employee).await?;
    assert!(provision.task_id.is_empty() && provision.stage_id.is_empty());
    assert_eq!(provision.run_spec_version, 3);
    let conversation = harness
        .store
        .load_run(provision.run_id.parse()?)
        .await?
        .context("conversation Run")?;
    assert!(conversation.require_task_id().is_err());
    let recovery = harness.store.recovery_run_page(project, None, 20).await?;
    assert!(
        recovery
            .iter()
            .any(|row| row.run_id == conversation.id && row.task_id.is_none()),
        "taskless Communication Runs remain readable in Project recovery history"
    );
    assert_eq!(
        conversation
            .assignment
            .communication()
            .context("owner")?
            .source_message_id,
        source
    );
    assert!(conversation.run_spec.get("surface_id").is_none());
    assert_eq!(
        conversation.run_spec["binding"]["surface"],
        json!({"mode":"none"})
    );
    assert_eq!(
        conversation.context_manifest["context_task_id"],
        json!(first)
    );
    assert_eq!(harness.store.list_runs_for_project(project).await?.len(), 3);
    let scope = client(&root, &conversation)?;
    let (status, list) = call(&scope, "inbox.list", json!({}), Uuid::now_v7()).await?;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["source_message_id"], json!(source));
    for (tool, args) in [
        (
            "artifact.submit",
            json!({"artifact_kind":"note","title":"not allowed","body":{}}),
        ),
        (
            "outcome.submit",
            json!({"stage_id":"work","outcome":"completed"}),
        ),
        (
            "human.request",
            json!({"question":"Pause the context Task"}),
        ),
    ] {
        assert!(
            !call(&scope, tool, args, Uuid::now_v7())
                .await?
                .0
                .is_success(),
            "no Task write authority"
        );
    }
    assert!(
        !call(&scope, "communication.complete", json!({}), Uuid::now_v7())
            .await?
            .0
            .is_success()
    );
    assert_eq!(
        call(
            &scope,
            "inbox.acknowledge",
            json!({"target_message_id":source}),
            Uuid::now_v7()
        )
        .await?
        .0,
        StatusCode::OK
    );
    assert!(
        !call(&scope, "communication.complete", json!({}), Uuid::now_v7())
            .await?
            .0
            .is_success(),
        "ACK is not an answer"
    );
    let reply_id = Uuid::now_v7();
    let reply =
        json!({"target_message_id":source,"body":"Work is in progress; no Task was changed."});
    let answered = call(&scope, "inbox.reply", reply.clone(), reply_id).await?;
    assert_eq!(answered.0, StatusCode::OK, "{}", answered.1);
    assert_eq!(
        call(&scope, "inbox.reply", reply, reply_id).await?.1,
        answered.1
    );
    let complete_id = Uuid::now_v7();
    let completed = call(&scope, "communication.complete", json!({}), complete_id).await?;
    assert_eq!(completed.0, StatusCode::OK, "{}", completed.1);
    assert_eq!(
        call(&scope, "communication.complete", json!({}), complete_id)
            .await?
            .1,
        completed.1
    );
    assert!(
        !call(
            &scope,
            "inbox.reply",
            json!({"target_message_id":source,"body":"late answer"}),
            Uuid::now_v7()
        )
        .await?
        .0
        .is_success()
    );
    let mut tx = harness.store.begin().await?;
    tx.lock_project(project).await?;
    assert!(
        !tx.lock_employee_capacity(project, employee).await?,
        "completed conversation still physically occupies its slot"
    );
    tx.commit().await?;
    for task in [first, second] {
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
    }
    let ack = supervisor
        .send_observation_kind(
            &conversation,
            conversation.lease_fencing_token,
            1,
            RunEventKind::Stopped,
        )
        .await?;
    assert_eq!(ack.disposition, AcknowledgementDisposition::Accepted as i32);
    harness.core.dispatch_available(project).await?;
    assert_eq!(
        harness.store.list_runs_for_project(project).await?.len(),
        3,
        "own Employee reply must not recursively enqueue another Run"
    );
    let thread = conversation
        .assignment
        .communication()
        .context("owner")?
        .thread_id;
    let mut tx = harness.store.begin().await?;
    let revision = tx
        .lock_employee_thread(thread)
        .await?
        .context("thread")?
        .data()
        .revision;
    tx.commit().await?;
    let followup: Uuid = harness
        .execute(
            project,
            CommandName::SendEmployeeMessage,
            json!({
                "thread_id":thread,"expected_thread_revision":revision,"target":{"kind":"inbox"},
        "kind":"reply","requirement":"informational","body":"Thanks; what should we do next?",
                "reply_to":answered.1["reply"]["id"]
            }),
        )
        .await?
        .resource
        .context("follow-up")?
        .id
        .parse()?;
    harness.core.dispatch_available(project).await?;
    let next = supervisor.next_communication_provision(employee).await?;
    let next = harness
        .store
        .load_run(next.run_id.parse()?)
        .await?
        .context("follow-up Run")?;
    assert_ne!(next.assignment, conversation.assignment);
    assert_eq!(
        next.assignment
            .communication()
            .context("owner")?
            .source_message_id,
        followup
    );
    assert_eq!(
        next.context_manifest["source_message"]["reply_to"],
        answered.1["reply"]["id"]
    );
    let next_gateway = client(&root, &next)?;
    let history = call(&next_gateway, "inbox.list", json!({}), Uuid::now_v7()).await?;
    assert_eq!(history.0, StatusCode::OK, "{}", history.1);
    assert_eq!(
        history.1["messages"].as_array().context("history")?.len(),
        3
    );
    let reply = call(
        &next_gateway,
        "inbox.reply",
        json!({"target_message_id":followup,
        "body":"Continue the two active Tasks."}),
        Uuid::now_v7(),
    )
    .await?;
    assert_eq!(reply.0, StatusCode::OK, "{}", reply.1);
    assert_eq!(
        call(
            &next_gateway,
            "communication.complete",
            json!({}),
            Uuid::now_v7()
        )
        .await?
        .0,
        StatusCode::OK
    );
    harness.stop_project(project).await?;
    for run in harness.store.list_runs_for_project(project).await? {
        if run.id == conversation.id {
            continue;
        }
        supervisor.next_stop_for_run(&run.id.to_string()).await?;
        let ack = supervisor
            .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Stopped)
            .await?;
        assert_eq!(
            ack.disposition,
            AcknowledgementDisposition::Accepted as i32,
            "{}",
            ack.reason_code
        );
    }
    let mut tx = harness.store.begin().await?;
    tx.lock_project(project).await?;
    assert!(tx.lock_employee_capacity(project, employee).await?);
    tx.commit().await?;
    harness.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; manual Supervisor, no provider inference"]
async fn interrupted_conversation_preserves_input_and_retries_only_after_quiescence() -> Result<()>
{
    let (harness, project, employee, root) = fixture().await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    let task = harness
        .create_task(project, pipeline, "Context only; keep draft")
        .await?;
    let source = send_question(&harness, project, employee, task).await?;
    harness.start_project(project).await?;
    let provision = supervisor.next_communication_provision(employee).await?;
    let run = harness
        .store
        .load_run(provision.run_id.parse()?)
        .await?
        .context("run")?;
    let scoped = client(&root, &run)?;
    for (sequence, kind) in [
        (1, RunEventKind::Running),
        (2, RunEventKind::ProviderFailed),
    ] {
        let ack = supervisor
            .send_observation_kind(&run, run.lease_fencing_token, sequence, kind)
            .await?;
        assert_eq!(
            ack.disposition,
            AcknowledgementDisposition::Accepted as i32,
            "{}",
            ack.reason_code
        );
    }
    supervisor.next_stop_for_run(&run.id.to_string()).await?;
    assert_eq!(
        harness
            .store
            .load_task(task)
            .await?
            .context("context Task")?
            .task
            .lifecycle(),
        LifecycleStatus::Draft
    );
    assert!(
        harness
            .execute(
                project,
                CommandName::RetryCommunication,
                json!({"run_id":run.id,"reason":"Retry after failure"})
            )
            .await
            .is_err()
    );
    assert!(
        !call(
            &scoped,
            "inbox.acknowledge",
            json!({"target_message_id":source}),
            Uuid::now_v7()
        )
        .await?
        .0
        .is_success()
    );
    let ack = supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 3, RunEventKind::Stopped)
        .await?;
    assert_eq!(
        ack.disposition,
        AcknowledgementDisposition::Accepted as i32,
        "{}",
        ack.reason_code
    );
    harness
        .execute(
            project,
            CommandName::RetryCommunication,
            json!({"run_id":run.id,"reason":"Provider recovered; retry the retained question"}),
        )
        .await?;
    harness.core.dispatch_available(project).await?;
    let retry = supervisor.next_communication_provision(employee).await?;
    let retry = harness
        .store
        .load_run(retry.run_id.parse()?)
        .await?
        .context("retry")?;
    assert_ne!(run.id, retry.id);
    assert_eq!(retry.attempt_number, 2);
    assert_eq!(run.assignment, retry.assignment);
    assert_eq!(
        retry.context_manifest["source_message"]["id"],
        json!(source)
    );
    let old_terminal = supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 4, RunEventKind::Stopped)
        .await?;
    assert_eq!(
        old_terminal.disposition,
        AcknowledgementDisposition::Accepted as i32
    );
    let retry_gateway = client(&root, &retry)?;
    assert_eq!(
        call(
            &retry_gateway,
            "inbox.reply",
            json!({"target_message_id":source,"body":"Retried conversation answered."}),
            Uuid::now_v7()
        )
        .await?
        .0,
        StatusCode::OK
    );
    let completed = call(
        &retry_gateway,
        "communication.complete",
        json!({}),
        Uuid::now_v7(),
    )
    .await?;
    assert_eq!(
        completed.0,
        StatusCode::OK,
        "late observation of old attempt must not hold the new assignment: {}",
        completed.1
    );
    assert_eq!(
        harness
            .store
            .load_task(task)
            .await?
            .context("context Task")?
            .task
            .lifecycle(),
        LifecycleStatus::Draft
    );
    harness.stop_project(project).await?;
    supervisor.next_stop_for_run(&retry.id.to_string()).await?;
    supervisor
        .send_observation_kind(&retry, retry.lease_fencing_token, 1, RunEventKind::Stopped)
        .await?;
    harness.shutdown().await;
    Ok(())
}
