//! Cleanup cannot depend on PostgreSQL remaining healthy after paid Runs start.
use super::*;
use std::process::{Output, Stdio};

pub(super) fn supervisor_host(root: &Path) -> Result<String> {
    // Capture the test-owned local identity before any Run starts; no database
    // or credential is needed to narrow emergency inventory afterward.
    let snapshot = PrivateMaterialization::open(&root.join("journal.json"))?.read(64 * 1024)?;
    let value: Value = serde_json::from_slice(snapshot.expose())?;
    let host = value["host_id"]
        .as_str()
        .context("test Supervisor identity absent")?;
    ensure!(
        host.strip_prefix("m1-test-host-")
            .is_some_and(|id| Uuid::parse_str(id).is_ok()),
        "unexpected test Supervisor identity"
    );
    Ok(host.to_owned())
}

async fn podman(args: &[&str]) -> Result<Output> {
    Ok(tokio::time::timeout(
        Duration::from_secs(10),
        tokio::process::Command::new("podman")
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("bounded test-owned Podman cleanup command timed out")??)
}

async fn quiescent(harness: &M0Harness, run: &RunProjection) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let released: bool = sqlx::query_scalar(
                "SELECT released_at IS NOT NULL FROM run_environment_reservations WHERE run_id=$1",
            )
            .bind(run.id)
            .fetch_one(&harness.pool)
            .await?;
            if released {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .context("managed stop did not prove physical quiescence")?
}

/// Authorizes emergency cleanup by this fixture's exact private mount boundary,
/// including Runs whose canonical identity could not be read during a DB outage.
fn owns_runtime_mount(root: &Path, mounts: &Value) -> bool {
    mounts.as_array().is_some_and(|mounts| {
        mounts.iter().any(|mount| {
            let Some(source) = mount["Source"].as_str() else {
                return false;
            };
            if mount["Destination"] != "/run/forge" {
                return false;
            }
            let Ok(relative) = Path::new(source).strip_prefix(root.join("runtime")) else {
                return false;
            };
            let parts = relative.components().collect::<Vec<_>>();
            parts.len() == 2
                && parts[0]
                    .as_os_str()
                    .to_str()
                    .is_some_and(|part| Uuid::parse_str(part).is_ok())
                && parts[1]
                    .as_os_str()
                    .to_str()
                    .is_some_and(|part| part.parse::<u64>().is_ok_and(|epoch| epoch > 0))
        })
    })
}

async fn emergency_stop_owned(root: &Path, host_id: &str) -> Result<bool> {
    let listing = podman(&[
        "ps",
        "--all",
        "--no-trunc",
        "--filter",
        "label=forge.managed=true",
        "--filter",
        &format!("label=forge.host={host_id}"),
        "--format",
        "{{.ID}}",
    ])
    .await?;
    ensure!(
        listing.status.success() && listing.stdout.len() <= 64 * 1024,
        "cannot inventory live fixture containers safely"
    );
    let ids = std::str::from_utf8(&listing.stdout)?
        .lines()
        .collect::<Vec<_>>();
    let mut stopped = false;
    let mut failed = false;
    for id in ids {
        if id.len() != 64 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            failed = true;
            continue;
        }
        let Ok(inspect) = podman(&["inspect", "--format", "{{json .Mounts}}", id]).await else {
            failed = true;
            continue;
        };
        if !inspect.status.success() || inspect.stdout.len() > 64 * 1024 {
            failed = true;
            continue;
        }
        let Ok(mounts) = serde_json::from_slice::<Value>(&inspect.stdout) else {
            failed = true;
            continue;
        };
        if !owns_runtime_mount(root, &mounts) {
            continue;
        }
        let state = podman(&["inspect", "--format", "{{.State.Running}}", id]).await;
        if !state
            .as_ref()
            .is_ok_and(|state| state.status.success() && state.stdout == b"false\n")
        {
            stopped = true;
            let result = podman(&["kill", "--signal", "KILL", id]).await;
            failed |= !result.is_ok_and(|result| result.status.success());
            let state = podman(&["inspect", "--format", "{{.State.Running}}", id]).await;
            failed |=
                !state.is_ok_and(|state| state.status.success() && state.stdout == b"false\n");
        }
    }
    ensure!(
        !failed,
        "emergency cleanup incomplete; inspect exact retained fixture containers"
    );
    Ok(stopped)
}

