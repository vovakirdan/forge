use std::{
    env, fs, future::Future, os::unix::fs::PermissionsExt, path::PathBuf, sync::Arc, time::Duration,
};

use anyhow::{Context, Result, bail};
use forge_application::CommandEnvelope;
use forge_core::{CoreActors, CoreError, CoreService, SupervisorHub, SupervisorService};
use forge_domain::{ActorId, DomainError, LifecycleStatus, PipelineVersionId, ProjectId, TaskId};
use forge_protocol::{
    supervisor::v1::supervisor_control_server::SupervisorControlServer,
    wire::{CommandName, CommandReceipt, CommandRequest},
};
use forge_storage::{PostgresStore, RunProjection, StoredTask};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::{
    net::UnixListener,
    sync::watch,
    task::JoinHandle,
    time::{sleep, timeout},
};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;
use uuid::Uuid;

use super::supervisor::ManualSupervisor;

const WAIT_TIMEOUT: Duration = Duration::from_secs(8);
const POLL_INTERVAL: Duration = Duration::from_millis(15);

/// Refuses an integration run that has not explicitly opted into local services.
pub fn require_integration() -> Result<()> {
    if env::var("FORGE_INTEGRATION").as_deref() == Ok("1") {
        Ok(())
    } else {
        bail!("set FORGE_INTEGRATION=1 and start the local Forge development topology")
    }
}

/// One isolated Core and local gRPC listener backed by the shared development database.
pub struct M0Harness {
    /// Canonical Core command service exercised by acceptance tests.
    pub core: CoreService,
    /// Read-only storage boundary used to assert durable results.
    pub store: PostgresStore,
    /// Test-local pool used only for query-only acceptance assertions.
    pub pool: PgPool,
    hub: Arc<SupervisorHub>,
    socket_path: PathBuf,
    temp_dir: PathBuf,
    shutdown: watch::Sender<bool>,
    grpc_task: Option<JoinHandle<Result<(), tonic::transport::Error>>>,
    fake_task: Option<JoinHandle<Result<(), forge_supervisor::SupervisorError>>>,
}

