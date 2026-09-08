//! Real Core/PG/UDS, manual Supervisor and synthetic credentials; no inference.
#[path = "m2_communication_runs/support.rs"]
#[allow(dead_code)]
mod support;

use anyhow::{Context, Result};
use forge_domain::{
    LifecycleStatus,
    resolution::{EscalationState, Resolver},
};
use forge_protocol::{
    supervisor::v1::{AcknowledgementDisposition, RunEventKind},
    wire::CommandName,
};
use forge_testkit::m0::single_stage_pipeline;
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no inference"]
async fn task_question_rejects_spoofed_authority_and_holds_exact_stage() -> Result<()> {
    let (h, project, _, root) = support::fixture().await?;
    let mut supervisor = h.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    let pipeline = h.create_pipeline(project, single_stage_pipeline()).await?;
    let task = h
        .create_task(project, pipeline, "Ask before acting")
        .await?;
    h.approve_task(project, task).await?;
    h.start_project(project).await?;
    let provision = supervisor.next_provision_for_task(task).await?;
    let run = h
        .store
        .load_run(provision.run_id.parse()?)
        .await?
        .context("run")?;
    let client = support::client(&root, &run)?;
    let before = h.store.load_task(task).await?.context("task")?.task;
    for args in [
        json!({"question":"Which option?","category":"technical_decision","task_id":Uuid::now_v7()}),
        json!({"question":"Which option?","category":"technical_decision","route_key":"absent"}),
    ] {
        assert!(
            !support::call(&client, "escalation.raise", args, Uuid::now_v7())
                .await?
                .0
                .is_success()
        );
    }
    assert_eq!(h.store.load_task(task).await?.context("task")?.task, before);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM escalations WHERE project_id=$1")
        .bind(project.as_uuid())
        .fetch_one(&h.pool)
        .await?;
    assert_eq!(count, 0, "invalid questions roll back receipts and state");
    let id = Uuid::now_v7();
    let args = json!({"question":"Is this operation authorized?","category":"action_approval"});
    let (status, receipt) = support::call(&client, "escalation.raise", args.clone(), id).await?;
    assert!(status.is_success(), "{receipt}");
    assert_eq!(
        support::call(&client, "escalation.raise", args, id)
            .await?
            .1,
        receipt
    );
    let mut tx = h.store.begin().await?;
    let escalation = tx
        .load_escalation(receipt["escalation_id"].as_str().context("id")?.parse()?)
        .await?
        .context("question")?;
    let source = escalation.source.task().context("Task source")?;
    assert_eq!(source.task_id, task);
    assert_eq!(Some(source.stage_visit), before.current_stage_visit());
    assert_eq!(
        escalation.created_by.id().as_uuid(),
        run.require_employee_id()?.as_uuid()
    );
    let EscalationState::Assigned { assignment_id } = escalation.state else {
        anyhow::bail!("Human expected")
    };
    assert_eq!(
        tx.load_resolution_assignment(assignment_id)
            .await?
            .context("assignment")?
            .resolver,
        Resolver::Human
    );
    tx.commit().await?;
    let waiting = h.store.load_task(task).await?.context("task")?.task;
    assert_eq!(waiting.lifecycle(), LifecycleStatus::Waiting);
    assert_eq!(waiting.current_stage_id(), before.current_stage_id());
    assert!(h.execute(project, CommandName::ResumeTask, json!({"task_id":task,
        "expected_task_revision":waiting.revision().get(), "wait_condition_id":source.wait_condition_id})).await.is_err());
    let answer = json!({"escalation_id":escalation.id, "expected_escalation_revision":escalation.revision,
        "assignment_id":assignment_id, "lease_generation":escalation.generation,
        "answer":{"disposition":"continue_stage","summary":"Proceed within the narrow stated scope"}});
    assert!(
        h.execute(project, CommandName::SubmitHumanResolution, answer.clone())
            .await
            .is_err(),
        "answer cannot resume a physically active source"
    );
    let ack = supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Stopped)
        .await?;
    assert_eq!(ack.disposition, AcknowledgementDisposition::Accepted as i32);
    h.execute(project, CommandName::SubmitHumanResolution, answer)
        .await?;
    let resolved = h.store.load_task(task).await?.context("task")?.task;
    assert!(
        resolved
            .wait_conditions()
            .all(|wait| wait.id() != source.wait_condition_id)
    );
    assert_eq!(resolved.current_stage_id(), before.current_stage_id());
    assert_eq!(resolved.artifact_links().len(), 1);
    h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no inference"]
