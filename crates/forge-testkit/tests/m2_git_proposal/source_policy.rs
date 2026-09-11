use super::*;
use forge_domain::{ProjectId, TaskId};
use forge_storage::RunProjection;
use forge_testkit::m0::ManualSupervisor;

pub(super) async fn freeze_and_change(
    harness: &M0Harness,
    supervisor: &mut ManualSupervisor,
    project: ProjectId,
    task: TaskId,
    run: &RunProjection,
) -> Result<()> {
    assert_eq!(run.run_spec_version, 6);
    let spec = run.run_spec.clone();
    validate_version_boundary(&spec)?;
    let request = &spec["source_request"];
    assert_eq!(request["policy_revision"], 1);
    assert_eq!(request["policy"]["mode"], "latest_target");
    let descriptor = json!({"schema_version":1,"repository_id":request["repository_id"],"target_ref":request["target_ref"],
        "policy_revision":1,"policy":{"mode":"latest_target"},"selected_revision":"d".repeat(40),"object_format":"sha1",
        "bundle_file":"source.bundle","bundle_sha256":"e".repeat(64),"bundle_bytes":100});
    let ack = supervisor
        .send_observation_details(
            run,
            run.lease_fencing_token,
            1,
            RunEventKind::Provisioning,
            json!({"git_source":descriptor}),
        )
        .await?;
    assert_eq!(
        ack.disposition,
        AcknowledgementDisposition::Accepted as i32,
        "{ack:?}"
    );
    let revision = harness
        .store
        .load_task(task)
        .await?
        .context("Task")?
        .task
        .revision();
    harness.execute(project,CommandName::SetTaskGitSourcePolicy,json!({"task_id":task,"expected_policy_revision":1,"policy":{"mode":"pinned_commit","commit":"a".repeat(40)},"reason":"Future run input only"})).await?;
    assert_eq!(
        harness
            .store
            .load_task(task)
            .await?
            .context("Task")?
            .task
            .revision(),
        revision
    );
    assert_eq!(
        harness
            .store
            .load_run(run.id)
            .await?
            .context("Run")?
            .run_spec,
        spec
    );
    assert_eq!(
        harness.store.run_diagnostics(run.id).await?["git_source"],
        descriptor
    );
    let audit:Value=sqlx::query_scalar("SELECT payload->'git_source' FROM event_log WHERE project_id=$1 AND event_type='run_observed' AND payload->>'run_id'=$2 AND payload->'git_source' IS NOT NULL ORDER BY project_sequence DESC LIMIT 1")
        .bind(project.as_uuid()).bind(run.id.to_string()).fetch_one(&harness.pool).await?;
    assert_eq!(audit, descriptor);
    Ok(())
}

fn validate_version_boundary(spec: &Value) -> Result<()> {
    use forge_domain::runtime::RuntimeLaunchSpec;
    let decoded: RuntimeLaunchSpec = serde_json::from_value(spec.clone())?;
    assert_eq!(decoded.schema_version, 6);
    let mut missing = spec.clone();
    missing["source_request"] = Value::Null;
    assert!(serde_json::from_value::<RuntimeLaunchSpec>(missing).is_err());
    let mut mismatch = spec.clone();
    mismatch["source_request"]["initial_revision"] = json!("f".repeat(40));
    assert!(serde_json::from_value::<RuntimeLaunchSpec>(mismatch).is_err());
    let mut legacy = spec.clone();
    legacy["schema_version"] = json!(2);
    assert!(serde_json::from_value::<RuntimeLaunchSpec>(legacy.clone()).is_err());
    legacy
        .as_object_mut()
        .context("RunSpec object")?
        .remove("source_request");
    legacy
        .as_object_mut()
        .context("RunSpec object")?
        .remove("file_inputs");
    let decoded: RuntimeLaunchSpec = serde_json::from_value(legacy)?;
    assert_eq!(decoded.schema_version, 2);
    assert!(decoded.source_request.is_none());
    Ok(())
}

