//! HTTP projections preserve Project scope and distinct control facts.

use anyhow::{Context, Result};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use forge_domain::{
    ProjectId,
    runtime::{BootRecoveryPolicy, RecoveryAssessment},
};
use forge_protocol::wire::CommandName;
use forge_storage::PostgresStore;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use super::{
    active_runs::running_task,
    employees,
    fixture::{Backend, BackendKind},
    resolution,
};

async fn get(router: &Router, uri: String) -> Result<(StatusCode, Value)> {
    let response = router
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty())?)
        .await?;
    let status = response.status();
    let body = serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await?)?;
    Ok((status, body))
}

#[tokio::test]
#[ignore = "requires local PostgreSQL; validates resolver catalog and scoped detail"]
async fn resolver_routes_and_escalation_detail_are_scoped() -> Result<()> {
    let (fixture, task) = resolution::setup(BackendKind::Postgres).await?;
    let bob = employees::create(&fixture, "Bob").await?;
    for key in ["engineering", "operations"] {
        fixture
            .execute(
                CommandName::ConfigureResolverRoute,
                json!({
                    "route_key":key,"employee_ids":[bob],"assignment_timeout_seconds":60
                }),
            )
            .await?;
    }
    let id = resolution::raise(&fixture, task, json!({"route_key":"engineering"})).await?;
    let before = resolution::escalation(&fixture, id).await?;
    fixture.execute(CommandName::RerouteEscalation, json!({
        "escalation_id":id,"expected_escalation_revision":before.revision,"reason":"Assign resolver"
    })).await?;
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!()
    };
    let router = forge_core::router(fixture.core(pool));
    let base = format!("/v1/projects/{}", fixture.project_id);
    let (status, page) = get(&router, format!("{base}/resolver-routes?limit=1")).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["items"][0]["key"], "engineering");
    assert_eq!(page["items"][0]["employee_ids"], json!([bob]));
    assert!(page["items"][0]["updated_at"].as_str().is_some());
    let cursor = page["next_cursor"].as_str().context("route cursor")?;
    let (_, next) = get(
        &router,
        format!("{base}/resolver-routes?limit=1&cursor={cursor}"),
    )
    .await?;
    assert_eq!(next["items"][0]["key"], "operations");
    assert!(next["next_cursor"].is_null());
    let (status, detail) = get(&router, format!("{base}/escalations/{id}")).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["route"]["key"], "engineering");
    assert_eq!(detail["route"]["revision"], 1);
    assert_eq!(detail["source"]["task_id"], json!(task));
    assert!(detail["latest_assignment"]["id"].as_str().is_some());
    assert_eq!(
        get(&router, format!("{base}/resolver-routes?limit=51"))
            .await?
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        get(
            &router,
            format!("/v1/projects/{}/escalations/{id}", ProjectId::new())
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&router, format!("{base}/escalations/{}", Uuid::now_v7()))
            .await?
            .0,
        StatusCode::NOT_FOUND
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL; validates next-Run history and physical recovery facts"]
async fn next_run_and_recovery_reads_keep_history_and_physical_hold_distinct() -> Result<()> {
    let setup = running_task(BackendKind::Postgres).await?;
    let fixture = &setup.fixture;
    let next = employees::create(fixture, "Next worker").await?;
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!()
    };
    let router = forge_core::router(fixture.core(pool));
    let base = format!("/v1/projects/{}", fixture.project_id);
    let (_, defaults) = get(&router, format!("{base}/recovery")).await?;
    assert_eq!(
        defaults,
        json!({"boot_policy":"recover_safe_then_hold","hold":false})
    );
    fixture
        .task_command(
            CommandName::SetNextRunEmployee,
            setup.task,
            json!({"employee_id":next,"reason":"Use next worker"}),
        )
        .await?;
    let (_, active) = get(&router, format!("{base}/next-run-constraints")).await?;
    assert_eq!(active["items"][0]["state"]["status"], "pending");
    assert_eq!(active["items"][0]["employee_id"], json!(next));
    fixture
        .task_command(
            CommandName::ClearNextRunEmployee,
            setup.task,
            json!({"reason":"Back to regular admission"}),
        )
        .await?;
    let (_, history) = get(&router, format!("{base}/next-run-constraints")).await?;
    assert_eq!(history["items"][0]["state"]["status"], "cancelled");
    assert_eq!(history["items"][0]["state"]["reason"], "cleared");
    let store = PostgresStore::from_pool(pool.clone());
    let mut settings_tx = store.begin().await?;
    settings_tx
        .configure_boot_recovery_policy(fixture.project_id, BootRecoveryPolicy::ManualHold)
        .await?;
    settings_tx.commit().await?;
    let (_, settings) = get(&router, format!("{base}/recovery")).await?;
    assert_eq!(settings, json!({"boot_policy":"manual_hold","hold":false}));
    let (_, runs) = get(&router, format!("{base}/recovery-runs")).await?;
    assert_eq!(runs["items"][0]["run_id"], json!(setup.run.id));
    assert!(runs["items"][0]["created_at"].as_str().is_some());
    assert_eq!(runs["items"][0]["lease_active"], true);
    assert_eq!(runs["items"][0]["reservation_released"], false);
    assert!(runs["items"][0]["accepted_assessment"].is_null());
    let readiness_url = format!("{base}/recovery-runs/{}/assessment-readiness", setup.run.id);
    let (_, active_readiness) = get(&router, readiness_url.clone()).await?;
    assert_eq!(active_readiness["assessment"], "not_started_confirmed");
    assert_eq!(active_readiness["eligible"], false);
    assert_eq!(active_readiness["reason_code"], "execution_not_retired");
    assert_eq!(active_readiness["run_id"], json!(setup.run.id));
    assert_eq!(
        active_readiness["lease_fencing_token"],
        json!(setup.run.lease_fencing_token)
    );
    assert_eq!(
        active_readiness["environment_epoch"],
        json!(setup.run.environment_epoch)
    );
    assert!(active_readiness["project_revision"].as_u64().is_some());
    assert!(active_readiness["task_revision"].as_u64().is_some());
    let mut tx = store.begin().await?;
    tx.quarantine_run(setup.run.id, "test_physical_hold", false)
        .await?;
    tx.commit().await?;
    let (_, unresolved) = get(&router, readiness_url.clone()).await?;
    assert_eq!(unresolved["eligible"], false);
    assert_eq!(unresolved["reason_code"], "physical_state_unresolved");
    let mut tx = store.begin().await?;
    tx.accept_recovery_assessment(
        setup.run.id,
        forge_domain::CommandId::new(),
        RecoveryAssessment::Unknown,
        None,
    )
    .await?;
    tx.commit().await?;
    let (_, held) = get(&router, format!("{base}/recovery-runs?limit=1")).await?;
    assert_eq!(held["items"][0]["lease_active"], false);
    assert_eq!(held["items"][0]["reservation_state"], "unknown");
    assert_eq!(held["items"][0]["reservation_released"], false);
    assert_eq!(held["items"][0]["accepted_assessment"], "unknown");
    let (_, assessed_readiness) = get(&router, readiness_url).await?;
    assert_eq!(assessed_readiness["eligible"], false);
    assert_eq!(assessed_readiness["reason_code"], "already_assessed");
    assert_eq!(
        get(&router, format!("{base}/recovery-runs?limit=51"))
            .await?
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        get(
            &router,
            format!("/v1/projects/{}/recovery", ProjectId::new())
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL; validates safe non-start assessment preview"]
async fn recovery_assessment_readiness_requires_quiescence_and_current_task() -> Result<()> {
    let setup = running_task(BackendKind::Postgres).await?;
    let fixture = &setup.fixture;
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!()
    };
    let router = forge_core::router(fixture.core(pool));
    let url = format!(
        "/v1/projects/{}/recovery-runs/{}/assessment-readiness",
        fixture.project_id, setup.run.id
    );
    let store = PostgresStore::from_pool(pool.clone());
    let mut tx = store.begin().await?;
    tx.quarantine_run(setup.run.id, "test_quiescent", true)
        .await?;
    tx.configure_boot_recovery_policy(fixture.project_id, BootRecoveryPolicy::ManualHold)
        .await?;
    tx.commit().await?;
    let facts = store
        .recovery_assessment_read(fixture.project_id, setup.run.id)
        .await?
        .context("recovery facts")?;
    assert_eq!(facts.task_id, Some(setup.task.as_uuid()));
    let (status, ready) = get(&router, url.clone()).await?;
    assert_eq!(status, StatusCode::OK, "{ready}");
    assert_eq!(ready["eligible"], true);
    assert!(ready["reason_code"].is_null());
    assert_eq!(ready["task_id"], json!(setup.task));
    assert!(ready["run_revision"].as_u64().is_some());
    assert_eq!(
        get(
            &router,
            format!(
                "/v1/projects/{}/recovery-runs/{}/assessment-readiness",
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
                "/v1/projects/{}/recovery-runs/{}/assessment-readiness",
                fixture.project_id,
                Uuid::now_v7()
            )
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    let mut tx = store.begin().await?;
    tx.configure_boot_recovery_policy(fixture.project_id, BootRecoveryPolicy::RecoverSafeThenHold)
        .await?;
    tx.commit().await?;
    let (_, missing_wait) = get(&router, url).await?;
    assert_eq!(missing_wait["eligible"], false);
    assert_eq!(missing_wait["reason_code"], "recovery_wait_missing");
    Ok(())
}
