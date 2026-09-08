//! Exact fenced reporting, not management delegation through the worker Gateway.
use super::*;
use forge_domain::{ProjectId, TaskId};
use forge_storage::RunProjection;

pub(super) async fn report(
    harness: &M0Harness,
    client: &reqwest::Client,
    project: ProjectId,
    task: TaskId,
    run: &RunProjection,
) -> Result<()> {
    let artifact: Uuid =
        sqlx::query_scalar("SELECT id FROM artifacts WHERE project_id=$1 AND task_id=$2 LIMIT 1")
            .bind(project.as_uuid())
            .bind(task.as_uuid())
            .fetch_one(&harness.pool)
            .await?;
    let args = json!({"description":"A separate issue observed while working","severity":"owner-defined","evidence":[artifact]});
    let message = Uuid::now_v7();
    let call = |tool: &str, args: Value, id: Uuid| {
        client
            .post("http://localhost/tools")
            .json(&json!({"tool":tool,"arguments":args,"message_id":id}))
    };
    let before = harness.store.load_task(task).await?.context("Task")?.task;
    let response = call("finding.report", args.clone(), message).send().await?;
    assert!(response.status().is_success(), "{}", response.text().await?);
    let result = response.json::<Value>().await?;
    assert_eq!(result["task_created"], false);
    assert_eq!(
        call("finding.report", args.clone(), message)
            .send()
            .await?
            .json::<Value>()
            .await?,
        result
    );
    let id: Uuid = result["finding_id"]
        .as_str()
        .context("Finding id")?
        .parse()?;
    let mut tx = harness.store.begin().await?;
    let finding = tx.lock_finding(id).await?.context("Finding")?;
    assert_eq!(
        finding.reported_by.id().as_uuid(),
        run.require_employee_id()?.as_uuid()
    );
    assert_eq!(finding.source_task_id, task);
    assert_eq!(finding.source_run.context("Run provenance")?.run_id, run.id);
    assert_eq!(
        finding.source_run.context("Run provenance")?.fencing_token,
        run.lease_fencing_token
    );
    assert_eq!(
        finding.evidence,
        BTreeSet::from([forge_domain::ArtifactId::from(artifact)])
    );
    tx.commit().await?;
    let mut forged = args.clone();
    forged["source_task_id"] = json!(Uuid::now_v7());
    assert!(
        !call("finding.report", forged, Uuid::now_v7())
            .send()
            .await?
            .status()
            .is_success()
    );
    let mut foreign = args.clone();
    foreign["evidence"] = json!([Uuid::now_v7()]);
    assert!(
        !call("finding.report", foreign, Uuid::now_v7())
            .send()
            .await?
            .status()
            .is_success()
    );
    let mut changed = args.clone();
    changed["description"] = json!("Conflicting retry");
    assert!(
        !call("finding.report", changed, message)
            .send()
            .await?
            .status()
            .is_success()
    );
    assert!(
        !call("promote_finding", json!({"finding_id":id}), Uuid::now_v7())
            .send()
            .await?
            .status()
            .is_success()
    );
    assert_eq!(
        harness.store.load_task(task).await?.context("Task")?.task,
        before
    );
    assert_eq!(harness.store.list_tasks(project).await?.len(), 1);
    assert_eq!(
        harness
            .store
            .finding_page(project, None, None, 100)
            .await?
            .len(),
        1
    );
    Ok(())
}
