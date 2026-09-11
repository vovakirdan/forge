//! Real sandbox admission with the native mailbox producer, without provider inference.

use super::*;
use forge_protocol::{
    runtime::RuntimeInputConfig,
    supervisor::v1::{
        DeliverRuntimeInput, RuntimeMessageInput, StopMode, StopRun, deliver_runtime_input::Action,
    },
};
use forge_provider_common::native_input::NativeMailbox;

type Worker = tokio::task::JoinHandle<Result<(), crate::SupervisorError>>;

async fn sibling(first: &Fixture) -> Fixture {
    let mut provision = first.provision.clone();
    provision.command_id = new_id();
    provision.run_id = new_id();
    provision.task_id = new_id();
    provision.context_snapshot_id = new_id();
    provision.lease_fencing_token += 1;
    let mut spec = first.spec.clone();
    spec.surface_id = uuid::Uuid::now_v7();
    provision.run_spec_json = serde_json::to_string(&spec).unwrap();
    let grant = scoped_directory(&first.config.grants_directory, &provision);
    private_directory(grant.parent().unwrap()).unwrap();
    private_directory(&grant).unwrap();
    let original = scoped_directory(&first.config.grants_directory, &first.provision);
    for name in ["invocation.json", "stdin"] {
        private_write(&grant.join(name), &fs::read(original.join(name)).unwrap()).unwrap();
    }
    let gateway = first.config.gateways_directory.join(&provision.run_id);
    private_directory(&gateway).unwrap();
    Fixture {
        config: first.config.clone(),
        provision,
        spec,
        registry: first.registry.clone(),
        _gateway: Some(UnixListener::bind(gateway.join("gateway.sock")).unwrap()),
    }
}

async fn launch(backend: &PodmanBackend, fixture: &Fixture) -> Worker {
    let control = fixture
        .registry
        .register(&fixture.provision, &fixture.config.boot_id)
        .await
        .unwrap()
        .unwrap();
    let backend = backend.clone();
    let registry = fixture.registry.clone();
    let provision = fixture.provision.clone();
    tokio::spawn(async move { backend.run(provision, registry, control, false).await })
}

async fn wait_for_child(fixture: &Fixture, worker: &Worker) -> Result<(), String> {
    let result = fixture
        .config
        .state_directory
        .join("surfaces")
        .join(fixture.spec.surface_id.to_string())
        .join("worktree/fixture-result.txt");
    timeout(Duration::from_secs(30), async {
        loop {
            if result.exists() {
                return Ok(());
            }
            if worker.is_finished() {
                return Err("sandbox exited before its fixture child started".into());
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .map_err(|_| "sandbox start deadline elapsed".to_owned())?
}

fn mailbox(fixture: &Fixture) -> std::io::Result<NativeMailbox> {
    let private = fixture
        .config
        .state_directory
        .join("synthetic-native-driver");
    private_directory(&private).map_err(std::io::Error::other)?;
    let evidence = scoped_directory(
        &fixture.config.state_directory.join("evidence"),
        &fixture.provision,
    );
    // This is the actual shared driver constructor, not a hand-built directory fixture.
    let mailbox = NativeMailbox::open(
        RuntimeInputConfig {
            run_id: fixture.provision.run_id.clone(),
            fencing_token: fixture.provision.lease_fencing_token,
            environment_epoch: fixture.provision.environment_epoch,
        },
        &private,
        scoped_directory(&fixture.config.grants_directory, &fixture.provision).join("inputs"),
        evidence.join("input-receipts"),
    )?;
    mailbox.accepted(&DeliverRuntimeInput {
        command_id: new_id(),
        run_id: fixture.provision.run_id.clone(),
        lease_fencing_token: fixture.provision.lease_fencing_token,
        environment_epoch: fixture.provision.environment_epoch,
        sequence: 1,
        action: Some(Action::Message(RuntimeMessageInput {
            source_message_json: "{}".into(),
        })),
    })?;
    Ok(mailbox)
}

#[tokio::test]
#[ignore = "requires rootless Podman and fixture image; synthetic native input, no credentials or inference"]
async fn second_sandbox_starts_while_first_retains_native_input_receipts() {
    let first = Fixture::new(true).await;
    let second = sibling(&first).await;
    let backend = PodmanBackend::new(&first.config);
    let mut workers = vec![launch(&backend, &first).await];
    let outcome: Result<(), String> = async {
        wait_for_child(&first, &workers[0]).await?;
        let _mailbox = mailbox(&first).map_err(|error| error.to_string())?;
        workers.push(launch(&backend, &second).await);
        wait_for_child(&second, &workers[1]).await?;
        for fixture in [&first, &second] {
            let name = format!(
                "forge-run-{}-{}-{}",
                fixture.provision.run_id,
                fixture.provision.lease_fencing_token,
                fixture.provision.environment_epoch
            );
            let output = timeout(
                Duration::from_secs(10),
                Command::new("podman")
                    .args(["inspect", "--format", "{{.State.Running}}", &name])
                    .kill_on_drop(true)
                    .output(),
            )
            .await
            .map_err(|_| "physical overlap inspection timed out".to_owned())?
            .map_err(|error| error.to_string())?;
            if !output.status.success() || output.stdout != b"true\n" {
                return Err("both exact sandbox containers must be running".into());
            }
        }
        Ok(())
    }
    .await;
    // Stop and clean only these two synthetic sandboxes, including the regression's failure path.
    for fixture in [&first, &second] {
        fixture
            .registry
            .request_stop(&StopRun {
                command_id: new_id(),
                run_id: fixture.provision.run_id.clone(),
                lease_fencing_token: fixture.provision.lease_fencing_token,
                environment_epoch: fixture.provision.environment_epoch,
                mode: StopMode::Graceful as i32,
                grace_period_ms: 1000,
                reason_code: "fixture_stop".into(),
            })
            .await;
    }
    let mut errors = Vec::new();
    for mut worker in workers {
        match timeout(Duration::from_secs(15), &mut worker).await {
            Ok(Ok(Ok(()))) => {}
            Ok(result) => errors.push(format!("{result:?}")),
            Err(_) => {
                worker.abort();
                let _ = worker.await;
                errors.push("sandbox monitor stop timed out".into());
            }
        }
    }
    first.cleanup().await;
    second.cleanup().await;
    assert!(outcome.is_ok(), "{outcome:?}; monitor results: {errors:?}");
    assert!(errors.is_empty(), "monitor results: {errors:?}");
}
