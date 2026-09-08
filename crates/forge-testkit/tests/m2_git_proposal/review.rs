//! Generic stage names and outcomes; no implicit QA/test-discovery behavior.
use super::*;
use forge_domain::{EmployeeId, ProjectId, TaskId, candidate_review::CandidateVerdict};
use forge_testkit::m0::ManualSupervisor;
use std::path::Path;

pub(super) fn unbound_pipeline() -> Value {
    let mut value = pipeline("review_pass");
    value["entry_stage_id"] = json!("examine");
    let mut stage = value["stages"][1].clone();
    stage["acceptance_policy"]["independent"] = json!(false);
    value["stages"] = json!([stage]);
    value["transitions"] = json!([
        {"from_stage_id":"examine","outcome":"yes","target":{"kind":"done"}},
        {"from_stage_id":"examine","outcome":"again","target":{"kind":"done"}}
    ]);
    value
}

pub(super) fn pipeline(mode: &str) -> Value {
    let mut stage = json!({"id":"examine","name":"Owner-defined examination","executor_kind":"employee",
        "outcomes":["yes","again"],"workspace":{"kind":"git","access":"read_only"}});
    if mode != "qa_report" {
        stage["acceptance_policy"] = json!({"kind":"candidate_review","independent":true,"verdicts":{"yes":"accepted","again":"rejected"}});
    }
    json!({"name":"Revision assessment","task_kinds":["delivery"],"entry_stage_id":"work","max_stage_visits":6,
    "stages":[{"id":"work","name":"Work","executor_kind":"employee","outcomes":["completed"],"workspace":{"kind":"git","access":"read_write"}},stage],
    "transitions":[
        {"from_stage_id":"work","outcome":"completed","target":{"kind":"stage","stage_id":"examine"},"artifact_requirements":[{"kind":"stage_evidence","minimum_count":1}]},
        {"from_stage_id":"examine","outcome":"yes","target":{"kind":"done"},"artifact_requirements":[{"kind":"stage_evidence","minimum_count":1}]},
        {"from_stage_id":"examine","outcome":"again","target":{"kind":"stage","stage_id":"work"},"artifact_requirements":[{"kind":"stage_evidence","minimum_count":1}]}
    ]})
}

