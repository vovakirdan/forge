//! Real canonical input intent and UDS receipt/Gateway boundary; no paid model.
use super::support::{call, client, fixture_mode};
use anyhow::{Context, Result};
use forge_domain::{
    ContextSnapshot,
    communication::{RuntimeInputOutcome, TaskMessageContext},
};
use forge_protocol::{
    supervisor::v1::{
        AcknowledgementDisposition as Disposition, RunEventKind, RuntimeInputStatus,
        deliver_runtime_input::Action,
    },
    wire::CommandName,
};
use forge_testkit::m0::single_stage_pipeline;
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; synthetic credential, manual native observations"]
async fn runtime_acceptance_is_fenced_durable_and_does_not_satisfy_employee_ack() -> Result<()> {
    let (harness, project, employee, root) = fixture_mode(true).await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    let task = harness
        .create_task(project, pipeline, "Controlled writer")
        .await?;
    harness.approve_task(project, task).await?;
    harness.start_project(project).await?;
    let provision = supervisor.next_provision_for_task(task).await?;
    let run = harness
        .store
        .load_run(provision.run_id.parse()?)
        .await?
        .context("Run")?;
    supervisor
        .send_observation(&run, run.lease_fencing_token, 1)
        .await?;
    let context: ContextSnapshot = serde_json::from_value(run.context_manifest.clone())?;
    let context = context.data();
    let target = TaskMessageContext {
        task_id: task,
        pipeline_version_id: context.pipeline_version_id,
        stage_id: context.stage_id.clone(),
        stage_visit: context.stage_visit.context("visit")?,
    };
    let thread: Uuid = harness
        .execute(
            project,
            CommandName::OpenEmployeeThread,
            json!({"employee_id":employee,"task_id":task}),
        )
        .await?
        .resource
        .context("thread")?
        .id
        .parse()?;
    let message:Uuid=harness.execute(project,CommandName::SendEmployeeMessage,json!({"thread_id":thread,"expected_thread_revision":1,"target":{"kind":"exact_run","context":target,"run_id":run.id,"fencing_token":run.lease_fencing_token,"environment_epoch":run.environment_epoch},"kind":"instruction","requirement":"acknowledged","body":"Keep compatibility with the existing interface."})).await?.resource.context("message")?.id.parse()?;
    let input = supervisor.next_runtime_input(run.id).await?;
    let Some(Action::Message(source)) = &input.action else {
        anyhow::bail!("expected source input")
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&source.source_message_json)?["id"],
        json!(message)
    );
    let mut stale = input.clone();
    stale.environment_epoch += 1;
    assert_eq!(
        supervisor
            .input_receipt(&stale, RuntimeInputStatus::RuntimeAccepted, Uuid::now_v7())
            .await?
            .disposition,
        Disposition::IgnoredStale as i32
    );
    let receipt = Uuid::now_v7();
    assert_eq!(
        supervisor
            .input_receipt(&input, RuntimeInputStatus::RuntimeAccepted, receipt)
            .await?
            .disposition,
        Disposition::Accepted as i32
    );
    let native_audit: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM event_log WHERE project_id=$1 AND event_type='runtime_input_observed' AND payload->>'input_id'=$2 ORDER BY project_sequence DESC LIMIT 1"
    ).bind(project.as_uuid()).bind(&input.command_id).fetch_one(&harness.pool).await?;
    assert_eq!(native_audit["source_message_id"], json!(message));
    assert_eq!(native_audit["run_id"], json!(run.id));
    assert_eq!(
        native_audit["fencing_token"],
        json!(run.lease_fencing_token)
    );
    assert_eq!(
        native_audit["environment_epoch"],
        json!(run.environment_epoch)
    );
    assert_eq!(native_audit["outcome"], json!("runtime_accepted"));
    assert_eq!(
        supervisor
            .input_receipt(&input, RuntimeInputStatus::RuntimeAccepted, receipt)
            .await?
            .disposition,
        Disposition::Accepted as i32
    );
    let mut tx = harness.store.begin().await?;
    tx.lock_project(project).await?;
    assert_eq!(
        tx.runtime_input(input.command_id.parse()?)
            .await?
            .context("input")?
            .outcome,
        Some(RuntimeInputOutcome::RuntimeAccepted)
    );
    assert_eq!(
        tx.pending_task_messages(project, &target).await?,
        vec![message],
        "native receipt is not Employee ACK"
    );
    tx.commit().await?;
    let scoped = client(&root, &run)?;
    assert!(
        call(
            &scoped,
            "inbox.acknowledge",
            json!({"target_message_id":message}),
            Uuid::now_v7()
        )
        .await?
        .0
        .is_success()
    );
    let mut tx = harness.store.begin().await?;
    tx.lock_project(project).await?;
    assert!(tx.pending_task_messages(project, &target).await?.is_empty());
    tx.commit().await?;
    bounded_source_scan_advances(&harness, &run, message, &target).await?;
    let queued:Uuid=harness.execute(project,CommandName::SendEmployeeMessage,json!({"thread_id":thread,"expected_thread_revision":2,"target":{"kind":"task_execution","context":target},"kind":"instruction","requirement":"acknowledged","body":"An instruction not yet consumed."})).await?.resource.context("queued")?.id.parse()?;
    let undelivered = loop {
        let input = supervisor.next_runtime_input(run.id).await?;
        if matches!(&input.action,Some(Action::Message(source)) if source.source_message_json.contains(&queued.to_string()))
        {
            break input;
        }
    };
    harness.stop_project(project).await?;
    supervisor.next_stop_for_run(&run.id.to_string()).await?;
    let close = loop {
        let input = supervisor.next_runtime_input(run.id).await?;
        if matches!(input.action, Some(Action::CloseAfterTurn(_))) {
            break input;
        }
    };
    assert_eq!(
        supervisor
            .input_receipt(&close, RuntimeInputStatus::InputClosed, Uuid::now_v7())
            .await?
            .disposition,
        Disposition::Accepted as i32
    );
    let mut tx = harness.store.begin().await?;
    tx.lock_project(project).await?;
    assert_eq!(
        tx.runtime_input(undelivered.command_id.parse()?)
            .await?
            .context("undelivered")?
            .outcome,
        Some(RuntimeInputOutcome::DeliveryUnknown)
    );
    assert_eq!(
        tx.pending_task_messages(project, &target).await?,
        vec![queued]
    );
    tx.commit().await?;
    supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 2, RunEventKind::Stopped)
        .await?;
    // A delayed native echo is an exact historical fact, even after Task waiting
    // and physical quiescence. It refines unknown but cannot satisfy the barrier.
    let late = Uuid::now_v7();
    assert_eq!(
        supervisor
            .input_receipt(&undelivered, RuntimeInputStatus::RuntimeAccepted, late)
            .await?
            .disposition,
        Disposition::Accepted as i32
    );
    assert_eq!(
        supervisor
            .input_receipt(&undelivered, RuntimeInputStatus::RuntimeAccepted, late)
            .await?
            .disposition,
        Disposition::Accepted as i32
    );
    assert_eq!(
        supervisor
            .input_receipt(&undelivered, RuntimeInputStatus::DeliveryUnknown, late)
            .await?
            .disposition,
        Disposition::Rejected as i32,
        "same receipt identity cannot change fact"
    );
    let mut tx = harness.store.begin().await?;
    tx.lock_project(project).await?;
    assert_eq!(
        tx.runtime_input(undelivered.command_id.parse()?)
            .await?
            .context("refined")?
            .outcome,
        Some(RuntimeInputOutcome::RuntimeAccepted)
    );
    assert_eq!(
        tx.pending_task_messages(project, &target).await?,
        vec![queued]
    );
    tx.commit().await?;
    harness.shutdown().await;
    Ok(())
}

