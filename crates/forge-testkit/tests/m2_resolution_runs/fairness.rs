//! PostgreSQL query proof using rollback-only queue projections, not synthetic domain history.
use anyhow::Result;
use forge_domain::ProjectId;
use forge_storage::PostgresStore;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires local PostgreSQL; rollback-only private projection schema"]
async fn retained_resolvers_cannot_starve_question_after_bounded_page() -> Result<()> {
    anyhow::ensure!(
        std::env::var("FORGE_INTEGRATION").as_deref() == Ok("1"),
        "explicit integration opt-in required"
    );
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&std::env::var("FORGE_DATABASE_URL")?)
        .await?;
    let schema = format!("resolver_scan_{}", Uuid::now_v7().simple());
    sqlx::raw_sql(&format!("BEGIN; CREATE SCHEMA {schema}; SET LOCAL search_path TO {schema};
        CREATE TABLE escalations(id uuid,project_id uuid,escalation_state text);
        CREATE TABLE resolution_assignments(id uuid,escalation_id uuid);
        CREATE TABLE leases(id uuid,lease_state text);
        CREATE TABLE runs(id uuid,lease_id uuid,resolution_assignment_id uuid);
        CREATE TABLE run_environment_reservations(run_id uuid,released_at timestamptz);
        CREATE FUNCTION forge_lease_owns_run(l leases,r runs) RETURNS boolean LANGUAGE SQL IMMUTABLE AS 'SELECT l.id=r.lease_id'"))
        .execute(&pool).await?;
    let project = ProjectId::new();
    let mut blocked = Vec::new();
    let mut held_runs = Vec::new();
    for index in 0..256 {
        let id = Uuid::now_v7();
        let assignment = Uuid::now_v7();
        let run = Uuid::now_v7();
        let lease = Uuid::now_v7();
        blocked.push(id);
        held_runs.push(run);
        sqlx::query("INSERT INTO escalations VALUES($1,$2,'queued')")
            .bind(id)
            .bind(project.as_uuid())
            .execute(&pool)
            .await?;
        sqlx::query("INSERT INTO resolution_assignments VALUES($1,$2)")
            .bind(assignment)
            .bind(id)
            .execute(&pool)
            .await?;
        sqlx::query("INSERT INTO runs VALUES($1,$2,$3)")
            .bind(run)
            .bind(lease)
            .bind(assignment)
            .execute(&pool)
            .await?;
        sqlx::query("INSERT INTO leases VALUES($1,$2)")
            .bind(lease)
            .bind(if index % 2 == 0 { "active" } else { "revoked" })
            .execute(&pool)
            .await?;
        if index % 2 == 1 {
            sqlx::query("INSERT INTO run_environment_reservations VALUES($1,NULL)")
                .bind(run)
                .execute(&pool)
                .await?;
        }
    }
    let eligible = Uuid::now_v7();
    sqlx::query("INSERT INTO escalations VALUES($1,$2,'queued')")
        .bind(eligible)
        .bind(project.as_uuid())
        .execute(&pool)
        .await?;
    let store = PostgresStore::from_pool(pool.clone());
    assert_eq!(
        store.resolution_queue_ids(Some(project), None).await?,
        vec![eligible]
    );
    let watched = store.resolution_queue_ids(None, None).await?;
    assert_eq!(watched.len(), 256);
    assert_eq!(
        watched, blocked,
        "watchdog must still reconcile retained scopes"
    );
    let next = store
        .resolution_queue_ids(None, watched.last().copied())
        .await?;
    assert_eq!(next[0], eligible, "global rotation remains intact");
    // Logical revoke alone stays blocked; positive physical release makes this
    // exact old question eligible, without changing any other queued question.
    sqlx::query(
        "UPDATE run_environment_reservations SET released_at=clock_timestamp() WHERE run_id=$1",
    )
    .bind(held_runs[1])
    .execute(&pool)
    .await?;
    assert_eq!(
        store.resolution_queue_ids(Some(project), None).await?,
        vec![blocked[1], eligible]
    );
    sqlx::query("ROLLBACK").execute(&pool).await?;
    pool.close().await;
    Ok(())
}
