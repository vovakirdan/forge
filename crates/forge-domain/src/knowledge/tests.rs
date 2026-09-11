use super::*;
use crate::{ActorId, CommandId, ProjectId, Timestamp};
use uuid::Uuid;

fn author() -> KnowledgePageMutation {
    KnowledgePageMutation::Author {
        page_id: Uuid::now_v7(),
        expected_page_revision: 0,
        kind: KnowledgePageKind::Policy,
        content: KnowledgePageContent {
            title: "Review policy".into(),
            markdown: "All changes require review.".into(),
            source_refs: vec![],
        },
    }
}

fn apply(
    current: Option<&KnowledgePage>,
    mutation: &KnowledgePageMutation,
    actor: Actor,
) -> Result<KnowledgePage, crate::DomainError> {
    KnowledgePage::apply(
        current,
        current.map_or_else(ProjectId::new, |page| page.project_id),
        mutation,
        actor,
        CommandId::new(),
        Timestamp::now_utc(),
        "a".repeat(64),
    )
}

#[test]
fn author_publish_supersede_withdraw_preserves_each_prior_revision() {
    let actor = Actor::human(ActorId::new());
    let draft = apply(None, &author(), actor).unwrap();
    assert!(!draft.is_authoritative());
    let published = apply(
        Some(&draft),
        &KnowledgePageMutation::Publish {
            page_id: draft.id,
            expected_page_revision: 1,
        },
        actor,
    )
    .unwrap();
    assert!(published.is_authoritative());
    let amended = apply(
        Some(&published),
        &KnowledgePageMutation::Supersede {
            page_id: draft.id,
            expected_page_revision: 2,
            content: KnowledgePageContent {
                title: "Review policy".into(),
                markdown: "Two reviews are required.".into(),
                source_refs: vec![],
            },
        },
        actor,
    )
    .unwrap();
    assert_eq!(amended.supersedes_revision, Some(2));
    assert_eq!(published.content.markdown, "All changes require review.");
    let withdrawn = apply(
        Some(&amended),
        &KnowledgePageMutation::Withdraw {
            page_id: draft.id,
            expected_page_revision: 3,
        },
        actor,
    )
    .unwrap();
    assert!(!withdrawn.is_authoritative());
    assert_eq!(withdrawn.revision, 4);
}

#[test]
fn employee_and_core_cannot_author_authority() {
    for actor in [
        Actor::employee(ActorId::new()),
        Actor::core(ActorId::new()),
        Actor::supervisor(ActorId::new()),
    ] {
        assert!(apply(None, &author(), actor).is_err());
    }
}

#[test]
fn stale_page_command_and_implicit_published_edit_are_rejected() {
    let actor = Actor::human(ActorId::new());
    let mutation = author();
    let draft = apply(None, &mutation, actor).unwrap();
    assert!(apply(Some(&draft), &mutation, actor).is_err());
    let published = apply(
        Some(&draft),
        &KnowledgePageMutation::Publish {
            page_id: draft.id,
            expected_page_revision: 1,
        },
        actor,
    )
    .unwrap();
    let KnowledgePageMutation::Author { kind, content, .. } = mutation else {
        panic!("author fixture")
    };
    assert!(
        apply(
            Some(&published),
            &KnowledgePageMutation::Author {
                page_id: draft.id,
                expected_page_revision: 2,
                kind,
                content
            },
            actor
        )
        .is_err()
    );
}

#[test]
fn source_free_derived_memory_and_scope_widening_are_not_valid() {
    let employee = crate::EmployeeId::new();
    let entry = DerivedMemoryEntry {
        id: Uuid::now_v7(),
        project_id: ProjectId::new(),
        revision: 1,
        subject: DerivedMemoryKind::EmployeeMemoryEntry {
            employee_id: employee,
            task_id: None,
        },
        markdown: "Checked the source.".into(),
        source_refs: vec![],
        content_hash: "a".repeat(64),
        coverage: None,
        created_by_job_id: Uuid::now_v7(),
        created_at: Timestamp::now_utc(),
        withdrawn: false,
    };
    assert!(entry.validate().is_err());
    assert!(entry.subject.scope().visible_to(Some(employee)));
    assert!(
        !entry
            .subject
            .scope()
            .visible_to(Some(crate::EmployeeId::new()))
    );
    assert!(!entry.subject.scope().visible_to(None));
}

#[test]
fn knowledge_transport_round_trips_bounded_editorial_content() {
    let input = author();
    let value = serde_json::to_value(&input).unwrap();
    assert_eq!(
        serde_json::from_value::<KnowledgePageMutation>(value).unwrap(),
        input
    );
}

#[test]
fn oversized_markdown_and_duplicate_evidence_are_refused() {
    let actor = Actor::human(ActorId::new());
    let mut input = author();
    if let KnowledgePageMutation::Author { content, .. } = &mut input {
        content.markdown = "x".repeat(MAX_KNOWLEDGE_MARKDOWN_BYTES + 1);
    }
    assert!(apply(None, &input, actor).is_err());
    let source = KnowledgeSourceRef::Event {
        event_id: crate::EventId::new(),
    };
    assert!(validate_sources(&[source.clone(), source]).is_err());
}
