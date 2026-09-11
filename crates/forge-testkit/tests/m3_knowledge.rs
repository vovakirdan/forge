//! Real PostgreSQL/Core command acceptance. No inference or shared schema mutation.

use anyhow::{Context, Result};
use forge_application::CommandEnvelope;
use forge_domain::{
    EmployeeId, ProjectId, Timestamp,
    knowledge::{
        DerivedMemoryEntry, DerivedMemoryKind, KnowledgePageStatus, KnowledgeSourceRef, MemoryScope,
    },
};
use forge_protocol::wire::{CommandName, CommandRequest, CommandStatus};
use forge_testkit::m0::M0Harness;
use serde_json::{Value, json};
use uuid::Uuid;

async fn http_json(
    router: axum::Router,
    method: &str,
    uri: &str,
    body: Value,
) -> Result<(axum::http::StatusCode, Value)> {
    use tower::ServiceExt;
    let request = axum::http::Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .body(axum::body::Body::from(serde_json::to_vec(&body)?))?;
    let response = router.oneshot(request).await?;
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024).await?;
    Ok((status, serde_json::from_slice(&body)?))
}

fn command(name: &str) -> Result<CommandName> {
    Ok(serde_json::from_value(json!(name))?)
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference"]
async fn knowledge_http_reads_preserve_scope_and_revision_pagination() -> Result<()> {
    let h = M0Harness::start_configured(Ok).await?;
    let project = h.create_project("knowledge http").await?;
    let other = h.create_project("other knowledge http").await?;
    let page = Uuid::now_v7();
    let router = forge_core::router(h.core.clone());
    let (status, receipt) = http_json(
        router.clone(),
        "POST",
        "/v1/commands/author_knowledge_page",
        json!({"project_id":project,"expected_revision":1,"payload":page_input(page)}),
    )
    .await?;
    assert_eq!(status, axum::http::StatusCode::OK, "{receipt}");
    invoke(
        &h,
        project,
        "publish_knowledge_page",
        json!({"page_id":page,"expected_page_revision":1}),
    )
    .await?;
    let path = format!("/v1/projects/{project}/knowledge/{page}/history?limit=1");
    let (status, history) = http_json(router.clone(), "GET", &path, Value::Null).await?;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert_eq!(history["next_cursor"], "1");
    assert_eq!(history["items"][0]["status"], "draft");
    let (_, next) = http_json(
        router.clone(),
        "GET",
        &format!("{path}&after=1"),
        Value::Null,
    )
    .await?;
    assert_eq!(next["items"][0]["status"], "published");
    assert!(next["next_cursor"].is_null());
    let (status, _) = http_json(
        router.clone(),
        "GET",
        &format!("/v1/projects/{other}/knowledge/{page}"),
        Value::Null,
    )
    .await?;
    assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
    let (status, _) = http_json(
        router,
        "GET",
        &format!("/v1/projects/{project}/knowledge?limit=101"),
        Value::Null,
    )
    .await?;
    assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
    h.shutdown().await;
    Ok(())
}

async fn invoke(
    h: &M0Harness,
    project: ProjectId,
    name: &str,
    payload: Value,
) -> Result<forge_protocol::wire::CommandReceipt> {
    h.execute(project, command(name)?, payload).await
}

fn page_input(page: Uuid) -> Value {
    json!({"page_id":page,"expected_page_revision":0,"kind":"policy","title":"Review policy","markdown":"Require independent review.","source_refs":[]})
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference"]
async fn named_knowledge_commands_preserve_authority_history_receipts_and_projection_identity()
-> Result<()> {
    let h = M0Harness::start_configured(Ok).await?;
    let project = h.create_project("knowledge history").await?;
    let id = Uuid::now_v7();
    let envelope = CommandEnvelope::parse(
        command("author_knowledge_page")?,
        CommandRequest {
            project_id: project.to_string(),
            expected_revision: h
                .store
                .load_project(project)
                .await?
                .context("project")?
                .revision(),
            payload: page_input(id).as_object().cloned().context("payload")?,
        },
        Uuid::now_v7().to_string(),
    )?;
    let applied = h.core.execute_command(envelope.clone()).await?;
    let before = h.store.pending_knowledge_projections(project, 100).await?;
    let replay = h.core.execute_command(envelope).await?;
    assert_eq!(replay.status, CommandStatus::Replayed);
    assert_eq!(replay.event_ids, applied.event_ids);
    assert_eq!(
        h.store.pending_knowledge_projections(project, 100).await?,
        before
    );
    assert!(
        h.store
            .list_published_knowledge_pages(project)
            .await?
            .is_empty()
    );
    invoke(
        &h,
        project,
        "publish_knowledge_page",
        json!({"page_id":id,"expected_page_revision":1}),
    )
    .await?;
    let published = h
        .store
        .load_knowledge_page(project, id)
        .await?
        .context("page")?;
    assert!(published.is_authoritative());
    invoke(&h,project,"supersede_knowledge_page",json!({"page_id":id,"expected_page_revision":2,"title":"Review policy","markdown":"Require two independent reviews.","source_refs":[]})).await?;
    invoke(
        &h,
        project,
        "withdraw_knowledge_page",
        json!({"page_id":id,"expected_page_revision":3}),
    )
    .await?;
    let history = h.store.knowledge_page_history(project, id).await?;
    assert_eq!(history.len(), 4);
    assert_eq!(history[1], published);
    assert_eq!(history[3].status, KnowledgePageStatus::Withdrawn);
    assert!(
        h.store
            .list_published_knowledge_pages(project)
            .await?
            .is_empty()
    );
    let counts: (i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM event_log WHERE project_id=$1 AND event_type='knowledge_page_changed'),(SELECT count(*) FROM outbox o JOIN event_log e ON e.id=o.event_id WHERE e.project_id=$1 AND e.event_type='knowledge_page_changed')")
        .bind(project.as_uuid()).fetch_one(&h.pool).await?;
    assert_eq!(counts, (4, 4));
    let operations = h.store.pending_knowledge_projections(project, 100).await?;
    assert_eq!(operations.len(), 4);
    let mut tx = h.store.begin().await?;
    assert!(
        !tx.mark_knowledge_projection_ready(before[0].id, &"0".repeat(64))
            .await?
    );
    assert!(
        !tx.mark_knowledge_projection_ready(before[0].id, &before[0].content_hash)
            .await?
    );
    tx.commit().await?;
    let mut tx = h.store.begin().await?;
    assert_eq!(tx.rebuild_knowledge_projections(project).await?, 4);
    tx.commit().await?;
    assert_eq!(
        h.store
            .pending_knowledge_projections(project, 100)
            .await?
            .iter()
            .map(|operation| operation.id)
            .collect::<Vec<_>>(),
        operations
            .iter()
            .map(|operation| operation.id)
            .collect::<Vec<_>>()
    );
    h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference"]
async fn unknown_and_cross_project_evidence_cannot_create_knowledge() -> Result<()> {
    let h = M0Harness::start_configured(Ok).await?;
    let project = h.create_project("knowledge source").await?;
    let other = h.create_project("other knowledge source").await?;
    let foreign = h.store.list_events(other, None, 1).await?.remove(0).id;
    for event in [foreign, forge_domain::EventId::new()] {
        let page = Uuid::now_v7();
        let revision = h
            .store
            .load_project(project)
            .await?
            .context("project")?
            .revision();
        let mut payload = page_input(page);
        payload["source_refs"] = json!([{"kind":"event","event_id":event}]);
        assert!(
            invoke(&h, project, "author_knowledge_page", payload)
                .await
                .is_err()
        );
        assert!(h.store.load_knowledge_page(project, page).await?.is_none());
        assert_eq!(
            h.store
                .load_project(project)
                .await?
                .context("project")?
                .revision(),
            revision
        );
    }
    assert!(
        h.store
            .pending_knowledge_projections(project, 100)
            .await?
            .is_empty()
    );
    h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference"]
async fn withdrawal_remains_available_after_a_cited_rule_is_withdrawn() -> Result<()> {
    let h = M0Harness::start_configured(Ok).await?;
    let project = h.create_project("withdraw cited rule").await?;
    let source = Uuid::now_v7();
    let dependent = Uuid::now_v7();
    invoke(&h, project, "author_knowledge_page", page_input(source)).await?;
    invoke(
        &h,
        project,
        "publish_knowledge_page",
        json!({"page_id":source,"expected_page_revision":1}),
    )
    .await?;
    let mut payload = page_input(dependent);
    payload["source_refs"] = json!([{"kind":"knowledge_page","page_id":source,"revision":2}]);
    invoke(&h, project, "author_knowledge_page", payload).await?;
    invoke(
        &h,
        project,
        "publish_knowledge_page",
        json!({"page_id":dependent,"expected_page_revision":1}),
    )
    .await?;
    for id in [source, dependent] {
        invoke(
            &h,
            project,
            "withdraw_knowledge_page",
            json!({"page_id":id,"expected_page_revision":2}),
        )
        .await?;
    }
    assert!(
        h.store
            .list_published_knowledge_pages(project)
            .await?
            .is_empty()
    );
    assert_eq!(
        h.store.knowledge_page_history(project, dependent).await?[2]
            .content
            .source_refs,
        vec![KnowledgeSourceRef::KnowledgePage {
            page_id: source,
            revision: 2
        }]
    );
    h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference"]
async fn receipt_failure_rolls_back_page_revision_audit_and_projection_backlog() -> Result<()> {
    let h = M0Harness::start_configured(Ok).await?;
    let project = h.create_project("knowledge atomicity").await?;
    let before = h
        .store
        .load_project(project)
        .await?
        .context("project")?
        .revision();
    // The failure trigger belongs only to this harness's randomly named schema.
    sqlx::raw_sql("CREATE FUNCTION reject_knowledge_receipt() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.command_name='author_knowledge_page' THEN RAISE EXCEPTION 'injected knowledge receipt failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER fail_knowledge_receipt BEFORE INSERT ON idempotency_keys FOR EACH ROW EXECUTE FUNCTION reject_knowledge_receipt();")
        .execute(&h.pool).await?;
    let id = Uuid::now_v7();
    assert!(
        invoke(&h, project, "author_knowledge_page", page_input(id))
            .await
            .is_err()
    );
    assert!(h.store.load_knowledge_page(project, id).await?.is_none());
    assert!(
        h.store
            .knowledge_page_history(project, id)
            .await?
            .is_empty()
    );
    assert!(
        h.store
            .pending_knowledge_projections(project, 100)
            .await?
            .is_empty()
    );
    assert_eq!(
        h.store
            .load_project(project)
            .await?
            .context("project")?
            .revision(),
        before
    );
    assert_eq!(h.store.list_events(project, None, 100).await?.len(), 1);
    h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference"]
async fn derived_storage_preserves_personal_scope_immutable_revisions_and_exact_replay()
-> Result<()> {
    let h = M0Harness::start_configured(Ok).await?;
    let project = h.create_project("personal knowledge").await?;
    h.create_employee(project, "Bob").await?;
    let employee = h
        .store
        .list_employees(project)
        .await?
        .remove(0)
        .employee
        .id();
    let source = h.store.list_events(project, None, 1).await?.remove(0).id;
    let mut entry = DerivedMemoryEntry {
        id: Uuid::now_v7(),
        project_id: project,
        revision: 1,
        subject: DerivedMemoryKind::EmployeeMemoryEntry {
            employee_id: employee,
            task_id: None,
        },
        markdown: "hello".into(),
        source_refs: vec![KnowledgeSourceRef::Event { event_id: source }],
        content_hash: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".into(),
        coverage: None,
        created_by_job_id: Uuid::now_v7(),
        created_at: Timestamp::now_utc(),
        withdrawn: false,
    };
    let mut tx = h.store.begin().await?;
    tx.lock_project(project).await?.context("project")?;
    tx.validate_knowledge_sources(project, entry.subject.scope(), &entry.source_refs)
        .await?;
    assert!(tx.insert_derived_memory_revision(&entry).await?);
    assert!(!tx.insert_derived_memory_revision(&entry).await?);
    tx.commit().await?;
    assert_eq!(
        h.store
            .visible_derived_memory(project, Some(employee), None, 100)
            .await?,
        vec![entry.clone()]
    );
    assert!(
        h.store
            .visible_derived_memory(project, Some(EmployeeId::new()), None, 100)
            .await?
            .is_empty()
    );
    assert!(
        h.store
            .visible_derived_memory(project, None, None, 100)
            .await?
            .is_empty()
    );
    assert_eq!(
        entry.subject.scope(),
        MemoryScope::Personal {
            employee_id: employee
        }
    );
    let original = entry.clone();
    entry.markdown = "a tampered retry".into();
    let mut tx = h.store.begin().await?;
    assert!(tx.insert_derived_memory_revision(&entry).await.is_err());
    drop(tx);
    entry = original.clone();
    entry.revision = 2;
    entry.withdrawn = true;
    let mut tx = h.store.begin().await?;
    assert!(tx.insert_derived_memory_revision(&entry).await?);
    tx.commit().await?;
    assert!(
        h.store
            .visible_derived_memory(project, Some(employee), None, 100)
            .await?
            .is_empty()
    );
    assert_eq!(
        h.store.derived_memory_history(project, entry.id).await?,
        vec![original, entry]
    );
    assert_eq!(
        h.store
            .pending_knowledge_projections(project, 100)
            .await?
            .len(),
        2
    );
    h.shutdown().await;
    Ok(())
}
