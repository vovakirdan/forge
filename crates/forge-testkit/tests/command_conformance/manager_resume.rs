//! Shared alarm commands plus real PostgreSQL deadline execution; no provider involved.
use super::{
    active_runs::running_task,
    atomicity::assert_faults,
    fixture::{Backend, BackendKind, Fixture},
};
use anyhow::{Context, Result};
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use forge_application::{Clock, CommandTransaction};
use forge_domain::{ActorKind, LifecycleStatus, ScheduledResumeState, Timestamp};
use forge_protocol::wire::{CommandName, CommandStatus};
use forge_storage::PostgresStore;
use serde_json::json;
use tower::ServiceExt;
use uuid::Uuid;

async fn paused(kind: BackendKind) -> Result<(Fixture, forge_domain::TaskId)> {
    let fixture = Fixture::create(kind).await?;
    let version = fixture
        .pipeline(forge_testkit::m0::single_stage_pipeline())
        .await?;
    let task = fixture
        .create_task(version, "Explicit delayed continuation")
        .await?;
    fixture
        .task_command(CommandName::ApproveTask, task, json!({}))
        .await?;
    fixture
        .task_command(CommandName::PauseTask, task, json!({"mode":"graceful"}))
        .await?;
    fixture
        .execute(CommandName::StartProjectExecution, json!({}))
        .await?;
    Ok((fixture, task))
}

fn deadline(fixture: &Fixture) -> Timestamp {
    Timestamp::from_offset_date_time(
        fixture.clock.now().as_offset_date_time()
            + time::Duration::hours(1)
            + time::Duration::nanoseconds(123_456_789),
    )
}

fn deadline_json(fixture: &Fixture) -> String {
    deadline(fixture)
        .as_offset_date_time()
        .format(&time::format_description::well_known::Rfc3339)
        .expect("RFC3339")
}

async fn schedule(fixture: &Fixture, task: forge_domain::TaskId) -> Result<Uuid> {
    let snapshot = fixture.task(task).await?;
    let wait = snapshot
        .wait_conditions()
        .find(|wait| matches!(wait.kind(), forge_domain::TaskWaitKind::ManualPause))
        .context("manual wait")?
        .id();
    let receipt=fixture.task_command(CommandName::ScheduleTaskResume,task,
        json!({"wait_condition_id":wait,"not_before":deadline_json(fixture),"reason":"Quota reset requested by owner"})).await?;
    Ok(receipt.resource.context("alarm")?.id.parse()?)
}

#[tokio::test]
#[ignore = "requires local PostgreSQL; validates scoped resume schedule pages"]
async fn resume_schedule_read_tracks_pending_and_cancelled_without_mutating_tasks() -> Result<()> {
    let (fixture, task) = paused(BackendKind::Postgres).await?;
    let first = schedule(&fixture, task).await?;
    let version = fixture.task(task).await?.pipeline().pipeline_version_id();
    let second_task = fixture
        .create_task(version, "Another delayed continuation")
        .await?;
    fixture
        .task_command(CommandName::ApproveTask, second_task, json!({}))
        .await?;
    fixture
        .task_command(
            CommandName::PauseTask,
            second_task,
            json!({"mode":"graceful"}),
        )
        .await?;
    let second = schedule(&fixture, second_task).await?;
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!()
    };
    let router = forge_core::router(fixture.core(pool));
    let read = |uri: String| Request::builder().uri(uri).body(Body::empty());
    let base = format!("/v1/projects/{}/resume-schedules", fixture.project_id);
    let response = router
        .clone()
        .oneshot(read(format!("{base}?limit=1"))?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let page: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await?)?;
    assert_eq!(page["items"].as_array().context("first page")?.len(), 1);
    let cursor = page["next_cursor"].as_str().context("cursor")?;
    let response = router
        .clone()
        .oneshot(read(format!("{base}?limit=1&cursor={cursor}"))?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let next: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await?)?;
    assert_eq!(next["items"].as_array().context("second page")?.len(), 1);
    assert!(next["next_cursor"].is_null());
    let ids = [
        page["items"][0]["id"].clone(),
        next["items"][0]["id"].clone(),
    ];
    assert!(ids.contains(&json!(first)) && ids.contains(&json!(second)));
    assert_eq!(page["items"][0]["state"]["status"], "pending");
    assert!(page["items"][0]["not_before"].as_str().is_some());
    assert!(page["items"][0]["created_at"].as_str().is_some());

    fixture
        .execute(
            CommandName::CancelTaskResume,
            json!({"schedule_id":first,"reason":"Owner changed plan"}),
        )
        .await?;
    let response = router.clone().oneshot(read(base.clone())?).await?;
    let all: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await?)?;
    let cancelled = all["items"]
        .as_array()
        .context("items")?
        .iter()
        .find(|item| item["id"] == json!(first))
        .context("cancelled schedule")?;
    assert_eq!(cancelled["state"]["status"], "cancelled");
    assert!(cancelled["state"]["at"].as_str().is_some());
    assert_eq!(cancelled["task_id"], json!(task));
    assert_eq!(
        router
            .clone()
            .oneshot(read(format!("{base}?limit=51"))?)
            .await?
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        router
            .oneshot(read(format!(
                "/v1/projects/{}/resume-schedules",
                forge_domain::ProjectId::new()
            ))?)
            .await?
            .status(),
        StatusCode::NOT_FOUND
    );
    Ok(())
}

