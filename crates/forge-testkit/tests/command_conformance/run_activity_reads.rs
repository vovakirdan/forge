//! Browser Run activity receives coordinates and receipt metadata only.

use anyhow::{Context, Result};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use forge_domain::{
    EvidenceLocation, EvidenceObject, EvidenceObjectInput, EvidenceScope, EvidenceStream,
    ProjectId, StageVisit, Timestamp,
};
use forge_storage::PostgresStore;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use super::{
    active_runs::running_task_with_visit,
    fixture::{Backend, BackendKind},
};

async fn get(router: &Router, uri: String) -> Result<(StatusCode, Value)> {
    let response = router
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty())?)
        .await?;
    let status = response.status();
    let body = serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
    Ok((status, body))
}

#[tokio::test]
#[ignore = "requires local PostgreSQL; validates safe Run coordinates and evidence receipts"]
async fn run_activity_reads_are_scoped_bounded_and_hide_storage_coordinates() -> Result<()> {
    let setup = running_task_with_visit(BackendKind::Postgres, Some(StageVisit::INITIAL)).await?;
    let fixture = &setup.fixture;
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!()
    };
    let store = PostgresStore::from_pool(pool.clone());
    let mut tx = store.begin().await?;
    for sequence in 1..=3 {
        let pending = EvidenceObject::new(EvidenceObjectInput {
            id: Uuid::now_v7(),
            scope: EvidenceScope {
                project_id: fixture.project_id,
                task_id: Some(setup.task),
                run_id: setup.run.id,
            },
            stream: EvidenceStream::Diagnostic,
            sequence,
            sha256: format!("{sequence:064x}"),
            size_bytes: 64,
            redaction_policy_reference: "policy/v1".into(),
            location: EvidenceLocation::PendingUpload,
            created_at: Timestamp::now_utc(),
        })?;
        tx.record_evidence_object(&pending, false).await?;
        tx.record_evidence_object(&pending.stored()?, true).await?;
    }
    tx.commit().await?;
    let router = forge_core::router(fixture.core(pool));
    let base = format!("/v1/projects/{}/runs/{}", fixture.project_id, setup.run.id);
    let (status, context) = get(&router, format!("{base}/context")).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(context["availability"], "available");
    assert_eq!(context["coordinates"]["run_id"], json!(setup.run.id));
    assert_eq!(context["coordinates"]["task_id"], json!(setup.task));
    assert_eq!(context["coordinates"]["stage_visit"], 1);
    for forbidden in [
        "task_spec",
        "knowledge_context",
        "control_instruction",
        "prior_handoff",
        "artifacts",
        "prompt",
        "auth",
    ] {
        assert!(!serde_json::to_string(&context)?.contains(forbidden));
    }
    let (status, first) = get(&router, format!("{base}/evidence?limit=2")).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["items"].as_array().context("first page")?.len(), 2);
    let cursor = first["next_cursor"].as_str().context("cursor")?;
    let (_, second) = get(&router, format!("{base}/evidence?limit=2&cursor={cursor}")).await?;
    assert_eq!(second["items"].as_array().context("second page")?.len(), 1);
    assert!(second["next_cursor"].is_null());
    for item in first["items"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second["items"].as_array().unwrap())
    {
        assert_eq!(item["content_availability"], "unavailable");
        assert_eq!(item["storage_state"], "stored");
        assert!(item["created_at"].as_str().is_some());
        assert!(item.get("object_key").is_none());
    }
    let (_, detail) = get(&router, base.clone()).await?;
    assert_eq!(
        detail["diagnostics"]["evidence"]
            .as_array()
            .context("diagnostic evidence")?
            .len(),
        3
    );
    assert!(!serde_json::to_string(&detail)?.contains("object_key"));
    assert_eq!(
        get(&router, format!("{base}/evidence?limit=51")).await?.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        get(
            &router,
            format!(
                "/v1/projects/{}/runs/{}/context",
                ProjectId::new(),
                setup.run.id
            )
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(
            &router,
            format!(
                "/v1/projects/{}/runs/{}/evidence",
                fixture.project_id,
                Uuid::now_v7()
            )
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    Ok(())
}
