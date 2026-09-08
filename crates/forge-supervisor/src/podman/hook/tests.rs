use super::*;
use crate::{
    SupervisorConfig,
    journal::Journal,
    surface::pinned_fixture::{Fixture, git},
};
use forge_domain::{
    HookAssignmentRef, PipelineVersionId, ProjectHookVersion, StageId, TaskGitBinding, TaskId,
    Timestamp,
    git::{GitBranchRef, GitObjectId, LocalGitPath},
    runtime::ResourceLimits,
};
use forge_protocol::supervisor::v1::{
    HookExecutionAssignment, RunEventKind, provision_run::Assignment,
};
use std::{collections::BTreeSet, fs};
use uuid::Uuid;

mod runtime;

fn launch(fixture: &Fixture) -> (PodmanBackend, ProvisionRun, HookRunSpec) {
    let SurfaceSpec::GitCandidateSnapshot {
        repository,
        base_ref,
        ..
    } = &fixture.spec.binding.surface
    else {
        panic!("pinned fixture");
    };
    let mut provision = fixture.provision.clone();
    let owner = HookAssignmentRef {
        invocation_id: Uuid::now_v7(),
        task_id: TaskId::from(provision.task_id.parse::<Uuid>().unwrap()),
        pipeline_version_id: PipelineVersionId::new(),
        stage_id: StageId::new("explicit_hook").unwrap(),
        stage_visit: 3,
        candidate_proposal_id: Uuid::now_v7(),
    };
    let spec = HookRunSpec {
        schema_version: 5,
        project_id: fixture.spec.project_id,
        run_id: provision.run_id.parse().unwrap(),
        assignment: owner.clone(),
        hook: ProjectHookVersion {
            id: Uuid::now_v7(),
            project_id: fixture.spec.project_id,
            name: "Owner checks".into(),
            image: format!("fixture@sha256:{}", "a".repeat(64)),
            command: vec!["/usr/bin/true".into()],
            workdir: ".".into(),
            limits: ResourceLimits::default(),
            max_output_bytes: 1024 * 1024,
            applicable_task_kinds: BTreeSet::new(),
            required: true,
            created_at: Timestamp::now_utc(),
        },
        binding: TaskGitBinding {
            repository_id: Uuid::now_v7(),
            source: LocalGitPath::new(repository).unwrap(),
            target_ref: GitBranchRef::new("refs/heads/main").unwrap(),
            initial_base: GitObjectId::new(base_ref).unwrap(),
            surface_id: fixture.spec.surface_id,
        },
        candidate: fixture.candidate.clone(),
    };
    provision.run_spec_version = 5;
    provision.task_id.clear();
    provision.stage_id.clear();
    provision.employee_id.clear();
    provision.run_spec_json = serde_json::to_string(&spec).unwrap();
    provision.assignment = Some(Assignment::Hook(HookExecutionAssignment {
        invocation_id: owner.invocation_id.to_string(),
        task_id: owner.task_id.to_string(),
        pipeline_version_id: owner.pipeline_version_id.to_string(),
        stage_id: owner.stage_id.as_str().into(),
        stage_visit: owner.stage_visit,
        candidate_proposal_id: owner.candidate_proposal_id.to_string(),
    }));
    let mut config = SupervisorConfig::new(
        fixture.root.join("socket"),
        "fixture-host".into(),
        "fixture-boot".into(),
    );
    config.state_directory = fixture.root.clone();
    config.grants_directory = fixture.root.join("grants");
    (PodmanBackend::new(&config), provision, spec)
}

#[tokio::test]
async fn hook_v5_context_is_not_an_employee_or_task_writer() {
    let fixture = Fixture::new().await;
    let (_, provision, spec) = launch(&fixture);
    crate::execution_assignment::validate(&provision).unwrap();
    let invocation = invocation(&spec);
    assert_eq!(invocation.adapter_id, "project_hook");
    assert!(
        invocation.credential_files.is_empty()
            && invocation.managed_files.is_empty()
            && invocation.runtime_input.is_none()
    );
    assert!(
        !serde_json::to_string(&invocation)
            .unwrap()
            .contains("gateway")
    );
    for field in ["employee", "task", "stage"] {
        let mut bad = provision.clone();
        match field {
            "employee" => bad.employee_id = crate::new_id(),
            "task" => bad.task_id = crate::new_id(),
            _ => bad.stage_id = "foreign".into(),
        };
        assert!(crate::execution_assignment::validate(&bad).is_err());
    }
    let mut bad = provision.clone();
    if let Some(Assignment::Hook(owner)) = &mut bad.assignment {
        owner.stage_visit += 1;
    }
    assert!(crate::execution_assignment::validate(&bad).is_err());
    let mut journal = Journal::open(
        &fixture.root.join("hook-journal"),
        "fixture-host",
        1024 * 1024,
    )
    .unwrap();
    assert!(journal.register(&provision, "fixture-boot").unwrap());
    assert!(!journal.register(&provision, "fixture-boot").unwrap());
    assert!(journal.records()[0].git_source.is_none());
    drop(journal);
    let mut journal = Journal::open(
        &fixture.root.join("hook-journal"),
        "fixture-host",
        1024 * 1024,
    )
    .unwrap();
    assert!(!journal.register(&provision, "fixture-boot").unwrap());
}

