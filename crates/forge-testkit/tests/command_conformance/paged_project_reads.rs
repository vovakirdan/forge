//! SQL pages are bounded before decoding Runs and Pipeline definitions.

use anyhow::{Context, Result};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use forge_domain::{ProjectId, TaskId};
use forge_protocol::wire::CommandName;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use super::{
    employees,
    fixture::{Backend, BackendKind, Fixture},
};

async fn get(router: &Router, uri: String) -> Result<(StatusCode, Value)> {
    let response = router
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty())?)
        .await?;
    let status = response.status();
    let body = serde_json::from_slice(&to_bytes(response.into_body(), 256 * 1024).await?)?;
    Ok((status, body))
}

fn graph(label: &str) -> Value {
    json!({"task_kinds":["delivery"],"entry_stage_id":"work",
        "stages":[{"id":"work","name":"Work","executor_kind":"employee","outcomes":["completed"],
            "instructions":label,"workspace":{"kind":"any","access":"read_only"}}],
        "transitions":[{"from_stage_id":"work","outcome":"completed","target":{"kind":"done"}}]})
}

#[tokio::test]
#[ignore = "requires local PostgreSQL; validates bounded Pipeline version pages"]
async fn pipeline_versions_page_more_than_twice_without_n_plus_one() -> Result<()> {
    let fixture = Fixture::create(BackendKind::Postgres).await?;
    let mut initial = graph("Version 1");
    initial["name"] = json!("A versioned catalog");
    let first = fixture.pipeline(initial).await?;
    let pipeline = fixture.snapshot().await?.versions[&first].pipeline_id();
    let mut expected = vec![first.to_string()];
    for version in 2..=13 {
        let receipt = fixture
            .execute(
                CommandName::PublishPipelineVersion,
                json!({
                    "pipeline_id":pipeline,"expected_pipeline_revision":version-1,
                    "definition":graph(&format!("Version {version}"))
                }),
            )
            .await?;
        expected.push(receipt.resource.context("version")?.id);
    }
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!()
    };
    let router = forge_core::router(fixture.core(pool));
    let base = format!("/v1/projects/{}/pipelines", fixture.project_id);
    let mut found = Vec::new();
    let mut cursor = None;
    for page_number in 0..3 {
        let query = cursor.as_ref().map_or("?limit=5".to_owned(), |value| {
            format!("?limit=5&cursor={value}")
        });
        let (status, page) = get(&router, format!("{base}{query}")).await?;
        assert_eq!(status, StatusCode::OK);
        let items = page["items"].as_array().context("items")?;
        assert_eq!(items.len(), if page_number < 2 { 5 } else { 3 });
        found.extend(
            items
                .iter()
                .map(|item| item["id"].as_str().unwrap().to_owned()),
        );
        cursor = page["next_cursor"].as_str().map(str::to_owned);
    }
    assert_eq!(found, expected);
    assert!(cursor.is_none());
    assert_eq!(
        get(&router, format!("{base}?cursor={}", Uuid::now_v7()))
            .await?
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        get(
            &router,
            format!("/v1/projects/{}/pipelines", ProjectId::new())
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL; validates bounded Project Run pages"]
async fn project_runs_page_more_than_twice_in_creation_order() -> Result<()> {
    let fixture = Fixture::create(BackendKind::Postgres).await?;
    let pipeline = fixture
        .pipeline(forge_testkit::m0::single_stage_pipeline())
        .await?;
    let employee = employees::create(&fixture, "Worker").await?;
    fixture
        .execute(CommandName::StartProjectExecution, json!({}))
        .await?;
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!()
    };
    for index in 0..13 {
        let task: TaskId = fixture
            .create_task(pipeline, &format!("Run {index}"))
            .await?;
        fixture
            .task_command(CommandName::ApproveTask, task, json!({}))
            .await?;
        let queue:Uuid=sqlx::query_scalar("UPDATE queue_entries SET queue_state='leased' WHERE project_id=$1 AND task_id=$2 AND queue_state='queued' RETURNING id")
            .bind(fixture.project_id.as_uuid()).bind(task.as_uuid()).fetch_one(pool).await?;
        let lease = Uuid::now_v7();
        let fence:i64=sqlx::query_scalar("INSERT INTO leases(id,project_id,task_id,queue_entry_id,employee_id,expires_at) VALUES($1,$2,$3,$4,$5,clock_timestamp()+interval '1 hour') RETURNING fencing_token")
            .bind(lease).bind(fixture.project_id.as_uuid()).bind(task.as_uuid()).bind(queue).bind(employee.as_uuid()).fetch_one(pool).await?;
        sqlx::query("INSERT INTO runs(id,project_id,task_id,queue_entry_id,lease_id,employee_id,stage_id,attempt_number,lease_fencing_token,run_spec_version,run_spec) VALUES($1,$2,$3,$4,$5,$6,'work',1,$7,1,'{}'::jsonb)")
            .bind(Uuid::now_v7()).bind(fixture.project_id.as_uuid()).bind(task.as_uuid()).bind(queue).bind(lease).bind(employee.as_uuid()).bind(fence).execute(pool).await?;
    }
    let expected = forge_storage::PostgresStore::from_pool(pool.clone())
        .list_runs_for_project(fixture.project_id)
        .await?
        .into_iter()
        .map(|run| run.id.to_string())
        .collect::<Vec<_>>();
    assert_eq!(expected.len(), 13);
    let router = forge_core::router(fixture.core(pool));
    let base = format!("/v1/projects/{}/runs", fixture.project_id);
    let mut found = Vec::new();
    let mut cursor = None;
    for page_number in 0..3 {
        let query = cursor.as_ref().map_or("?limit=5".to_owned(), |value| {
            format!("?limit=5&cursor={value}")
        });
        let (status, page) = get(&router, format!("{base}{query}")).await?;
        assert_eq!(status, StatusCode::OK);
        let items = page["items"].as_array().context("items")?;
        assert_eq!(items.len(), if page_number < 2 { 5 } else { 3 });
        found.extend(
            items
                .iter()
                .map(|item| item["id"].as_str().unwrap().to_owned()),
        );
        cursor = page["next_cursor"].as_str().map(str::to_owned);
    }
    assert_eq!(found, expected);
    assert!(cursor.is_none());
    assert_eq!(
        get(&router, format!("{base}?cursor=invalid")).await?.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        get(&router, format!("{base}?limit=101")).await?.0,
        StatusCode::BAD_REQUEST
    );
    Ok(())
}
