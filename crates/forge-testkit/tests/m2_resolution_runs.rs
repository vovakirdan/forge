//! PostgreSQL/Core/UDS proof with manual Supervisor and synthetic subscription.
#[path = "m2_resolution_runs/admission_fairness.rs"]
mod admission_fairness;
#[path = "m2_resolution_runs/communication.rs"]
mod communication;
#[path = "m2_resolution_runs/fairness.rs"]
mod fairness;
#[path = "m2_communication_runs/support.rs"]
#[allow(dead_code)] // Shared fixtures also support conversation-only scenarios.
mod support;
use anyhow::{Context, Result};
use forge_domain::{
    LifecycleStatus, ProjectId, TaskId,
    resolution::{EscalationState, Resolver},
};
use forge_protocol::{
    supervisor::v1::{AcknowledgementDisposition, RunEventKind},
    wire::CommandName,
};
use forge_testkit::m0::{M0Harness, single_stage_pipeline};
use serde_json::json;
use uuid::Uuid;

async fn question(
    h: &M0Harness,
    project: ProjectId,
    employees: Vec<forge_domain::EmployeeId>,
) -> Result<(TaskId, Uuid)> {
    let pipeline = h.create_pipeline(project, single_stage_pipeline()).await?;
    let task = h
        .create_task(project, pipeline, "A question is not stage completion")
        .await?;
    h.approve_task(project, task).await?;
    h.execute(
        project,
        CommandName::ConfigureResolverRoute,
        json!({"route_key":"engineering","employee_ids":employees,"assignment_timeout_seconds":60}),
    )
    .await?;
    let revision = h
        .store
        .load_task(task)
        .await?
        .context("task")?
        .task
        .revision()
        .get();
    let id=h.execute(project,CommandName::RaiseEscalation,json!({"task_id":task,"expected_task_revision":revision,"route_key":"engineering","category":"technical_decision","question":"Which documented option should we use?"})).await?.resource.context("question")?.id.parse()?;
    Ok((task, id))
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference"]
async fn resolution_run_is_taskless_one_shot_and_accepts_only_question_answer() -> Result<()> {
    let (h, project, bob, root) = support::fixture_mode(true).await?;
    let (task, id) = question(&h, project, vec![bob]).await?;
    let mut supervisor = h.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    h.start_project(project).await?;
    let provision = supervisor.next_provision().await?;
    assert_eq!(provision.run_spec_version, 4);
    assert!(provision.task_id.is_empty() && provision.stage_id.is_empty());
    let run = h
        .store
        .load_run(provision.run_id.parse()?)
        .await?
        .context("resolver")?;
    assert_eq!(
        run.assignment.resolution().context("owner")?.escalation_id,
        id
    );
    assert!(run.require_task_id().is_err());
    let invocation: serde_json::Value = serde_json::from_slice(&std::fs::read(
        root.join("grants")
            .join(run.id.to_string())
            .join(run.environment_epoch.to_string())
            .join("invocation.json"),
    )?)?;
    assert!(
        invocation["runtime_input"].is_null(),
        "v4 is genuinely one-shot even with live Employee profile"
    );
    let args = invocation["args"].as_array().context("args")?;
    assert!(args.iter().any(|arg| arg == "--no-session-persistence"));
    assert_eq!(run.run_spec["binding"]["surface"], json!({"mode":"none"}));
    assert!(
        run.run_spec["binding"]["execution_profile"]["capability_profile"]["capabilities"]
            .as_array()
            .context("capabilities")?
            .contains(&json!("live_input")),
        "canonical profile remains unchanged"
    );
    let client = support::client(&root, &run)?;
    assert!(
        support::call(&client, "resolution.read", json!({}), Uuid::now_v7())
            .await?
            .0
            .is_success()
    );
    assert!(
        !support::call(
            &client,
            "outcome.submit",
            json!({"stage_id":"work","outcome":"completed"}),
            Uuid::now_v7()
        )
        .await?
        .0
        .is_success()
    );
    assert!(!support::call(&client,"resolution.submit",json!({"disposition":"continue_stage","summary":"Use A","recommended_outcome_key":"invented"}),Uuid::now_v7()).await?.0.is_success());
    let answer_id = Uuid::now_v7();
    let args = json!({"disposition":"continue_stage","summary":"Use documented option A"});
    let (status, answer) =
        support::call(&client, "resolution.submit", args.clone(), answer_id).await?;
    assert!(status.is_success(), "{answer}");
    let retry = support::call(&client, "resolution.submit", args, answer_id).await?;
    assert!(retry.0.is_success(), "{}", retry.1);
    assert_eq!(retry.1, answer);
    assert!(
        !support::call(
            &client,
            "resolution.submit",
            json!({"disposition":"continue_stage","summary":"A changed claim"}),
            answer_id
        )
        .await?
        .0
        .is_success()
    );
    assert!(
        !support::call(
            &client,
            "resolution.submit",
            json!({"disposition":"continue_stage","summary":"Use documented option A"}),
            Uuid::now_v7()
        )
        .await?
        .0
        .is_success()
    );
    let mcp = |body| {
        client
            .post("http://localhost/mcp")
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-11-25")
            .json(&body)
    };
    let initialized=mcp(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"resolution-retry","version":"1"}}})).send().await?;
    assert!(initialized.status().is_success());
    let replay:serde_json::Value=mcp(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"forge_submit_resolution","arguments":{"message_id":answer_id,"disposition":"continue_stage","summary":"Use documented option A"}}})).send().await?.json().await?;
    assert_eq!(replay["result"]["structuredContent"], answer, "{replay}");
    let denied:serde_json::Value=mcp(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"forge_submit_resolution","arguments":{"message_id":Uuid::now_v7(),"disposition":"continue_stage","summary":"Use documented option A"}}})).send().await?.json().await?;
    assert_eq!(denied["result"]["isError"], json!(true), "{denied}");
    let task = h.store.load_task(task).await?.context("task")?.task;
    assert_eq!(task.lifecycle(), LifecycleStatus::Ready);
    assert_eq!(task.current_stage_id().context("stage")?.as_str(), "work");
    assert_eq!(task.artifact_links().len(), 1);
    let mut tx = h.store.begin().await?;
    tx.lock_project(project).await?;
    assert!(
        !tx.lock_employee_capacity(project, bob).await?,
        "logical answer does not release sandbox capacity"
    );
    assert!(tx.resolution_has_live_execution(id).await?);
    tx.commit().await?;
    h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference"]