pub async fn alarm_intent_and_cancellation_are_atomic(kind: BackendKind) -> Result<()> {
    let (fixture, task) = paused(kind).await?;
    let before = fixture.task(task).await?;
    let command=fixture.envelope(CommandName::ScheduleTaskResume,fixture.snapshot().await?.projects[&fixture.project_id].revision(),
        json!({"task_id":task,"expected_task_revision":before.revision().get(),"wait_condition_id":before.wait_conditions().next().context("wait")?.id(),"not_before":deadline_json(&fixture),"reason":"Wait for quota"}),"schedule-once");
    assert_faults(&fixture, &command).await?;
    let receipt = fixture.execute_as(&command, &fixture.context).await?;
    assert_eq!(
        fixture.execute_as(&command, &fixture.context).await?.status,
        CommandStatus::Replayed
    );
    assert_eq!(before, fixture.task(task).await?);
    let id: Uuid = receipt.resource.context("schedule")?.id.parse()?;
    let mut duplicate = command.clone();
    duplicate.expected_project_revision =
        fixture.snapshot().await?.projects[&fixture.project_id].revision();
    duplicate.idempotency_key = forge_application::IdempotencyKey::new("duplicate-alarm")?;
    fixture
        .assert_unchanged_after(&duplicate, &fixture.context)
        .await?;
    let cancellation = fixture.envelope(
        CommandName::CancelTaskResume,
        fixture.snapshot().await?.projects[&fixture.project_id].revision(),
        json!({"schedule_id":id,"reason":"Owner changed plan"}),
        "cancel-alarm",
    );
    assert_faults(&fixture, &cancellation).await?;
    fixture.execute_as(&cancellation, &fixture.context).await?;
    assert_eq!(before, fixture.task(task).await?);
    let stored = match &fixture.backend {
        Backend::Memory(store) => store.begin().await.lock_task_resume_schedule(id).await?,
        Backend::Postgres(pool) => {
            PostgresStore::from_pool(pool.clone())
                .begin()
                .await?
                .lock_task_resume_schedule(id)
                .await?
        }
    }
    .context("retained schedule")?;
    assert!(matches!(
        stored.state,
        ScheduledResumeState::Cancelled { .. }
    ));
    assert_eq!(stored.created_by, fixture.context.actor);
    assert_eq!(stored.reason, "Wait for quota");
    fixture.snapshot().await?.assert_audit_atomic();
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL; no provider or Supervisor"]
async fn manager_alarm_is_durable_exactly_once_and_preserves_issuer_audit() -> Result<()> {
    let (fixture, task) = paused(BackendKind::Postgres).await?;
    let id = schedule(&fixture, task).await?;
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!()
    };
    // A newly constructed Core has no in-memory timer registration.
    let unreconciled = fixture.core(pool);
    assert_eq!(
        unreconciled.resume_due_tasks(deadline(&fixture)).await?,
        0,
        "unknown boot inventory cannot consume an alarm"
    );
    let core = fixture.core(pool).with_fake_runtime();
    assert_eq!(core.resume_due_tasks(fixture.clock.now()).await?, 0);
    let just_before = Timestamp::from_offset_date_time(
        deadline(&fixture).as_offset_date_time() - time::Duration::nanoseconds(1),
    );
    assert_eq!(
        core.resume_due_tasks(just_before).await?,
        0,
        "microsecond scan cannot apply a nanosecond-early alarm"
    );
    let before = fixture.task(task).await?;
    let (left, right) = tokio::join!(
        core.resume_due_tasks(deadline(&fixture)),
        core.resume_due_tasks(deadline(&fixture))
    );
    assert_eq!(left? + right?, 1);
    assert_eq!(core.resume_due_tasks(deadline(&fixture)).await?, 0);
    let resumed = fixture.task(task).await?;
    assert_eq!(resumed.lifecycle(), LifecycleStatus::Ready);
    assert_eq!(resumed.revision().get(), before.revision().get() + 1);
    let store = PostgresStore::from_pool(pool.clone());
    let mut tx = store.begin().await?;
    let result = tx.lock_task_resume_schedule(id).await?.context("result")?;
    assert!(matches!(result.state, ScheduledResumeState::Applied { .. }));
    assert_eq!(result.created_by, fixture.context.actor);
    tx.commit().await?;
    let snapshot = fixture.snapshot().await?;
    let event = snapshot
        .rows("event_log")
        .iter()
        .find(|event| event["event_type"] == "task_resumed")
        .context("resume Event")?;
    assert_eq!(event["actor"]["kind"], json!(ActorKind::SystemManager));
    assert_eq!(
        snapshot
            .rows("queue_entries")
            .iter()
            .filter(|q| q["queue_state"] == "queued")
            .count(),
        1
    );
    snapshot.assert_audit_atomic();
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL; no provider or Supervisor"]
async fn manager_alarm_rejects_changed_task_closed_project_and_retained_execution() -> Result<()> {
    for mode in [
        "task_changed",
        "project_stopped",
        "recovery_hold",
        "execution_retained",
    ] {
        let (fixture, task) = if mode == "execution_retained" {
            let setup = running_task(BackendKind::Postgres).await?;
            setup
                .fixture
                .task_command(CommandName::PauseTask, setup.task, json!({"mode":"force"}))
                .await?;
            (setup.fixture, setup.task)
        } else {
            paused(BackendKind::Postgres).await?
        };
        let id = schedule(&fixture, task).await?;
        let Backend::Postgres(pool) = &fixture.backend else {
            unreachable!()
        };
        match mode {
            "task_changed" => {
                fixture
                    .task_command(
                        CommandName::SetTaskPriority,
                        task,
                        json!({"priority":"normal"}),
                    )
                    .await?;
            }
            "project_stopped" => {
                fixture
                    .execute(CommandName::StopProjectExecution, json!({}))
                    .await?;
            }
            "recovery_hold" => {
                let store = PostgresStore::from_pool(pool.clone());
                let mut tx = store.begin().await?;
                tx.set_recovery_hold(fixture.project_id, true).await?;
                tx.commit().await?;
            }
            _ => {}
        }
        let before = fixture.task(task).await?;
        let core = fixture.core(pool).with_fake_runtime();
        assert_eq!(core.resume_due_tasks(deadline(&fixture)).await?, 1);
        assert_eq!(core.resume_due_tasks(deadline(&fixture)).await?, 0);
        assert_eq!(fixture.task(task).await?, before);
        let store = PostgresStore::from_pool(pool.clone());
        let mut tx = store.begin().await?;
        let result = tx.lock_task_resume_schedule(id).await?.context("result")?;
        let state = serde_json::to_value(result.state)?;
        assert_eq!(state["status"], "rejected");
        assert_eq!(state["reason"], mode);
    }
    Ok(())
}
