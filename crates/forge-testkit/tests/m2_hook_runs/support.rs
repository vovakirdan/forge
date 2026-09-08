//! A private real Git repository, synthetic provider auth, and controlled observations.
use super::*;
use forge_domain::{ProjectId, TaskId, Timestamp};
use forge_testkit::m0::{M0Harness, ManualSupervisor};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub struct Fixture {
    pub h: M0Harness,
    pub supervisor: ManualSupervisor,
    pub project: ProjectId,
    pub task: TaskId,
    pub root: PathBuf,
    pub hook: Uuid,
    pub candidate: forge_domain::git::GitCandidate,
}

fn git(path: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(["-C", path.to_str().context("path")?])
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "fixture Git command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim().into())
}
pub async fn setup(mode: &str) -> Result<Fixture> {
    setup_with_limits(mode, forge_domain::admission::AdmissionLimits::default()).await
}
pub async fn setup_with_limits(
    mode: &str,
    limits: forge_domain::admission::AdmissionLimits,
) -> Result<Fixture> {
    let (h, project, _employee, root) = provider_fixture::fixture().await?;
    h.core.clone().with_admission_limits(limits).await?;
    let repo = root.join("isolated-hook-repository");
    std::fs::create_dir(&repo)?;
    git(&repo, &["init", "-b", "accepted"])?;
    git(
        &repo,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--allow-empty",
            "-m",
            "Hook candidate",
        ],
    )?;
    let candidate = forge_domain::git::GitCandidate {
        commit: forge_domain::git::GitObjectId::new(git(&repo, &["rev-parse", "HEAD"])?)?,
        tree: forge_domain::git::GitObjectId::new(git(&repo, &["rev-parse", "HEAD^{tree}"])?)?,
    };
    let hook=h.execute(project,CommandName::ConfigureProjectHook,json!({"name":"Explicit repository command","image":format!("localhost/synthetic@sha256:{}","0".repeat(64)),"command":["/bin/true"],"workdir":".","limits":{"cpu_millis":1000,"memory_bytes":134217728,"pids":32,"wall_seconds":60,"stop_grace_seconds":1},"max_output_bytes":4096,"applicable_task_kinds":if mode=="skipped"{vec!["analysis"]}else{vec![]},"required":mode!="advisory"})).await?.resource.context("hook")?.id.parse()?;
    let version = h.create_pipeline(project, pipeline(mode, hook)).await?;
    let task = h
        .create_task(
            project,
            version,
            "A configured hook checks a pinned candidate",
        )
        .await?;
    let repository = h
        .execute(
            project,
            CommandName::RegisterProjectRepository,
            json!({"name":"hook-repo","source":repo,"target_ref":"refs/heads/accepted"}),
        )
        .await?
        .resource
        .context("repository")?
        .id;
    let revision = h
        .store
        .load_task(task)
        .await?
        .context("Task")?
        .task
        .revision()
        .get();
    h.execute(project,CommandName::BindTaskGitRepository,json!({"task_id":task,"expected_task_revision":revision,"repository_id":repository,"initial_base":candidate.commit})).await?;
    h.approve_task(project, task).await?;
    let mut supervisor = h.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    h.start_project(project).await?;
    let mut fixture = Fixture {
        h,
        supervisor,
        project,
        task,
        root,
        hook,
        candidate,
    };
    submit_writer(&mut fixture, mode == "missing_required").await?;
    Ok(fixture)
}
pub async fn submit_writer(f: &mut Fixture, may_be_rejected: bool) -> Result<()> {
    let run =
        f.h.store
            .load_run(
                f.supervisor
                    .next_provision_for_task(f.task)
                    .await?
                    .run_id
                    .parse()?,
            )
            .await?
            .context("writer")?;
    let client = provider_fixture::client(&f.root, &run)?;
    let evidence = Uuid::now_v7();
    let report=provider_fixture::call(&client,"artifact.submit",json!({"artifact_kind":"stage_evidence","title":"Synthetic writer report","body":{"result":"This test does not run a provider"}}),evidence).await?;
    anyhow::ensure!(report.0.is_success(), "{}", report.1);
    let proposal=provider_fixture::call(&client,"outcome.submit",json!({"stage_id":"work","outcome":"completed","artifact_submission_message_ids":[evidence],"candidate_commit":f.candidate.commit}),Uuid::now_v7()).await?;
    anyhow::ensure!(proposal.0.is_success(), "{}", proposal.1);
    f.supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Stopped)
        .await?;
    tick(&f.h).await?;
    let request = f.supervisor.next_git_inspection().await?;
    let ack = f
        .supervisor
        .send_git_inspection(
            forge_protocol::supervisor::v1::GitCandidateInspectionResult {
                message_id: Uuid::now_v7().to_string(),
                request_command_id: request.command_id,
                run_id: run.id.to_string(),
                lease_fencing_token: run.lease_fencing_token,
                environment_epoch: run.environment_epoch,
                commit: f.candidate.commit.as_str().to_owned(),
                tree: f.candidate.tree.as_str().to_owned(),
                result_code: forge_protocol::supervisor::v1::GitCandidateInspectionCode::Verified
                    as i32,
                host_id: request.host_id,
                boot_id: request.boot_id,
            },
        )
        .await?;
    if !may_be_rejected {
        anyhow::ensure!(
            ack.disposition == AcknowledgementDisposition::Accepted as i32,
            "{ack:?}"
        );
    }
    tick(&f.h).await?;
    Ok(())
}
pub fn advance_candidate(f: &mut Fixture) -> Result<()> {
    let repo = f.root.join("isolated-hook-repository");
    git(
        &repo,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--allow-empty",
            "-m",
            "Changed candidate",
        ],
    )?;
    f.candidate.commit = forge_domain::git::GitObjectId::new(git(&repo, &["rev-parse", "HEAD"])?)?;
    Ok(())
}
pub async fn tick(h: &M0Harness) -> Result<()> {
    let core = h.core.clone();
    tokio::spawn(async move {
        core.watchdog_tick(
            Timestamp::now_utc(),
            forge_core::WatchdogDeadlines::default(),
        )
        .await
    })
    .await??;
    Ok(())
}
pub(super) fn pipeline(mode: &str, hook: Uuid) -> Value {
    let mut value = json!({"name":"Explicit hook pipeline","task_kinds":["delivery"],"entry_stage_id":"work","max_stage_visits":8,
        "stages":[{"id":"work","name":"Work","executor_kind":"employee","outcomes":["completed","checks"],"workspace":{"kind":"git","access":"read_write"}},
            {"id":"checks","name":"Owner-selected command","executor_kind":"system","outcomes":["passed","failed","timed_out","skipped"],"system_action":{"kind":"project_hook","hook_version_id":hook,"outcomes":{"passed":"passed","failed":"failed","timed_out":"timed_out","skipped":"skipped"}}}],
        "transitions":[{"from_stage_id":"work","outcome":"completed","target":if mode=="missing_required"{json!({"kind":"done"})}else{json!({"kind":"stage","stage_id":"checks"})},"artifact_requirements":[{"kind":"stage_evidence","minimum_count":1}]},
            {"from_stage_id":"work","outcome":"checks","target":{"kind":"stage","stage_id":"checks"}},
            {"from_stage_id":"checks","outcome":"passed","target":{"kind":"done"}},
            {"from_stage_id":"checks","outcome":"skipped","target":{"kind":"done"}},
            {"from_stage_id":"checks","outcome":"failed","target":if mode=="advisory"{json!({"kind":"done"})}else{json!({"kind":"stage","stage_id":"work"})}},
            {"from_stage_id":"checks","outcome":"timed_out","target":{"kind":"stage","stage_id":"work"}}]});
    if mode == "history" {
        value["stages"].as_array_mut().unwrap().push(json!({"id":"gate","name":"Human routing","executor_kind":"human","outcomes":["again","rework","done"]}));
        value["transitions"][2]["target"] = json!({"kind":"stage","stage_id":"gate"});
        value["transitions"].as_array_mut().unwrap().extend([
            json!({"from_stage_id":"gate","outcome":"again","target":{"kind":"stage","stage_id":"checks"}}),
            json!({"from_stage_id":"gate","outcome":"rework","target":{"kind":"stage","stage_id":"work"}}),
            json!({"from_stage_id":"gate","outcome":"done","target":{"kind":"done"}}),
        ]);
    }
    value
}
