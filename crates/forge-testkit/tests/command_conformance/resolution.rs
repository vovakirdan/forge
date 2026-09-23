//! Resolver commands share one transaction engine and preserve Task-stage authority.
use super::{
    atomicity::assert_faults,
    employees,
    fixture::{BackendKind, Fixture},
};
use anyhow::{Context, Result};
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use forge_domain::{
    Actor, ActorId, ArtifactProducer, LifecycleStatus, TaskId, TaskWaitKind, resolution::*,
};
use forge_protocol::wire::{CommandName, CommandStatus};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

pub(super) async fn setup(kind: BackendKind) -> Result<(Fixture, TaskId)> {
    let fixture = Fixture::create(kind).await?;
    let version = fixture
        .pipeline(forge_testkit::m0::single_stage_pipeline())
        .await?;
    let task = fixture
        .create_task(version, "Resolve a question, not a stage")
        .await?;
    fixture
        .task_command(CommandName::ApproveTask, task, json!({}))
        .await?;
    Ok((fixture, task))
}
fn canonical(row: &Value) -> &Value {
    row.get("canonical_snapshot").unwrap_or(row)
}
pub(super) async fn escalation(fixture: &Fixture, id: Uuid) -> Result<Escalation> {
    let snapshot = fixture.snapshot().await?;
    let value = snapshot
        .rows("escalations")
        .iter()
        .map(canonical)
        .find(|value| value["id"] == json!(id))
        .context("escalation")?;
    Ok(serde_json::from_value(value.clone())?)
}
async fn answer_payload(fixture: &Fixture, id: Uuid, disposition: &str) -> Result<Value> {
    let value = escalation(fixture, id).await?;
    let EscalationState::Assigned { assignment_id } = value.state else {
        anyhow::bail!("expected assignment")
    };
    Ok(
        json!({"escalation_id":id,"expected_escalation_revision":value.revision,"assignment_id":assignment_id,"lease_generation":value.generation,
        "answer":{"disposition":disposition,"summary":"Documented technical answer"}}),
    )
}
pub(super) async fn raise(fixture: &Fixture, task: TaskId, extra: Value) -> Result<Uuid> {
    let mut input =
        json!({"category":"technical_decision","question":"Which documented approach applies?"});
    input
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    Ok(fixture
        .task_command(CommandName::RaiseEscalation, task, input)
        .await?
        .resource
        .context("escalation ref")?
        .id
        .parse()?)
}

