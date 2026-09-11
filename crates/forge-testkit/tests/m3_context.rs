//! Canonical context and real provider-input materialization, with a non-executing Supervisor.
//! Synthetic credentials only: no inference, embeddings, or personal AgentMemory service.

#[path = "m2_communication_runs/support.rs"]
#[allow(dead_code)]
mod support;

use anyhow::{Context, Result};
use forge_domain::{
    EmployeeId, ProjectId, Timestamp,
    knowledge::{DerivedMemoryEntry, DerivedMemoryKind, KnowledgeSourceRef},
};
use forge_protocol::wire::CommandName;
use forge_provider_common::SecretBytes;
use forge_storage::RunProjection;
use forge_testkit::m0::{M0Harness, single_stage_pipeline};
use serde_json::{Value, json};
use uuid::Uuid;

async fn publish(h: &M0Harness, project: ProjectId, kind: &str, markdown: &str) -> Result<Uuid> {
    let page_id = Uuid::now_v7();
    h.execute(project,CommandName::AuthorKnowledgePage,json!({"page_id":page_id,"expected_page_revision":0,"kind":kind,"title":"Canonical context","markdown":markdown,"source_refs":[]})).await?;
    h.execute(
        project,
        CommandName::PublishKnowledgePage,
        json!({"page_id":page_id,"expected_page_revision":1}),
    )
    .await?;
    Ok(page_id)
}

async fn memory(
    h: &M0Harness,
    project: ProjectId,
    employee: Option<EmployeeId>,
    source: KnowledgeSourceRef,
) -> Result<Uuid> {
    let entry = DerivedMemoryEntry {
        id: Uuid::now_v7(),
        project_id: project,
        revision: 1,
        subject: employee.map_or(
            DerivedMemoryKind::ProjectKnowledgeEntry { task_id: None },
            |employee_id| DerivedMemoryKind::EmployeeMemoryEntry {
                employee_id,
                task_id: None,
            },
        ),
        markdown: "hello".into(),
        source_refs: vec![source],
        content_hash: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".into(),
        coverage: None,
        created_by_job_id: Uuid::now_v7(),
        created_at: Timestamp::now_utc(),
        withdrawn: false,
    };
    let mut tx = h.store.begin().await?;
    tx.lock_project(project).await?.context("project")?;
    tx.insert_derived_memory_revision(&entry).await?;
    tx.commit().await?;
    Ok(entry.id)
}

fn assert_actual_prompt(run: &RunProjection, root: &std::path::Path) -> Result<Value> {
    let instruction = run.run_spec["instruction"]
        .as_str()
        .context("provider instruction")?;
    let contract: Value = serde_json::from_str(instruction)?;
    assert_eq!(
        contract["knowledge_context"],
        run.context_manifest["knowledge_context"]
    );
    let stdin: Value = serde_json::from_slice(&std::fs::read(
        root.join("grants")
            .join(run.id.to_string())
            .join(run.environment_epoch.to_string())
            .join("stdin"),
    )?)?;
    assert_eq!(
        stdin["message"]["content"],
        format!(
            "{}\n\n{}\n\n{instruction}",
            run.run_spec["binding"]["system_prompt"]
                .as_str()
                .context("system prompt")?,
            run.run_spec["binding"]["employee_prompt"]
                .as_str()
                .context("employee prompt")?
        )
    );
    Ok(contract["knowledge_context"].clone())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; synthetic provider input, no inference"]
async fn task_prompt_freezes_canonical_scoped_memory_and_refresh_never_mutates_initial_context()
-> Result<()> {
    let (mut h, project, employee, root) = support::fixture().await?;
    // Closed test-owned port proves initial prompt compilation never needs AgentMemory.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    drop(listener);
    h.core = h.core.clone().with_agentmemory(
        &endpoint,
        SecretBytes::new(b"synthetic-token".to_vec()),
        std::time::Duration::from_millis(100),
    )?;
    h.create_employee(project, "Other private owner").await?;
    let other = h
        .store
        .list_employees(project)
        .await?
        .into_iter()
        .map(|row| row.employee.id())
        .find(|id| *id != employee)
        .context("other Employee")?;
    let policy = publish(&h, project, "policy", "Require independent review.").await?;
    publish(&h, project, "decision", "Use the existing compiler.").await?;
    let event_id = h.store.list_events(project, None, 1).await?.remove(0).id;
    let own = memory(
        &h,
        project,
        Some(employee),
        KnowledgeSourceRef::Event { event_id },
    )
    .await?;
    let private = memory(
        &h,
        project,
        Some(other),
        KnowledgeSourceRef::Event { event_id },
    )
    .await?;
    let stale = memory(
        &h,
        project,
        None,
        KnowledgeSourceRef::KnowledgePage {
            page_id: policy,
            revision: 1,
        },
    )
    .await?;
    let deleted = memory(
        &h,
        project,
        None,
        KnowledgeSourceRef::Event {
            event_id: forge_domain::EventId::new(),
        },
    )
    .await?;
    let mut supervisor = h.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    let pipeline = h.create_pipeline(project, single_stage_pipeline()).await?;
    let task = h.create_task(project, pipeline, "Frozen knowledge").await?;
    h.approve_task(project, task).await?;
    h.start_project(project).await?;
    let provision = supervisor.next_provision_for_task(task).await?;
    let run = h
        .store
        .load_run(provision.run_id.parse()?)
        .await?
        .context("Run")?;
    let bundle = assert_actual_prompt(&run, &root)?;
    assert_eq!(
        bundle["required_pages"]
            .as_array()
            .context("required")?
            .len(),
        2
    );
    let selected = bundle["derived_memory"].as_array().context("memory")?;
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0]["id"], json!(own));
    for denied in [private, stale, deleted] {
        assert!(!selected.iter().any(|entry| entry["id"] == json!(denied)));
    }
    h.execute(project,CommandName::SupersedeKnowledgePage,json!({"page_id":policy,"expected_page_revision":2,"title":"Revised policy","markdown":"Require two independent reviews.","source_refs":[]})).await?;
    let client = support::client(&root, &run)?;
    let message = Uuid::now_v7();
    let (status, refreshed) = support::call(&client, "memory.refresh", json!({}), message).await?;
    assert!(status.is_success(), "{refreshed}");
    assert_eq!(refreshed["delivery"], "tool_response");
    assert_ne!(
        refreshed["context_snapshot_id"],
        run.context_manifest["context_snapshot_id"]
    );
    assert_eq!(
        support::call(&client, "memory.refresh", json!({}), message)
            .await?
            .1,
        refreshed
    );
    assert_eq!(
        h.store
            .load_run(run.id)
            .await?
            .context("Run")?
            .context_manifest,
        run.context_manifest
    );
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_knowledge_context_refreshes WHERE run_id=$1")
            .bind(run.id)
            .fetch_one(&h.pool)
            .await?;
    assert_eq!(rows, 1);
    assert_eq!(
        refreshed["context"]["knowledge_context"]["required_pages"]
            .as_array()
            .context("refreshed rules")?
            .iter()
            .find(|page| page["id"] == json!(policy))
            .context("policy")?["revision"],
        3
    );
    h.stop_project(project).await?;
    assert!(
        !support::call(&client, "memory.refresh", json!({}), Uuid::now_v7())
            .await?
            .0
            .is_success()
    );
    h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; synthetic provider input, no inference"]
