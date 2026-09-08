//! Real private Git commits + Core/PG/Gateway; controlled Supervisor, no inference.
use super::*;
use forge_domain::{
    EmployeeId, TaskId, Timestamp,
    candidate_review::CandidateVerdict,
    git::{GitCandidate, GitObjectId},
    git_delivery::TaskGitCandidate,
};
use forge_storage::RunProjection;
use forge_supervisor::git::GitBackend;
use std::path::{Path, PathBuf};

pub(super) fn pipeline() -> Value {
    json!({"name":"QA writes tests on the same Task","task_kinds":["delivery"],"entry_stage_id":"work","max_stage_visits":8,
    "stages":[
        {"id":"work","name":"Implementation","executor_kind":"employee","outcomes":["completed"],"workspace":{"kind":"git","access":"read_write"}},
        {"id":"examine","name":"Independent examination","executor_kind":"employee","outcomes":["tests","yes"],"workspace":{"kind":"git","access":"read_only"},"acceptance_policy":{"kind":"candidate_review","independent":true,"verdicts":{"tests":"accepted","yes":"accepted"}}},
        {"id":"add_tests","name":"QA: develop regression tests","executor_kind":"employee","outcomes":["completed"],"workspace":{"kind":"git","access":"read_write"}}
    ],
    "transitions":[
        {"from_stage_id":"work","outcome":"completed","target":{"kind":"stage","stage_id":"examine"},"artifact_requirements":[{"kind":"stage_evidence","minimum_count":1}]},
        {"from_stage_id":"examine","outcome":"tests","target":{"kind":"stage","stage_id":"add_tests"},"artifact_requirements":[{"kind":"stage_evidence","minimum_count":1}]},
        {"from_stage_id":"examine","outcome":"yes","target":{"kind":"done"},"artifact_requirements":[{"kind":"stage_evidence","minimum_count":1}]},
        {"from_stage_id":"add_tests","outcome":"completed","target":{"kind":"stage","stage_id":"examine"},"artifact_requirements":[{"kind":"stage_evidence","minimum_count":1}]}
    ]})
}

struct Fixture {
    setup: setup::Setup,
    task: TaskId,
    source: PathBuf,
    worktree: PathBuf,
    base: GitCandidate,
}

#[tokio::test]
#[ignore = "requires PG/NATS; actual private Git commits, controlled Supervisor, no provider"]
async fn qa_test_development_creates_new_candidate_requiring_new_independent_review() -> Result<()>
{
    let mut f = Box::pin(create()).await?;
    let result = Box::pin(exercise(&mut f)).await;
    f.setup.harness.stop_project(f.setup.project).await?;
    f.setup.harness.shutdown().await;
    result
}

async fn create() -> Result<Fixture> {
    let setup = setup::create("qa_writer").await?;
    let source = setup.root.join("mock-source");
    std::fs::create_dir(&source)?;
    git(&source, &["init", "-b", "accepted"])?;
    git(&source, &["commit", "--allow-empty", "-m", "Isolated base"])?;
    let base = candidate(&source)?;
    let worktree = GitBackend::default()
        .snapshot_candidate(&source, &base, &setup.root.join("private-task-copy"))
        .await?;
    let task = setup
        .harness
        .create_task(
            setup.project,
            setup.version,
            "Implement deduplication, then explicitly ask QA to add regression tests",
        )
        .await?;
    let repository = setup
        .harness
        .execute(
            setup.project,
            CommandName::RegisterProjectRepository,
            json!({"name":"QA fixture source","source":source,"target_ref":"refs/heads/accepted"}),
        )
        .await?
        .resource
        .context("repository")?
        .id;
    let revision = setup
        .harness
        .store
        .load_task(task)
        .await?
        .context("Task")?
        .task
        .revision()
        .get();
    setup.harness.execute(setup.project,CommandName::BindTaskGitRepository,
        json!({"task_id":task,"expected_task_revision":revision,"repository_id":repository,"initial_base":base.commit})).await?;
    setup.harness.approve_task(setup.project, task).await?;
    setup.harness.start_project(setup.project).await?;
    Ok(Fixture {
        setup,
        task,
        source,
        worktree,
        base,
    })
}

