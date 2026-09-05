use super::*;

#[tokio::test]
#[ignore = "requires local PostgreSQL and NATS; run `just test-integration`"]
async fn m0_terminal_run_failure_releases_capacity_and_interrupts_its_task() -> Result<()> {
    assert_terminal_run_interruption("M0 provider failure", RunEventKind::ProviderFailed).await
}

#[tokio::test]
#[ignore = "requires local PostgreSQL and NATS; run `just test-integration`"]
async fn m0_environment_loss_releases_capacity_and_interrupts_its_task() -> Result<()> {
    assert_terminal_run_interruption("M0 environment loss", RunEventKind::EnvironmentLost).await
}

async fn assert_terminal_run_interruption(
    project_name: &str,
    terminal_kind: RunEventKind,
) -> Result<()> {
    let harness = M0Harness::start().await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    let project = harness.create_project(project_name).await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    harness.create_employee(project, "failure worker").await?;
    let interrupted = harness
        .create_task(project, pipeline, "interrupted task")
        .await?;
    let next = harness.create_task(project, pipeline, "next task").await?;
    harness.approve_task(project, interrupted).await?;
    harness.approve_task(project, next).await?;
    harness.start_project(project).await?;

    let first = supervisor.next_provision_for_task(interrupted).await?;
    assert_eq!(first.task_id, interrupted.as_uuid().to_string());
    let run = harness.wait_for_run_count(interrupted, 1).await?.remove(0);
    let acknowledgement = supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 1, terminal_kind)
        .await?;
    assert_eq!(
        acknowledgement.disposition,
        AcknowledgementDisposition::Accepted as i32,
        "terminal failure was rejected: {acknowledgement:?}"
    );
    harness
        .wait_for_task(interrupted, has_interrupted_wait)
        .await?;
    let second = supervisor.next_provision_for_task(next).await?;
    assert_eq!(second.task_id, next.as_uuid().to_string());

    drop(supervisor);
    harness.shutdown().await;
    Ok(())
}
