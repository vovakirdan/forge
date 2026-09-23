//! Real isolated PostgreSQL and fake HTTP prove the disposable-index boundary.
//! No model inference, personal service, or provider credential is used.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU16, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result};
use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use forge_core::{MemorySearchDegradation, MemorySearchDocument, MemorySearchMode};
use forge_domain::{
    EmployeeId, ProjectId, Timestamp,
    knowledge::{DerivedMemoryEntry, DerivedMemoryKind, KnowledgeSourceRef},
};
use forge_provider_common::SecretBytes;
use forge_testkit::m0::M0Harness;
use serde_json::{Value, json};
use tokio::{
    net::TcpListener,
    sync::{Mutex, Notify},
    task::JoinHandle,
};
use uuid::Uuid;

struct FakeIndex {
    endpoint: String,
    state: Arc<IndexState>,
    task: JoinHandle<()>,
}
struct IndexState {
    status: AtomicU16,
    imports: Mutex<Vec<Value>>,
    forgets: Mutex<Vec<Uuid>>,
    results: Mutex<Vec<Value>>,
    gate: AtomicBool,
    entered: Notify,
    release: Notify,
}
impl Drop for FakeIndex {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl FakeIndex {
    async fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = format!("http://{}", listener.local_addr()?);
        let state = Arc::new(IndexState {
            status: AtomicU16::new(200),
            imports: Mutex::default(),
            forgets: Mutex::default(),
            results: Mutex::default(),
            gate: AtomicBool::new(false),
            entered: Notify::new(),
            release: Notify::new(),
        });
        let app = Router::new()
            .route("/agentmemory/import", post(import))
            .route("/agentmemory/forget", post(forget))
            .route("/agentmemory/search", post(search))
            .with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("fake HTTP server");
        });
        Ok(Self {
            endpoint,
            state,
            task,
        })
    }
    async fn harness(&self) -> Result<M0Harness> {
        M0Harness::start_configured(|core| {
            Ok(core.with_agentmemory(
                &self.endpoint,
                SecretBytes::new(b"synthetic-index-token".to_vec()),
                Duration::from_secs(5),
            )?)
        })
        .await
    }
    async fn hits(&self, ids: &[Uuid]) {
        *self.state.results.lock().await=ids.iter().enumerate().map(|(i,id)|json!({"obsId":id,"score":100.0-i as f64,"title":"FORGED TITLE","content":"FORGED UNTRUSTED INDEX CONTENT"})).collect();
    }
}
async fn import(
    State(state): State<Arc<IndexState>>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    state.imports.lock().await.push(body);
    if state.gate.swap(false, Ordering::SeqCst) {
        state.entered.notify_one();
        state.release.notified().await;
    }
    let status = StatusCode::from_u16(state.status.load(Ordering::SeqCst)).unwrap();
    (
        status,
        Json(
            json!({"success":status.is_success(),"storage":"complete","indexing":{"version":1,"mode":"strict","expected":1,"bm25Indexed":1,"vectorIndexed":1,"persisted":true}}),
        ),
    )
}
async fn forget(State(state): State<Arc<IndexState>>, Json(body): Json<Value>) -> Json<Value> {
    state
        .forgets
        .lock()
        .await
        .push(serde_json::from_value(body["memoryId"].clone()).unwrap());
    Json(json!({"success":true,"deleted":0}))
}
async fn search(State(state): State<Arc<IndexState>>) -> (StatusCode, Json<Value>) {
    let status = StatusCode::from_u16(state.status.load(Ordering::SeqCst)).unwrap();
    (
        status,
        Json(json!({"format":"compact","results":state.results.lock().await.clone()})),
    )
}
async fn invoke(h: &M0Harness, project: ProjectId, name: &str, payload: Value) -> Result<()> {
    h.execute(project, serde_json::from_value(json!(name))?, payload)
        .await?;
    Ok(())
}
async fn publish_page(h: &M0Harness, project: ProjectId) -> Result<Uuid> {
    let id = Uuid::now_v7();
    invoke(h,project,"author_knowledge_page",json!({"page_id":id,"expected_page_revision":0,"kind":"policy","title":"Review policy","markdown":"hello canonical policy","source_refs":[]})).await?;
    invoke(
        h,
        project,
        "publish_knowledge_page",
        json!({"page_id":id,"expected_page_revision":1}),
    )
    .await?;
    Ok(id)
}
async fn eligible(h: &M0Harness) -> Result<()> {
    // Test-only retry clock override affects this harness's projection metadata only.
    sqlx::query("UPDATE knowledge_projection_operations SET not_before=clock_timestamp()")
        .execute(&h.pool)
        .await?;
    Ok(())
}
async fn projection(
    h: &M0Harness,
    project: ProjectId,
    object: Uuid,
    revision: u64,
) -> Result<Uuid> {
    Ok(sqlx::query_scalar("SELECT id FROM knowledge_projection_operations WHERE project_id=$1 AND object_id=$2 AND revision=$3")
        .bind(project.as_uuid()).bind(object).bind(i64::try_from(revision)?).fetch_one(&h.pool).await?)
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; fake AgentMemory HTTP, no inference"]
async fn strict_ack_retry_and_rebuild_preserve_identity_and_canonical_content() -> Result<()> {
    let index = FakeIndex::start().await?;
    let h = index.harness().await?;
    let project = h.create_project("projection retry").await?;
    let id = publish_page(&h, project).await?;
    let operation = projection(&h, project, id, 2).await?;
    index.state.status.store(503, Ordering::SeqCst);
    let report = h.core.drain_memory_projections().await?;
    assert_eq!((report.indexed, report.retired, report.deferred), (0, 1, 1));
    assert!(
        !h.store
            .load_knowledge_projection(project, operation)
            .await?
            .context("operation")?
            .index_ready
    );
    assert!(
        h.store
            .pending_knowledge_projections(project, 100)
            .await?
            .is_empty(),
        "retry cooldown"
    );
    index.state.status.store(429, Ordering::SeqCst);
    eligible(&h).await?;
    assert_eq!(h.core.drain_memory_projections().await?.deferred, 1);
    index.state.status.store(200, Ordering::SeqCst);
    eligible(&h).await?;
    assert_eq!(h.core.drain_memory_projections().await?.indexed, 1);
    let imports = index.state.imports.lock().await.clone();
    assert_eq!(imports.len(), 3);
    assert!(
        imports.windows(2).all(|pair| pair[0] == pair[1]),
        "exact immutable retry envelope"
    );
    assert_eq!(
        imports[0]["exportData"]["memories"][0]["id"],
        json!(operation)
    );
    index.hits(&[operation]).await;
    let found = h.core.query_search(project, None, "hello", 10).await?;
    assert_eq!(found.mode, MemorySearchMode::Indexed);
    assert_eq!(found.results.len(), 1);
    let MemorySearchDocument::KnowledgePage(record) = &found.results[0].document else {
        panic!("page")
    };
    assert_eq!(record.content.markdown, "hello canonical policy");
    assert!(!serde_json::to_string(&found)?.contains("FORGED"));
    let mut tx = h.store.begin().await?;
    assert_eq!(tx.rebuild_knowledge_projections(project).await?, 2);
    tx.commit().await?;
    assert_eq!(h.core.drain_memory_projections().await?.indexed, 1);
    assert_eq!(
        index.state.imports.lock().await.last().context("import")?,
        &imports[0]
    );
    h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; fake AgentMemory HTTP, no inference"]
async fn stale_ack_cannot_publish_superseded_revision_and_commands_do_not_wait_for_http()
-> Result<()> {
    let index = FakeIndex::start().await?;
    let h = index.harness().await?;
    let project = h.create_project("projection race").await?;
    let id = publish_page(&h, project).await?;
    let old = projection(&h, project, id, 2).await?;
    index.state.gate.store(true, Ordering::SeqCst);
    let core = h.core.clone();
    let delivery = tokio::spawn(async move { core.drain_memory_projections().await });
    tokio::time::timeout(Duration::from_secs(3), index.state.entered.notified()).await?;
    tokio::time::timeout(
        Duration::from_secs(2),
        invoke(
            &h,
            project,
            "withdraw_knowledge_page",
            json!({"page_id":id,"expected_page_revision":2}),
        ),
    )
    .await??;
    // A second independently constructed Core shares the database fencing lock.
    let other = forge_core::CoreService::new(
        h.store.clone(),
        forge_core::CoreActors::new(forge_domain::ActorId::new(), forge_domain::ActorId::new()),
        Arc::new(forge_core::SupervisorHub::default()),
    )
    .with_agentmemory(
        &index.endpoint,
        SecretBytes::new(b"synthetic-index-token".to_vec()),
        Duration::from_secs(5),
    )?;
    assert_eq!(other.drain_memory_projections().await?.indexed, 0);
    index.state.release.notify_one();
    assert_eq!(delivery.await??.indexed, 0);
    let old_state = h
        .store
        .load_knowledge_projection(project, old)
        .await?
        .context("old")?;
    assert!(!old_state.index_ready);
    assert!(old_state.retirement_requested);
    eligible(&h).await?;
    assert_eq!(h.core.drain_memory_projections().await?.retired, 2);
    assert!(index.state.forgets.lock().await.contains(&old));
    index.hits(&[old]).await;
    assert!(
        h.core
            .query_search(project, None, "hello", 10)
            .await?
            .results
            .is_empty()
    );
    h.shutdown().await;
    Ok(())
}

async fn personal(
    h: &M0Harness,
    project: ProjectId,
    employee: Option<EmployeeId>,
    source: KnowledgeSourceRef,
) -> Result<Uuid> {
    let entry = DerivedMemoryEntry {
        id: Uuid::now_v7(),
        project_id: project,
        revision: 1,
        subject: match employee {
            Some(employee_id) => DerivedMemoryKind::EmployeeMemoryEntry {
                employee_id,
                task_id: None,
            },
            None => DerivedMemoryKind::ProjectKnowledgeEntry { task_id: None },
        },
        markdown: "hello".into(),
        source_refs: vec![source],
        content_hash: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".into(),
        coverage: None,
        created_by_job_id: Uuid::now_v7(),
        created_at: Timestamp::now_utc(),
        withdrawn: false,
    };
    let mut tx = h.store.begin().await?;
    tx.lock_project(project).await?.context("project")?;
    tx.validate_knowledge_sources(project, entry.subject.scope(), &entry.source_refs)
        .await?;
    tx.insert_derived_memory_revision(&entry).await?;
    tx.commit().await?;
    Ok(entry.id)
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; fake AgentMemory HTTP, no inference"]
async fn canonical_scope_source_withdrawal_and_outage_fallback_reject_index_authority() -> Result<()>
{
    let index = FakeIndex::start().await?;
    let h = index.harness().await?;
    let project = h.create_project("projection scope").await?;
    h.create_employee(project, "Bob").await?;
    h.create_employee(project, "Alice").await?;
    let employees = h.store.list_employees(project).await?;
    let bob = employees[0].employee.id();
    let alice = employees[1].employee.id();
    let page = publish_page(&h, project).await?;
    let source = KnowledgeSourceRef::KnowledgePage {
        page_id: page,
        revision: 2,
    };
    let bob_entry = personal(&h, project, Some(bob), source.clone()).await?;
    let alice_entry = personal(&h, project, Some(alice), source.clone()).await?;
    let common = personal(&h, project, None, source).await?;
    let foreign = h.create_project("foreign scope").await?;
    let foreign_page = publish_page(&h, foreign).await?;
    assert_eq!(h.core.drain_memory_projections().await?.indexed, 5);
    let mut ids = Vec::new();
    for (project, id, revision) in [
        (project, bob_entry, 1),
        (project, alice_entry, 1),
        (project, common, 1),
        (foreign, foreign_page, 2),
        (project, page, 2),
    ] {
        ids.push(projection(&h, project, id, revision).await?);
    }
    ids.push(Uuid::now_v7());
    index.hits(&ids).await;
    let found = h.core.query_search(project, Some(bob), "hello", 10).await?;
    assert_eq!(found.results.len(), 3);
    assert!(
        found
            .results
            .iter()
            .all(|hit| hit.projection_id != Some(ids[1]) && hit.projection_id != Some(ids[3]))
    );
    assert_eq!(
        found.degradation,
        Some(MemorySearchDegradation::RejectedProjection)
    );
    assert_eq!(
        h.core
            .query_search(project, None, "hello", 10)
            .await?
            .results
            .len(),
        2
    );
    index.state.status.store(503, Ordering::SeqCst);
    let fallback = h.core.query_search(project, Some(bob), "hello", 10).await?;
    assert_eq!(fallback.mode, MemorySearchMode::CanonicalFallback);
    assert_eq!(fallback.results.len(), 3);
    assert!(
        fallback
            .results
            .iter()
            .all(|hit| hit.projection_id.is_none() && hit.score.is_none())
    );
    invoke(
        &h,
        project,
        "withdraw_knowledge_page",
        json!({"page_id":page,"expected_page_revision":2}),
    )
    .await?;
    assert!(
        h.core
            .query_search(project, Some(bob), "hello", 10)
            .await?
            .results
            .is_empty(),
        "withdrawn source excluded during outage"
    );
    index.state.status.store(200, Ordering::SeqCst);
    assert!(
        h.core
            .query_search(project, Some(bob), "hello", 10)
            .await?
            .results
            .is_empty(),
        "withdrawn source excluded before projection cleanup"
    );
    assert!(
        h.core
            .query_search(foreign, Some(bob), "hello", 10)
            .await
            .is_err()
    );
    h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; local UDS, no inference"]
async fn operator_memory_http_is_project_scoped_and_exposes_disabled_index_explicitly() -> Result<()>
{
    let h = M0Harness::start_configured(Ok).await?;
    let project = h.create_project("memory operator API").await?;
    let other = h.create_project("other memory API").await?;
    h.create_employee(project, "Bob").await?;
    let employee = h
        .store
        .list_employees(project)
        .await?
        .remove(0)
        .employee
        .id();
    let page = publish_page(&h, project).await?;
    let id = personal(
        &h,
        project,
        Some(employee),
        KnowledgeSourceRef::KnowledgePage {
            page_id: page,
            revision: 2,
        },
    )
    .await?;
    let api = forge_testkit::m0::LocalHttpApi::start(h.core.clone())?;
    let client = reqwest::Client::builder()
        .unix_socket(api.socket())
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()?;
    let get = |path: String| client.get(format!("http://localhost{path}"));
    let status: Value = get(format!("/v1/projects/{project}/memory/status"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(status["configured"], false);
    assert_eq!(status["indexed"], 0);
    let result: Value = get(format!(
        "/v1/projects/{project}/memory/search?query=hello&employee_id={employee}"
    ))
    .send()
    .await?
    .error_for_status()?
    .json()
    .await?;
    assert_eq!(result["mode"], "canonical_fallback");
    assert_eq!(result["degradation"], "not_configured");
    assert_eq!(result["results"].as_array().context("results")?.len(), 2);
    for hit in result["results"].as_array().context("search hits")? {
        assert!(hit["document"]["record"]["created_at"].is_string());
        if hit["document"]["kind"] == "knowledge_page" {
            assert!(hit["document"]["record"]["revised_at"].is_string());
        }
    }
    let result: Value = get(format!(
        "/v1/projects/{project}/memory?employee_id={employee}&limit=1"
    ))
    .send()
    .await?
    .error_for_status()?
    .json()
    .await?;
    assert_eq!(result["items"][0]["id"], json!(id));
    assert!(result["items"][0]["created_at"].is_string());
    let history: Value = get(format!(
        "/v1/projects/{project}/memory/{id}/history?limit=1"
    ))
    .send()
    .await?
    .error_for_status()?
    .json()
    .await?;
    assert_eq!(history["items"][0]["revision"], 1);
    assert!(history["items"][0]["created_at"].is_string());
    assert_eq!(history["next_cursor"], "1");
    for suffix in [
        format!("memory/{id}"),
        format!("memory/{id}/history"),
        format!("employees/{employee}/onboarding"),
    ] {
        assert_eq!(
            get(format!("/v1/projects/{other}/{suffix}"))
                .send()
                .await?
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        get(format!(
            "/v1/projects/{project}/memory/search?query=hello&limit=0"
        ))
        .send()
        .await?
        .status(),
        StatusCode::BAD_REQUEST
    );
    api.shutdown().await;
    h.shutdown().await;
    Ok(())
}