async fn exercise(f: &mut Fixture) -> Result<()> {
    let writer = next_run(f, "work", "read_write").await?;
    assert_eq!(writer.require_employee_id()?, f.setup.employee);
    std::fs::write(
        f.worktree.join("deduplicate.py"),
        "def deduplicate(values):\n    return sorted(set(values))\n",
    )?;
    git(&f.worktree, &["add", "--", "deduplicate.py"])?;
    git(&f.worktree, &["commit", "-m", "Implement deduplication"])?;
    let first = candidate(&f.worktree)?;
    let first_evidence = submit_evidence(f, &writer, "Implementation report").await?;
    assert_eq!(
        submit_outcome(f, &writer, "completed", &first.commit, first_evidence)
            .await?
            .1["status"],
        "proposal_saved"
    );
    readonly_employee(f, writer.require_employee_id()?).await?;
    let accepted_first = finalize_writer(f, &writer, &first).await?;

    let reviewer = next_run(f, "examine", "read_only").await?;
    assert_ne!(
        reviewer.require_employee_id()?,
        writer.require_employee_id()?
    );
    assert_snapshot(&reviewer, &accepted_first);
    let review_evidence =
        submit_evidence(f, &reviewer, "Initial revision accepted; QA must add tests").await?;
    assert_eq!(
        submit_outcome(f, &reviewer, "tests", &first.commit, review_evidence)
            .await?
            .1["status"],
        "accepted"
    );
    assert_eq!(
        f.setup.harness.store.list_runs(f.task).await?.len(),
        2,
        "QA writer cannot overlap the physically retained reviewer"
    );

    let qa: EmployeeId = f
        .setup
        .harness
        .execute(
            f.setup.project,
            CommandName::CreateEmployee,
            json!({"name":"QA test developer","role":"qa","stage_eligibility":{"mode":"any"}}),
        )
        .await?
        .resource
        .context("QA Employee")?
        .id
        .parse()?;
    f.setup
        .harness
        .execute(
            f.setup.project,
            CommandName::ConfigureEmployeeRuntime,
            json!({"employee_id":qa,"binding":f.setup.binding}),
        )
        .await?;
    let revision = f
        .setup
        .harness
        .store
        .load_task(f.task)
        .await?
        .context("Task")?
        .task
        .revision()
        .get();
    f.setup.harness.execute(f.setup.project,CommandName::SetNextRunEmployee,
        json!({"task_id":f.task,"expected_task_revision":revision,"employee_id":qa,"reason":"Explicit QA test development assignment"})).await?;
    stopped(f, &reviewer).await?;
    let qa_run = next_run(f, "add_tests", "read_write").await?;
    assert_eq!(qa_run.require_employee_id()?, qa);
    let qa_spec: Value = qa_run.run_spec.clone();
    assert_eq!(qa_spec["surface_id"], writer.run_spec["surface_id"]);
    assert_eq!(qa_spec["binding"]["surface"]["mode"], "git_worktree");
    assert_eq!(qa_spec["binding"]["surface"]["repository"], json!(f.source));
    std::fs::write(
        f.worktree.join("test_deduplicate.py"),
        "from deduplicate import deduplicate\n\ndef test_duplicates_are_removed_and_sorted():\n    assert deduplicate([3, 1, 3, 2]) == [1, 2, 3]\n",
    )?;
    git(&f.worktree, &["add", "--", "test_deduplicate.py"])?;
    git(
        &f.worktree,
        &["commit", "-m", "QA adds explicit regression test"],
    )?;
    let second = candidate(&f.worktree)?;
    assert_ne!(first.commit, second.commit);
    assert_ne!(first.tree, second.tree);
    assert_eq!(
        git(&f.worktree, &["rev-parse", "HEAD^"])?,
        first.commit.as_str()
    );
    let qa_evidence = submit_evidence(f, &qa_run, "QA added a committed regression test").await?;
    assert_eq!(
        submit_outcome(f, &qa_run, "completed", &second.commit, qa_evidence)
            .await?
            .1["status"],
        "proposal_saved"
    );
    assert_eq!(
        current_candidate(f).await?.proposal_id,
        accepted_first.proposal_id,
        "submission alone does not replace the accepted candidate"
    );
    readonly_employee(f, qa).await?;
    let accepted_second = finalize_writer(f, &qa_run, &second).await?;
    assert_ne!(accepted_first.proposal_id, accepted_second.proposal_id);
    assert_eq!(
        accepted_second.contributors,
        BTreeSet::from([writer.require_employee_id()?, qa])
    );
    assert_ne!(accepted_first.artifact_ids, accepted_second.artifact_ids);

    let second_reviewer = next_run(f, "examine", "read_only").await?;
    assert_eq!(
        second_reviewer.require_employee_id()?,
        reviewer.require_employee_id()?
    );
    assert!(
        !accepted_second
            .contributors
            .contains(&second_reviewer.require_employee_id()?)
    );
    assert_snapshot(&second_reviewer, &accepted_second);
    let before = f
        .setup
        .harness
        .store
        .list_candidate_reviews(f.setup.project, f.task)
        .await?;
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].candidate_proposal_id, accepted_first.proposal_id);
    assert_eq!(before[0].candidate, first);
    assert_ne!(
        f.setup
            .harness
            .store
            .load_task(f.task)
            .await?
            .context("Task")?
            .task
            .lifecycle(),
        LifecycleStatus::Done
    );
    let evidence = submit_evidence(f, &second_reviewer, "Independent review of QA tests").await?;
    let stale = submit_outcome(f, &second_reviewer, "yes", &first.commit, evidence).await?;
    assert!(
        !stale.0.is_success(),
        "stale first revision cannot satisfy the second review"
    );
    assert_eq!(
        f.setup
            .harness
            .store
            .list_candidate_reviews(f.setup.project, f.task)
            .await?,
        before
    );
    assert_eq!(
        submit_outcome(f, &second_reviewer, "yes", &second.commit, evidence)
            .await?
            .1["status"],
        "accepted"
    );
    let reviews = f
        .setup
        .harness
        .store
        .list_candidate_reviews(f.setup.project, f.task)
        .await?;
    assert_eq!(reviews.len(), 2);
    assert_eq!(
        reviews[0].candidate_proposal_id,
        accepted_second.proposal_id
    );
    assert_eq!(reviews[0].candidate, second);
    assert_eq!(reviews[0].verdict, CandidateVerdict::Accepted);
    assert_eq!(
        reviews[0].subject_artifact_ids,
        accepted_second.artifact_ids
    );
    assert_eq!(reviews[0].stage_id, reviews[1].stage_id);
    assert!(reviews[0].stage_visit > reviews[1].stage_visit);
    let task = f
        .setup
        .harness
        .store
        .load_task(f.task)
        .await?
        .context("Task")?
        .task;
    assert_eq!(task.id(), f.task);
    assert_eq!(task.lifecycle(), LifecycleStatus::Done);
    assert_eq!(
        f.setup
            .harness
            .store
            .list_tasks(f.setup.project)
            .await?
            .len(),
        1
    );
    assert_eq!(
        candidate(&f.source)?,
        f.base,
        "registered source is untouched"
    );
    stopped(f, &second_reviewer).await?;
    Ok(())
}