#[tokio::test]
async fn hook_gets_writable_private_candidate_without_mutable_task_files() {
    let fixture = Fixture::new().await;
    let (backend, provision, spec) = launch(&fixture);
    let manifest = fs::read(&fixture.manifest).unwrap();
    fs::write(fixture.writer.join("tracked.txt"), "later dirty writer\n").unwrap();
    fs::write(fixture.writer.join("ignored.txt"), "not candidate\n").unwrap();
    let copy = backend.hook_snapshot(&provision, &spec).await.unwrap();
    assert!(copy.starts_with(fixture.root.join("hook-snapshots")));
    assert_eq!(
        git(&copy.join("worktree"), &["rev-parse", "HEAD"]).await,
        spec.candidate.commit.as_str()
    );
    assert_eq!(
        fs::read_to_string(copy.join("worktree/tracked.txt")).unwrap(),
        "accepted revision\n"
    );
    assert!(!copy.join("worktree/ignored.txt").exists());
    fs::write(
        copy.join("worktree/generated-result.txt"),
        "private test output",
    )
    .unwrap();
    assert!(!fixture.writer.join("generated-result.txt").exists());
    assert_eq!(
        fs::read_to_string(fixture.writer.join("tracked.txt")).unwrap(),
        "later dirty writer\n"
    );
    assert_eq!(fs::read(&fixture.manifest).unwrap(), manifest);
    assert!(backend.hook_snapshot(&provision, &spec).await.is_err());
    let mut foreign = spec;
    foreign.assignment.task_id = TaskId::new();
    assert!(backend.hook_snapshot(&provision, &foreign).await.is_err());
}

#[tokio::test]
async fn hook_exit_requires_complete_report_and_durable_stop_reason_survives_restart() {
    let fixture = Fixture::new().await;
    let (backend, provision, spec) = launch(&fixture);
    let good = RunnerExit {
        exit_code: Some(0),
        output_incomplete: false,
        stop_requested: false,
    };
    assert_eq!(
        result(&spec, Some(&good), 0, None, false).verdict,
        HookVerdict::Passed
    );
    assert_eq!(
        result(&spec, None, 0, None, false).verdict,
        HookVerdict::Failed
    );
    let incomplete = RunnerExit {
        output_incomplete: true,
        ..good
    };
    assert_eq!(
        result(&spec, Some(&incomplete), 0, None, false).verdict,
        HookVerdict::Failed
    );
    assert_eq!(
        result(&spec, Some(&incomplete), 0, Some("wall_limit"), false).verdict,
        HookVerdict::TimedOut
    );
    assert_eq!(
        result(&spec, None, 137, None, true).verdict,
        HookVerdict::Interrupted
    );
    let directory = fixture.root.join("hook-journal");
    let registry =
        RunRegistry::new(Journal::open(&directory, "fixture-host", 1024 * 1024).unwrap());
    registry.register(&provision, "fixture-boot").await.unwrap();
    registry
        .emit(
            &provision,
            RunEventKind::Stopping,
            serde_json::json!({"reason_code":"wall_limit"}),
        )
        .await
        .unwrap();
    {
        let mut journal = registry.journal.lock().await;
        let id = crate::journal::message_id(journal.pending().next().unwrap())
            .unwrap()
            .clone();
        journal
            .acknowledge(&forge_protocol::supervisor::v1::CoreAcknowledgement {
                message_id: crate::new_id(),
                acknowledged_message_id: id,
                disposition: forge_protocol::supervisor::v1::AcknowledgementDisposition::Accepted
                    as i32,
                reason_code: String::new(),
                message: String::new(),
            })
            .unwrap();
        assert_eq!(journal.pending().count(), 0);
    }
    drop(registry);
    let registry =
        RunRegistry::new(Journal::open(&directory, "fixture-host", 1024 * 1024).unwrap());
    let control = RunControl::new(
        provision.run_id.clone(),
        provision.lease_fencing_token,
        provision.environment_epoch,
    );
    let report = backend
        .hook_result(&provision, &spec, &registry, &control, 137)
        .await;
    assert_eq!(report.verdict, HookVerdict::TimedOut);
    assert!(report.output_incomplete);
    assert_eq!(report.candidate, spec.candidate);
}

#[tokio::test]
async fn hook_container_arguments_mount_only_private_copy_and_no_gateway_or_secrets() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new().await;
    let (mut backend, provision, spec) = launch(&fixture);
    let stub = fixture.root.join("inspect-create-stub");
    let output = fixture.root.join("created-arguments");
    let script = format!(
        "#!/bin/sh\ncase \"$1\" in\ninfo) printf 'true v2\\n';;\ncontainer) exit 1;;\ncreate) printf '%s\\n' \"$@\" > '{}'; printf '%064d\\n' 0;;\nstart) exit 0;;\n*) exit 125;;\nesac\n",
        output.display()
    );
    fs::write(&stub, script).unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o700)).unwrap();
    backend.config.podman_binary = stub;
    let registry = RunRegistry::new(
        Journal::open(
            &fixture.root.join("hook-journal"),
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
    assert!(
        backend
            .create_hook(&provision, &spec, &registry, &control)
            .await
            .unwrap()
    );
    let args = fs::read_to_string(output).unwrap();
    for flag in [
        "--network=none",
        "--read-only",
        "--cap-drop=all",
        "--pull=never",
        "--entrypoint=/usr/local/bin/forge-runner",
    ] {
        assert!(args.lines().any(|arg| arg == flag));
    }
    assert!(!args.contains("gateway") && !args.contains("secrets"));
    assert!(
        !args.contains(fixture.writer.to_str().unwrap())
            && !args.contains(fixture.source.to_str().unwrap())
    );
    assert_eq!(args.lines().filter(|arg| *arg == "--mount").count(), 4);
    assert!(args.contains("target=/workspace,rw"));
    assert!(args.contains("target=/run/forge-input,ro"));
    let input = scoped_directory(&backend.config.grants_directory, &provision);
    let written: RunnerInvocation = read_private_json(&input.join("invocation.json")).unwrap();
    assert!(written.credential_files.is_empty() && written.runtime_input.is_none());
    assert!(fs::read(input.join("stdin")).unwrap().is_empty());
}