async fn required_context_overflow_refuses_dispatch_instead_of_silently_dropping_policy()
-> Result<()> {
    let (h, project, _, _) = support::fixture().await?;
    publish(&h, project, "policy", &"a".repeat(65_536)).await?;
    publish(&h, project, "decision", &"b".repeat(65_536)).await?;
    let mut supervisor = h.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    let pipeline = h.create_pipeline(project, single_stage_pipeline()).await?;
    let task = h
        .create_task(project, pipeline, "Required overflow")
        .await?;
    h.approve_task(project, task).await?;
    h.start_project(project).await?;
    let error = h
        .core
        .dispatch_available(project)
        .await
        .expect_err("must refuse required overflow");
    assert!(
        error
            .to_string()
            .contains("context.required_knowledge_overflow"),
        "{error}"
    );
    assert!(h.store.list_runs_for_project(project).await?.is_empty());
    h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; synthetic provider input, no inference"]
async fn communication_prompt_includes_required_policy_without_granting_task_authority()
-> Result<()> {
    let (h, project, employee, root) = support::fixture().await?;
    publish(&h, project, "policy", "Respect project confidentiality.").await?;
    let pipeline = h.create_pipeline(project, single_stage_pipeline()).await?;
    let task = h
        .create_task(project, pipeline, "Conversation context only")
        .await?;
    support::send_question(&h, project, employee, task).await?;
    let mut supervisor = h.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    h.start_project(project).await?;
    let provision = supervisor.next_communication_provision(employee).await?;
    let run = h
        .store
        .load_run(provision.run_id.parse()?)
        .await?
        .context("Run")?;
    assert_eq!(
        assert_actual_prompt(&run, &root)?["required_pages"]
            .as_array()
            .context("rules")?
            .len(),
        1
    );
    assert!(
        run.context_manifest["capability_grants"]
            .as_array()
            .context("grants")?
            .contains(&json!("memory.refresh"))
    );
    assert!(
        !run.context_manifest["capability_grants"]
            .as_array()
            .context("grants")?
            .contains(&json!("outcome.submit"))
    );
    h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; synthetic provider input, no inference"]
async fn resolution_prompt_includes_required_decision_without_granting_task_authority() -> Result<()>
{
    let (h, project, employee, root) = support::fixture().await?;
    publish(
        &h,
        project,
        "decision",
        "Keep the established architecture.",
    )
    .await?;
    let pipeline = h.create_pipeline(project, single_stage_pipeline()).await?;
    let task = h
        .create_task(project, pipeline, "Resolve a question")
        .await?;
    h.approve_task(project, task).await?;
    h.execute(project,CommandName::ConfigureResolverRoute,json!({"route_key":"engineering","employee_ids":[employee],"assignment_timeout_seconds":60})).await?;
    let revision = h
        .store
        .load_task(task)
        .await?
        .context("Task")?
        .task
        .revision()
        .get();
    h.execute(project,CommandName::RaiseEscalation,json!({"task_id":task,"expected_task_revision":revision,"route_key":"engineering","category":"technical_decision","question":"Which architecture is accepted?"})).await?;
    let mut supervisor = h.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    h.start_project(project).await?;
    let provision = supervisor.next_provision().await?;
    assert_eq!(provision.run_spec_version, 4);
    let run = h
        .store
        .load_run(provision.run_id.parse()?)
        .await?
        .context("Run")?;
    assert_eq!(
        assert_actual_prompt(&run, &root)?["required_pages"]
            .as_array()
            .context("rules")?
            .len(),
        1
    );
    h.shutdown().await;
    Ok(())
}