async fn resolution_decline_waits_for_physical_quiescence_before_human_fallback() -> Result<()> {
    let (h, project, bob, root) = support::fixture().await?;
    let (task, id) = question(&h, project, vec![bob]).await?;
    let mut supervisor = h.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    h.start_project(project).await?;
    let provision = supervisor.next_provision().await?;
    let run = h
        .store
        .load_run(provision.run_id.parse()?)
        .await?
        .context("resolver")?;
    let client = support::client(&root, &run)?;
    let declined = support::call(
        &client,
        "resolution.decline",
        json!({"reason":"Need another resolver"}),
        Uuid::now_v7(),
    )
    .await?;
    assert!(declined.0.is_success(), "{}", declined.1);
    h.core.dispatch_available(project).await?;
    let mut tx = h.store.begin().await?;
    assert_eq!(
        tx.load_escalation(id).await?.context("question")?.state,
        EscalationState::Queued
    );
    tx.commit().await?;
    let mut tx = h.store.begin().await?;
    tx.lock_project(project).await?;
    tx.revoke_run_lease(run.id, run.lease_fencing_token, run.environment_epoch)
        .await?;
    tx.commit().await?;
    h.core.dispatch_available(project).await?;
    let mut tx = h.store.begin().await?;
    assert_eq!(
        tx.load_escalation(id).await?.context("question")?.state,
        EscalationState::Queued,
        "logical revocation without physical proof is insufficient"
    );
    tx.commit().await?;
    let ack = supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Stopped)
        .await?;
    assert_eq!(ack.disposition, AcknowledgementDisposition::Accepted as i32);
    h.core.dispatch_available(project).await?;
    let mut tx = h.store.begin().await?;
    let current = tx.load_escalation(id).await?.context("question")?;
    let EscalationState::Assigned { assignment_id } = current.state else {
        anyhow::bail!("expected Human fallback: {:?}", current.state)
    };
    let assignment = tx
        .load_resolution_assignment(assignment_id)
        .await?
        .context("assignment")?;
    assert_eq!(assignment.resolver, Resolver::Human);
    assert!(assignment.lease.expires_at.is_none());
    assert_eq!(assignment.lease.generation, 2);
    tx.commit().await?;
    assert_eq!(
        h.store
            .load_task(task)
            .await?
            .context("task")?
            .task
            .lifecycle(),
        LifecycleStatus::Waiting
    );
    assert_eq!(
        h.store.list_runs_for_project(project).await?.len(),
        1,
        "Human resolution creates no fake Run/Employee"
    );
    h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference"]
