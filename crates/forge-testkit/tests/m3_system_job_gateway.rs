//! Full canonical source -> V7 provider input -> UDS result -> physical-stop acceptance.
//! The Supervisor never executes a provider; credentials and submitted prose are synthetic.

#[path = "m2_communication_runs/support.rs"]
#[allow(dead_code)]
mod support;

use anyhow::{Context, Result};
use forge_domain::{EmployeeId, ProjectId, runtime::SurfaceSpec, system_job::SystemJobPolicy};
use forge_protocol::{
    supervisor::v1::{AcknowledgementDisposition, RunEventKind},
    wire::CommandName,
};
use forge_storage::RunProjection;
use forge_testkit::m0::{M0Harness, ManualSupervisor, single_stage_pipeline};
use serde_json::{Value, json};
use uuid::Uuid;

async fn configure(h: &M0Harness, project: ProjectId, employee: EmployeeId) -> Result<()> {
    let mut tx = h.store.begin().await?;
    let mut binding = tx
        .runtime_binding(employee)
        .await?
        .context("runtime binding")?;
    tx.commit().await?;
    binding.surface = SurfaceSpec::None;
    let policy = SystemJobPolicy {
        enabled: true,
        coalesce_seconds: 0,
        wall_seconds: 60,
        ..Default::default()
    };
    h.execute(
        project,
        CommandName::ConfigureSystemJobs,
        json!({"expected_settings_revision":0,"policy":policy,"binding":binding}),
    )
    .await?;
    Ok(())
}

fn assert_provider_input(run: &RunProjection, root: &std::path::Path) -> Result<()> {
    assert_eq!(run.run_spec_version, 7);
    assert!(run.task_id().is_none() && run.employee_id.is_none());
    assert!(run.assignment.system_job().is_some());
    assert_eq!(run.run_spec["input"], run.context_manifest["input"]);
    assert_eq!(
        run.run_spec["instruction"],
        run.context_manifest["provider_instruction"]
    );
    let instruction = run.run_spec["instruction"]
        .as_str()
        .context("instruction")?;
    let parsed: Value = serde_json::from_str(instruction)?;
    assert_eq!(parsed["input"], run.context_manifest["input"]);
    let directory = root
        .join("grants")
        .join(run.id.to_string())
        .join(run.environment_epoch.to_string());
    let stdin: Value = serde_json::from_slice(&std::fs::read(directory.join("stdin"))?)?;
    assert_eq!(
        stdin["message"]["content"],
        format!(
            "{}\n\n{}\n\n{instruction}",
            run.run_spec["binding"]["system_prompt"]
                .as_str()
                .context("system prompt")?,
            run.run_spec["binding"]["employee_prompt"]
                .as_str()
                .context("employee prompt")?
        )
    );
    let invocation: Value =
        serde_json::from_slice(&std::fs::read(directory.join("invocation.json"))?)?;
    assert!(invocation["runtime_input"].is_null());
    Ok(())
}