pub(super) async fn stop_and_collect(
    harness: &mut M0Harness,
    project: ProjectId,
    tasks: &[TaskId; 2],
    mut runs: Vec<RunProjection>,
    root: &Path,
    host_id: &str,
) -> Result<Vec<Value>> {
    let stop = tokio::time::timeout(Duration::from_secs(15), harness.stop_project(project)).await;
    let mut failed = !matches!(stop, Ok(Ok(())));
    // Previously captured identities survive DB failure. Discovery is best effort,
    // bounded, and never suppresses the independent local-container cleanup pass.
    for task in tasks {
        match tokio::time::timeout(Duration::from_secs(5), harness.store.list_runs(*task)).await {
            Ok(Ok(found)) => {
                for run in found {
                    if !runs.iter().any(|prior| prior.id == run.id) {
                        runs.push(run);
                    }
                }
            }
            _ => failed = true,
        }
    }
    for run in &runs {
        if quiescent(harness, run).await.is_err() {
            failed = true;
            let _ = podman(&["kill", "--signal", "KILL", &container(run)]).await;
        }
    }
    // End the local dispatch/worker owner before inspecting Created or missing
    // environments; otherwise an in-flight worker could start after the scan.
    failed |= harness.stop_runtime_supervisor_for_cleanup().await.is_err();
    // Always runs, even when SQL failed before either Run identity was obtained.
    let emergency = emergency_stop_owned(root, host_id).await;
    failed |= !matches!(emergency, Ok(false));
    let mut reports = Vec::new();
    for run in &runs {
        let evidence = tokio::time::timeout(Duration::from_secs(15), async {
            harness.core.collect_run_evidence(run.id).await?;
            let diagnostics = harness.store.run_diagnostics(run.id).await?;
            ensure!(
                diagnostics["runtime_report"].is_object(),
                "missing live runtime report"
            );
            ensure!(
                diagnostics["handoff"]["outcome"]["kind"] == "interrupted",
                "missing interrupted live handoff"
            );
            Ok::<_, anyhow::Error>(diagnostics)
        })
        .await;
        match evidence {
            Ok(Ok(report)) => reports.push(report),
            _ => failed = true,
        }
    }
    // Inspect again after bounded evidence I/O to catch already-issued Podman
    // operations settling after worker cancellation. No owner remains to retry.
    failed |= !matches!(emergency_stop_owned(root, host_id).await, Ok(false));
    ensure!(
        !failed,
        "live stop/evidence failed; exact-container emergency cleanup attempted, inspect retained private evidence"
    );
    Ok(reports)
}

#[test]
fn emergency_inventory_requires_exact_private_runtime_mount() {
    let root = Path::new("/tmp/fc-owned");
    let run = Uuid::now_v7();
    let own =
        json!([{"Source":format!("/tmp/fc-owned/runtime/{run}/1"),"Destination":"/run/forge"}]);
    assert!(owns_runtime_mount(root, &own));
    for source in [
        format!("/tmp/fc-other/runtime/{run}/1"),
        format!("/tmp/fc-owned-extra/runtime/{run}/1"),
        "/tmp/fc-owned/runtime/not-a-run/1".into(),
        format!("/tmp/fc-owned/runtime/{run}/0"),
        format!("/tmp/fc-owned/runtime/{run}/1/child"),
    ] {
        assert!(!owns_runtime_mount(
            root,
            &json!([{"Source":source,"Destination":"/run/forge"}])
        ));
    }
    assert!(!owns_runtime_mount(
        root,
        &json!([{"Source":format!("/tmp/fc-owned/runtime/{run}/1"),"Destination":"/another/mount"}])
    ));
}