async fn resolution_timeout_revokes_old_answer_and_cancel_supersedes_without_fabrication()
-> Result<()> {
    let (h, project, bob, root) = support::fixture().await?;
    let (task, id) = question(&h, project, vec![bob]).await?;
    let mut supervisor = h.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    h.start_project(project).await?;
    let provision = supervisor.next_provision().await?;
    let run = h
        .store
        .load_run(provision.run_id.parse()?)
        .await?
        .context("resolver")?;
    let context: forge_domain::resolution::ResolutionContext =
        serde_json::from_value(run.context_manifest.clone())?;
    let deadline = context
        .data()
        .assignment
        .lease
        .expires_at
        .context("employee deadline")?;
    h.core.reconcile_resolutions_at(deadline).await?;
    assert!(
        !support::call(
            &support::client(&root, &run)?,
            "resolution.submit",
            json!({"disposition":"continue_stage","summary":"Late answer"}),
            Uuid::now_v7()
        )
        .await?
        .0
        .is_success()
    );
    h.core.dispatch_available(project).await?;
    let mut tx = h.store.begin().await?;
    assert_eq!(
        tx.load_escalation(id).await?.context("question")?.state,
        EscalationState::Queued
    );
    assert!(tx.resolution_has_live_execution(id).await?);
    tx.commit().await?;
    let revision = h
        .store
        .load_task(task)
        .await?
        .context("task")?
        .task
        .revision()
        .get();
    h.execute(project,CommandName::CancelTask,json!({"task_id":task,"expected_task_revision":revision,"cancellation_reason_key":"unspecified"})).await?;
    h.core.reconcile_resolutions().await?;
    let mut tx = h.store.begin().await?;
    assert!(matches!(
        tx.load_escalation(id).await?.context("question")?.state,
        EscalationState::Superseded { .. }
    ));
    tx.commit().await?;
    assert_eq!(
        h.store
            .load_task(task)
            .await?
            .context("task")?
            .task
            .artifact_links()
            .len(),
        0,
        "obsolete question receives no invented answer Artifact"
    );
    h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference"]
async fn resolution_skips_busy_employee_and_obeys_generic_employee_stop() -> Result<()> {
    let (h, project, bob, root) = support::fixture().await?;
    h.create_employee(project, "Zoe").await?;
    let zoe = h
        .store
        .list_employees(project)
        .await?
        .into_iter()
        .find(|employee| employee.employee.id() != bob)
        .context("Zoe")?
        .employee
        .id();
    let mut tx = h.store.begin().await?;
    let binding = tx.runtime_binding(bob).await?.context("binding")?;
    tx.commit().await?;
    h.execute(
        project,
        CommandName::ConfigureEmployeeRuntime,
        json!({"employee_id":zoe,"binding":binding}),
    )
    .await?;
    let (question_task, id) = question(&h, project, vec![bob, zoe]).await?;
    let pipeline = h
        .store
        .load_task(question_task)
        .await?
        .context("question Task")?
        .task
        .pipeline()
        .pipeline_version_id();
    let work = h
        .create_task(project, pipeline, "Bob is already busy")
        .await?;
    h.approve_task(project, work).await?;
    let mut supervisor = h.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    h.start_project(project).await?;
    let writer = supervisor.next_provision_for_task(work).await?;
    assert_eq!(writer.employee_id, bob.to_string());
    let provision = supervisor.next_provision().await?;
    let run = h
        .store
        .load_run(provision.run_id.parse()?)
        .await?
        .context("resolver")?;
    assert_eq!(run.require_employee_id()?, zoe);
    assert_eq!(
        run.assignment.resolution().context("owner")?.escalation_id,
        id
    );
    let mut tx = h.store.begin().await?;
    tx.lock_project(project).await?;
    assert!(
        tx.lock_active_runs_for_task(project, question_task)
            .await?
            .is_empty(),
        "question context is not execution ownership"
    );
    assert!(!tx.lock_employee_capacity(project, zoe).await?);
    tx.commit().await?;
    let revision = h
        .store
        .list_employees(project)
        .await?
        .into_iter()
        .find(|employee| employee.employee.id() == zoe)
        .context("Zoe")?
        .employee
        .revision();
    h.execute(
        project,
        CommandName::StopEmployee,
        json!({"employee_id":zoe,"expected_employee_revision":revision,"mode":"force"}),
    )
    .await?;
    assert_eq!(
        h.store
            .load_run(run.id)
            .await?
            .context("run")?
            .desired_state,
        forge_storage::RunDesiredState::ForceStopRequested
    );
    assert!(
        !support::call(
            &support::client(&root, &run)?,
            "resolution.read",
            json!({}),
            Uuid::now_v7()
        )
        .await?
        .0
        .is_success()
    );
    let mut tx = h.store.begin().await?;
    assert!(tx.resolution_has_live_execution(id).await?);
    tx.commit().await?;
    assert_eq!(
        h.store
            .load_task(question_task)
            .await?
            .context("Task")?
            .task
            .lifecycle(),
        LifecycleStatus::Waiting
    );
    h.shutdown().await;
    Ok(())
}
