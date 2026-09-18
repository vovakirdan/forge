//! Priority commands do not replace lifecycle, Pipeline policy or physical ownership.

use anyhow::Result;
use forge_domain::{LifecycleStatus, PriorityLevel, PriorityLevelId, PriorityScheme};
use forge_protocol::wire::CommandName;
use forge_testkit::m0::single_stage_pipeline;
use serde_json::json;

use super::{
    active_runs::running_task,
    commands::{complete_human, edge, human_evidence_pipeline, reject},
    fixture::{BackendKind, Fixture, base_time},
};

pub async fn lifecycle_queue_and_replay(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let pipeline = fixture.pipeline(single_stage_pipeline()).await?;
    let draft = fixture.create_task(pipeline, "Draft priority").await?;
    let ready = fixture.create_task(pipeline, "Queued priority").await?;
    let waiting = fixture.create_task(pipeline, "Blocked priority").await?;
    fixture
        .execute(CommandName::CreateDependency, edge(draft, waiting))
        .await?;
    fixture
        .task_command(CommandName::ApproveTask, waiting, json!({}))
        .await?;
    fixture
        .task_command(CommandName::ApproveTask, ready, json!({}))
        .await?;
    for (task_id, lifecycle) in [
        (draft, LifecycleStatus::Draft),
        (ready, LifecycleStatus::Ready),
        (waiting, LifecycleStatus::Waiting),
    ] {
        let before = fixture.snapshot().await?;
        let task = &before.tasks[&task_id];
        assert_eq!(task.lifecycle(), lifecycle);
        let envelope = fixture.envelope(
            CommandName::SetTaskPriority,
            before.projects[&fixture.project_id].revision(),
            json!({
                "task_id":task_id,"expected_task_revision":task.revision().get(),"priority":"high"
            }),
            &format!("priority-{task_id}"),
        );
        let receipt = fixture.execute_as(&envelope, &fixture.context).await?;
        let after = fixture.snapshot().await?;
        let changed = &after.tasks[&task_id];
        assert_eq!(changed.lifecycle(), lifecycle);
        assert_eq!(changed.current_stage_id(), task.current_stage_id());
        assert_eq!(changed.revision().get(), task.revision().get() + 1);
        assert_eq!(changed.priority_level_id().as_str(), "high");
        assert_eq!(
            receipt.project_revision,
            before.projects[&fixture.project_id].revision() + 1
        );
        assert_eq!(
            after.rows("event_log").len(),
            before.rows("event_log").len() + 1
        );
        assert!(
            after
                .rows("event_log")
                .iter()
                .any(|event| event["id"] == receipt.event_ids[0]
                    && event["event_type"] == "task_priority_changed")
        );
        let replay = fixture.execute_as(&envelope, &fixture.context).await?;
        assert_eq!(replay.command_id, receipt.command_id);
        assert_eq!(fixture.snapshot().await?.raw, after.raw);
        let active_queue = after
            .rows("queue_entries")
            .iter()
            .filter(|entry| entry["task_id"] == json!(task_id) && entry["queue_state"] == "queued")
            .collect::<Vec<_>>();
        if lifecycle == LifecycleStatus::Ready {
            assert_eq!(active_queue.len(), 1);
            assert_eq!(active_queue[0]["priority_level_id"], "high");
            assert_eq!(active_queue[0]["priority_rank"], 100);
            assert_eq!(active_queue[0]["task_revision"], changed.revision().get());
        } else {
            assert!(active_queue.is_empty());
        }
        after.assert_audit_atomic();
    }

    let human_pipeline = fixture.pipeline(human_evidence_pipeline()).await?;
    let done = fixture.create_task(human_pipeline, "Done priority").await?;
    fixture
        .task_command(CommandName::ApproveTask, done, json!({}))
        .await?;
    complete_human(&fixture, done, "accepted").await?;
    fixture
        .task_command(
            CommandName::CancelTask,
            draft,
            json!({"cancellation_reason_key":"unspecified"}),
        )
        .await?;
    for task_id in [done, draft] {
        reject(&fixture, CommandName::SetTaskPriority, json!({
            "task_id":task_id,"expected_task_revision":fixture.task(task_id).await?.revision().get(),"priority":"low"
        })).await?;
    }
    for (revision, priority) in [
        (fixture.task(ready).await?.revision().get(), "unknown"),
        (1, "low"),
    ] {
        reject(
            &fixture,
            CommandName::SetTaskPriority,
            json!({
                "task_id":ready,"expected_task_revision":revision,"priority":priority
            }),
        )
        .await?;
    }
    // No public scheme mutation exists; retired-level policy is exercised directly,
    // without altering a canonical Project using an unsupported test-only command.
    let mut retired = PriorityLevel::new(PriorityLevelId::new("old")?, "Old", 1)?;
    retired.retire();
    let scheme = PriorityScheme::new(
        [
            retired,
            PriorityLevel::new(PriorityLevelId::new("active")?, "Active", 2)?,
        ],
        PriorityLevelId::new("active")?,
    )?;
    let mut task = fixture.task(ready).await?;
    let before = task.clone();
    assert!(
        task.set_priority(&scheme, PriorityLevelId::new("old")?, base_time())
            .is_err()
    );
    assert_eq!(serde_json::to_value(task)?, serde_json::to_value(before)?);
    Ok(())
}

pub async fn active_run_is_unchanged(kind: BackendKind) -> Result<()> {
    let running = running_task(kind).await?;
    let fixture = &running.fixture;
    let before = fixture.snapshot().await?;
    let task = &before.tasks[&running.task];
    assert_eq!(task.lifecycle(), LifecycleStatus::InProgress);
    fixture
        .task_command(
            CommandName::SetTaskPriority,
            running.task,
            json!({"priority":"high"}),
        )
        .await?;
    let after = fixture.snapshot().await?;
    assert_eq!(after.tasks[&running.task].lifecycle(), task.lifecycle());
    assert_eq!(
        after.tasks[&running.task].current_stage_id(),
        task.current_stage_id()
    );
    assert_eq!(
        after.tasks[&running.task].priority_level_id().as_str(),
        "high"
    );
    for name in [
        "runs",
        "queue_entries",
        "leases",
        "run_environment_reservations",
    ] {
        assert_eq!(
            before.raw.get(name),
            after.raw.get(name),
            "{name} ownership changed"
        );
    }
    after.assert_audit_atomic();
    Ok(())
}
