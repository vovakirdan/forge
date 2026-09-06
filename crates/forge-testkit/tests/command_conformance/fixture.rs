use std::{env, sync::Arc};

use anyhow::{Context, Result};
use forge_application::{
    CommandContext, CommandEnvelope, CommandError, RepositoryError, execute_in_transaction,
};
use forge_domain::{Actor, PipelineVersionId, ProjectId, Task, TaskId, Timestamp};
use forge_protocol::wire::{CommandName, CommandReceipt, CommandRequest};
use forge_storage::PostgresStore;
use forge_testkit::reference::{ManualClock, MemoryStore};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use time::macros::datetime;
use uuid::Uuid;

use super::{compatibility, snapshot::Snapshot};

#[derive(Clone, Copy)]
pub enum BackendKind {
    Memory,
    Postgres,
}

#[derive(Clone)]
pub enum Backend {
    Memory(MemoryStore),
    Postgres(PgPool),
}

#[derive(Clone)]
pub struct Fixture {
    pub backend: Backend,
    pub clock: ManualClock,
    pub context: CommandContext,
    pub project_id: ProjectId,
}

impl Fixture {
    pub async fn new(kind: BackendKind) -> Result<Self> {
        let backend = match kind {
            BackendKind::Memory => Backend::Memory(MemoryStore::default()),
            BackendKind::Postgres => Backend::Postgres(isolated_pool().await?),
        };
        let project_id = compatibility::PROJECT_ID.parse()?;
        Ok(Self {
            backend,
            clock: ManualClock::new(base_time()),
            context: CommandContext::local_human(
                project_id,
                Actor::human(compatibility::HUMAN_ID.parse()?),
                Actor::core(compatibility::CORE_ID.parse()?),
            ),
            project_id,
        })
    }

    pub async fn create(kind: BackendKind) -> Result<Self> {
        let fixture = Self::new(kind).await?;
        fixture
            .execute(
                CommandName::CreateProject,
                json!({"name": "Legacy project"}),
            )
            .await?;
        Ok(fixture)
    }

    pub fn envelope(
        &self,
        name: CommandName,
        revision: u64,
        payload: Value,
        key: &str,
    ) -> CommandEnvelope {
        CommandEnvelope::parse(
            name,
            CommandRequest {
                project_id: self.project_id.to_string(),
                expected_revision: revision,
                payload: payload.as_object().expect("fixture payload object").clone(),
            },
            key,
        )
        .expect("structurally valid fixture command")
    }

    pub async fn execute(&self, name: CommandName, payload: Value) -> Result<CommandReceipt> {
        let revision = self
            .snapshot()
            .await?
            .projects
            .get(&self.project_id)
            .map_or(0, forge_domain::Project::revision);
        let envelope = self.envelope(name, revision, payload, &Uuid::now_v7().to_string());
        Ok(self.execute_as(&envelope, &self.context).await?)
    }

    pub async fn execute_as(
        &self,
        envelope: &CommandEnvelope,
        context: &CommandContext,
    ) -> Result<CommandReceipt, CommandError> {
        match &self.backend {
            Backend::Memory(store) => {
                let mut transaction = store.begin().await;
                let receipt =
                    execute_in_transaction(&mut transaction, context, envelope, &self.clock)
                        .await?;
                transaction.commit().await?;
                Ok(receipt)
            }
            Backend::Postgres(pool) => {
                let store = PostgresStore::from_pool(pool.clone());
                let mut transaction = store.begin().await.map_err(unavailable)?;
                let receipt =
                    execute_in_transaction(&mut transaction, context, envelope, &self.clock)
                        .await?;
                transaction.commit().await.map_err(unavailable)?;
                Ok(receipt)
            }
        }
    }

    pub async fn snapshot(&self) -> Result<Snapshot> {
        match &self.backend {
            Backend::Memory(store) => Snapshot::memory(store.snapshot().await),
            Backend::Postgres(pool) => Snapshot::postgres(pool).await,
        }
    }

    pub async fn task(&self, id: TaskId) -> Result<Task> {
        self.snapshot()
            .await?
            .tasks
            .remove(&id)
            .context("fixture Task")
    }

    pub async fn pipeline(&self, payload: Value) -> Result<PipelineVersionId> {
        let receipt = self.execute(CommandName::CreatePipeline, payload).await?;
        let pipeline_id = receipt.resource.context("Pipeline resource")?.id;
        self.snapshot()
            .await?
            .versions
            .values()
            .find(|version| version.pipeline_id().to_string() == pipeline_id)
            .map(forge_domain::PipelineVersion::id)
            .context("created Pipeline version")
    }

    pub async fn create_task(&self, pipeline: PipelineVersionId, title: &str) -> Result<TaskId> {
        let receipt = self
            .execute(
                CommandName::CreateTask,
                json!({
                    "title": title,
                    "description": "Command conformance fixture",
                    "definition_of_done": "The pinned stage accepts its required evidence.",
                    "kind": "delivery", "priority": "normal",
                    "pipeline_version_id": pipeline
                }),
            )
            .await?;
        Ok(receipt.resource.context("Task resource")?.id.parse()?)
    }

    pub async fn task_command(
        &self,
        name: CommandName,
        task_id: TaskId,
        mut extra: Value,
    ) -> Result<CommandReceipt> {
        extra["task_id"] = json!(task_id);
        extra["expected_task_revision"] = json!(self.task(task_id).await?.revision().get());
        self.execute(name, extra).await
    }

    pub async fn assert_unchanged_after(
        &self,
        envelope: &CommandEnvelope,
        context: &CommandContext,
    ) -> Result<CommandError> {
        let before = self.snapshot().await?.raw;
        let error = self
            .execute_as(envelope, context)
            .await
            .expect_err("command must reject");
        assert_eq!(
            before,
            self.snapshot().await?.raw,
            "rejection must be atomic"
        );
        Ok(error)
    }

    pub fn core(&self, pool: &PgPool) -> forge_core::CoreService {
        forge_core::CoreService::new(
            PostgresStore::from_pool(pool.clone()),
            forge_core::CoreActors::new(self.context.actor.id(), self.context.core_actor.id()),
            Arc::new(forge_core::SupervisorHub::default()),
        )
    }
}

pub fn base_time() -> Timestamp {
    Timestamp::from_offset_date_time(datetime!(2026-09-06 12:00:00 UTC))
}

pub fn unavailable(_: forge_storage::StorageError) -> CommandError {
    RepositoryError::Unavailable.into()
}

async fn isolated_pool() -> Result<PgPool> {
    forge_testkit::m0::require_integration()?;
    let database_url = env::var("FORGE_DATABASE_URL").context("FORGE_DATABASE_URL required")?;
    let schema = format!("forge_command_test_{}", Uuid::now_v7().simple());
    let bootstrap = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await?;
    // Generated identifier only. Retain this private schema on success/failure.
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&bootstrap)
        .await?;
    bootstrap.close().await;
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .after_connect(move |connection, _| {
            let schema = schema.clone();
            Box::pin(async move {
                sqlx::query("SELECT set_config('search_path', $1, false)")
                    .bind(schema)
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await?;
    forge_storage::apply_migrations_and_validate(&pool).await?;
    Ok(pool)
}
