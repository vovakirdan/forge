//! Source history changes future Runs without invalidating current Task evidence.
use super::{
    atomicity::assert_faults,
    fixture::{Backend, BackendKind, Fixture},
};
use anyhow::{Context, Result};
use forge_domain::{Actor, ActorId, TaskId};
use forge_protocol::wire::{CommandName, CommandStatus};
use forge_testkit::m0::single_stage_pipeline;
use serde_json::{Value, json};
use uuid::Uuid;

async fn bound(fixture: &Fixture) -> Result<TaskId> {
    let registered = fixture.execute(CommandName::RegisterProjectRepository,
        json!({"name":"Unborn fixture","source":"/tmp/forge-unborn-source","target_ref":"refs/heads/main"})).await?;
    let repository: Uuid = registered.resource.context("repository")?.id.parse()?;
    let version = fixture.pipeline(single_stage_pipeline()).await?;
    let task = fixture.create_task(version, "Future writer").await?;
    fixture.task_command(CommandName::BindTaskGitRepository, task,
        json!({"repository_id":repository,"initial_base":{"kind":"unborn","object_format":"sha1"}})).await?;
    Ok(task)
}

fn payload(task: TaskId, revision: u64, policy: Value) -> Value {
    json!({"task_id":task,"expected_policy_revision":revision,"policy":policy,"reason":"Operator source selection"})
}

pub async fn future_policy_atomicity(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let task_id = bound(&fixture).await?;
    let task = fixture.task(task_id).await?;
    let before = serde_json::to_value(&task)?;
    let revision = fixture.snapshot().await?.projects[&fixture.project_id].revision();
    let command = fixture.envelope(
        CommandName::SetTaskGitSourcePolicy,
        revision,
        payload(
            task_id,
            1,
            json!({"mode":"pinned_commit","commit":"a".repeat(40)}),
        ),
        "set-source-pin",
    );
    assert_faults(&fixture, &command).await?;
    let accepted = fixture.execute_as(&command, &fixture.context).await?;
    let replay = fixture.execute_as(&command, &fixture.context).await?;
    assert_eq!(replay.status, CommandStatus::Replayed);
    assert_eq!(accepted.event_ids, replay.event_ids);
    assert_eq!(serde_json::to_value(fixture.task(task_id).await?)?, before);
    assert_eq!(
        fixture
            .snapshot()
            .await?
            .rows("task_git_source_policies")
            .len(),
        1
    );
    fixture
        .execute(
            CommandName::SetTaskGitSourcePolicy,
            payload(task_id, 2, json!({"mode":"latest_target"})),
        )
        .await?;
    assert_eq!(serde_json::to_value(fixture.task(task_id).await?)?, before);
    assert_eq!(
        fixture
            .snapshot()
            .await?
            .rows("task_git_source_policies")
            .len(),
        2
    );
    if let Backend::Postgres(pool) = &fixture.backend {
        let stored = forge_storage::PostgresStore::from_pool(pool.clone());
        let setting = stored
            .task_git_source_setting(fixture.project_id, task_id)
            .await?;
        assert_eq!(setting.revision, 3);
        assert_eq!(
            setting.policy,
            forge_domain::git::TaskGitSourcePolicy::LatestTarget
        );
        let base: Option<String> = sqlx::query_scalar("SELECT base_sha FROM tasks WHERE id=$1")
            .bind(task_id.as_uuid())
            .fetch_one(pool)
            .await?;
        assert!(base.is_none(), "unborn has no fabricated base commit");
        assert!(
            sqlx::query("DELETE FROM task_git_source_policies WHERE task_id=$1")
                .bind(task_id.as_uuid())
                .execute(pool)
                .await
                .is_err()
        );
    }
    fixture.snapshot().await?.assert_audit_atomic();
    Ok(())
}

pub async fn source_authority_and_staleness(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let task = bound(&fixture).await?;
    let version = fixture.task(task).await?.pipeline().pipeline_version_id();
    let unbound = fixture.create_task(version, "No Git binding").await?;
    let revision = fixture.snapshot().await?.projects[&fixture.project_id].revision();
    let latest = json!({"mode":"latest_target"});
    for (id, expected, policy) in [
        (task, 2, latest.clone()),
        (unbound, 1, latest.clone()),
        (TaskId::new(), 1, latest.clone()),
        (
            task,
            1,
            json!({"mode":"pinned_commit","commit":"b".repeat(64)}),
        ),
    ] {
        let command = fixture.envelope(
            CommandName::SetTaskGitSourcePolicy,
            revision,
            payload(id, expected, policy),
            &Uuid::now_v7().to_string(),
        );
        fixture
            .assert_unchanged_after(&command, &fixture.context)
            .await?;
    }
    let command = fixture.envelope(
        CommandName::SetTaskGitSourcePolicy,
        revision,
        payload(task, 1, latest),
        "source-authority",
    );
    let mut denied = fixture.context.clone();
    // Even an accidentally broad named capability does not turn an Employee into management.
    denied.actor = Actor::employee(ActorId::new());
    fixture.assert_unchanged_after(&command, &denied).await?;
    let mut manager = fixture.context.clone();
    manager.actor = Actor::system_manager(ActorId::new());
    fixture.execute_as(&command, &manager).await?;
    Ok(())
}
