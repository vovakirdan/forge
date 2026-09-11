//! Producer-shaped canonical events; no model, provider credential or raw log import.
use anyhow::{Context, Result};
use forge_domain::{
    Actor, ActorId, AggregateRef, CommandId, DomainEvent, DomainEventInput, DomainEventKind,
    EventId, ProjectId, TaskId, Timestamp,
};
use forge_storage::StoredEvent;
use forge_testkit::m0::{M0Harness, single_stage_pipeline};
use serde_json::{Value, json};
use uuid::Uuid;

async fn task_run(h: &M0Harness) -> Result<(ProjectId, TaskId, Uuid)> {
    let project = h.create_project("Source ownership fixture").await?;
    let pipeline = h.create_pipeline(project, single_stage_pipeline()).await?;
    h.create_employee(project, "Source author").await?;
    let task = h
        .create_task(project, pipeline, "Canonical evidence")
        .await?;
    h.approve_task(project, task).await?;
    h.start_project(project).await?;
    let run = h.wait_for_run_count(task, 1).await?.remove(0).id;
    h.stop_project(project).await?;
    Ok((project, task, run))
}

async fn append(
    h: &M0Harness,
    project: ProjectId,
    kind: DomainEventKind,
    payload: Value,
) -> Result<StoredEvent> {
    let mut tx = h.store.begin().await?;
    let mut owner = tx.lock_project(project).await?.context("Project")?;
    let prior = owner.revision();
    let observed = time::OffsetDateTime::now_utc();
    // PostgreSQL preserves microseconds; keep the fixture's expected value exact.
    let now = Timestamp::from_offset_date_time(
        observed.replace_nanosecond(observed.nanosecond() / 1_000 * 1_000)?,
    );
    owner.record_child_mutation(now)?;
    tx.update_project(&owner, prior).await?;
    let event = DomainEvent::new(
        EventId::new(),
        DomainEventInput {
            project_id: project,
            aggregate: AggregateRef::Project(project),
            aggregate_revision: owner.revision(),
            kind,
            actor: Actor::core(ActorId::new()),
            command_id: CommandId::new(),
            reason: None,
            occurred_at: now,
            schema_version: 1,
            payload,
        },
    )?;
    let stored = tx.append_event_and_outbox(&event).await?;
    tx.commit().await?;
    Ok(stored)
}

async fn ingest(h: &M0Harness, project: ProjectId) -> Result<usize> {
    let mut tx = h.store.begin().await?;
    tx.lock_project(project).await?.context("Project")?;
    let count = tx.ingest_system_job_events(project, 0).await?;
    tx.commit().await?;
    Ok(count)
}

