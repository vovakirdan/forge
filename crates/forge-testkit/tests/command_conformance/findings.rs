//! Report/triage/promotion through the same engine on reference and PostgreSQL.
use super::{
    atomicity::assert_faults,
    fixture::{Backend, BackendKind, Fixture},
};
use anyhow::{Context, Result};
use forge_application::CommandTransaction;
use forge_domain::{Actor, ActorId, LifecycleStatus, TaskId, TaskSource, finding::Finding};
use forge_protocol::wire::{CommandName, CommandStatus};
use forge_storage::PostgresStore;
use forge_testkit::m0::single_stage_pipeline;
use serde_json::{Value, json};
use uuid::Uuid;

pub async fn report_promote(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let pipeline = fixture.pipeline(single_stage_pipeline()).await?;
    let source = fixture.create_task(pipeline, "Source work").await?;
    let report=fixture.envelope(CommandName::ReportFinding,fixture.snapshot().await?.projects[&fixture.project_id].revision(),json!({"source_task_id":source,"description":"A separate defect was noticed","severity":"owner-defined"}),"report-finding");
    assert_faults(&fixture, &report).await?;
    let receipt = fixture.execute_as(&report, &fixture.context).await?;
    let id: Uuid = receipt.resource.context("Finding")?.id.parse()?;
    assert_eq!(
        fixture.execute_as(&report, &fixture.context).await?.status,
        CommandStatus::Replayed
    );
    assert_eq!(fixture.snapshot().await?.tasks.len(), 1);
    assert_eq!(fixture.task(source).await?.revision().get(), 1);
    let finding = load(&fixture, id).await?;
    assert_eq!(finding.state.key(), "open");
    assert_eq!(finding.reported_by, fixture.context.actor);
    assert!(finding.source_run.is_none());
    let promote=fixture.envelope(CommandName::PromoteFinding,fixture.snapshot().await?.projects[&fixture.project_id].revision(),json!({"finding_id":id,"expected_finding_revision":1,"reason":"Make this separate work", "task":{"title":"Explicitly promoted","description":"Owner chosen scope","pipeline_version_id":pipeline,"priority":"normal","kind":"delivery"}}),"promote-finding");
    let mut employee = fixture.context.clone();
    employee.actor = Actor::employee(ActorId::new());
    employee.capabilities = vec![CommandName::ReportFinding];
    fixture.assert_unchanged_after(&promote, &employee).await?;
    assert_faults(&fixture, &promote).await?;
    let receipt = fixture.execute_as(&promote, &fixture.context).await?;
    let task: TaskId = receipt.resource.context("promoted Task")?.id.parse()?;
    assert_eq!(
        fixture.execute_as(&promote, &fixture.context).await?.status,
        CommandStatus::Replayed
    );
    assert_eq!(fixture.snapshot().await?.tasks.len(), 2);
    let promoted = fixture.task(task).await?;
    assert_eq!(promoted.lifecycle(), LifecycleStatus::Draft);
    assert_eq!(promoted.source(), TaskSource::PromotedFinding);
    assert_eq!(load(&fixture, id).await?.state.target_task(), Some(task));
    let duplicate = fixture.envelope(
        CommandName::PromoteFinding,
        fixture.snapshot().await?.projects[&fixture.project_id].revision(),
        promote.canonical_payload().clone(),
        "new-key-duplicate-promotion",
    );
    fixture
        .assert_unchanged_after(&duplicate, &fixture.context)
        .await?;
    fixture.snapshot().await?.assert_audit_atomic();
    Ok(())
}
pub async fn triage_refusals(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let pipeline = fixture.pipeline(single_stage_pipeline()).await?;
    let task = fixture.create_task(pipeline, "Observed work").await?;
    let id: Uuid = fixture
        .execute(
            CommandName::ReportFinding,
            json!({"source_task_id":task,"description":"Observation","severity":"low"}),
        )
        .await?
        .resource
        .context("Finding")?
        .id
        .parse()?;
    for payload in [
        json!({"finding_id":id,"expected_finding_revision":2,"reason":"stale","decision":{"kind":"ignore"}}),
        json!({"finding_id":id,"expected_finding_revision":1,"reason":"wrong scope","decision":{"kind":"attach","task_id":Uuid::now_v7()}}),
    ] {
        let command = fixture.envelope(
            CommandName::TriageFinding,
            fixture.snapshot().await?.projects[&fixture.project_id].revision(),
            payload,
            "bad-triage",
        );
        fixture
            .assert_unchanged_after(&command, &fixture.context)
            .await?;
    }
    let input = |revision, decision: Value| json!({"finding_id":id,"expected_finding_revision":revision,"reason":"Human triage","decision":decision});
    fixture
        .execute(
            CommandName::TriageFinding,
            input(1, json!({"kind":"attach","task_id":task})),
        )
        .await?;
    assert_eq!(load(&fixture, id).await?.state.key(), "attached");
    assert_immutable(&fixture, id).await?;
    fixture
        .execute(
            CommandName::TriageFinding,
            input(2, json!({"kind":"ignore"})),
        )
        .await?;
    assert_eq!(load(&fixture, id).await?.state.key(), "ignored");
    let command = fixture.envelope(
        CommandName::TriageFinding,
        fixture.snapshot().await?.projects[&fixture.project_id].revision(),
        input(3, json!({"kind":"attach","task_id":task})),
        "terminal-triage",
    );
    fixture
        .assert_unchanged_after(&command, &fixture.context)
        .await?;
    assert_eq!(fixture.snapshot().await?.tasks.len(), 1);
    Ok(())
}
async fn assert_immutable(fixture: &Fixture, id: Uuid) -> Result<()> {
    let finding = load(fixture, id).await?;
    let mut forged = finding.clone();
    forged.triage(
        finding.revision,
        forge_domain::finding::FindingState::Ignored {
            reason: "Valid triage but invalid report change".into(),
            by: fixture.context.actor,
            at: finding.reported_at,
        },
    )?;
    forged.description = "Rewrite immutable report".into();
    match &fixture.backend {
        Backend::Memory(store) => {
            let mut tx = store.begin().await;
            assert!(tx.update_finding(&forged, finding.revision).await.is_err());
        }
        Backend::Postgres(pool) => {
            let store = PostgresStore::from_pool(pool.clone());
            let mut tx = store.begin().await?;
            assert!(tx.update_finding(&forged, finding.revision).await.is_err());
        }
    }
    assert_eq!(load(fixture, id).await?, finding);
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit PostgreSQL; bounded local HTTP contract"]
async fn findings_http_reads_preserve_triage_and_scope() -> Result<()> {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let fixture = Fixture::create(BackendKind::Postgres).await?;
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!()
    };
    let pipeline = fixture.pipeline(single_stage_pipeline()).await?;
    let task = fixture.create_task(pipeline, "Finding read source").await?;
    let mut ids = Vec::new();
    for description in ["First observation", "Second observation"] {
        ids.push(
            fixture
                .execute(
                    CommandName::ReportFinding,
                    json!({"source_task_id":task,"description":description,"severity":"normal"}),
                )
                .await?
                .resource
                .context("Finding")?
                .id,
        );
    }
    let before = fixture.snapshot().await?.raw;
    let router = forge_core::router(fixture.core(pool));
    let get = |path: String| {
        let router = router.clone();
        async move {
            let response = router
                .oneshot(Request::builder().uri(path).body(Body::empty())?)
                .await?;
            let status = response.status();
            let value: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
            Ok::<_, anyhow::Error>((status, value))
        }
    };
    let path = format!("/v1/projects/{}/findings", fixture.project_id);
    let (status, first) = get(format!("{path}?limit=1&task_id={task}")).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["items"][0]["id"], ids[0]);
    assert_eq!(first["next_cursor"], ids[0]);
    assert!(
        first["items"][0]["reported_at"]
            .as_str()
            .is_some_and(|time| time.contains('T'))
    );
    let (_, second) = get(format!("{path}?limit=1&after={}", ids[0])).await?;
    assert_eq!(second["items"][0]["id"], ids[1]);
    assert!(second["next_cursor"].is_null());
    for suffix in [
        "limit=0",
        "limit=101",
        "after=invalid",
        "task_id=invalid",
        "unknown=true",
    ] {
        assert_eq!(
            get(format!("{path}?{suffix}")).await?.0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        get(format!("/v1/projects/{}/findings", Uuid::now_v7()))
            .await?
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(format!("{path}?task_id={}", Uuid::now_v7())).await?.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(fixture.snapshot().await?.raw, before);
    Ok(())
}
async fn load(fixture: &Fixture, id: Uuid) -> Result<Finding> {
    match &fixture.backend {
        Backend::Memory(store) => {
            let mut tx = store.begin().await;
            Ok(tx.lock_finding(id).await?.context("Finding")?)
        }
        Backend::Postgres(pool) => {
            let store = PostgresStore::from_pool(pool.clone());
            let mut tx = store.begin().await?;
            Ok(tx.lock_finding(id).await?.context("Finding")?)
        }
    }
}
