//! Independent test Core instances cannot reconcile each other's queues or boot state.

use anyhow::Result;
use forge_testkit::m0::{M0Harness, single_stage_pipeline};

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; creates retained, test-owned schemas"]
async fn each_harness_has_private_schema_queue_and_host_generation() -> Result<()> {
    let first = M0Harness::start().await?;
    let project = first.create_project("Retained first fixture").await?;
    first
        .create_employee(project, "Disconnected worker")
        .await?;
    let pipeline = first
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    let task = first
        .create_task(project, pipeline, "Unclaimed first-fixture work")
        .await?;
    first.approve_task(project, task).await?;
    first.start_project(project).await?;
    let mut state = first.store.begin().await?;
    state
        .record_execution_host("first-host", "first-boot")
        .await?;
    state.commit().await?;

    let second = M0Harness::start().await?;
    let first_schema: String = sqlx::query_scalar("SELECT current_schema()")
        .fetch_one(&first.pool)
        .await?;
    let second_schema: String = sqlx::query_scalar("SELECT current_schema()")
        .fetch_one(&second.pool)
        .await?;
    assert_ne!(first_schema, second_schema);
    assert!(second_schema.starts_with("forge_test_"));
    assert!(second.store.list_project_ids().await?.is_empty());
    assert!(second.store.execution_host().await?.is_none());
    let mut second_connection = second.pool.acquire().await?;
    let mut parallel_connection = second.pool.acquire().await?;
    for connection in [&mut second_connection, &mut parallel_connection] {
        let path: String = sqlx::query_scalar("SHOW search_path")
            .fetch_one(&mut **connection)
            .await?;
        assert_eq!(
            path, second_schema,
            "every pool connection excludes public fallback"
        );
    }
    drop(second_connection);
    drop(parallel_connection);
    let mut supervisor = second.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    assert!(
        first.store.list_runs_for_project(project).await?.is_empty(),
        "second fixture must not claim foreign work on attach"
    );
    assert!(
        first.store.load_task(task).await?.is_some(),
        "old fixture history is retained"
    );
    first.stop_project(project).await?;
    Ok(())
}
