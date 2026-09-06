use super::*;

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; deterministic held-lock watchdog clock regression"]
async fn watchdog_uses_post_lock_mutation_time_not_its_deadline_clock() -> Result<()> {
    let fixture = Fixture::new().await?;
    let mut supervisor = fixture.supervisor(&fixture.boot).await?;
    supervisor.reconcile_empty().await?;
    let (task, _) = fixture.tasks().await?;
    supervisor.next_provision_for_task(task).await?;
    let run = fixture.harness.wait_for_run_count(task, 1).await?.remove(0);
    supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Running)
        .await?;
    sqlx::query(
        "UPDATE runs SET liveness_observed_at=clock_timestamp()-INTERVAL '91 seconds' WHERE id=$1",
    )
    .bind(run.id)
    .execute(&fixture.harness.pool)
    .await?;

    let mut writer = fixture.harness.store.begin().await?;
    let mut project = writer
        .lock_project(fixture.project)
        .await?
        .context("held project")?;
    let tick_time = Timestamp::now_utc();
    let tick = fixture
        .harness
        .core
        .watchdog_tick(tick_time, WatchdogDeadlines::default());
    tokio::pin!(tick);
    tokio::select! {
        result = &mut tick => {
            result?;
            anyhow::bail!("watchdog unexpectedly passed the held Project lock");
        },
        () = tokio::time::sleep(std::time::Duration::from_millis(50)) => {},
    }
    // Test-only management-style mutation commits while the watchdog, whose
    // deadline clock was already captured, is blocked at this Project lock.
    let stored = writer.lock_task(task).await?.context("held Task")?;
    let mut updated = stored.task;
    let task_revision = updated.revision().get();
    let project_revision = project.revision();
    let changed_at = Timestamp::now_utc();
    assert!(changed_at > tick_time);
    updated.set_priority(
        project.priority_scheme(),
        updated.priority_level_id().clone(),
        changed_at,
    )?;
    project.record_child_mutation(changed_at)?;
    writer
        .update_task(&updated, stored.persistence, task_revision)
        .await?;
    writer.update_project(&project, project_revision).await?;
    writer.commit().await?;
    let report = tokio::time::timeout(std::time::Duration::from_secs(10), &mut tick).await??;
    assert!(report.quarantined >= 1);
    let updated = fixture.harness.required_task(task).await?.task;
    assert_eq!(updated.lifecycle(), LifecycleStatus::Waiting);
    assert!(updated.updated_at() >= changed_at);
    fixture.finish(&mut supervisor).await
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no future deadline timestamps in canonical Task state"]
async fn future_watchdog_deadline_clock_does_not_poison_later_management_commands() -> Result<()> {
    let fixture = Fixture::new().await?;
    let mut supervisor = fixture.supervisor(&fixture.boot).await?;
    supervisor.reconcile_empty().await?;
    let (task, _) = fixture.tasks().await?;
    supervisor.next_provision_for_task(task).await?;
    let future = Timestamp::from_offset_date_time(
        Timestamp::now_utc().as_offset_date_time() + std::time::Duration::from_secs(3600),
    );
    fixture
        .harness
        .core
        .watchdog_tick(future, WatchdogDeadlines::default())
        .await?;
    let updated = fixture.harness.required_task(task).await?.task;
    assert_eq!(updated.lifecycle(), LifecycleStatus::Waiting);
    assert!(updated.updated_at() < future);
    // Previously the injected future time stamped both Project and Task, making
    // this ordinary current-time management command fail until the clock caught up.
    fixture.harness.stop_project(fixture.project).await?;
    fixture.finish(&mut supervisor).await
}
