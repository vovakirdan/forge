//! Dependency reads retain canonical command scope and leave revisions/events untouched.

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

use super::{
    commands::{complete_human, edge, human_evidence_pipeline},
    fixture::{Backend, BackendKind, Fixture},
};

async fn get(router: &Router, path: &str) -> Result<(StatusCode, Value)> {
    let response = router
        .clone()
        .oneshot(Request::builder().uri(path).body(Body::empty())?)
        .await?;
    let status = response.status();
    Ok((
        status,
        serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await?)?,
    ))
}

#[tokio::test]
#[ignore = "requires explicit local PostgreSQL; coherent dependency read contract"]
async fn dependency_pages_are_bidirectional_scoped_coherent_and_read_only() -> Result<()> {
    let fixture = Fixture::create(BackendKind::Postgres).await?;
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!()
    };
    let version = fixture.pipeline(human_evidence_pipeline()).await?;
    let root = fixture.create_task(version, "Root").await?;
    let empty = fixture.create_task(version, "Empty").await?;
    let mut blockers = Vec::new();
    for index in 0..23 {
        let task = fixture
            .create_task(version, &format!("Prerequisite {index}"))
            .await?;
        fixture
            .execute(CommandName::CreateDependency, edge(task, root))
            .await?;
        blockers.push(task);
    }
    fixture
        .task_command(CommandName::ApproveTask, blockers[1], json!({}))
        .await?;
    complete_human(&fixture, blockers[1], "accepted").await?;
    fixture
        .task_command(
            CommandName::CancelTask,
            blockers[2],
            json!({"cancellation_reason_key":"unspecified"}),
        )
        .await?;
    let before = fixture.snapshot().await?;
    let router = forge_core::router(fixture.core(pool));
    let prefix = format!("/v1/projects/{}/tasks", fixture.project_id);
    let base = format!("{prefix}/{root}/dependencies/blocked_by");
    let (status, first) = get(&router, &base).await?;
    assert_eq!(status, StatusCode::OK);
    let items = first["items"].as_array().context("items")?;
    assert_eq!(items.len(), 20);
    assert_eq!(items[0]["condition_state"], "pending");
    assert_eq!(items[1]["condition_state"], "satisfied");
    assert_eq!(items[2]["condition_state"], "blocker_cancelled");
    for (index, item) in items.iter().enumerate() {
        assert_eq!(item["blocker_task_id"], blockers[index].to_string());
        assert_eq!(item["blocked_task_id"], root.to_string());
        assert_eq!(item["required_condition"], "task_done");
        assert_eq!(item["related_task"]["id"], blockers[index].to_string());
        assert_eq!(
            item["related_task"]["title"],
            format!("Prerequisite {index}")
        );
    }
    let cursor = first["next_cursor"].as_str().context("cursor")?;
    let (_, second) = get(&router, &format!("{base}?cursor={cursor}")).await?;
    assert_eq!(second["items"].as_array().context("second")?.len(), 3);
    assert!(second.get("next_cursor").is_none());
    for (index, state) in [(0, "pending"), (1, "satisfied"), (2, "blocker_cancelled")] {
        let (_, outbound) = get(
            &router,
            &format!("{prefix}/{}/dependencies/blocks", blockers[index]),
        )
        .await?;
        assert_eq!(outbound["items"][0]["condition_state"], state);
        assert_eq!(outbound["items"][0]["related_task"]["id"], root.to_string());
        // Outbound condition follows the blocker, not the related draft Task.
        assert_eq!(outbound["items"][0]["related_task"]["lifecycle"], "draft");
    }
    for direction in ["blocks", "blocked_by"] {
        assert_eq!(
            get(
                &router,
                &format!("{prefix}/{empty}/dependencies/{direction}")
            )
            .await?
            .1,
            json!({"items":[]})
        );
    }
    for query in [
        "limit=0",
        "limit=101",
        "limit=1&limit=2",
        "after=x",
        "cursor=",
    ] {
        let (status, _) = get(&router, &format!("{base}?{query}")).await?;
        assert!(status == StatusCode::BAD_REQUEST || status == StatusCode::CONFLICT);
    }
    for path in [
        format!("{prefix}/{empty}/dependencies/blocked_by?cursor={cursor}"),
        format!("{prefix}/{root}/dependencies/blocks?cursor={cursor}"),
    ] {
        let (status, body) = get(&router, &path).await?;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["error"]["code"], "cursor_invalid");
    }
    for path in [
        format!("{prefix}/{}/dependencies/blocks", TaskId::new()),
        format!(
            "/v1/projects/{}/tasks/{root}/dependencies/blocks",
            ProjectId::new()
        ),
    ] {
        assert_eq!(get(&router, &path).await?.0, StatusCode::NOT_FOUND);
    }
    assert_eq!(before.raw, fixture.snapshot().await?.raw);
    fixture
        .execute(
            CommandName::RemoveDependency,
            json!({"blocker_task_id":blockers[19],"blocked_task_id":root}),
        )
        .await?;
    let after_removal = fixture.snapshot().await?;
    let (status, body) = get(&router, &format!("{base}?cursor={cursor}")).await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "cursor_invalid");
    assert_eq!(after_removal.raw, fixture.snapshot().await?.raw);
    Ok(())
}
