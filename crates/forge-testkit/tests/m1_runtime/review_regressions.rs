//! Regression reproductions for the independent M1 closeout review.
use super::*;
use forge_domain::{EvidenceScope, EvidenceStream};
use forge_protocol::supervisor::v1::AcknowledgementDisposition;
use forge_storage::evidence::{EvidenceRedaction, EvidenceSpool, SpoolLimits};

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; manual Supervisor, no inference"]
async fn failed_nonstart_releases_reservation_and_allows_named_recovery() -> Result<()> {
    let (harness, root, project) = fixture().await?;
    enroll(
        &harness,
        &root,
        project,
        &format!("localhost/synthetic@sha256:{}", "0".repeat(64)),
    )
    .await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    harness
        .execute(
            project,
            CommandName::ConfigureBootRecoveryPolicy,
            json!({"policy":"recover_safe_then_hold"}),
        )
        .await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    let task = harness
        .create_task(project, pipeline, "confirmed nonstart")
        .await?;
    harness.approve_task(project, task).await?;
    harness.start_project(project).await?;
    supervisor.next_provision_for_task(task).await?;
    let run = harness.wait_for_run_count(task, 1).await?.remove(0);
    let failed = supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::StartFailed)
        .await?;
    assert_eq!(
        failed.disposition,
        AcknowledgementDisposition::Accepted as i32
    );
    let reserved: bool = sqlx::query_scalar(
        "SELECT released_at IS NULL FROM run_environment_reservations WHERE run_id=$1",
    )
    .bind(run.id)
    .fetch_one(&harness.pool)
    .await?;
    assert!(
        reserved,
        "StartFailed alone is not proof of physical quiescence"
    );
    assert!(
        harness
            .execute(
                project,
                CommandName::AcceptRunRecoveryAssessment,
                json!({"run_id":run.id,"assessment":"not_started_confirmed"})
            )
            .await
            .is_err()
    );
    let stopped = supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 2, RunEventKind::Stopped)
        .await?;
    assert_eq!(
        stopped.disposition,
        AcknowledgementDisposition::Accepted as i32
    );
    let reserved: bool = sqlx::query_scalar(
        "SELECT released_at IS NULL FROM run_environment_reservations WHERE run_id=$1",
    )
    .bind(run.id)
    .fetch_one(&harness.pool)
    .await?;
    assert!(
        !reserved,
        "positive proof releases without a Supervisor reconnect"
    );
    harness
        .execute(
            project,
            CommandName::AcceptRunRecoveryAssessment,
            json!({"run_id":run.id,"assessment":"not_started_confirmed"}),
        )
        .await?;
    let replacement = supervisor.next_provision_for_task(task).await?;
    assert_ne!(replacement.run_id, run.id.to_string());
    harness.stop_project(project).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; synthetic orphan spool, no inference"]
async fn orphan_stdout_prefix_is_incomplete_and_cannot_trigger_secret_cleanup() -> Result<()> {
    let (harness, root, project) = fixture().await?;
    enroll(
        &harness,
        &root,
        project,
        &format!("localhost/synthetic@sha256:{}", "0".repeat(64)),
    )
    .await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    let task = harness
        .create_task(project, pipeline, "crashed evidence import")
        .await?;
    harness.approve_task(project, task).await?;
    harness.start_project(project).await?;
    supervisor.next_provision_for_task(task).await?;
    let run = harness.wait_for_run_count(task, 1).await?.remove(0);
    let evidence = make_private_descendants(
        &root,
        [
            "evidence".into(),
            run.id.to_string(),
            run.environment_epoch.to_string(),
        ],
    )?;
    let stdout = vec![b'x'; 128 * 1024];
    PrivateMaterialization::create(
        &evidence.join("stdout.jsonl"),
        &SecretBytes::new(stdout.clone()),
    )?;
    PrivateMaterialization::create(&evidence.join("stderr.log"), &SecretBytes::new(Vec::new()))?;
    // Restore the on-disk state from a crashed collector: one complete 64 KiB
    // chunk, no stream completion receipt, and a larger retained source file.
    let crash_root = root.join("crash-spool");
    let spool = EvidenceSpool::open(&crash_root, SpoolLimits::default())?;
    let mut collector = spool.start_stream(
        EvidenceScope {
            project_id: project,
            task_id: Some(task),
            run_id: run.id,
        },
        EvidenceStream::Stdout,
        EvidenceRedaction::new("fixture".into(), Vec::new())?,
    )?;
    assert_eq!(collector.push(&stdout[..64 * 1024]).chunks.len(), 1);
    drop(collector); // crash before finish/commit
    drop(spool);
    fs::rename(
        crash_root.join(run.id.to_string()),
        root.join("spool").join(run.id.to_string()),
    )?;
    harness.stop_project(project).await?;
    supervisor.next_stop_for_run(&run.id.to_string()).await?;
    supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Stopped)
        .await?;
    harness.core.collect_run_evidence(run.id).await?;
    let diagnostics = harness.store.run_diagnostics(run.id).await?;
    let streams = diagnostics["streams"].as_array().context("streams")?;
    assert!(
        streams
            .iter()
            .any(|stream| stream["stream"] == "stdout" && stream["incomplete"] == true)
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM run_secret_cleanup WHERE run_id=$1")
        .bind(run.id)
        .fetch_one(&harness.pool)
        .await?;
    assert_eq!(count, 0);
    assert!(
        root.join("grants")
            .join(run.id.to_string())
            .join(run.environment_epoch.to_string())
            .join("auth.json")
            .exists()
    );
    assert!(
        evidence.join("stdout.jsonl").exists(),
        "full raw source remains available for recovery"
    );
    Ok(())
}