impl M0Harness {
    /// Starts a Core service and its real local Supervisor gRPC endpoint.
    pub async fn start() -> Result<Self> {
        require_integration()?;
        let database_url = required_environment("FORGE_DATABASE_URL")?;
        let nats_url = required_environment("FORGE_NATS_URL")?;
        let nats = connect_nats(&nats_url).await?;
        nats.flush().await.context("flush local NATS connection")?;

        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect(&database_url)
            .await
            .context("connect local Forge PostgreSQL")?;
        let store = PostgresStore::from_pool(pool.clone());
        store
            .validate_schema()
            .await
            .context("validate Forge schema")?;

        let temp_dir = test_directory()?;
        let socket_path = temp_dir.join("core.sock");
        let listener = UnixListener::bind(&socket_path)
            .with_context(|| format!("bind test Supervisor socket {}", socket_path.display()))?;
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))
            .context("restrict test Supervisor socket")?;

        let hub = Arc::new(SupervisorHub::default());
        let core = CoreService::new(
            store.clone(),
            CoreActors::new(ActorId::new(), ActorId::new()),
            Arc::clone(&hub),
        );
        let (shutdown, shutdown_receiver) = watch::channel(false);
        let grpc_task = tokio::spawn(
            Server::builder()
                .add_service(SupervisorControlServer::new(SupervisorService::new(
                    core.clone(),
                )))
                .serve_with_incoming_shutdown(
                    UnixListenerStream::new(listener),
                    wait_for_shutdown(shutdown_receiver),
                ),
        );

        Ok(Self {
            core,
            store,
            pool,
            hub,
            socket_path,
            temp_dir,
            shutdown,
            grpc_task: Some(grpc_task),
            fake_task: None,
        })
    }

    /// Attaches the real deterministic M0 fake Supervisor to this Core.
    pub async fn attach_fake_supervisor(&mut self) -> Result<()> {
        if self.fake_task.is_some() {
            bail!("a fake Supervisor is already attached")
        }
        let mut config = forge_supervisor::SupervisorConfig::new(
            self.socket_path.clone(),
            format!("m0-test-host-{}", Uuid::now_v7()),
            format!("m0-test-boot-{}", Uuid::now_v7()),
        );
        config.reconnect_delay = Duration::from_millis(10);
        self.fake_task = Some(tokio::spawn(forge_supervisor::run(
            config,
            self.shutdown.subscribe(),
        )));
        self.wait_for_supervisor(true).await
    }

    /// Attaches a deliberately non-executing Supervisor for scheduler assertions.
    pub async fn attach_manual_supervisor(&self) -> Result<ManualSupervisor> {
        let supervisor = ManualSupervisor::connect(&self.socket_path).await?;
        self.wait_for_supervisor(true).await?;
        Ok(supervisor)
    }

    /// Connects a fresh JetStream context using the owner-provided local endpoint.
    ///
    /// The acceptance suite deliberately does not share a client with Core: this
    /// proves that durable outbox delivery is observable by an independent
    /// consumer rather than by an in-process test double.
    pub async fn jetstream_context(&self) -> Result<async_nats::jetstream::Context> {
        let nats_url = required_environment("FORGE_NATS_URL")?;
        let client = connect_nats(&nats_url).await?;
        Ok(async_nats::jetstream::new(client))
    }

    /// Creates a stopped Project and returns its caller-reserved identity.
    pub async fn create_project(&self, name: &str) -> Result<ProjectId> {
        let project_id = ProjectId::new();
        self.execute(
            project_id,
            CommandName::CreateProject,
            json!({ "name": name }),
        )
        .await?;
        Ok(project_id)
    }

    /// Creates one immutable Pipeline version and returns its generated identity.
    pub async fn create_pipeline(
        &self,
        project_id: ProjectId,
        payload: Value,
    ) -> Result<PipelineVersionId> {
        self.execute(project_id, CommandName::CreatePipeline, payload)
            .await?;
        let versions = self.store.list_pipeline_versions(project_id).await?;
        versions
            .into_iter()
            .next()
            .map(|version| version.id())
            .context("created Pipeline version is missing")
    }

    /// Creates an Employee eligible for every stage of its Project.
    pub async fn create_employee(&self, project_id: ProjectId, name: &str) -> Result<()> {
        self.execute(
            project_id,
            CommandName::CreateEmployee,
            json!({
                "name": name,
                "role": "m0_worker",
                "stage_eligibility": {"mode": "any"}
            }),
        )
        .await?;
        Ok(())
    }

    /// Creates one delivery Task pinned to the supplied immutable version.
    pub async fn create_task(
        &self,
        project_id: ProjectId,
        pipeline_version_id: PipelineVersionId,
        title: &str,
    ) -> Result<TaskId> {
        let receipt = self
            .execute(
                project_id,
                CommandName::CreateTask,
                json!({
                    "title": title,
                    "description": "M0 acceptance fixture",
                    "definition_of_done": "M0 pipeline outcome and required evidence are recorded.",
                    "kind": "delivery",
                    "pipeline_version_id": pipeline_version_id.as_uuid().to_string(),
                    "priority": "normal"
                }),
            )
            .await?;
        resource_id(&receipt, "task")
    }

    /// Makes a draft Task eligible for its entry stage.
    pub async fn approve_task(&self, project_id: ProjectId, task_id: TaskId) -> Result<()> {
        let task = self.required_task(task_id).await?;
        self.execute(
            project_id,
            CommandName::ApproveTask,
            json!({
                "task_id": task_id.as_uuid().to_string(),
                "expected_task_revision": task.task.revision().get()
            }),
        )
        .await?;
        Ok(())
    }

    /// Adds the M0 `task_done` hard dependency edge.
    pub async fn create_dependency(
        &self,
        project_id: ProjectId,
        blocker_task_id: TaskId,
        blocked_task_id: TaskId,
    ) -> Result<()> {
        self.execute(
            project_id,
            CommandName::CreateDependency,
            json!({
                "blocker_task_id": blocker_task_id.as_uuid().to_string(),
                "blocked_task_id": blocked_task_id.as_uuid().to_string(),
                "required_condition": "task_done"
            }),
        )
        .await?;
        Ok(())
    }

    /// Opens the durable Project execution gate.
    pub async fn start_project(&self, project_id: ProjectId) -> Result<()> {
        self.execute(
            project_id,
            CommandName::StartProjectExecution,
            json!({ "reason": "m0_acceptance" }),
        )
        .await?;
        Ok(())
    }

    /// Closes the durable Project execution gate and requests active Run stops.
    pub async fn stop_project(&self, project_id: ProjectId) -> Result<()> {
        self.execute(
            project_id,
            CommandName::StopProjectExecution,
            json!({ "reason": "m0_acceptance" }),
        )
        .await?;
        Ok(())
    }

    /// Cancels an exhausted or otherwise mutable Task with the built-in reason.
    pub async fn cancel_task(&self, project_id: ProjectId, task_id: TaskId) -> Result<()> {
        let task = self.required_task(task_id).await?;
        self.execute(
            project_id,
            CommandName::CancelTask,
            json!({
                "task_id": task_id.as_uuid().to_string(),
                "expected_task_revision": task.task.revision().get(),
                "cancellation_reason_key": "unspecified",
                "note": "m0 acceptance cleanup"
            }),
        )
        .await?;
        Ok(())
    }

    /// Changes one Task's configured project-defined priority level.
    pub async fn set_task_priority(
        &self,
        project_id: ProjectId,
        task_id: TaskId,
        priority: &str,
    ) -> Result<()> {
        let task = self.required_task(task_id).await?;
        self.execute(
            project_id,
            CommandName::SetTaskPriority,
            json!({
                "task_id": task_id.as_uuid().to_string(),
                "expected_task_revision": task.task.revision().get(),
                "priority": priority
            }),
        )
        .await?;
        Ok(())
    }

    /// Resolves the sole active human/external Pipeline-stage wait.
    pub async fn submit_external_stage_outcome(
        &self,
        project_id: ProjectId,
        task_id: TaskId,
        outcome: &str,
    ) -> Result<()> {
        let stored = self.required_task(task_id).await?;
        let stage_id = stored
            .task
            .current_stage_id()
            .context("manual Task has no current stage")?;
        let wait = stored
            .task
            .wait_conditions()
            .find(|condition| condition.kind() == &forge_domain::TaskWaitKind::DecisionRequired)
            .context("manual Task has no active decision wait")?;
        self.execute(
            project_id,
            CommandName::SubmitExternalStageOutcome,
            json!({
                "task_id": task_id.as_uuid().to_string(),
                "expected_task_revision": stored.task.revision().get(),
                "stage_id": stage_id.as_str(),
                "outcome": outcome,
                "wait_condition_id": wait.id().as_uuid().to_string(),
                "artifacts": [],
                "outcome_artifact_ids": []
            }),
        )
        .await?;
        Ok(())
    }

    /// Waits for a Task to reach the supplied lifecycle state.
    pub async fn wait_for_lifecycle(
        &self,
        task_id: TaskId,
        lifecycle: LifecycleStatus,
    ) -> Result<StoredTask> {
        self.wait_for_task(task_id, |task| task.lifecycle() == lifecycle)
            .await
    }

    /// Waits until a durable Task snapshot satisfies a pure assertion predicate.
    pub async fn wait_for_task<F>(&self, task_id: TaskId, predicate: F) -> Result<StoredTask>
    where
        F: Fn(&forge_domain::Task) -> bool,
    {
        wait_until("Task state", || async {
            let task = self.required_task(task_id).await?;
            Ok(predicate(&task.task).then_some(task))
        })
        .await
    }

    /// Waits until one Task has emitted at least the requested number of Runs.
    pub async fn wait_for_run_count(
        &self,
        task_id: TaskId,
        minimum: usize,
    ) -> Result<Vec<RunProjection>> {
        wait_until("Run count", || async {
            let runs = self.store.list_runs(task_id).await?;
            Ok((runs.len() >= minimum).then_some(runs))
        })
        .await
    }

    /// Counts only queue rows that remain dispatchable under the Project gate.
    pub async fn queued_count(&self, project_id: ProjectId) -> Result<i64> {
        sqlx::query_scalar(
            "SELECT count(*) FROM queue_entries WHERE project_id = $1 AND queue_state = 'queued'",
        )
        .bind(project_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .context("count queued M0 entries")
    }

    /// Counts every queue snapshot retained for one Task, including a lease.
    pub async fn queue_entry_count_for_task(&self, task_id: TaskId) -> Result<i64> {
        sqlx::query_scalar("SELECT count(*) FROM queue_entries WHERE task_id = $1")
            .bind(task_id.as_uuid())
            .fetch_one(&self.pool)
            .await
            .context("count Task queue snapshots")
    }

    /// Waits until the current supervisor stream has disconnected.
    pub async fn wait_for_supervisor_disconnected(&self) -> Result<()> {
        self.wait_for_supervisor(false).await
    }

    /// Stops local helper tasks and removes only the unique test socket directory.
    pub async fn shutdown(mut self) {
        let _ = self.shutdown.send(true);
        if let Some(task) = self.fake_task.take() {
            let _ = timeout(Duration::from_secs(2), task).await;
        }
        if let Some(task) = self.grpc_task.take() {
            task.abort();
            let _ = task.await;
        }
        let _ = fs::remove_file(&self.socket_path);
        let _ = fs::remove_dir(&self.temp_dir);
    }

    async fn execute(
        &self,
        project_id: ProjectId,
        name: CommandName,
        payload: Value,
    ) -> Result<CommandReceipt> {
        let payload = payload
            .as_object()
            .cloned()
            .context("M0 command payload must be an object")?;
        for attempt in 0..4 {
            let expected_revision = if name == CommandName::CreateProject {
                0
            } else {
                self.store
                    .load_project(project_id)
                    .await?
                    .context("M0 Project is missing")?
                    .revision()
            };
            let envelope = CommandEnvelope::parse(
                name,
                CommandRequest {
                    project_id: project_id.as_uuid().to_string(),
                    expected_revision,
                    payload: payload.clone(),
                },
                Uuid::now_v7().to_string(),
            )?;
            match self.core.execute_command(envelope).await {
                Ok(receipt) => return Ok(receipt),
                Err(error) if retryable_revision_race(&error) && attempt < 3 => {
                    sleep(POLL_INTERVAL).await;
                }
                Err(error) => return Err(error.into()),
            }
        }
        bail!("M0 command exhausted revision retries")
    }

    async fn required_task(&self, task_id: TaskId) -> Result<StoredTask> {
        self.store
            .load_task(task_id)
            .await?
            .context("M0 Task is missing")
    }

    async fn wait_for_supervisor(&self, connected: bool) -> Result<()> {
        wait_until("Supervisor connection", || async {
            Ok((self.hub.is_connected().await == connected).then_some(()))
        })
        .await
    }
}

