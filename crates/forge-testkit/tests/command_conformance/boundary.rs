use anyhow::Result;
use forge_application::{CommandContext, CommandError};
use forge_domain::{Actor, DomainError, ProjectId};
use forge_protocol::wire::{CommandName, CommandStatus};
use forge_testkit::m0::single_stage_pipeline;
use serde_json::json;
use uuid::Uuid;

use super::{
    compatibility::CREATE_HASH,
    fixture::{BackendKind, Fixture},
};

pub async fn idempotency(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::new(kind).await?;
    let envelope = fixture.envelope(
        CommandName::CreateProject,
        0,
        json!({"name":"Legacy project"}),
        "legacy-create",
    );
    let applied = fixture.execute_as(&envelope, &fixture.context).await?;
    let snapshot = fixture.snapshot().await?;
    assert_eq!(applied.status, CommandStatus::Applied);
    assert_eq!(
        snapshot.rows("idempotency_keys")[0]["request_hash"],
        CREATE_HASH
    );
    let replay = fixture.execute_as(&envelope, &fixture.context).await?;
    assert_replay(&applied, &replay);
    assert_eq!(snapshot.raw, fixture.snapshot().await?.raw);
    let start = fixture.envelope(
        CommandName::StartProjectExecution,
        1,
        json!({}),
        "start-command",
    );
    fixture.execute_as(&start, &fixture.context).await?;
    let later = fixture.snapshot().await?.raw;
    assert_replay(
        &applied,
        &fixture.execute_as(&envelope, &fixture.context).await?,
    );
    assert_eq!(later, fixture.snapshot().await?.raw);
    for (name, revision, payload) in [
        (CommandName::CreateProject, 0, json!({"name":"Different"})),
        (CommandName::StartProjectExecution, 0, json!({})),
        (CommandName::StartProjectExecution, 1, json!({})),
    ] {
        let conflicting = fixture.envelope(name, revision, payload, "legacy-create");
        assert!(matches!(
            fixture
                .assert_unchanged_after(&conflicting, &fixture.context)
                .await?,
            CommandError::IdempotencyConflict
        ));
    }
    for (revision, payload) in [(2, json!({})), (1, json!({"reason":null}))] {
        let conflicting = fixture.envelope(
            CommandName::StartProjectExecution,
            revision,
            payload,
            "start-command",
        );
        assert!(matches!(
            fixture
                .assert_unchanged_after(&conflicting, &fixture.context)
                .await?,
            CommandError::IdempotencyConflict
        ));
    }
    let mut other_actor = fixture.context.clone();
    other_actor.actor = Actor::human(Uuid::now_v7().into());
    assert!(matches!(
        fixture
            .assert_unchanged_after(&envelope, &other_actor)
            .await?,
        CommandError::IdempotencyConflict
    ));
    fixture.snapshot().await?.assert_audit_atomic();
    Ok(())
}

pub async fn revisions(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let stale = fixture.envelope(
        CommandName::StartProjectExecution,
        0,
        json!({}),
        "stale-project",
    );
    assert!(matches!(
        fixture
            .assert_unchanged_after(&stale, &fixture.context)
            .await?,
        CommandError::Domain(DomainError::StaleProjectRevision {
            expected: 0,
            actual: 1
        })
    ));
    let pipeline = fixture.pipeline(single_stage_pipeline()).await?;
    let task = fixture.create_task(pipeline, "Revision boundary").await?;
    fixture
        .task_command(
            CommandName::AmendDraft,
            task,
            json!({"patch":{"title":"New revision"}}),
        )
        .await?;
    let revision = fixture.snapshot().await?.projects[&fixture.project_id].revision();
    let stale_task = fixture.envelope(
        CommandName::ApproveTask,
        revision,
        json!({"task_id":task,"expected_task_revision":1}),
        "stale-task",
    );
    assert!(matches!(
        fixture
            .assert_unchanged_after(&stale_task, &fixture.context)
            .await?,
        CommandError::InvalidTransport {
            field: "expected_task_revision",
            ..
        }
    ));
    let valid = fixture.envelope(
        CommandName::ApproveTask,
        revision,
        json!({"task_id":task,"expected_task_revision":fixture.task(task).await?.revision().get()}),
        "stale-task",
    );
    fixture.execute_as(&valid, &fixture.context).await?;
    let missing = fixture.envelope(
        CommandName::SetTaskPriority,
        fixture.snapshot().await?.projects[&fixture.project_id].revision(),
        json!({"task_id":Uuid::now_v7(),"expected_task_revision":1,"priority":"high"}),
        "missing-task",
    );
    assert!(matches!(
        fixture
            .assert_unchanged_after(&missing, &fixture.context)
            .await?,
        CommandError::NotFound { .. }
    ));
    Ok(())
}

