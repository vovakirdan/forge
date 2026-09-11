//! Repository enrollment and Task pinning execute identically on both stores.
use anyhow::{Context, Result};
use forge_application::CommandTransaction;
use forge_domain::{Actor, ActorId, ProjectId, TaskWorkSurface};
use forge_protocol::wire::{CommandName, CommandStatus};
use forge_testkit::m0::single_stage_pipeline;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    atomicity::assert_faults,
    fixture::{Backend, BackendKind, Fixture},
};

fn registration(name: &str) -> Value {
    // Deliberately absent: enrollment is operator intent, not filesystem inspection.
    json!({"name":name,"source":"/tmp/forge-explicit-test-source","target_ref":"refs/heads/main"})
}

pub async fn binding_round_trip(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let command = fixture.envelope(
        CommandName::RegisterProjectRepository,
        1,
        registration("Mock repository"),
        "register-source",
    );
    assert_faults(&fixture, &command).await?;
    let registered = fixture.execute_as(&command, &fixture.context).await?;
    let repository: Uuid = registered
        .resource
        .as_ref()
        .context("repository")?
        .id
        .parse()?;
    let replay = fixture.execute_as(&command, &fixture.context).await?;
    assert_eq!(replay.status, CommandStatus::Replayed);
    assert_eq!(registered.resource, replay.resource);
    assert_eq!(
        fixture.snapshot().await?.rows("project_repositories").len(),
        1
    );
    if let Backend::Postgres(pool) = &fixture.backend {
        let sources = forge_storage::PostgresStore::from_pool(pool.clone())
            .list_project_repositories(fixture.project_id)
            .await?;
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].id, repository);
    }
    let version = fixture.pipeline(single_stage_pipeline()).await?;
    let task_id = fixture.create_task(version, "Pinned writer").await?;
    let revision = fixture.snapshot().await?.projects[&fixture.project_id].revision();
    let command = fixture.envelope(CommandName::BindTaskGitRepository, revision, json!({"task_id":task_id,"expected_task_revision":1,"repository_id":repository,"initial_base":"a".repeat(40)}), "bind-source");
    assert_faults(&fixture, &command).await?;
    let accepted = fixture.execute_as(&command, &fixture.context).await?;
    let task = fixture.task(task_id).await?;
    let TaskWorkSurface::Git(binding) = task.work_surface() else {
        anyhow::bail!("missing Git binding")
    };
    assert_eq!(binding.repository_id, repository);
    assert_eq!(
        binding
            .initial_base
            .commit()
            .context("committed base")?
            .as_str(),
        "a".repeat(40)
    );
    assert_eq!(task.revision().get(), 2);
    assert_eq!(
        fixture
            .execute_as(&command, &fixture.context)
            .await?
            .event_ids,
        accepted.event_ids
    );
    fixture
        .task_command(CommandName::ApproveTask, task_id, json!({}))
        .await?;
    assert_eq!(
        fixture.task(task_id).await?.work_surface(),
        task.work_surface()
    );
    let another = fixture.create_task(version, "Independent writer").await?;
    fixture
        .task_command(
            CommandName::BindTaskGitRepository,
            another,
            json!({"repository_id":repository,"initial_base":"a".repeat(40)}),
        )
        .await?;
    assert_ne!(
        fixture.task(another).await?.work_surface(),
        task.work_surface()
    );
    fixture.snapshot().await?.assert_audit_atomic();
    Ok(())
}

pub async fn refusals_are_atomic(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let registered = fixture
        .execute(
            CommandName::RegisterProjectRepository,
            registration("Mock repository"),
        )
        .await?;
    let repository: Uuid = registered.resource.context("repository")?.id.parse()?;
    let version = fixture.pipeline(single_stage_pipeline()).await?;
    let task = fixture.create_task(version, "Mutable draft").await?;
    let revision = fixture.snapshot().await?.projects[&fixture.project_id].revision();
    let duplicate = fixture.envelope(
        CommandName::RegisterProjectRepository,
        revision,
        registration("Mock repository"),
        "duplicate",
    );
    fixture
        .assert_unchanged_after(&duplicate, &fixture.context)
        .await?;
    let payload = json!({"task_id":task,"expected_task_revision":1,"repository_id":repository,"initial_base":"b".repeat(40)});
    let mut denied = fixture.context.clone();
    denied.actor = Actor::employee(ActorId::new());
    denied.capabilities.clear();
    let command = fixture.envelope(
        CommandName::BindTaskGitRepository,
        revision,
        payload.clone(),
        "denied",
    );
    fixture.assert_unchanged_after(&command, &denied).await?;
    for (field, value) in [
        ("expected_task_revision", json!(9)),
        ("repository_id", json!(Uuid::now_v7())),
    ] {
        let mut invalid = payload.clone();
        invalid[field] = value;
        let command =
            fixture.envelope(CommandName::BindTaskGitRepository, revision, invalid, field);
        fixture
            .assert_unchanged_after(&command, &fixture.context)
            .await?;
    }
    let mut other = fixture.clone();
    other.project_id = ProjectId::new();
    other.context.project_id = other.project_id;
    other
        .execute(CommandName::CreateProject, json!({"name":"Other Project"}))
        .await?;
    let foreign = other
        .execute(
            CommandName::RegisterProjectRepository,
            registration("Foreign source"),
        )
        .await?
        .resource
        .context("foreign source")?
        .id;
    let mut invalid = payload.clone();
    invalid["repository_id"] = json!(foreign);
    let command = fixture.envelope(
        CommandName::BindTaskGitRepository,
        revision,
        invalid,
        "cross-project",
    );
    fixture
        .assert_unchanged_after(&command, &fixture.context)
        .await?;
    fixture
        .execute(CommandName::BindTaskGitRepository, payload.clone())
        .await?;
    let revision = fixture.snapshot().await?.projects[&fixture.project_id].revision();
    let mut rebinding = payload;
    rebinding["expected_task_revision"] = json!(2);
    let command = fixture.envelope(
        CommandName::BindTaskGitRepository,
        revision,
        rebinding,
        "rebind",
    );
    fixture
        .assert_unchanged_after(&command, &fixture.context)
        .await?;
    let ready = fixture
        .create_task(version, "Approved unbound Task")
        .await?;
    fixture
        .task_command(CommandName::ApproveTask, ready, json!({}))
        .await?;
    let command = fixture.envelope(CommandName::BindTaskGitRepository,
        fixture.snapshot().await?.projects[&fixture.project_id].revision(),
        json!({"task_id":ready,"expected_task_revision":fixture.task(ready).await?.revision().get(),"repository_id":repository,"initial_base":"b".repeat(40)}), "late-bind");
    fixture
        .assert_unchanged_after(&command, &fixture.context)
        .await?;
    Ok(())
}