async fn communication_question_holds_conversation_not_context_task() -> Result<()> {
    let (h, project, employee, root) = support::fixture().await?;
    let mut supervisor = h.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    let pipeline = h.create_pipeline(project, single_stage_pipeline()).await?;
    let task = h.create_task(project, pipeline, "Context only").await?;
    h.execute(
        project,
        CommandName::ConfigureResolverRoute,
        json!({"route_key":"lead",
        "employee_ids":[employee],"assignment_timeout_seconds":60}),
    )
    .await?;
    let source_message = support::send_question(&h, project, employee, task).await?;
    h.start_project(project).await?;
    let provision = supervisor.next_communication_provision(employee).await?;
    let run = h
        .store
        .load_run(provision.run_id.parse()?)
        .await?
        .context("run")?;
    let before = h.store.load_task(task).await?.context("task")?.task;
    let client = support::client(&root, &run)?;
    let id = Uuid::now_v7();
    let args = json!({"question":"May I disclose this material?","category":"action_approval","route_key":"lead"});
    let (status, receipt) = support::call(&client, "escalation.raise", args.clone(), id).await?;
    assert!(status.is_success(), "{receipt}");
    assert!(receipt["wait_condition_id"].is_null());
    assert_eq!(
        support::call(&client, "escalation.raise", args, id)
            .await?
            .1,
        receipt
    );
    let mut tx = h.store.begin().await?;
    tx.lock_project(project).await?;
    let escalation = tx
        .load_escalation(receipt["escalation_id"].as_str().context("id")?.parse()?)
        .await?
        .context("question")?;
    assert!(tx.communication_escalation_is_current(&escalation).await?);
    let source = escalation
        .source
        .communication()
        .context("Communication source")?;
    assert_eq!(source.run_id, run.id);
    assert_eq!(source.assignment.source_message_id, source_message);
    assert_eq!(source.fencing_token, run.lease_fencing_token);
    let EscalationState::Assigned { assignment_id } = escalation.state else {
        anyhow::bail!("Human expected")
    };
    assert_eq!(
        tx.load_resolution_assignment(assignment_id)
            .await?
            .context("assignment")?
            .resolver,
        Resolver::Human,
        "action approval cannot be delegated to an Employee resolver"
    );
    assert!(
        !tx.lock_employee_capacity(project, employee).await?,
        "held conversation still occupies physical capacity"
    );
    tx.commit().await?;
    assert_eq!(h.store.load_task(task).await?.context("task")?.task, before);
    assert_eq!(h.store.list_runs_for_project(project).await?.len(), 1);
    assert!(
        !support::call(
            &client,
            "inbox.reply",
            json!({"target_message_id":source_message,"body":"late"}),
            Uuid::now_v7()
        )
        .await?
        .0
        .is_success()
    );
    let state: (String, String) = sqlx::query_as("SELECT c.state,r.desired_state FROM communication_assignments c JOIN runs r ON r.communication_assignment_id=c.id WHERE r.id=$1")
        .bind(run.id).fetch_one(&h.pool).await?;
    assert_eq!(state.0, "held");
    assert!(!matches!(
        state.1.as_str(),
        "running" | "provision_requested"
    ));
    h.shutdown().await;
    Ok(())
}
