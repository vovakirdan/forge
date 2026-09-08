//! A prior pass is evidence for its own candidate, never a permanent bypass.
use super::*;

async fn passed_at_human_gate() -> Result<(support::Fixture, HookRunSpec)> {
    let mut f = Box::pin(support::setup("history")).await?;
    let (run, spec) = hook_run(&mut f).await?;
    f.supervisor
        .send_hook_result(&run, 1, Some(&result(&spec, HookVerdict::Passed)))
        .await?;
    support::tick(&f.h).await?;
    let task = f.h.store.load_task(f.task).await?.context("Task")?.task;
    assert_eq!(task.lifecycle(), LifecycleStatus::Waiting);
    assert_eq!(task.current_stage_id().context("stage")?.as_str(), "gate");
    assert!(gate(&f, None).await?);
    Ok((f, spec))
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference or Hook process"]
async fn newer_running_and_failed_invocation_shadow_previous_pass() -> Result<()> {
    let (mut f, first) = passed_at_human_gate().await?;
    f.h.submit_external_stage_outcome(f.project, f.task, "again")
        .await?;
    support::tick(&f.h).await?;
    let (run, second) = hook_run(&mut f).await?;
    assert_eq!(
        first.assignment.candidate_proposal_id,
        second.assignment.candidate_proposal_id
    );
    assert!(second.assignment.stage_visit > first.assignment.stage_visit);
    assert!(
        !gate(&f, None).await?,
        "new running invocation shadows old pass"
    );
    f.supervisor
        .send_hook_result(&run, 1, Some(&result(&second, HookVerdict::Failed)))
        .await?;
    support::tick(&f.h).await?;
    assert!(
        !gate(&f, None).await?,
        "latest failure cannot fall back to earlier pass"
    );
    drop(f.supervisor);
    f.h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference or Hook process"]
async fn changed_candidate_does_not_inherit_previous_candidate_pass() -> Result<()> {
    let (mut f, first) = passed_at_human_gate().await?;
    f.h.submit_external_stage_outcome(f.project, f.task, "rework")
        .await?;
    support::advance_candidate(&mut f)?;
    f.h.core
        .dispatch_available(f.project)
        .await
        .context("dispatch after human decision")?;
    support::tick(&f.h).await?;
    Box::pin(support::submit_writer(&mut f, false))
        .await
        .context("second writer")?;
    let (run, second) = hook_run(&mut f).await.context("second Hook")?;
    assert_ne!(
        first.assignment.candidate_proposal_id,
        second.assignment.candidate_proposal_id
    );
    assert_ne!(first.candidate.commit, second.candidate.commit);
    assert!(
        gate(&f, Some(first.assignment.candidate_proposal_id)).await?,
        "old evidence is retained"
    );
    assert!(!gate(&f, None).await?, "current candidate has no pass");
    assert!(!gate(&f, Some(second.assignment.candidate_proposal_id)).await?);
    f.supervisor
        .send_hook_result(&run, 1, Some(&result(&second, HookVerdict::Passed)))
        .await?;
    support::tick(&f.h).await?;
    assert!(gate(&f, None).await?);
    drop(f.supervisor);
    f.h.shutdown().await;
    Ok(())
}
