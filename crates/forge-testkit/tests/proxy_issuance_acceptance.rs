//! Real Core/SQL issuance fencing against a deterministic local HTTP broker.
//! Failure injection only; no public provider, login, or paid inference.

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use forge_core::WatchdogDeadlines;
use forge_domain::{
    CredentialBinding, CredentialDeliveryMode, ExecutionProfileInput, ProjectId, TaskId, Timestamp,
    runtime::{ResourceLimits, RuntimeBinding, SurfaceAccess, SurfaceSpec},
};
use forge_protocol::wire::CommandName;
use forge_provider_common::{MasterKeySource, PrivateMaterialization, SecretBytes, SecretStore};
use forge_testkit::m0::{M0Harness, ManualSupervisor, single_stage_pipeline};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::PermissionsExt,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex, Notify};
use uuid::Uuid;

#[path = "proxy_issuance_acceptance/inference_revocation.rs"]
mod inference_revocation;

#[derive(Default)]
struct Broker {
    route: Mutex<Option<Value>>,
    key: Mutex<Option<Value>>,
    last_issued: Mutex<Option<Value>>,
    create_started: Notify,
    release_create: Notify,
    lose_response: AtomicBool,
    inference_calls: AtomicUsize,
    inference_started: Notify,
    release_inference_headers: Notify,
}

