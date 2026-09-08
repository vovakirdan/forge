//! One invalid pinned policy becomes a visible wait, not a global dispatch error.
use super::*;

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; canonical Tasks, no provider or Hook process"]
async fn invalid_first_hook_policy_does_not_block_later_valid_task() -> Result<()> {
    let (h, project, _employee, _root) = provider_fixture::fixture().await?;
    let hook:Uuid=h.execute(project,CommandName::ConfigureProjectHook,json!({"name":"Explicit optional-by-kind check","image":format!("localhost/synthetic@sha256:{}","0".repeat(64)),"command":["/bin/true"],"workdir":".","limits":{"cpu_millis":1000,"memory_bytes":134217728,"pids":32,"wall_seconds":60,"stop_grace_seconds":1},"max_output_bytes":4096,"applicable_task_kinds":["analysis"],"required":true})).await?.resource.context("Hook")?.id.parse()?;
    let mut invalid = support::pipeline("advisory", hook);
    invalid["entry_stage_id"] = json!("checks");
    invalid["name"] = json!("Required failure cannot complete Task");
    let bad_version = h.create_pipeline(project, invalid).await?;
    let bad = h
        .create_task(project, bad_version, "First: contradictory pinned policy")
        .await?;
    h.approve_task(project, bad).await?;
    let mut valid = support::pipeline("skipped", hook);
    valid["entry_stage_id"] = json!("checks");
    valid["name"] = json!("Later valid policy");
    let good_version = h.create_pipeline(project, valid).await?;
    let good = h
        .create_task(project, good_version, "Second: nonapplicable hook")
        .await?;
    h.approve_task(project, good).await?;
    assert_eq!(
        h.store
            .load_task(bad)
            .await?
            .context("Bad")?
            .task
            .lifecycle(),
        LifecycleStatus::InProgress
    );
    assert_eq!(
        h.store
            .load_task(good)
            .await?
            .context("Good")?
            .task
            .lifecycle(),
        LifecycleStatus::InProgress
    );
    let order: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM tasks WHERE project_id=$1 ORDER BY updated_at,id")
            .bind(project.as_uuid())
            .fetch_all(&h.pool)
            .await?;
    assert_eq!(order, vec![bad.as_uuid(), good.as_uuid()]);
    let mut supervisor = h.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    h.start_project(project).await?;
    support::tick(&h).await?;
    let bad = h.store.load_task(bad).await?.context("Bad")?.task;
    assert_eq!(bad.lifecycle(), LifecycleStatus::Waiting);
    assert!(bad.wait_conditions().next().is_some());
    assert_eq!(
        h.store
            .load_task(good)
            .await?
            .context("Good")?
            .task
            .lifecycle(),
        LifecycleStatus::Done
    );
    assert!(h.store.list_runs_for_project(project).await?.is_empty());
    drop(supervisor);
    h.shutdown().await;
    Ok(())
}