pub async fn storage_guards_preserve_immutable_source(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let repository = fixture
        .execute(
            CommandName::RegisterProjectRepository,
            registration("Immutable source"),
        )
        .await?
        .resource
        .context("repository")?
        .id;
    let version = fixture.pipeline(single_stage_pipeline()).await?;
    let task = fixture.create_task(version, "Pinned source").await?;
    fixture
        .task_command(
            CommandName::BindTaskGitRepository,
            task,
            json!({"repository_id":repository,"initial_base":"c".repeat(40)}),
        )
        .await?;
    let before = fixture.snapshot().await?.raw;
    match &fixture.backend {
        Backend::Memory(store) => {
            reject_tampered_source(&mut store.begin().await, task).await?;
            reject_tampered_surface(&mut store.begin().await, task).await?;
        }
        Backend::Postgres(pool) => {
            let store = forge_storage::PostgresStore::from_pool(pool.clone());
            reject_tampered_source(&mut store.begin().await?, task).await?;
            reject_tampered_surface(&mut store.begin().await?, task).await?;
            let id: Uuid = repository.parse()?;
            assert!(
                sqlx::query("UPDATE project_repositories SET name='changed' WHERE id=$1")
                    .bind(id)
                    .execute(pool)
                    .await
                    .is_err()
            );
            assert!(
                sqlx::query("DELETE FROM project_repositories WHERE id=$1")
                    .bind(id)
                    .execute(pool)
                    .await
                    .is_err()
            );
        }
    }
    assert_eq!(before, fixture.snapshot().await?.raw);
    Ok(())
}

async fn reject_tampered_source(
    transaction: &mut impl CommandTransaction,
    task: forge_domain::TaskId,
) -> Result<()> {
    let stored = transaction.lock_task(task).await?.context("Task")?;
    let mut value = serde_json::to_value(&stored.task)?;
    value["work_surface"]["git"]["source"] = json!("/tmp/not-registered-source");
    value["revision"] = json!(stored.task.revision().get() + 1);
    let tampered: forge_domain::Task = serde_json::from_value(value)?;
    assert!(
        transaction
            .update_task(&tampered, stored.persistence, stored.task.revision().get())
            .await
            .is_err()
    );
    Ok(())
}

async fn reject_tampered_surface(
    transaction: &mut impl CommandTransaction,
    task: forge_domain::TaskId,
) -> Result<()> {
    let mut stored = transaction.lock_task(task).await?.context("Task")?;
    let mut value = serde_json::to_value(&stored.task)?;
    value["revision"] = json!(stored.task.revision().get() + 1);
    let tampered: forge_domain::Task = serde_json::from_value(value)?;
    stored.persistence.task_work_surface_id = Some(Uuid::now_v7());
    assert!(
        transaction
            .update_task(&tampered, stored.persistence, stored.task.revision().get())
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn repository_malformed_sources_and_revision_aliases_reject_before_storage() -> Result<()> {
    let fixture = Fixture::create(BackendKind::Memory).await?;
    for (field, value) in [
        ("source", "/"),
        ("source", "/tmp/../private"),
        ("source", "relative"),
        ("target_ref", "main"),
        ("target_ref", "refs/heads/../secret"),
    ] {
        let mut input = registration("Invalid source");
        input[field] = json!(value);
        let parsed = forge_application::CommandEnvelope::parse(
            CommandName::RegisterProjectRepository,
            forge_protocol::wire::CommandRequest {
                project_id: fixture.project_id.to_string(),
                expected_revision: 1,
                payload: input.as_object().context("payload")?.clone(),
            },
            "invalid",
        );
        assert!(parsed.is_err());
    }
    for alias in ["main", "HEAD", "abcdef", "HEAD^{commit}"] {
        let parsed = forge_application::CommandEnvelope::parse(
            CommandName::BindTaskGitRepository,
            forge_protocol::wire::CommandRequest {
                project_id: fixture.project_id.to_string(),
                expected_revision: 1,
                payload: json!({"task_id":Uuid::now_v7(),"expected_task_revision":1,
                    "repository_id":Uuid::now_v7(),"initial_base":alias})
                .as_object()
                .context("binding payload")?
                .clone(),
            },
            "invalid-base",
        );
        assert!(parsed.is_err());
    }
    Ok(())
}