async fn model_info(State(state): State<Arc<Broker>>) -> Json<Value> {
    Json(json!({"data":state.route.lock().await.iter().collect::<Vec<_>>()}))
}
async fn model_new(State(state): State<Arc<Broker>>, Json(mut body): Json<Value>) -> StatusCode {
    // The fake sanitized catalog never returns the upstream credential.
    if let Some(parameters) = body["litellm_params"].as_object_mut() {
        parameters.remove("api_key");
    }
    *state.route.lock().await = Some(body);
    StatusCode::OK
}
async fn key_info(State(state): State<Arc<Broker>>) -> Response {
    match state.key.lock().await.as_ref() {
        Some(key) => Json(json!({"info":key})).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
async fn key_generate(State(state): State<Arc<Broker>>, Json(mut body): Json<Value>) -> StatusCode {
    state.create_started.notify_one();
    state.release_create.notified().await;
    body["expires"] = body["metadata"]["forge"]["expires_at"].clone();
    body["spend"] = json!(0.0);
    if let Some(object) = body.as_object_mut() {
        object.remove("key");
        object.remove("duration");
    }
    *state.last_issued.lock().await = Some(body.clone());
    *state.key.lock().await = Some(body);
    if state.lose_response.load(Ordering::SeqCst) {
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        StatusCode::OK
    }
}
async fn key_delete(State(state): State<Arc<Broker>>) -> StatusCode {
    *state.key.lock().await = None;
    StatusCode::OK
}

struct Fixture {
    harness: M0Harness,
    project: ProjectId,
    task: TaskId,
    broker: Arc<Broker>,
    server: tokio::task::JoinHandle<Result<(), std::io::Error>>,
    _supervisor: ManualSupervisor,
    root: std::path::PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Fixture {
    async fn new() -> Result<Self> {
        let broker = Arc::new(Broker::default());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = format!("http://{}", listener.local_addr()?);
        let router = Router::new()
            .route("/model/info", get(model_info))
            .route("/model/new", post(model_new))
            .route("/key/info", get(key_info))
            .route("/key/generate", post(key_generate))
            .route("/key/delete", post(key_delete))
            .route(
                "/v1/chat/completions",
                post(inference_revocation::inference),
            )
            .with_state(broker.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await });
        let root = std::env::temp_dir().join(format!("fproxy-{}", Uuid::now_v7()));
        fs::create_dir(&root)?;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        let store = SecretStore::initialize(&root.join("secrets"), MasterKeySource::OwnerOnlyFile)?;
        let master = PrivateMaterialization::create(
            &root.join("master"),
            &SecretBytes::new(b"sk-synthetic-master-not-real".to_vec()),
        )?;
        let execution_root = root.clone();
        let harness = M0Harness::start_configured(move |core| {
            Ok(core
                .with_secret_store(store)
                .with_execution_root(execution_root)?
                .with_inference_proxy(&endpoint, master.path())?)
        })
        .await?;
        let project = harness
            .create_project("Proxy issuance failure injection")
            .await?;
        let secret_id = Uuid::now_v7();
        let binding_id = Uuid::now_v7();
        let source = PrivateMaterialization::create(
            &root.join("api-source"),
            &SecretBytes::new(b"sk-synthetic-upstream-not-real".to_vec()),
        )?;
        harness.execute(project, CommandName::EnrollCredential, json!({
            "secret_id":secret_id,"binding_id":binding_id,"kind":"api_key","source_file":source.path()
        })).await?;
        harness
            .create_employee(project, "Synthetic API worker")
            .await?;
        let mode = CredentialDeliveryMode::ProxyOnly;
        let profile = ExecutionProfileInput {
            id: Uuid::now_v7(),
            revision: 1,
            project_id: project,
            adapter_id: "opencode_runtime".into(),
            adapter_version: "1.18.29".into(),
            provider_id: "openai".into(),
            model: "synthetic".into(),
            credential_binding: CredentialBinding {
                id: binding_id,
                project_id: project,
                secret_id,
                account_id: None,
                allowed_delivery_modes: BTreeSet::from([mode]),
            },
            credential_delivery: mode,
            capability_profile: forge_provider_opencode::OpenCodeAdapter::capabilities(),
        }
        .try_into()?;
        let binding = RuntimeBinding {
            execution_profile: profile,
            image: format!("localhost/synthetic@sha256:{}", "0".repeat(64)),
            surface: SurfaceSpec::FilesystemSandbox,
            access: SurfaceAccess::ReadWrite,
            limits: ResourceLimits::default(),
            budget: Default::default(),
            system_prompt: "Synthetic fixture.".into(),
            employee_prompt: "No inference.".into(),
        };
        let employee = harness.store.list_employees(project).await?.remove(0);
        harness
            .execute(
                project,
                CommandName::ConfigureEmployeeRuntime,
                json!({
                    "employee_id":employee.employee.id(),"binding":binding
                }),
            )
            .await?;
        let mut supervisor = harness
            .attach_manual_supervisor_with_identity(
                &format!("proxy-host-{}", Uuid::now_v7()),
                &format!("proxy-boot-{}", Uuid::now_v7()),
            )
            .await?;
        supervisor.reconcile_empty().await?;
        let pipeline = harness
            .create_pipeline(project, single_stage_pipeline())
            .await?;
        let task = harness
            .create_task(project, pipeline, "Proxy issuance race")
            .await?;
        harness.approve_task(project, task).await?;
        Ok(Self {
            harness,
            project,
            task,
            broker,
            server,
            _supervisor: supervisor,
            root,
        })
    }

    async fn tick(&self) -> Result<()> {
        self.harness
            .core
            .watchdog_tick(Timestamp::now_utc(), WatchdogDeadlines::default())
            .await?;
        Ok(())
    }

    async fn due_now(&self, run: Uuid) -> Result<()> {
        // Explicit clock fault injection, not a production cleanup path.
        sqlx::query("UPDATE run_proxy_keys SET next_revocation_attempt_at=clock_timestamp() WHERE run_id=$1")
            .bind(run).execute(&self.harness.pool).await?;
        Ok(())
    }
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; deterministic HTTP broker, no real provider"]
async fn revoke_before_delayed_create_still_compensates_and_settles() -> Result<()> {
    let fixture = Fixture::new().await?;
    let start = fixture.harness.start_project(fixture.project);
    tokio::pin!(start);
    tokio::select! {
        result = &mut start => { result?; anyhow::bail!("start finished before injected pause"); },
        () = fixture.broker.create_started.notified() => {},
        () = tokio::time::sleep(Duration::from_secs(10)) => anyhow::bail!("broker issue not reached"),
    }
    let run = fixture
        .harness
        .wait_for_run_count(fixture.task, 1)
        .await?
        .remove(0);
    fixture.harness.stop_project(fixture.project).await?;
    fixture.tick().await?;
    let first = fixture
        .harness
        .store
        .begin()
        .await?
        .run_proxy_key(run.id)
        .await?
        .context("proxy intent")?;
    assert!(first.revoked && first.issuance_pending);
    assert!(
        fixture.broker.key.lock().await.is_none(),
        "first revoke sees absence"
    );
    fixture.broker.release_create.notify_one();
    tokio::time::timeout(Duration::from_secs(10), &mut start).await??;
    assert!(
        fixture.broker.key.lock().await.is_none(),
        "late creation must be compensated"
    );
    let settled = fixture
        .harness
        .store
        .begin()
        .await?
        .run_proxy_key(run.id)
        .await?
        .context("settled intent")?;
    assert!(settled.revoked && !settled.issuance_pending);
    fixture.due_now(run.id).await?;
    assert!(
        !fixture
            .harness
            .store
            .pending_retired_proxy_revocations()
            .await?
            .contains(&run.id)
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; deterministic HTTP broker, no real provider"]
async fn lost_issue_response_keeps_cleanup_durable_after_an_absent_key_observation() -> Result<()> {
    let fixture = Fixture::new().await?;
    fixture.broker.lose_response.store(true, Ordering::SeqCst);
    fixture.broker.release_create.notify_one();
    fixture.harness.start_project(fixture.project).await?;
    let run = fixture
        .harness
        .wait_for_run_count(fixture.task, 1)
        .await?
        .remove(0);
    let saved = fixture
        .harness
        .store
        .begin()
        .await?
        .run_proxy_key(run.id)
        .await?
        .context("ambiguous intent")?;
    assert!(saved.revoked && saved.issuance_pending);
    assert!(fixture.broker.key.lock().await.is_none());
    fixture.due_now(run.id).await?;
    // A fresh repository connection has no knowledge of the lost HTTP operation.
    let reloaded = forge_storage::PostgresStore::from_pool(fixture.harness.pool.clone());
    assert!(
        reloaded
            .pending_retired_proxy_revocations()
            .await?
            .contains(&run.id)
    );
    fixture.tick().await?;
    let absent = reloaded
        .begin()
        .await?
        .run_proxy_key(run.id)
        .await?
        .context("retained ambiguity")?;
    assert!(
        absent.revoked && absent.issuance_pending,
        "404 does not settle an outstanding issue"
    );
    // Model server-side completion after a crashed/timeout client already saw 404.
    *fixture.broker.key.lock().await = fixture.broker.last_issued.lock().await.clone();
    fixture.due_now(run.id).await?;
    fixture.tick().await?;
    assert!(
        fixture.broker.key.lock().await.is_none(),
        "durable retry deletes a resurrected key"
    );
    assert!(
        reloaded
            .begin()
            .await?
            .run_proxy_key(run.id)
            .await?
            .context("still uncertain")?
            .issuance_pending
    );
    fixture.harness.stop_project(fixture.project).await?;
    Ok(())
}