#[tokio::test]
#[ignore = "requires local PostgreSQL; validates scoped escalation pages and assignment receipts"]
async fn escalation_read_tracks_queue_assignment_and_resolution() -> Result<()> {
    let (fixture, task) = setup(BackendKind::Postgres).await?;
    let first = raise(&fixture, task, json!({})).await?;
    let version = fixture.task(task).await?.pipeline().pipeline_version_id();
    let second_task = fixture
        .create_task(version, "Another resolution question")
        .await?;
    fixture
        .task_command(CommandName::ApproveTask, second_task, json!({}))
        .await?;
    let bob = employees::create(&fixture, "Bob").await?;
    fixture
        .execute(
            CommandName::ConfigureResolverRoute,
            json!({"route_key":"engineering","employee_ids":[bob],"assignment_timeout_seconds":60}),
        )
        .await?;
    let assigned = raise(&fixture, second_task, json!({"route_key":"engineering"})).await?;
    let before = escalation(&fixture, assigned).await?;
    fixture.execute(CommandName::RerouteEscalation, json!({"escalation_id":assigned,"expected_escalation_revision":before.revision,"reason":"Assign resolver"})).await?;

    let super::fixture::Backend::Postgres(pool) = &fixture.backend else {
        unreachable!()
    };
    let router = forge_core::router(fixture.core(pool));
    let base = format!("/v1/projects/{}/escalations", fixture.project_id);
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("{base}?limit=1"))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let page: Value = serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await?)?;
    let cursor = page["next_cursor"].as_str().context("cursor")?;
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("{base}?limit=1&cursor={cursor}"))
                .body(Body::empty())?,
        )
        .await?;
    let next: Value = serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await?)?;
    assert!(next["next_cursor"].is_null());
    let items = [&page["items"][0], &next["items"][0]];
    let first_assignment = items
        .iter()
        .find(|item| item["id"] == json!(first))
        .context("first escalation")?;
    assert_eq!(first_assignment["state"]["status"], "assigned");
    assert_eq!(
        first_assignment["latest_assignment"]["state"]["status"],
        "active"
    );
    let assignment = items
        .iter()
        .find(|item| item["id"] == json!(assigned))
        .context("assigned")?;
    assert_eq!(assignment["state"]["status"], "assigned");
    assert_eq!(assignment["latest_assignment"]["state"]["status"], "active");
    assert_eq!(assignment["source"]["kind"], "task");
    assert!(assignment["created_at"].as_str().is_some());
    assert!(
        assignment["latest_assignment"]["issued_at"]
            .as_str()
            .is_some()
    );
    assert!(
        serde_json::to_string(assignment)?
            .find("fencing_token")
            .is_none()
    );

    fixture
        .execute(
            CommandName::SubmitHumanResolution,
            answer_payload(&fixture, assigned, "continue_stage").await?,
        )
        .await?;
    let response = router
        .clone()
        .oneshot(Request::builder().uri(base.clone()).body(Body::empty())?)
        .await?;
    let all: Value = serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await?)?;
    let resolved = all["items"]
        .as_array()
        .context("items")?
        .iter()
        .find(|item| item["id"] == json!(assigned))
        .context("resolved")?;
    assert_eq!(resolved["state"]["status"], "resolved");
    assert_eq!(resolved["latest_assignment"]["state"]["status"], "answered");
    assert_eq!(
        router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("{base}?limit=21"))
                    .body(Body::empty())?
            )
            .await?
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        router
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/v1/projects/{}/escalations",
                        forge_domain::ProjectId::new()
                    ))
                    .body(Body::empty())?
            )
            .await?
            .status(),
        StatusCode::NOT_FOUND
    );
    Ok(())
}

pub async fn human_answer_is_atomic_and_only_clears_its_own_wait(kind: BackendKind) -> Result<()> {
    let (fixture, task) = setup(kind).await?;
    fixture
        .task_command(CommandName::PauseTask, task, json!({"mode":"graceful"}))
        .await?;
    let before = fixture.task(task).await?;
    let envelope=fixture.envelope(CommandName::RaiseEscalation,fixture.snapshot().await?.projects[&fixture.project_id].revision(),json!({"task_id":task,"expected_task_revision":before.revision().get(),"category":"clarification","question":"What should we do?"}),"raise-once");
    assert_faults(&fixture, &envelope).await?;
    let receipt = fixture.execute_as(&envelope, &fixture.context).await?;
    assert_eq!(
        fixture
            .execute_as(&envelope, &fixture.context)
            .await?
            .status,
        CommandStatus::Replayed
    );
    let id = receipt.resource.context("escalation")?.id.parse()?;
    let waiting = fixture.task(task).await?;
    assert_eq!(waiting.wait_conditions().count(), 2);
    assert_eq!(waiting.current_stage_visit(), before.current_stage_visit());
    let payload = answer_payload(&fixture, id, "continue_stage").await?;
    let answer = fixture.envelope(
        CommandName::SubmitHumanResolution,
        fixture.snapshot().await?.projects[&fixture.project_id].revision(),
        payload,
        "answer-once",
    );
    assert_faults(&fixture, &answer).await?;
    fixture.execute_as(&answer, &fixture.context).await?;
    assert_eq!(
        fixture.execute_as(&answer, &fixture.context).await?.status,
        CommandStatus::Replayed
    );
    let resolved = fixture.task(task).await?;
    assert_eq!(resolved.lifecycle(), LifecycleStatus::Waiting);
    assert_eq!(resolved.wait_conditions().count(), 1);
    assert_eq!(
        resolved.wait_conditions().next().unwrap().kind(),
        &TaskWaitKind::ManualPause
    );
    assert_eq!(resolved.artifact_links().len(), 1);
    assert_eq!(
        resolved.artifact_links()[0].producer(),
        ArtifactProducer::ResolutionAssignment
    );
    assert_eq!(resolved.current_stage_id(), before.current_stage_id());
    assert!(matches!(
        escalation(&fixture, id).await?.state,
        EscalationState::Resolved { .. }
    ));
    let snapshot = fixture.snapshot().await?;
    assert!(
        snapshot
            .rows("queue_entries")
            .iter()
            .all(|entry| entry["queue_state"] != "queued")
    );
    assert_eq!(snapshot.tasks.len(), 1);
    snapshot.assert_audit_atomic();
    Ok(())
}