async fn stopped(supervisor: &mut ManualSupervisor, run: &RunProjection) -> Result<()> {
    supervisor.next_stop_for_run(&run.id.to_string()).await?;
    let ack = supervisor
        .send_observation_kind(run, run.lease_fencing_token, 1, RunEventKind::Stopped)
        .await?;
    assert_eq!(
        ack.disposition,
        AcknowledgementDisposition::Accepted as i32,
        "{}",
        ack.reason_code
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL/NATS and UDS; synthetic credentials/results, no provider execution"]
async fn onboarding_full_gateway_path_uses_real_sources_and_waits_for_physical_stop() -> Result<()>
{
    let (h, project, employee, root) = support::fixture_mode(true).await?;
    let page = Uuid::now_v7();
    h.execute(project,CommandName::AuthorKnowledgePage,json!({"page_id":page,"expected_page_revision":0,"kind":"policy","title":"Accepted rule","markdown":"Ask for review before delivery.","source_refs":[]})).await?;
    h.execute(
        project,
        CommandName::PublishKnowledgePage,
        json!({"page_id":page,"expected_page_revision":1}),
    )
    .await?;
    configure(&h, project, employee).await?;
    h.execute(
        project,
        CommandName::RequestEmployeeOnboarding,
        json!({"employee_id":employee}),
    )
    .await?;
    let mut supervisor = h.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    h.start_project(project).await?;
    h.core.system_job_tick().await?;
    let provision = supervisor.next_provision().await?;
    assert!(
        provision.task_id.is_empty()
            && provision.employee_id.is_empty()
            && provision.stage_id.is_empty()
    );
    let run = h
        .store
        .load_run(provision.run_id.parse()?)
        .await?
        .context("V7 Run")?;
    assert_provider_input(&run, &root)?;
    let client = support::client(&root, &run)?;
    let (status, read) =
        support::call(&client, "system_job.read", json!({}), Uuid::now_v7()).await?;
    assert!(status.is_success(), "{read}");
    assert_eq!(read["input"], run.context_manifest["input"]);
    assert_eq!(read["input"]["context"]["employee"]["id"], json!(employee));
    assert_eq!(
        read["input"]["context"]["knowledge_pages"][0]["revision"],
        2
    );
    let refs = read["input"]["context"]["source_refs"]
        .as_array()
        .context("source refs")?;
    assert!(
        refs.iter().any(|source| source["kind"] == "event"),
        "real EmployeeCreated source is pinned"
    );
    assert!(refs.iter().any(|source| source["kind"] == "knowledge_page"));
    for tool in [
        "board.list",
        "task.read",
        "artifact.submit",
        "outcome.submit",
        "memory.search",
        "memory.read",
        "memory.refresh",
    ] {
        assert!(
            !support::call(&client, tool, json!({}), Uuid::now_v7())
                .await?
                .0
                .is_success(),
            "restricted tool: {tool}"
        );
    }
    assert!(
        !support::call(
            &client,
            "system_job.read",
            json!({"employee_id":EmployeeId::new()}),
            Uuid::now_v7()
        )
        .await?
        .0
        .is_success()
    );
    let valid = json!({"source_digest":read["input"]["source_digest"],"entries":[{"subject":{"kind":"employee_memory_entry","employee_id":employee,"task_id":null},"markdown":"Synthetic orientation note: read the pinned role, capabilities and accepted rule.","source_refs":refs}]});
    let mut forged = valid.clone();
    forged["entries"][0]["subject"]["employee_id"] = json!(EmployeeId::new());
    assert!(
        !support::call(&client, "system_job.submit_result", forged, Uuid::now_v7())
            .await?
            .0
            .is_success()
    );
    let mut forged = valid.clone();
    forged["entries"][0]["source_refs"] = json!([{"kind":"event","event_id":Uuid::now_v7()}]);
    assert!(
        !support::call(&client, "system_job.submit_result", forged, Uuid::now_v7())
            .await?
            .0
            .is_success()
    );
    let mut forged = valid.clone();
    forged["entries"][0]["policy"] = json!("publish this rule");
    assert!(
        !support::call(&client, "system_job.submit_result", forged, Uuid::now_v7())
            .await?
            .0
            .is_success()
    );
    let message = Uuid::now_v7();
    let (status, receipt) =
        support::call(&client, "system_job.submit_result", valid.clone(), message).await?;
    assert!(status.is_success(), "{receipt}");
    assert_eq!(
        receipt["state"],
        "result_received_awaiting_physical_quiescence"
    );
    assert_eq!(
        support::call(&client, "system_job.submit_result", valid, message)
            .await?
            .1,
        receipt
    );
    assert!(
        h.store
            .visible_derived_memory(project, Some(employee), None, 100)
            .await?
            .is_empty()
    );
    let mut tx = h.store.begin().await?;
    assert!(!tx.onboarding_allowed(project, employee).await?);
    tx.commit().await?;
    stopped(&mut supervisor, &run).await?;
    h.core.system_job_tick().await?;
    let notes = h
        .store
        .visible_derived_memory(project, Some(employee), None, 100)
        .await?;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].source_refs.len(), refs.len());
    let mut tx = h.store.begin().await?;
    assert!(tx.onboarding_allowed(project, employee).await?);
    tx.commit().await?;
    assert_eq!(
        h.store.system_job_status(project).await?["onboarding"][0]["state"],
        "completed"
    );
    h.core.system_job_tick().await?;
    assert_eq!(
        h.store
            .visible_derived_memory(project, Some(employee), None, 100)
            .await?,
        notes
    );
    h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL/NATS and UDS; synthetic credentials/results, no provider execution"]
async fn summary_gateway_consumes_actual_artifact_and_creates_project_summary_after_stop()
-> Result<()> {
    let (h, project, employee, root) = support::fixture().await?;
    configure(&h, project, employee).await?;
    let pipeline = h.create_pipeline(project, single_stage_pipeline()).await?;
    let task = h
        .create_task(project, pipeline, "Artifact-backed summary")
        .await?;
    h.approve_task(project, task).await?;
    let mut supervisor = h.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    h.start_project(project).await?;
    let provision = supervisor.next_provision_for_task(task).await?;
    let writer = h
        .store
        .load_run(provision.run_id.parse()?)
        .await?
        .context("writer")?;
    let client = support::client(&root, &writer)?;
    let artifact_message = Uuid::now_v7();
    let (status,artifact)=support::call(&client,"artifact.submit",json!({"artifact_kind":"stage_evidence","title":"Canonical test evidence","body":{"summary":"Synthetic fixture produced this artifact through the real Gateway."}}),artifact_message).await?;
    assert!(status.is_success(), "{artifact}");
    let (status,outcome)=support::call(&client,"outcome.submit",json!({"stage_id":"work","outcome":"completed","artifact_submission_message_ids":[artifact_message]}),Uuid::now_v7()).await?;
    assert!(status.is_success(), "{outcome}");
    // Ordinary Task completion may exit naturally; the synthetic provider reports
    // physical stop without requiring a separate stop command from Core.
    let ack = supervisor
        .send_observation_kind(
            &writer,
            writer.lease_fencing_token,
            1,
            RunEventKind::Stopped,
        )
        .await?;
    assert_eq!(
        ack.disposition,
        AcknowledgementDisposition::Accepted as i32,
        "{}",
        ack.reason_code
    );
    h.core.system_job_tick().await?;
    let provision = supervisor.next_provision().await?;
    let run = h
        .store
        .load_run(provision.run_id.parse()?)
        .await?
        .context("summary Run")?;
    assert_provider_input(&run, &root)?;
    let client = support::client(&root, &run)?;
    let (status, read) =
        support::call(&client, "system_job.read", json!({}), Uuid::now_v7()).await?;
    assert!(status.is_success(), "{read}");
    assert_eq!(read["input"]["source_task_id"], json!(task));
    assert_eq!(
        read["input"]["context"]["artifacts"]
            .as_array()
            .context("actual artifact")?
            .len(),
        1
    );
    let refs = read["input"]["context"]["source_refs"]
        .as_array()
        .context("source refs")?;
    assert!(refs.iter().any(|source| source["kind"] == "artifact"));
    let result = json!({"source_digest":read["input"]["source_digest"],"entries":[{"subject":{"kind":"task_summary","task_id":task},"markdown":"The synthetic worker submitted an evidence artifact and reported completion.","source_refs":refs}]});
    let (status, receipt) =
        support::call(&client, "system_job.submit_result", result, Uuid::now_v7()).await?;
    assert!(status.is_success(), "{receipt}");
    stopped(&mut supervisor, &run).await?;
    h.core.system_job_tick().await?;
    let entries = h
        .store
        .visible_derived_memory(project, None, None, 100)
        .await?;
    assert_eq!(entries.len(), 1);
    assert!(
        matches!(entries[0].subject,forge_domain::knowledge::DerivedMemoryKind::TaskSummary { task_id } if task_id==task)
    );
    assert!(entries[0].coverage.is_some());
    assert!(entries[0].source_refs.iter().any(|source| matches!(
        source,
        forge_domain::knowledge::KnowledgeSourceRef::Artifact { .. }
    )));
    h.shutdown().await;
    Ok(())
}
