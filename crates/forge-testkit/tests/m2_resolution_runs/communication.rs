//! A conversation decision grants no authority over its contextual Task.
use super::*;

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference"]
async fn communication_resolution_is_project_evidence_and_requires_explicit_fenced_retry()
-> Result<()> {
    let (h, project, employee, root) = support::fixture().await?;
    let version = h.create_pipeline(project, single_stage_pipeline()).await?;
    let task = h
        .create_task(project, version, "Read-only conversation context")
        .await?;
    let before = h.store.load_task(task).await?.context("Task")?.task;
    let message = support::send_question(&h, project, employee, task).await?;
    let mut supervisor = h.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    h.start_project(project).await?;
    let provision = supervisor.next_communication_provision(employee).await?;
    let run = h
        .store
        .load_run(provision.run_id.parse()?)
        .await?
        .context("Communication")?;
    let client = support::client(&root, &run)?;
    let (status, raised) = support::call(
        &client,
        "escalation.raise",
        json!({"question":"May I continue this conversation?","category":"clarification"}),
        Uuid::now_v7(),
    )
    .await?;
    assert!(status.is_success(), "{raised}");
    let id: Uuid = raised["escalation_id"]
        .as_str()
        .context("Escalation")?
        .parse()?;
    let mut tx = h.store.begin().await?;
    let escalation = tx.load_escalation(id).await?.context("Escalation")?;
    let EscalationState::Assigned { assignment_id } = escalation.state else {
        anyhow::bail!("Human assignment expected")
    };
    let assignment = tx
        .load_resolution_assignment(assignment_id)
        .await?
        .context("Resolution assignment")?;
    tx.commit().await?;
    h.execute(project,CommandName::SubmitHumanResolution,json!({"escalation_id":id,"expected_escalation_revision":escalation.revision,"assignment_id":assignment_id,"lease_generation":assignment.lease.generation,"answer":{"disposition":"continue_stage","summary":"Proceed only with a read-only explanation"}})).await?;
    assert_eq!(before, h.store.load_task(task).await?.context("Task")?.task);
    assert_eq!(
        h.store.list_runs_for_project(project).await?.len(),
        1,
        "resolution does not auto-restart provider or replenish budget"
    );
    let producer: (Option<Uuid>, Option<Uuid>, String) =
        sqlx::query_as("SELECT task_id,run_id,producer_type FROM artifacts WHERE producer_id=$1")
            .bind(assignment_id)
            .fetch_one(&h.pool)
            .await?;
    assert!(producer.0.is_none() && producer.1.is_none());
    assert_eq!(producer.2, "resolution_assignment");
    assert!(
        h.execute(
            project,
            CommandName::RetryCommunication,
            json!({"run_id":run.id,"reason":"Cannot retry before physical stop"})
        )
        .await
        .is_err()
    );
    supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Stopped)
        .await?;
    h.execute(
        project,
        CommandName::RetryCommunication,
        json!({"run_id":run.id,"reason":"Owner authorizes another attempt after the answer"}),
    )
    .await?;
    h.core.dispatch_available(project).await?;
    let provision = supervisor.next_communication_provision(employee).await?;
    let retry = h
        .store
        .load_run(provision.run_id.parse()?)
        .await?
        .context("Retry")?;
    assert_ne!(run.id, retry.id);
    assert_eq!(run.assignment, retry.assignment);
    assert_eq!(retry.attempt_number, 2);
    assert!(retry.lease_fencing_token > run.lease_fencing_token);
    let instruction: serde_json::Value = serde_json::from_str(
        retry.run_spec["instruction"]
            .as_str()
            .context("Instruction")?,
    )?;
    assert_eq!(
        instruction["resolution_answers"]
            .as_array()
            .context("Answers")?
            .len(),
        1
    );
    assert_eq!(
        retry.context_manifest["source_message"]["id"],
        json!(message)
    );
    if let Ok((status, _)) = support::call(
        &client,
        "inbox.reply",
        json!({"target_message_id":message,"body":"Old attempt"}),
        Uuid::now_v7(),
    )
    .await
    {
        assert!(
            !status.is_success(),
            "retired attempt must have no authority"
        );
    }
    assert_eq!(before, h.store.load_task(task).await?.context("Task")?.task);
    drop(supervisor);
    h.shutdown().await;
    Ok(())
}