/// A rollback-only storage contract isolates pagination from command delivery.
async fn bounded_source_scan_advances(
    harness: &forge_testkit::m0::M0Harness,
    run: &forge_storage::RunProjection,
    message: Uuid,
    target: &TaskMessageContext,
) -> Result<()> {
    use forge_domain::communication::{DeliveryScope, EmployeeMessage, RuntimeInputAction};
    let mut tx = harness.store.begin().await?;
    tx.lock_project(run.project_id).await?;
    let source = tx.load_employee_message(message).await?.context("source")?;
    let scope = DeliveryScope {
        project_id: run.project_id,
        employee_id: run.require_employee_id()?,
        thread_id: source.data().thread_id,
        run_id: run.id,
        fencing_token: run.lease_fencing_token,
        environment_epoch: run.environment_epoch,
        task_context: Some(target.clone()),
    };
    let mut ids = Vec::new();
    for sequence in 2..=202 {
        let mut data = source.data().clone();
        data.id = Uuid::now_v7();
        data.sequence = sequence;
        let source = EmployeeMessage::new(data)?;
        ids.push(source.data().id);
        tx.insert_employee_message(&source).await?;
    }
    let first = tx.messages_for_native_input(&scope).await?;
    assert_eq!(first.len(), 128);
    for source in first {
        let now = forge_domain::Timestamp::now_utc();
        let id = tx
            .create_runtime_input(
                run,
                &RuntimeInputAction::Message {
                    source: Box::new(source),
                },
                now,
            )
            .await?
            .context("created")?;
        assert!(
            tx.observe_runtime_input(
                id,
                Uuid::now_v7(),
                RuntimeInputOutcome::RuntimeAccepted,
                now
            )
            .await?
        );
    }
    let second = tx.messages_for_native_input(&scope).await?;
    assert_eq!(second.len(), 73);
    assert_eq!(
        second.last().context("last")?.data().id,
        *ids.last().context("id")?
    );
    // Deliberately roll back synthetic storage-only data; no forged audit history.
    drop(tx);
    Ok(())
}
