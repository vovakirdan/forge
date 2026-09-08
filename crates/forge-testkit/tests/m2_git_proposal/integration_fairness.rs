//! Projection-query regression, not fabricated canonical Task execution history.
use anyhow::Result;
use forge_domain::{ProjectId, TaskId};
use forge_storage::PostgresStore;
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires local PostgreSQL; session-private temporary projection tables"]
async fn integration_held_history_does_not_starve_new_work_at_the_scan_limit() -> Result<()> {
    anyhow::ensure!(
        std::env::var("FORGE_INTEGRATION").as_deref() == Ok("1"),
        "explicit integration opt-in required"
    );
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&std::env::var("FORGE_DATABASE_URL")?)
        .await?;
    // These session-local tables deliberately model only the scan projections.
    // No production table, migration constraint or canonical snapshot is changed.
    sqlx::raw_sql("CREATE TEMP TABLE git_integrations(project_id uuid,task_id uuid,updated_at timestamptz,state text,resolution text,command jsonb,result jsonb); CREATE TEMP TABLE tasks(project_id uuid,id uuid,updated_at timestamptz,pipeline_version_id uuid,lifecycle text,current_stage_id text); CREATE TEMP TABLE pipeline_versions(id uuid,definition jsonb)").execute(&pool).await?;
    let project = ProjectId::new();
    let task = TaskId::new();
    let version = Uuid::now_v7();
    for index in 0..64 {
        sqlx::query("INSERT INTO git_integrations VALUES($1,$2,clock_timestamp()-interval '1 hour','held',NULL,$3,$4)")
            .bind(project.as_uuid()).bind(Uuid::now_v7())
            .bind(if index%2==0 {None}else{Some(json!({"phase":"apply","command_id":"synthetic"}))})
            .bind(if index%2==0 {None}else{Some(json!({"code":"applied","command_id":"synthetic"}))})
            .execute(&pool).await?;
    }
    sqlx::query("INSERT INTO pipeline_versions VALUES($1,$2)")
        .bind(version)
        .bind(json!({"stages":{"publish_here":{"system_action":{"kind":"git_integration"}}}}))
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO tasks VALUES($1,$2,clock_timestamp(),$3,'in_progress','publish_here')",
    )
    .bind(project.as_uuid())
    .bind(task.as_uuid())
    .bind(version)
    .execute(&pool)
    .await?;
    let hook_version = Uuid::now_v7();
    sqlx::query("INSERT INTO pipeline_versions VALUES($1,$2)")
        .bind(hook_version)
        .bind(json!({"stages":{"publish_here":{"system_action":{"kind":"project_hook"}}}}))
        .execute(&pool)
        .await?;
    sqlx::query("INSERT INTO tasks VALUES($1,$2,clock_timestamp()-interval '2 hours',$3,'in_progress','publish_here')")
        .bind(project.as_uuid()).bind(Uuid::now_v7()).bind(hook_version).execute(&pool).await?;
    let work = PostgresStore::from_pool(pool.clone())
        .integration_work()
        .await?;
    assert_eq!(
        work,
        vec![(project, task)],
        "retained non-actionable holds must not consume the bounded scan"
    );
    pool.close().await;
    Ok(())
}
