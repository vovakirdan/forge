//! Session-local queue projections; this test does not fabricate canonical Tasks.
use anyhow::Result;
use forge_domain::{ProjectId, TaskId};
use forge_storage::PostgresStore;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires local PostgreSQL; rollback-only private projection schema"]
async fn hook_blockers_do_not_consume_the_bounded_task_or_project_scan() -> Result<()> {
    anyhow::ensure!(
        std::env::var("FORGE_INTEGRATION").as_deref() == Ok("1"),
        "explicit integration opt-in required"
    );
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&std::env::var("FORGE_DATABASE_URL")?)
        .await?;
    // PostgreSQL never resolves unqualified functions in its temporary schema.
    // A unique schema inside this session's transaction supplies compatible row
    // types and rolls back all DDL/data, including if the connection is dropped.
    let schema = format!("hook_scan_{}", Uuid::now_v7().simple());
    sqlx::raw_sql(&format!(
        "BEGIN; CREATE SCHEMA {schema}; SET LOCAL search_path TO {schema};
         CREATE TABLE projects(id uuid,execution_enabled bool);
         CREATE TABLE project_recovery_settings(project_id uuid,hold bool);
         CREATE TABLE tasks(id uuid,project_id uuid,pipeline_version_id uuid,
          lifecycle text,current_stage_id text,updated_at timestamptz);
         CREATE TABLE pipeline_versions(id uuid,definition jsonb);
         CREATE TABLE hook_invocations(id uuid,task_id uuid,state text);
         CREATE TABLE leases(id uuid,lease_state text);
         CREATE TABLE runs(id uuid,project_id uuid,task_id uuid,
          lease_id uuid,hook_invocation_id uuid);
         CREATE TABLE run_environment_reservations(run_id uuid,released_at timestamptz);
         CREATE TABLE task_dependencies(project_id uuid,blocked_task_id uuid,
          blocker_task_id uuid,required_blocker_lifecycle text);
         CREATE FUNCTION forge_lease_owns_run(l leases,r runs)
          RETURNS boolean LANGUAGE SQL IMMUTABLE AS 'SELECT l.id=r.lease_id'"
    ))
    .execute(&pool)
    .await?;
    let version = Uuid::now_v7();
    sqlx::query("INSERT INTO pipeline_versions VALUES($1,$2)")
        .bind(version)
        .bind(json!({"stages":{"checks":{"system_action":{"kind":"project_hook"}}}}))
        .execute(&pool)
        .await?;
    for index in 0..128 {
        let project = Uuid::now_v7();
        sqlx::query("INSERT INTO projects VALUES($1,$2)")
            .bind(project)
            .bind(index < 64)
            .execute(&pool)
            .await?;
        if index < 64 {
            sqlx::query("INSERT INTO project_recovery_settings VALUES($1,TRUE)")
                .bind(project)
                .execute(&pool)
                .await?;
        }
        insert_task(&pool, project, version, Uuid::now_v7()).await?;
    }
    let project = ProjectId::new();
    sqlx::query("INSERT INTO projects VALUES($1,TRUE)")
        .bind(project.as_uuid())
        .execute(&pool)
        .await?;
    let blocker = Uuid::now_v7();
    sqlx::query("INSERT INTO tasks VALUES($1,$2,NULL,'waiting',NULL,clock_timestamp())")
        .bind(blocker)
        .bind(project.as_uuid())
        .execute(&pool)
        .await?;
    for _ in 0..64 {
        let task = Uuid::now_v7();
        insert_task(&pool, project.as_uuid(), version, task).await?;
        sqlx::query("INSERT INTO task_dependencies VALUES($1,$2,$3,'done')")
            .bind(project.as_uuid())
            .bind(task)
            .bind(blocker)
            .execute(&pool)
            .await?;
    }
    for index in 0..64 {
        let task = Uuid::now_v7();
        insert_task(&pool, project.as_uuid(), version, task).await?;
        let run = Uuid::now_v7();
        let lease = Uuid::now_v7();
        let hook = (index % 2 == 0).then(Uuid::now_v7);
        sqlx::query("INSERT INTO leases VALUES($1,$2)")
            .bind(lease)
            .bind(if index < 32 { "active" } else { "revoked" })
            .execute(&pool)
            .await?;
        if let Some(hook) = hook {
            sqlx::query("INSERT INTO hook_invocations VALUES($1,$2,'held')")
                .bind(hook)
                .bind(task)
                .execute(&pool)
                .await?;
        }
        sqlx::query("INSERT INTO runs VALUES($1,$2,$3,$4,$5)")
            .bind(run)
            .bind(project.as_uuid())
            .bind(hook.is_none().then_some(task))
            .bind(lease)
            .bind(hook)
            .execute(&pool)
            .await?;
        if index >= 32 {
            sqlx::query("INSERT INTO run_environment_reservations VALUES($1,NULL)")
                .bind(run)
                .execute(&pool)
                .await?;
        }
    }
    let eligible = TaskId::new();
    insert_task(&pool, project.as_uuid(), version, eligible.as_uuid()).await?;
    let store = PostgresStore::from_pool(pool.clone());
    assert_eq!(store.hook_work(project).await?, vec![eligible]);
    assert_eq!(store.hook_projects().await?, vec![project]);
    sqlx::query("ROLLBACK").execute(&pool).await?;
    pool.close().await;
    Ok(())
}

async fn insert_task(pool: &PgPool, project: Uuid, version: Uuid, task: Uuid) -> Result<()> {
    sqlx::query("INSERT INTO tasks VALUES($1,$2,$3,'in_progress','checks',clock_timestamp())")
        .bind(task)
        .bind(project)
        .bind(version)
        .execute(pool)
        .await?;
    Ok(())
}