pub async fn routes_are_pinned_and_human_generations_fence_late_answers(
    kind: BackendKind,
) -> Result<()> {
    let (fixture, task) = setup(kind).await?;
    let bob = employees::create(&fixture, "Bob").await?;
    let zoe = employees::create(&fixture, "Zoe").await?;
    fixture
        .execute(
            CommandName::ConfigureResolverRoute,
            json!({"route_key":"engineering","employee_ids":[bob],"assignment_timeout_seconds":60}),
        )
        .await?;
    let id = raise(&fixture, task, json!({"route_key":"engineering"})).await?;
    assert_eq!(
        escalation(&fixture, id).await?.state,
        EscalationState::Queued
    );
    fixture.execute(CommandName::ConfigureResolverRoute,json!({"route_key":"engineering","employee_ids":[zoe],"assignment_timeout_seconds":120})).await?;
    let frozen = escalation(&fixture, id).await?.route.context("route")?;
    assert_eq!((frozen.revision, frozen.employee_ids), (1, vec![bob]));
    let before = escalation(&fixture, id).await?;
    fixture.execute(CommandName::RerouteEscalation,json!({"escalation_id":id,"expected_escalation_revision":before.revision,"reason":"Owner takes this question"})).await?;
    let old_answer = answer_payload(&fixture, id, "continue_stage").await?;
    let before = escalation(&fixture, id).await?;
    fixture.execute(CommandName::RerouteEscalation,json!({"escalation_id":id,"expected_escalation_revision":before.revision,"reason":"Replace prior handoff"})).await?;
    assert!(
        fixture
            .execute(CommandName::SubmitHumanResolution, old_answer)
            .await
            .is_err()
    );
    let payload = answer_payload(&fixture, id, "needs_management_change").await?;
    fixture
        .execute(CommandName::SubmitHumanResolution, payload)
        .await?;
    assert_eq!(
        fixture.task(task).await?.lifecycle(),
        LifecycleStatus::Waiting
    );
    assert!(matches!(
        escalation(&fixture, id).await?.state,
        EscalationState::NeedsManagementChange { .. }
    ));
    let snapshot = fixture.snapshot().await?;
    assert_eq!(snapshot.rows("resolution_assignments").len(), 2);
    snapshot.assert_audit_atomic();
    Ok(())
}

