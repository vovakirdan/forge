//! Executes an explicit Hook in real rootless Podman without provider credentials.
use super::*;
use forge_protocol::supervisor::v1::{EnvironmentPresence, supervisor_to_core};
use tokio::{process::Command, time::timeout};

async fn podman(args: &[&str]) -> std::io::Result<std::process::Output> {
    timeout(
        Duration::from_secs(10),
        Command::new("podman")
            .args(args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "bounded Podman fixture command timed out",
        )
    })?
}

async fn run_case(exit_code: i32, verdict: HookVerdict) {
    let fixture = Fixture::new().await;
    let (backend, mut provision, mut spec) = launch(&fixture);
    let image = podman(&[
        "image",
        "inspect",
        "localhost/forge-runner-fixture:m1",
        "--format",
        "{{.Digest}}",
    ])
    .await
    .unwrap();
    assert!(
        image.status.success(),
        "run just build-runtime-fixture first"
    );
    spec.hook.image = format!(
        "localhost/forge-runner-fixture@{}",
        String::from_utf8(image.stdout).unwrap().trim()
    );
    spec.hook.limits.wall_seconds = 15;
    spec.hook.limits.stop_grace_seconds = 1;
    spec.hook.command = vec![
        "/bin/sh".into(),
        "-c".into(),
        format!(
            "set -eu; test \"$(cat tracked.txt)\" = 'accepted revision'; \
             test ! -e ignored.txt; test ! -e /run/forge-gateway/gateway.sock; \
             test -z \"$(ls -A /run/forge-secrets)\"; printf 'private hook output\\n' > hook-result.txt; \
             printf 'explicit hook ran\\n'; exit {exit_code}"
        ),
    ];
    spec.validate().unwrap();
    provision.run_spec_json = serde_json::to_string(&spec).unwrap();
    fs::write(
        fixture.writer.join("tracked.txt"),
        "unaccepted writer changes\n",
    )
    .unwrap();
    fs::write(
        fixture.writer.join("ignored.txt"),
        "private writer residue\n",
    )
    .unwrap();
    let registry = RunRegistry::new(
        Journal::open(
            &fixture.root.join("real-hook-journal"),
            "fixture-host",
            1024 * 1024,
        )
        .unwrap(),
    );
    let control = registry
        .register(&provision, "fixture-boot")
        .await
        .unwrap()
        .unwrap();
    let result = timeout(
        Duration::from_secs(35),
        backend.run(provision.clone(), registry.clone(), control, false),
    )
    .await;
    let name = format!(
        "forge-run-{}-{}-{}",
        provision.run_id, provision.lease_fencing_token, provision.environment_epoch
    );
    if !matches!(&result, Ok(Ok(()))) {
        // Only this test's exact container; preserve its filesystem and evidence.
        let _ = podman(&["kill", "--signal", "KILL", &name]).await;
    }
    result.unwrap().unwrap();
    let state = podman(&["inspect", "--format", "{{.State.Running}}", &name])
        .await
        .unwrap();
    assert!(state.status.success());
    assert_eq!(state.stdout, b"false\n");
    let journal = registry.journal.lock().await;
    assert_eq!(
        journal.inventory()[0].presence,
        EnvironmentPresence::Quiescent as i32
    );
    let result = journal
        .pending()
        .find_map(|message| {
            let Some(supervisor_to_core::Message::ObservedRunEvent(event)) = &message.message
            else {
                return None;
            };
            if event.kind != RunEventKind::Stopped as i32 {
                return None;
            }
            let details: serde_json::Value = serde_json::from_str(&event.details_json).unwrap();
            details.get("hook_result").cloned()
        })
        .expect("positive physical stop must carry the typed Hook result");
    let result: HookExecutionResult = serde_json::from_value(result).unwrap();
    result.validate().unwrap();
    assert_eq!(result.verdict, verdict);
    assert_eq!(result.exit_code, Some(exit_code));
    assert_eq!(result.candidate, spec.candidate);
    assert!(!result.output_incomplete);
    let private = fixture.root.join("hook-snapshots").join(format!(
        "{}-{}-{}",
        provision.run_id, provision.lease_fencing_token, provision.environment_epoch
    ));
    assert_eq!(
        fs::read_to_string(private.join("worktree/hook-result.txt")).unwrap(),
        "private hook output\n"
    );
    assert!(!fixture.writer.join("hook-result.txt").exists());
    assert_eq!(
        fs::read_to_string(fixture.writer.join("tracked.txt")).unwrap(),
        "unaccepted writer changes\n"
    );
    eprintln!(
        "Retained stopped Hook container {name}, evidence {}",
        fixture.root.display()
    );
}

#[tokio::test]
#[ignore = "requires rootless Podman and rebuilt runtime fixture; no provider or credentials"]
async fn actual_hook_pass_uses_private_exact_candidate_and_reports_quiescence() {
    run_case(0, HookVerdict::Passed).await;
}

#[tokio::test]
#[ignore = "requires rootless Podman and rebuilt runtime fixture; no provider or credentials"]
async fn actual_hook_failure_retains_exit_without_success() {
    run_case(7, HookVerdict::Failed).await;
}