pub(super) async fn finish(
    harness: &M0Harness,
    supervisor: &mut ManualSupervisor,
    project: ProjectId,
    task: TaskId,
    root: &Path,
    mode: &str,
    writer: EmployeeId,
) -> Result<()> {
    let provision = supervisor.next_provision_for_task(task).await?;
    let run = harness
        .wait_for_run_count(task, 2)
        .await?
        .into_iter()
        .find(|run| run.id.to_string() == provision.run_id)
        .context("review Run")?;
    assert_ne!(
        run.require_employee_id()?,
        writer,
        "all eligible contributing writers must be excluded by policy"
    );
    let spec: Value = serde_json::from_str(&provision.run_spec_json)?;
    assert_eq!(spec["binding"]["access"], "read_only");
    assert_eq!(spec["binding"]["surface"]["mode"], "git_candidate_snapshot");
    assert_eq!(
        spec["binding"]["surface"]["candidate"]["commit"],
        "b".repeat(40)
    );
    let client = reqwest::Client::builder()
        .unix_socket(
            root.join("gateways")
                .join(run.id.to_string())
                .join("gateway.sock"),
        )
        .timeout(Duration::from_secs(8))
        .build()?;
    let call = |tool: &str, args: Value, id: Uuid| {
        client
            .post("http://localhost/tools")
            .json(&json!({"tool":tool,"arguments":args,"message_id":id}))
    };
    let evidence = Uuid::now_v7();
    let artifact=call("artifact.submit",json!({"artifact_kind":"stage_evidence","title":"Examination report","body":{"summary":"Synthetic examiner report"}}),evidence).send().await?;
    assert!(artifact.status().is_success(), "{}", artifact.text().await?);
    let outcome = if mode == "review_rework" {
        "again"
    } else {
        "yes"
    };
    let mut args = json!({"stage_id":"examine","outcome":outcome,"artifact_submission_message_ids":[evidence],"candidate_commit":"a".repeat(40)});
    assert!(
        !call("outcome.submit", args.clone(), Uuid::now_v7())
            .send()
            .await?
            .status()
            .is_success(),
        "stale reviewed revision must be rejected"
    );
    args["candidate_commit"] = json!("b".repeat(40));
    let result = call("outcome.submit", args, Uuid::now_v7()).send().await?;
    assert!(result.status().is_success(), "{}", result.text().await?);
    assert_eq!(
        result.json::<Value>().await?["status"],
        "accepted",
        "reader result is not a writer proposal"
    );
    let reviews = harness.store.list_candidate_reviews(project, task).await?;
    if mode == "qa_report" {
        assert!(
            reviews.is_empty(),
            "QA report alone does not imply a configured acceptance policy"
        );
    } else {
        assert_eq!(reviews.len(), 1);
        let review = &reviews[0];
        assert_eq!(review.candidate.commit.as_str(), "b".repeat(40));
        assert_eq!(review.employee_id, run.require_employee_id()?);
        assert_eq!(
            review.verdict,
            if mode == "review_rework" {
                CandidateVerdict::Rejected
            } else {
                CandidateVerdict::Accepted
            }
        );
        assert_eq!(review.subject_artifact_ids.len(), 1);
        assert_eq!(review.verdict_artifact_ids.len(), 1);
        assert!(
            review
                .subject_artifact_ids
                .is_disjoint(&review.verdict_artifact_ids)
        );
        assert_http_review(harness, project, task, review.id).await?;
    }
    let observed = harness.store.load_task(task).await?.context("Task")?.task;
    if mode == "review_rework" {
        assert_eq!(observed.id(), task);
        assert_eq!(
            observed.current_stage_id().context("stage")?.as_str(),
            "work"
        );
        assert_eq!(
            harness.store.list_runs(task).await?.len(),
            2,
            "a reader still owns a physical slot"
        );
        supervisor
            .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Stopped)
            .await?;
        supervisor.next_provision_for_task(task).await?;
        assert_eq!(harness.wait_for_run_count(task, 3).await?.len(), 3);
    } else {
        assert_eq!(observed.lifecycle(), LifecycleStatus::Done);
    }
    Ok(())
}

async fn assert_http_review(
    harness: &M0Harness,
    project: ProjectId,
    task: TaskId,
    id: Uuid,
) -> Result<()> {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let router = forge_core::router(harness.core.clone());
    let prefix = format!("/v1/projects/{project}/tasks/{task}/reviews");
    let get = |path: String| {
        let router = router.clone();
        async move {
            let response = router
                .oneshot(Request::builder().uri(path).body(Body::empty())?)
                .await?;
            let status = response.status();
            let value: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
            Ok::<_, anyhow::Error>((status, value))
        }
    };
    let (status, value) = get(format!("{prefix}?limit=1")).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["items"].as_array().context("review items")?.len(), 1);
    assert_eq!(value["items"][0]["id"], id.to_string());
    assert!(
        value["items"][0]["recorded_at"]
            .as_str()
            .is_some_and(|time| time.contains('T'))
    );
    assert!(value["next_cursor"].is_null());
    let (_, tail) = get(format!("{prefix}?after={id}")).await?;
    assert!(tail["items"].as_array().context("review tail")?.is_empty());
    for suffix in ["limit=0", "limit=101", "after=invalid", "extra=1"] {
        assert_eq!(
            get(format!("{prefix}?{suffix}")).await?.0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        get(format!(
            "/v1/projects/{}/tasks/{task}/reviews",
            Uuid::now_v7()
        ))
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        harness
            .store
            .list_candidate_reviews(project, task)
            .await?
            .len(),
        1
    );
    Ok(())
}