impl Drop for M0Harness {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        if let Some(task) = &self.grpc_task {
            task.abort();
        }
        if let Some(task) = &self.fake_task {
            task.abort();
        }
    }
}

fn required_environment(name: &str) -> Result<String> {
    env::var(name)
        .with_context(|| format!("set {name} through the owner-only Forge dev environment"))
}

async fn connect_nats(nats_url: &str) -> Result<async_nats::Client> {
    let address = nats_url
        .parse::<async_nats::ServerAddr>()
        .map_err(|_| anyhow::anyhow!("parse configured NATS JetStream URL"))?;
    let client = match (address.username(), address.password()) {
        (Some(token), None) => {
            async_nats::ConnectOptions::with_token(token.to_owned())
                .connect(nats_url)
                .await
        }
        (Some(user), Some(password)) => {
            async_nats::ConnectOptions::with_user_and_password(user.to_owned(), password.to_owned())
                .connect(nats_url)
                .await
        }
        (None, None) => async_nats::connect(nats_url).await,
        (None, Some(_)) => bail!("configured NATS URL has a password without a username"),
    };
    client.map_err(|_| anyhow::anyhow!("connect local NATS JetStream"))
}

fn test_directory() -> Result<PathBuf> {
    let directory = env::temp_dir().join(format!("forge-m0-acceptance-{}", Uuid::now_v7()));
    fs::create_dir(&directory).context("create isolated M0 test socket directory")?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
        .context("restrict M0 test socket directory")?;
    Ok(directory)
}

async fn wait_for_shutdown(mut receiver: watch::Receiver<bool>) {
    if !*receiver.borrow() {
        let _ = receiver.changed().await;
    }
}

async fn wait_until<T, F, Fut>(description: &str, mut check: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Option<T>>>,
{
    timeout(WAIT_TIMEOUT, async {
        loop {
            if let Some(value) = check().await? {
                return Ok(value);
            }
            sleep(POLL_INTERVAL).await;
        }
    })
    .await
    .with_context(|| format!("wait for {description}"))?
}

fn resource_id(receipt: &CommandReceipt, expected_kind: &str) -> Result<TaskId> {
    let resource = receipt
        .resource
        .as_ref()
        .context("command returned no resource")?;
    if resource.kind != expected_kind {
        bail!("expected {expected_kind} resource, got {}", resource.kind)
    }
    Ok(TaskId::from(
        Uuid::parse_str(&resource.id).context("parse Task resource identity")?,
    ))
}

fn retryable_revision_race(error: &CoreError) -> bool {
    matches!(
        error,
        CoreError::Domain(DomainError::StaleProjectRevision { .. })
    )
}