pub async fn dangerous_questions_and_refusals_preserve_authority(kind: BackendKind) -> Result<()> {
    let (fixture, task) = setup(kind).await?;
    let bob = employees::create(&fixture, "Bob").await?;
    fixture
        .execute(
            CommandName::ConfigureResolverRoute,
            json!({"route_key":"engineering","employee_ids":[bob],"assignment_timeout_seconds":60}),
        )
        .await?;
    let id=raise(&fixture,task,json!({"route_key":"engineering","category":"action_approval","question":"May the database be removed?"})).await?;
    let snapshot = fixture.snapshot().await?;
    assert_eq!(
        canonical(&snapshot.rows("resolution_assignments")[0])["resolver"]["kind"],
        "human"
    );
    assert!(
        canonical(&snapshot.rows("resolution_assignments")[0])["lease"]["expires_at"].is_null()
    );
    let valid = answer_payload(&fixture, id, "continue_stage").await?;
    for variant in 0..3 {
        let mut payload = valid.clone();
        match variant {
            0 => {
                payload["answer"]["recommended_outcome_key"] = json!("invented-planning");
            }
            1 => {
                payload["assignment_id"] = json!(Uuid::now_v7());
            }
            _ => {
                payload["lease_generation"] = json!(999);
            }
        }
        let envelope = fixture.envelope(
            CommandName::SubmitHumanResolution,
            fixture.snapshot().await?.projects[&fixture.project_id].revision(),
            payload,
            &Uuid::now_v7().to_string(),
        );
        fixture
            .assert_unchanged_after(&envelope, &fixture.context)
            .await?;
    }
    let mut actor = fixture.context.clone();
    actor.actor = Actor::employee(ActorId::from(bob.as_uuid()));
    actor.capabilities = vec![CommandName::SubmitHumanResolution];
    let envelope = fixture.envelope(
        CommandName::SubmitHumanResolution,
        fixture.snapshot().await?.projects[&fixture.project_id].revision(),
        valid,
        "employee-cannot-act-human",
    );
    fixture.assert_unchanged_after(&envelope, &actor).await?;
    fixture
        .task_command(
            CommandName::CancelTask,
            task,
            json!({"cancellation_reason_key":"unspecified"}),
        )
        .await?;
    let cancelled = fixture.task(task).await?;
    let payload = answer_payload(&fixture, id, "continue_stage").await?;
    assert!(
        fixture
            .execute(CommandName::SubmitHumanResolution, payload)
            .await
            .is_err()
    );
    assert_eq!(cancelled, fixture.task(task).await?);
    Ok(())
}

pub async fn active_source_cannot_be_resumed_by_a_human_answer(kind: BackendKind) -> Result<()> {
    let setup = super::active_runs::running_task(kind).await?;
    let id = raise(&setup.fixture, setup.task, json!({})).await?;
    let before = setup.fixture.snapshot().await?;
    let answer = answer_payload(&setup.fixture, id, "continue_stage").await?;
    assert!(
        setup
            .fixture
            .execute(CommandName::SubmitHumanResolution, answer)
            .await
            .is_err()
    );
    assert_eq!(before.raw, setup.fixture.snapshot().await?.raw);
    let answer = answer_payload(&setup.fixture, id, "needs_management_change").await?;
    setup
        .fixture
        .execute(CommandName::SubmitHumanResolution, answer)
        .await?;
    assert_eq!(
        setup.fixture.task(setup.task).await?.lifecycle(),
        LifecycleStatus::Waiting
    );
    Ok(())
}

pub async fn final_question_wait_resumes_same_stage_without_completing_it(
    kind: BackendKind,
) -> Result<()> {
    let (fixture, task) = setup(kind).await?;
    let original = fixture.task(task).await?;
    let id = raise(&fixture, task, json!({})).await?;
    let payload = answer_payload(&fixture, id, "continue_stage").await?;
    fixture
        .execute(CommandName::SubmitHumanResolution, payload)
        .await?;
    let current = fixture.task(task).await?;
    assert_eq!(current.lifecycle(), LifecycleStatus::Ready);
    assert_eq!(
        current.current_stage_visit(),
        original.current_stage_visit()
    );
    assert_eq!(current.current_stage_id(), original.current_stage_id());
    assert_eq!(current.wait_conditions().count(), 0);
    let before = fixture.snapshot().await?;
    assert_eq!(
        before
            .rows("queue_entries")
            .iter()
            .filter(|entry| entry["queue_state"] == "queued")
            .count(),
        1
    );
    if let super::fixture::Backend::Postgres(pool) = &fixture.backend {
        assert!(sqlx::query("UPDATE escalations SET canonical_snapshot=jsonb_set(canonical_snapshot,'{question}','\"replaced\"'::jsonb) WHERE id=$1").bind(id).execute(pool).await.is_err());
        assert!(
            sqlx::query("DELETE FROM resolution_assignments WHERE escalation_id=$1")
                .bind(id)
                .execute(pool)
                .await
                .is_err()
        );
        assert!(
            sqlx::query(
                "UPDATE resolution_assignments SET assignment_state='active' WHERE escalation_id=$1"
            )
            .bind(id)
            .execute(pool)
            .await
            .is_err()
        );
        assert_eq!(before.raw, fixture.snapshot().await?.raw);
    }
    Ok(())
}
