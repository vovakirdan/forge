//! Real private-schema persistence checks, not a hook execution acceptance claim.
use anyhow::{Context, Result};
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use forge_application::CommandEnvelope;
use forge_domain::{ProjectHookVersion, ProjectId};
use forge_protocol::wire::{CommandName, CommandRequest, CommandStatus};
use forge_testkit::m0::M0Harness;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires explicitly configured local PostgreSQL and NATS"]
async fn project_hook_versions_are_immutable_scoped_and_never_follow_same_name() -> Result<()> {
    let harness = M0Harness::start().await?;
    let project = harness.create_project("Hook registry").await?;
    let foreign = harness.create_project("Other project").await?;
    let payload = json!({"name":"Owner configured check","image":format!("fixture@sha256:{}","a".repeat(64)),"command":["/usr/bin/true"],"workdir":".","limits":{"cpu_millis":1000,"memory_bytes":67108864,"pids":32,"wall_seconds":60,"stop_grace_seconds":2},"max_output_bytes":4096,"applicable_task_kinds":["delivery"],"required":true});
    let envelope = CommandEnvelope::parse(
        CommandName::ConfigureProjectHook,
        CommandRequest {
            project_id: project.to_string(),
            expected_revision: 1,
            payload: payload.as_object().unwrap().clone(),
        },
        "configure-hook-version",
    )?;
    let first_receipt = harness.core.execute_command(envelope.clone()).await?;
    let replay = harness.core.execute_command(envelope).await?;
    assert_eq!(replay.status, CommandStatus::Replayed);
    assert_eq!(replay.resource, first_receipt.resource);
    assert_eq!(replay.event_ids, first_receipt.event_ids);
    let first_id: Uuid = first_receipt
        .resource
        .context("hook version resource")?
        .id
        .parse()?;
    let first = harness
        .core
        .read_project_hook_versions(project, None, 100)
        .await?
        .remove(0);
    assert_eq!(first.id, first_id);
    assert_eq!(first.project_id, project);
    let mut second = payload.clone();
    second["command"] = json!(["/usr/bin/false"]);
    let second = harness
        .execute(project, CommandName::ConfigureProjectHook, second)
        .await?
        .resource
        .context("new hook version")?
        .id;
    assert_ne!(second, first.id.to_string());
    let mut tx = harness.store.begin().await?;
    assert_eq!(
        tx.load_project_hook_version(project, first.id).await?,
        Some(first.clone())
    );
    assert!(
        tx.load_project_hook_version(foreign, first.id)
            .await?
            .is_none()
    );
    assert!(tx.insert_project_hook_version(&first).await.is_err());
    tx.commit().await?;
    assert_eq!(
        harness
            .store
            .list_project_hook_versions(project)
            .await?
            .len(),
        2
    );
    http_pages(&harness, project, foreign, &first).await?;
    let before = get(&harness, format!("/v1/projects/{project}")).await?.1["revision"].clone();
    let mut bad_config = payload;
    bad_config["workdir"] = json!("../other-task");
    assert!(
        harness
            .execute(project, CommandName::ConfigureProjectHook, bad_config)
            .await
            .is_err()
    );
    assert_eq!(
        get(&harness, format!("/v1/projects/{project}")).await?.1["revision"],
        before
    );
    assert_eq!(
        harness
            .core
            .read_project_hook_versions(project, None, 100)
            .await?
            .len(),
        2
    );
    assert!(
        sqlx::query("UPDATE project_hook_versions SET name='changed' WHERE id=$1")
            .bind(first.id)
            .execute(&harness.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM project_hook_versions WHERE id=$1")
            .bind(first.id)
            .execute(&harness.pool)
            .await
            .is_err()
    );
    let bad=sqlx::query("INSERT INTO project_hook_versions(id,project_id,name,canonical_snapshot,created_at) VALUES($1,$2,'bad','{}'::jsonb,clock_timestamp())")
        .bind(Uuid::now_v7()).bind(project.as_uuid()).execute(&harness.pool).await;
    assert!(bad.is_err());
    assert_eq!(
        harness
            .store
            .list_project_hook_versions(project)
            .await?
            .len(),
        2
    );
    harness.shutdown().await;
    Ok(())
}

async fn get(harness: &M0Harness, path: String) -> Result<(u16, Value)> {
    let response = forge_core::router(harness.core.clone())
        .oneshot(Request::builder().uri(path).body(Body::empty())?)
        .await?;
    let status = response.status().as_u16();
    let body = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((status, serde_json::from_slice(&body)?))
}
async fn http_pages(
    harness: &M0Harness,
    project: ProjectId,
    foreign: ProjectId,
    first: &ProjectHookVersion,
) -> Result<()> {
    let (status, page) = get(
        harness,
        format!("/v1/projects/{project}/hook-versions?limit=1"),
    )
    .await?;
    assert_eq!(status, 200);
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["items"][0]["id"], first.id.to_string());
    assert!(
        page["items"][0]["created_at"]
            .as_str()
            .unwrap()
            .contains('T')
    );
    let cursor = page["next_cursor"].as_str().context("next page")?;
    let (_, page) = get(
        harness,
        format!("/v1/projects/{project}/hook-versions?after={cursor}&limit=1"),
    )
    .await?;
    assert!(page["next_cursor"].is_null());
    assert_ne!(page["items"][0]["id"], first.id.to_string());
    let (_, page) = get(harness, format!("/v1/projects/{foreign}/hook-versions")).await?;
    assert!(page["items"].as_array().unwrap().is_empty());
    for suffix in ["?limit=0", "?limit=101", "?after=bad"] {
        assert_eq!(
            get(
                harness,
                format!("/v1/projects/{project}/hook-versions{suffix}")
            )
            .await?
            .0,
            400
        );
    }
    assert_eq!(
        get(
            harness,
            format!("/v1/projects/{}/hook-versions", Uuid::now_v7())
        )
        .await?
        .0,
        404
    );
    Ok(())
}