pub async fn authorization(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let original = fixture.envelope(
        CommandName::StartProjectExecution,
        1,
        json!({}),
        "authorized",
    );
    let mut contexts = Vec::new();
    let mut missing_capability = fixture.context.clone();
    missing_capability.capabilities.clear();
    contexts.push(missing_capability);
    contexts.push(CommandContext::local_human(
        fixture.project_id,
        Actor::employee(Uuid::now_v7().into()),
        fixture.context.core_actor,
    ));
    let mut wrong_project = fixture.context.clone();
    wrong_project.project_id = ProjectId::new();
    contexts.push(wrong_project);
    let mut forged_core = fixture.context.clone();
    forged_core.core_actor = fixture.context.actor;
    contexts.push(forged_core);
    for context in &contexts {
        assert!(matches!(
            fixture.assert_unchanged_after(&original, context).await?,
            CommandError::Forbidden
        ));
    }
    let applied = fixture.execute_as(&original, &fixture.context).await?;
    for context in &contexts {
        // A cached receipt never bypasses revoked capability or project scope.
        assert!(matches!(
            fixture.assert_unchanged_after(&original, context).await?,
            CommandError::Forbidden
        ));
    }
    assert_replay(
        &applied,
        &fixture.execute_as(&original, &fixture.context).await?,
    );

    let mut manager = fixture.context.clone();
    manager.actor = Actor::system_manager(Uuid::now_v7().into());
    manager.capabilities = vec![CommandName::StopProjectExecution];
    let stop = fixture.envelope(
        CommandName::StopProjectExecution,
        2,
        json!({}),
        "manager-stop",
    );
    fixture.execute_as(&stop, &manager).await?;
    let snapshot = fixture.snapshot().await?;
    assert_eq!(
        snapshot.rows("event_log").last().expect("manager audit")["actor"],
        json!(manager.actor)
    );
    let forbidden = fixture.envelope(
        CommandName::StartProjectExecution,
        3,
        json!({}),
        "manager-start",
    );
    assert!(matches!(
        fixture.assert_unchanged_after(&forbidden, &manager).await?,
        CommandError::Forbidden
    ));
    let mut delegated = fixture.context.clone();
    delegated.actor = Actor::employee(Uuid::now_v7().into());
    delegated.capabilities = vec![CommandName::StartProjectExecution];
    fixture.execute_as(&forbidden, &delegated).await?;
    assert_eq!(
        fixture
            .snapshot()
            .await?
            .rows("event_log")
            .last()
            .expect("delegated audit")["actor"],
        json!(delegated.actor)
    );
    Ok(())
}

pub async fn concurrency(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::new(kind).await?;
    let envelope = fixture.envelope(
        CommandName::CreateProject,
        0,
        json!({"name":"Race"}),
        "same-key",
    );
    let (left, right) = tokio::join!(
        fixture.execute_as(&envelope, &fixture.context),
        fixture.execute_as(&envelope, &fixture.context)
    );
    let (left, right) = (left?, right?);
    assert!(matches!(
        (left.status, right.status),
        (CommandStatus::Applied, CommandStatus::Replayed)
            | (CommandStatus::Replayed, CommandStatus::Applied)
    ));
    assert_eq!(left.command_id, right.command_id);
    assert_eq!(left.event_ids, right.event_ids);
    let first = fixture.envelope(
        CommandName::StartProjectExecution,
        1,
        json!({}),
        "race-first",
    );
    let second = fixture.envelope(
        CommandName::StopProjectExecution,
        1,
        json!({}),
        "race-second",
    );
    let (left, right) = tokio::join!(
        fixture.execute_as(&first, &fixture.context),
        fixture.execute_as(&second, &fixture.context)
    );
    let (success, failure) = match (left, right) {
        (Ok(success), Err(failure)) | (Err(failure), Ok(success)) => (success, failure),
        result => panic!("exactly one revision winner required: {result:?}"),
    };
    assert_eq!(success.project_revision, 2);
    assert!(matches!(
        failure,
        CommandError::Domain(DomainError::StaleProjectRevision {
            expected: 1,
            actual: 2
        })
    ));
    let snapshot = fixture.snapshot().await?;
    assert_eq!(snapshot.rows("idempotency_keys").len(), 2);
    assert_eq!(snapshot.rows("event_log").len(), 2);
    snapshot.assert_audit_atomic();
    Ok(())
}

fn assert_replay(
    applied: &forge_protocol::wire::CommandReceipt,
    replay: &forge_protocol::wire::CommandReceipt,
) {
    let mut expected = serde_json::to_value(applied).expect("applied receipt");
    expected["status"] = json!("replayed");
    assert_eq!(
        serde_json::to_value(replay).expect("replay receipt"),
        expected
    );
}
