use super::*;

#[tokio::test]
#[ignore = "requires local PostgreSQL and NATS; run `just test-integration`"]
async fn m0_cancelled_task_rejects_late_outcome_without_claiming_physical_stop() -> Result<()> {
    let harness = M0Harness::start().await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    let project = harness.create_project("Cancellation late outcome").await?;
    let version = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    harness
        .create_employee(project, "Cancellation worker")
        .await?;
    let task = harness
        .create_task(project, version, "Work cancelled by owner")
        .await?;
    harness.approve_task(project, task).await?;
    harness.start_project(project).await?;
    supervisor.next_provision_for_task(task).await?;
    let run = harness.wait_for_run_count(task, 1).await?.remove(0);
    for (sequence, kind) in [(1, RunEventKind::Provisioning), (2, RunEventKind::Running)] {
        assert_accepted(
            "observation",
            supervisor
                .send_observation_kind(&run, run.lease_fencing_token, sequence, kind)
                .await?,
        )?;
    }
    let (evidence, acknowledgement) = supervisor.submit_stage_evidence(&run, 3).await?;
    assert_accepted("evidence before cancellation", acknowledgement)?;
    let artifacts_before = harness.store.list_artifacts_for_task(task).await?;
    assert_eq!(artifacts_before.len(), 1);
    harness.cancel_task(project, task).await?;
    let cancelled = harness.required_task(task).await?.task;
    assert_eq!(cancelled.lifecycle(), LifecycleStatus::Cancelled);
    let pending = harness.store.load_run(run.id).await?.context("run")?;
    assert_eq!(pending.desired_state, RunDesiredState::StopRequested);
    assert_eq!(pending.observed_state, RunObservedState::Running);
    let acknowledgement = supervisor
        .submit_stage_outcome(&run, 4, "completed", vec![evidence])
        .await?;
    assert_ne!(
        acknowledgement.disposition,
        AcknowledgementDisposition::Accepted as i32
    );
    assert_eq!(harness.required_task(task).await?.task, cancelled);
    // Accepted evidence remains attached; rejecting a late outcome must not
    // erase previous work or attach additional artifacts.
    let artifacts_after = harness.store.list_artifacts_for_task(task).await?;
    assert_eq!(artifacts_after.len(), artifacts_before.len());
    assert_eq!(artifacts_after[0].artifact, artifacts_before[0].artifact);
    assert_eq!(artifacts_after[0].location.task_id, Some(task));
    assert_eq!(artifacts_after[0].location.run_id, Some(run.id));
    drop(supervisor);
    harness.shutdown().await;
    Ok(())
}