async fn next_run(f: &mut Fixture, stage: &str, access: &str) -> Result<RunProjection> {
    let provision = f.setup.supervisor.next_provision_for_task(f.task).await?;
    let run = f
        .setup
        .harness
        .store
        .load_run(provision.run_id.parse()?)
        .await?
        .context("Run")?;
    assert_eq!(run.require_task_id()?, f.task);
    assert_eq!(run.require_task_stage()?.stage_id.as_str(), stage);
    assert_eq!(run.run_spec["binding"]["access"], access);
    Ok(run)
}
async fn readonly_employee(f: &Fixture, employee: EmployeeId) -> Result<()> {
    let mut binding = f.setup.binding.clone();
    binding.access = SurfaceAccess::ReadOnly;
    f.setup
        .harness
        .execute(
            f.setup.project,
            CommandName::ConfigureEmployeeRuntime,
            json!({"employee_id":employee,"binding":binding}),
        )
        .await?;
    Ok(())
}
fn assert_snapshot(run: &RunProjection, accepted: &TaskGitCandidate) {
    assert_eq!(run.run_spec["surface_id"], json!(accepted.surface_id));
    assert_eq!(
        run.run_spec["binding"]["surface"]["mode"],
        "git_candidate_snapshot"
    );
    assert_eq!(
        run.run_spec["binding"]["surface"]["candidate"],
        json!(accepted.candidate)
    );
}
async fn submit_evidence(f: &Fixture, run: &RunProjection, title: &str) -> Result<Uuid> {
    let id = Uuid::now_v7();
    let response = call(
        f,
        run,
        "artifact.submit",
        json!({"artifact_kind":"stage_evidence","title":title,
        "body":{"summary":"Fixture-authored report; real Git commits, no provider execution"}}),
        id,
    )
    .await?;
    assert!(response.0.is_success(), "{}", response.1);
    Ok(id)
}
async fn submit_outcome(
    f: &Fixture,
    run: &RunProjection,
    outcome: &str,
    commit: &GitObjectId,
    evidence: Uuid,
) -> Result<(reqwest::StatusCode, Value)> {
    call(
        f,
        run,
        "outcome.submit",
        json!({"stage_id":run.require_task_stage()?.stage_id,"outcome":outcome,
        "artifact_submission_message_ids":[evidence],"candidate_commit":commit}),
        Uuid::now_v7(),
    )
    .await
}
async fn call(
    f: &Fixture,
    run: &RunProjection,
    tool: &str,
    arguments: Value,
    message: Uuid,
) -> Result<(reqwest::StatusCode, Value)> {
    let client = reqwest::Client::builder()
        .unix_socket(
            f.setup
                .root
                .join("gateways")
                .join(run.id.to_string())
                .join("gateway.sock"),
        )
        .timeout(Duration::from_secs(8))
        .build()?;
    let response = client
        .post("http://localhost/tools")
        .json(&json!({"tool":tool,"arguments":arguments,"message_id":message}))
        .send()
        .await?;
    Ok((response.status(), response.json().await?))
}
async fn current_candidate(f: &Fixture) -> Result<TaskGitCandidate> {
    let mut tx = f.setup.harness.store.begin().await?;
    tx.latest_accepted_git_candidate(f.setup.project, f.task)
        .await?
        .context("accepted candidate")
}
async fn stopped(f: &mut Fixture, run: &RunProjection) -> Result<()> {
    let ack = f
        .setup
        .supervisor
        .send_observation_kind(run, run.lease_fencing_token, 1, RunEventKind::Stopped)
        .await?;
    assert_eq!(ack.disposition, AcknowledgementDisposition::Accepted as i32);
    Ok(())
}
async fn finalize_writer(
    f: &mut Fixture,
    run: &RunProjection,
    expected: &GitCandidate,
) -> Result<TaskGitCandidate> {
    let mut tx = f.setup.harness.store.begin().await?;
    let proposal = tx.load_git_proposal(run.id).await?.context("proposal")?;
    assert_eq!(proposal.state, GitProposalState::AwaitingQuiescence);
    assert!(!tx.git_writer_quiescent(&proposal.proposal).await?);
    tx.commit().await?;
    stopped(f, run).await?;
    let core = f.setup.harness.core.clone();
    tokio::spawn(async move {
        core.watchdog_tick(
            Timestamp::now_utc(),
            forge_core::WatchdogDeadlines::default(),
        )
        .await
    })
    .await??;
    let request = f.setup.supervisor.next_git_inspection().await?;
    assert_eq!(request.run_id, run.id.to_string());
    assert_eq!(request.expected_commit, expected.commit.as_str());
    let observed = GitBackend::default()
        .verify_candidate(&f.worktree, &expected.commit, true)
        .await?;
    assert_eq!(observed, *expected);
    let ack = f
        .setup
        .supervisor
        .send_git_inspection(GitCandidateInspectionResult {
            message_id: Uuid::now_v7().to_string(),
            request_command_id: request.command_id,
            run_id: run.id.to_string(),
            lease_fencing_token: run.lease_fencing_token,
            environment_epoch: run.environment_epoch,
            commit: observed.commit.as_str().into(),
            tree: observed.tree.as_str().into(),
            result_code: GitCandidateInspectionCode::Verified as i32,
            host_id: request.host_id,
            boot_id: request.boot_id,
        })
        .await?;
    assert_eq!(ack.disposition, AcknowledgementDisposition::Accepted as i32);
    let accepted = current_candidate(f).await?;
    assert_eq!(accepted.proposal_id, proposal.proposal.id);
    assert_eq!(accepted.candidate, *expected);
    Ok(accepted)
}
fn candidate(path: &Path) -> Result<GitCandidate> {
    Ok(GitCandidate {
        commit: GitObjectId::new(git(path, &["rev-parse", "HEAD"])?)?,
        tree: GitObjectId::new(git(path, &["rev-parse", "HEAD^{tree}"])?)?,
    })
}
fn git(path: &Path, args: &[&str]) -> Result<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "core.hooksPath=/dev/null",
        ])
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
        "private fixture Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim().into())
}