pub(super) async fn before_stale(
    harness: &M0Harness,
    project: ProjectId,
    task: TaskId,
    mode: &str,
) -> Result<()> {
    if mode == "integration_future_source" {
        harness.execute(project,CommandName::SetTaskGitSourcePolicy,json!({"task_id":task,"expected_policy_revision":2,"policy":{"mode":"latest_target"},"reason":"Refresh following attempt"})).await?;
    }
    Ok(())
}

pub(super) async fn assert_held(
    harness: &M0Harness,
    project: ProjectId,
    task: TaskId,
) -> Result<()> {
    let held = harness.store.load_task(task).await?.context("Task")?.task;
    assert_eq!(held.lifecycle(), LifecycleStatus::Waiting);
    assert_eq!(
        held.current_stage_id()
            .context("integration stage")?
            .as_str(),
        "publish_here"
    );
    let mut tx = harness.store.begin().await?;
    assert_eq!(
        tx.integration_for_task(project, task)
            .await?
            .context("integration")?
            .state,
        forge_storage::IntegrationState::Held
    );
    tx.commit().await?;
    harness.core.dispatch_available(project).await?;
    assert_eq!(harness.store.list_runs(task).await?.len(), 1);
    assert_eq!(
        harness
            .store
            .task_git_source_setting(project, task)
            .await?
            .revision,
        2
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires PG/NATS; synthetic peer, no provider"]
async fn frozen_source_policy_and_receipt_survive_current_submission_then_next_run_refreshes()
-> Result<()> {
    scenario("integration_future_source").await
}
#[tokio::test]
#[ignore = "requires PG/NATS; synthetic peer, no provider"]
async fn pinned_source_staleness_requires_management_without_rework_loop() -> Result<()> {
    scenario("integration_pinned_stale").await
}

#[tokio::test]
#[ignore = "requires PG/NATS; synthetic peer, no provider"]
async fn pinned_policy_is_delivered_to_a_new_writer_and_survives_later_policy_changes() -> Result<()>
{
    let setup::Setup {
        root,
        harness,
        mut supervisor,
        project,
        version,
        ..
    } = Box::pin(setup::create("source_policy")).await?;
    let task = harness
        .create_task(project, version, "Pinned writer")
        .await?;
    let repository = harness
        .execute(
            project,
            CommandName::RegisterProjectRepository,
            json!({"name":"fixture","source":root.join("mock-source"),"target_ref":"refs/heads/accepted"}),
        )
        .await?
        .resource
        .context("repository")?
        .id;
    harness
        .execute(
            project,
            CommandName::BindTaskGitRepository,
            json!({"task_id":task,"expected_task_revision":1,"repository_id":repository,"initial_base":"a".repeat(40)}),
        )
        .await?;
    let pin = json!({"mode":"pinned_commit","commit":"b".repeat(40)});
    harness
        .execute(
            project,
            CommandName::SetTaskGitSourcePolicy,
            json!({"task_id":task,"expected_policy_revision":1,"policy":pin,"reason":"Pin next writer"}),
        )
        .await?;
    harness.approve_task(project, task).await?;
    harness.start_project(project).await?;
    let provision = supervisor.next_provision_for_task(task).await?;
    let run = harness.wait_for_run_count(task, 1).await?.remove(0);
    let spec: Value = serde_json::from_str(&provision.run_spec_json)?;
    assert_eq!(spec["source_request"]["policy_revision"], 2);
    assert_eq!(spec["source_request"]["policy"], pin);
    assert_eq!(spec, run.run_spec);
    harness
        .execute(
            project,
            CommandName::SetTaskGitSourcePolicy,
            json!({"task_id":task,"expected_policy_revision":2,"policy":{"mode":"latest_target"},"reason":"Future writers may refresh"}),
        )
        .await?;
    assert_eq!(
        harness
            .store
            .load_run(run.id)
            .await?
            .context("Run")?
            .run_spec,
        spec,
        "changing Task policy must not replace the active writer's pin"
    );
    harness
        .execute(project, CommandName::StopProjectExecution, json!({}))
        .await?;
    supervisor.next_stop_for_run(&run.id.to_string()).await?;
    let ack = supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Stopped)
        .await?;
    assert_eq!(ack.disposition, AcknowledgementDisposition::Accepted as i32);
    harness.shutdown().await;
    Ok(())
}