fn assert_source(context: &Value, source: &StoredEvent) -> Result<()> {
    let events = context["events"].as_array().context("events")?;
    let event = events
        .iter()
        .find(|e| e["id"] == json!(source.id))
        .context("event source")?;
    assert_eq!(event["sequence"], source.project_sequence);
    assert_eq!(event["actor"], json!(source.actor));
    let occurred = time::OffsetDateTime::parse(
        event["occurred_at"].as_str().context("source timestamp")?,
        &time::format_description::well_known::Rfc3339,
    )?;
    assert_eq!(occurred, source.occurred_at.as_offset_date_time());
    let payload: Value =
        serde_json::from_str(event["payload_preview"].as_str().context("preview")?)?;
    assert_eq!(payload, source.payload);
    assert!(
        context["source_refs"]
            .as_array()
            .context("source refs")?
            .contains(&json!({"kind":"event","event_id":source.id}))
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; non-executing M0 Supervisor only"]
async fn incident_and_git_producer_shapes_coalesce_and_enter_frozen_sources() -> Result<()> {
    let h = M0Harness::start().await?;
    let _supervisor = h.attach_manual_supervisor().await?;
    let (project, task, run) = task_run(&h).await?;
    let (_, _, foreign_run) = task_run(&h).await?;
    let mut tx = h.store.begin().await?;
    tx.lock_project(project).await?.context("Project")?;
    tx.initialize_system_job_cursor(project).await?;
    tx.commit().await?;

    // These must neither schedule inference nor become summary source text.
    // The private event deliberately carries a direct Task hint as well as a Run.
    for (kind, payload) in [
        (
            DomainEventKind::RunIncidentRaised,
            json!({"run_id":foreign_run,"kind":"foreign-run"}),
        ),
        (
            DomainEventKind::RunIncidentRaised,
            json!({"run_id":"not-a-uuid","kind":"malformed"}),
        ),
        (
            DomainEventKind::RunIncidentRaised,
            json!({"run_id":run,"task_id":task,"scope":{"visibility":"personal","employee_id":Uuid::now_v7()},"kind":"private-marker"}),
        ),
        (
            DomainEventKind::RunEvidenceRecorded,
            json!({"run_id":run,"task_id":task,"stream":"stdout","location":"raw-log-marker","chunk_count":1,"incomplete":false}),
        ),
    ] {
        append(&h, project, kind, payload).await?;
    }
    assert_eq!(ingest(&h, project).await?, 0);
    assert!(h.store.system_jobs(project).await?.is_empty());

    // Existing direct and legacy nested Task payload links remain supported.
    let direct = append(
        &h,
        project,
        DomainEventKind::TaskWaiting,
        json!({"task_id":task,"reason":"waiting for canonical evidence"}),
    )
    .await?;
    let nested = append(
        &h,
        project,
        DomainEventKind::TaskResumed,
        json!({"data":{"task_id":task},"reason":"canonical evidence available"}),
    )
    .await?;
    // Match Core watchdog and Supervisor Git inspection payload layouts.
    let incident = append(&h, project, DomainEventKind::RunIncidentRaised,
        json!({"run_id":run,"incident_id":Uuid::now_v7(),"kind":"stalled","assessment":{"recovery":"inspect"},"quiescent":false})).await?;
    let inspection = append(&h, project, DomainEventKind::GitCandidateInspected,
        json!({"proposal_id":Uuid::now_v7(),"result":{"message_id":Uuid::now_v7(),"command_id":Uuid::now_v7(),"code":"READY","run_id":run,"fencing_token":1,"environment_epoch":1,"commit":"a".repeat(40),"tree":"b".repeat(40),"host_id":"synthetic-host","boot_id":"synthetic-boot"}})).await?;
    assert_eq!(ingest(&h, project).await?, 4);
    assert_eq!(
        ingest(&h, project).await?,
        0,
        "cursor replay duplicates no work"
    );
    let jobs = h.store.system_jobs(project).await?;
    assert_eq!(jobs.len(), 1);
    let job = &jobs[0];
    assert_eq!(job.source_task_id, Some(task));
    assert_eq!(job.covered_sequence, inspection.project_sequence);
    let mut tx = h.store.begin().await?;
    tx.lock_project(project).await?.context("Project")?;
    let context = tx.system_job_source_context(job).await?;
    assert_source(&context, &direct)?;
    assert_source(&context, &nested)?;
    assert_source(&context, &incident)?;
    assert_source(&context, &inspection)?;
    for forbidden in [
        "private-marker",
        "raw-log-marker",
        "foreign-run",
        "malformed",
    ] {
        assert!(!context.to_string().contains(forbidden));
    }
    tx.set_system_job_state(job.id, "completed", None, false)
        .await?;
    tx.commit().await?;

    let late = append(&h, project, DomainEventKind::RunIncidentRaised,
        json!({"run_id":run,"incident_id":Uuid::now_v7(),"kind":"late-stop-evidence","quiescent":true})).await?;
    assert_eq!(ingest(&h, project).await?, 1);
    let latest = h.store.system_jobs(project).await?.remove(0);
    assert_eq!(latest.id, job.id);
    assert_eq!(latest.generation, job.generation + 1);
    assert_eq!(latest.covered_sequence, late.project_sequence);
    let mut tx = h.store.begin().await?;
    tx.lock_project(project).await?.context("Project")?;
    assert_source(&tx.system_job_source_context(&latest).await?, &late)?;
    tx.commit().await?;
    h.shutdown().await;
    Ok(())
}
