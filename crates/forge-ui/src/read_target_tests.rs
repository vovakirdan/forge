use crate::{
    http::ApiError,
    read_target::{CORE_BODY_LIMIT, DETAIL_BODY_LIMIT, ReadTarget},
};

const PROJECT: &str = "01988000-0000-7000-8000-000000000001";
const TASK: &str = "01988000-0000-7000-8000-000000000002";

#[test]
fn task_surface_reads_are_exact_scoped_and_bounded() {
    for resource in ["git-source-policy", "file-inputs"] {
        let path = format!("/api/projects/{PROJECT}/tasks/{TASK}/{resource}");
        let target = ReadTarget::parse(&path, None).unwrap();
        assert_eq!(
            target.path,
            format!("/v1/projects/{PROJECT}/tasks/{TASK}/{resource}")
        );
        assert_eq!(target.body_limit, DETAIL_BODY_LIMIT);
        assert!(ReadTarget::parse(&path, Some("limit=1")).is_err());
    }
    for resource in ["file-snapshots", "reviews", "integrations"] {
        let path = format!("/api/projects/{PROJECT}/tasks/{TASK}/{resource}");
        let target = ReadTarget::parse(&path, Some(&format!("limit=5&after={TASK}"))).unwrap();
        assert_eq!(
            target.path,
            format!("/v1/projects/{PROJECT}/tasks/{TASK}/{resource}?limit=5&after={TASK}")
        );
        assert_eq!(target.body_limit, DETAIL_BODY_LIMIT);
        for query in [
            "limit=0",
            "limit=101",
            "after=bad",
            "limit=5&limit=5",
            "object_key=secret",
        ] {
            assert!(ReadTarget::parse(&path, Some(query)).is_err());
        }
        assert!(ReadTarget::parse(&format!("{path}/raw"), None).is_err());
    }
    assert!(
        ReadTarget::parse(
            &format!("/api/projects/{PROJECT}/tasks/not-a-task/file-inputs"),
            None
        )
        .is_err()
    );
}

#[test]
fn delivery_evidence_is_thread_scoped_and_sequence_paged() {
    let path = format!("/api/projects/{PROJECT}/threads/{TASK}/delivery");
    let target = ReadTarget::parse(&path, Some("after=12&limit=7")).unwrap();
    assert_eq!(
        target.path,
        format!("/v1/projects/{PROJECT}/threads/{TASK}/delivery?limit=7&after=12")
    );
    assert_eq!(target.body_limit, DETAIL_BODY_LIMIT);
    for query in [
        "after=-1",
        "after=abc",
        "limit=0",
        "limit=101",
        "after=1&after=2",
        "message_id=x",
    ] {
        assert!(ReadTarget::parse(&path, Some(query)).is_err());
    }
    assert!(ReadTarget::parse(&format!("{path}/raw"), None).is_err());
}

#[test]
fn management_lists_and_escalation_detail_are_bounded() {
    for resource in [
        "resume-schedules",
        "escalations",
        "next-run-constraints",
        "recovery-runs",
    ] {
        let path = format!("/api/projects/{PROJECT}/{resource}");
        let target = ReadTarget::parse(&path, Some(&format!("limit=7&cursor={TASK}"))).unwrap();
        assert_eq!(
            target.path,
            format!("/v1/projects/{PROJECT}/{resource}?limit=7&cursor={TASK}")
        );
        assert_eq!(target.body_limit, DETAIL_BODY_LIMIT);
        for query in [
            "limit=0",
            "limit=21",
            "cursor=bad",
            "after=1",
            "limit=7&limit=7",
        ] {
            assert!(ReadTarget::parse(&path, Some(query)).is_err());
        }
    }
    let detail = format!("/api/projects/{PROJECT}/escalations/{TASK}");
    assert_eq!(
        ReadTarget::parse(&detail, None).unwrap().path,
        format!("/v1/projects/{PROJECT}/escalations/{TASK}")
    );
    assert!(ReadTarget::parse(&detail, Some("limit=1")).is_err());
    assert!(ReadTarget::parse(&format!("{detail}/raw"), None).is_err());
    let routes = format!("/api/projects/{PROJECT}/resolver-routes");
    assert_eq!(
        ReadTarget::parse(&routes, Some("limit=7&cursor=human_route"))
            .unwrap()
            .path,
        format!("/v1/projects/{PROJECT}/resolver-routes?limit=7&cursor=human_route")
    );
    for query in [
        "cursor=Bad",
        "cursor=route-name",
        "cursor=",
        "limit=21",
        "after=1",
    ] {
        assert!(ReadTarget::parse(&routes, Some(query)).is_err());
    }
    let recovery = format!("/api/projects/{PROJECT}/recovery");
    assert_eq!(
        ReadTarget::parse(&recovery, None).unwrap().path,
        format!("/v1/projects/{PROJECT}/recovery")
    );
    assert!(ReadTarget::parse(&recovery, Some("limit=1")).is_err());
    let readiness = format!("/api/projects/{PROJECT}/recovery-runs/{TASK}/assessment-readiness");
    assert_eq!(
        ReadTarget::parse(&readiness, None).unwrap().path,
        format!("/v1/projects/{PROJECT}/recovery-runs/{TASK}/assessment-readiness")
    );
    assert!(ReadTarget::parse(&readiness, Some("limit=1")).is_err());
    assert!(ReadTarget::parse(&format!("{readiness}/raw"), None).is_err());
}

#[test]
fn run_context_and_evidence_are_scoped_and_bounded() {
    let context = format!("/api/projects/{PROJECT}/runs/{TASK}/context");
    assert_eq!(
        ReadTarget::parse(&context, None).unwrap().path,
        format!("/v1/projects/{PROJECT}/runs/{TASK}/context")
    );
    assert!(ReadTarget::parse(&context, Some("x=1")).is_err());
    let evidence = format!("/api/projects/{PROJECT}/runs/{TASK}/evidence");
    assert_eq!(
        ReadTarget::parse(&evidence, Some(&format!("limit=50&cursor={TASK}")))
            .unwrap()
            .path,
        format!("/v1/projects/{PROJECT}/runs/{TASK}/evidence?limit=50&cursor={TASK}")
    );
    for query in [
        "limit=0",
        "limit=51",
        "cursor=nope",
        "after=1",
        "limit=20&limit=20",
    ] {
        assert!(ReadTarget::parse(&evidence, Some(query)).is_err());
    }
    assert!(ReadTarget::parse(&format!("{evidence}/raw"), None).is_err());
}

#[test]
fn task_handoffs_are_scoped_bounded_and_metadata_only() {
    let path = format!("/api/projects/{PROJECT}/tasks/{TASK}/handoffs");
    let target = ReadTarget::parse(&path, Some(&format!("limit=20&cursor={TASK}"))).unwrap();
    assert_eq!(
        target.path,
        format!("/v1/projects/{PROJECT}/tasks/{TASK}/handoffs?limit=20&cursor={TASK}")
    );
    assert_eq!(target.body_limit, DETAIL_BODY_LIMIT);
    for query in ["limit=21", "cursor=bad", "after=1", "limit=1&limit=1"] {
        assert!(ReadTarget::parse(&path, Some(query)).is_err());
    }
    assert!(ReadTarget::parse(&format!("{path}/content"), None).is_err());
}

#[test]
fn employee_runtime_metadata_has_no_arbitrary_subresource() {
    let path = format!("/api/projects/{PROJECT}/employees/{TASK}/runtime-metadata");
    assert_eq!(
        ReadTarget::parse(&path, None).unwrap().path,
        format!("/v1/projects/{PROJECT}/employees/{TASK}/runtime-metadata")
    );
    assert!(ReadTarget::parse(&path, Some("raw=1")).is_err());
    assert!(ReadTarget::parse(&format!("{path}/credential"), None).is_err());
}

#[test]
fn events_are_scoped_to_one_project_and_numeric_cursor() {
    let path = format!("/api/projects/{PROJECT}/events");
    let target = ReadTarget::parse(&path, Some("after=42")).unwrap();
    assert_eq!(
        target.path,
        format!("/v1/projects/{PROJECT}/events?after=42")
    );
    assert!(target.event_stream);
    for query in [
        "",
        "after=",
        "after=-1",
        "after=x",
        "after=1&after=2",
        "limit=2",
    ] {
        assert!(ReadTarget::parse(&path, Some(query)).is_err());
    }
    assert!(ReadTarget::parse("/api/projects/not-an-id/events", None).is_err());
}

#[test]
fn knowledge_and_memory_reads_have_closed_scope_and_query_shape() {
    let knowledge = format!("/api/projects/{PROJECT}/knowledge");
    let list = ReadTarget::parse(&knowledge, Some(&format!("after={TASK}&limit=5"))).unwrap();
    assert_eq!(
        list.path,
        format!("/v1/projects/{PROJECT}/knowledge?limit=5&after={TASK}")
    );
    let history = format!("{knowledge}/{TASK}/history");
    assert_eq!(
        ReadTarget::parse(&history, Some("after=2")).unwrap().path,
        format!("/v1/projects/{PROJECT}/knowledge/{TASK}/history?limit=20&after=2")
    );
    let search = format!("/api/projects/{PROJECT}/memory/search");
    assert_eq!(
        ReadTarget::parse(&search, Some("query=hello%20world&limit=3"))
            .unwrap()
            .path,
        format!("/v1/projects/{PROJECT}/memory/search?limit=3&query=hello%20world")
    );
    for query in [
        "query=",
        "query=x&query=y",
        "query=x&limit=0",
        "query=x&employee_id=bad",
    ] {
        assert!(ReadTarget::parse(&search, Some(query)).is_err());
    }
    for query in ["after=bad", "after=2&after=3", "cursor=x", "limit=101"] {
        assert!(ReadTarget::parse(&knowledge, Some(query)).is_err());
    }
    assert!(
        ReadTarget::parse(
            &format!("/api/projects/{PROJECT}/memory/status"),
            Some("limit=1")
        )
        .is_err()
    );
}

#[test]
fn resources_are_a_scoped_read_without_query_or_subpaths() {
    let path = format!("/api/projects/{PROJECT}/resources");
    assert_eq!(
        ReadTarget::parse(&path, None).unwrap().path,
        format!("/v1/projects/{PROJECT}/resources")
    );
    assert!(ReadTarget::parse(&path, Some("limit=1")).is_err());
    assert!(ReadTarget::parse(&format!("{path}/raw"), None).is_err());
}

#[test]
fn pipeline_catalog_and_task_property_schema_are_scoped_reads() {
    let catalog = format!("/api/projects/{PROJECT}/pipeline-catalog");
    assert_eq!(
        ReadTarget::parse(&catalog, Some(&format!("limit=5&cursor={TASK}")))
            .unwrap()
            .path,
        format!("/v1/projects/{PROJECT}/pipeline-catalog?limit=5&cursor={TASK}")
    );
    for query in [
        "cursor=bad",
        "limit=0",
        "limit=101",
        "after=x",
        "limit=1&limit=2",
    ] {
        assert!(ReadTarget::parse(&catalog, Some(query)).is_err());
    }
    let schema = format!("/api/projects/{PROJECT}/task-property-schema");
    assert_eq!(
        ReadTarget::parse(&schema, None).unwrap().path,
        format!("/v1/projects/{PROJECT}/task-property-schema")
    );
    assert!(ReadTarget::parse(&schema, Some("revision=1")).is_err());
}

#[test]
fn project_catalog_is_only_a_bounded_paginated_read() {
    let target = ReadTarget::parse("/api/projects", None).unwrap();
    assert_eq!(target.path, "/v1/projects?limit=20");
    assert_eq!(target.body_limit, CORE_BODY_LIMIT);
    assert!(target.cursor_conflict);
    let next = ReadTarget::parse("/api/projects", Some("limit=20&cursor=abc%26def")).unwrap();
    assert_eq!(next.path, "/v1/projects?limit=20&cursor=abc%26def");
    for query in [
        "limit=0",
        "limit=101",
        "cursor=",
        "actor=owner",
        "limit=2&limit=3",
    ] {
        assert!(ReadTarget::parse("/api/projects", Some(query)).is_err());
    }
    for path in ["/api/projects/", "/api/projects/all"] {
        assert!(ReadTarget::parse(path, None).is_err());
    }
}

#[test]
fn employee_roster_is_only_a_scoped_paginated_read() {
    let path = format!("/api/projects/{PROJECT}/employees");
    let target = ReadTarget::parse(&path, Some("limit=20&cursor=opaque%2Fid")).unwrap();
    assert_eq!(
        target.path,
        format!("/v1/projects/{PROJECT}/employees?limit=20&cursor=opaque%2Fid")
    );
    assert_eq!(target.body_limit, CORE_BODY_LIMIT);
    assert!(target.cursor_conflict);
    for query in [
        "",
        "limit=0",
        "cursor=",
        "limit=20&limit=1",
        "state=enabled",
    ] {
        assert!(matches!(
            ReadTarget::parse(&path, Some(query)),
            Err(ApiError::BadRequest)
        ));
    }
    for suffix in ["/", "/all", &format!("/{TASK}/config")] {
        assert!(matches!(
            ReadTarget::parse(&format!("{path}{suffix}"), None),
            Err(ApiError::NotFound)
        ));
    }
}

#[test]
fn employee_profile_and_run_history_are_closed_scoped_reads() {
    let profile = format!("/api/projects/{PROJECT}/employees/{TASK}");
    let target = ReadTarget::parse(&profile, None).unwrap();
    assert_eq!(
        target.path,
        format!("/v1/projects/{PROJECT}/employees/{TASK}")
    );
    assert_eq!(target.body_limit, DETAIL_BODY_LIMIT);
    assert!(!target.cursor_conflict);
    assert!(matches!(
        ReadTarget::parse(&profile, Some("limit=1")),
        Err(ApiError::NotFound)
    ));

    let history = format!("{profile}/runs");
    let target = ReadTarget::parse(&history, Some("limit=20&cursor=run%2Fid")).unwrap();
    assert_eq!(
        target.path,
        format!("/v1/projects/{PROJECT}/employees/{TASK}/runs?limit=20&cursor=run%2Fid")
    );
    assert_eq!(target.body_limit, CORE_BODY_LIMIT);
    assert!(target.cursor_conflict);
    for query in ["", "limit=0", "cursor=", "employee_id=other"] {
        assert!(matches!(
            ReadTarget::parse(&history, Some(query)),
            Err(ApiError::BadRequest)
        ));
    }
    for suffix in ["/", "/all", "/diagnostics"] {
        assert!(matches!(
            ReadTarget::parse(&format!("{history}{suffix}"), None),
            Err(ApiError::NotFound)
        ));
    }
}

#[test]
fn employee_operations_are_one_scoped_read_without_query() {
    let path = format!("/api/projects/{PROJECT}/employees/{TASK}/operations");
    let target = ReadTarget::parse(&path, None).unwrap();
    assert_eq!(
        target.path,
        format!("/v1/projects/{PROJECT}/employees/{TASK}/operations")
    );
    assert!(!target.event_stream);
    assert!(ReadTarget::parse(&path, Some("limit=1")).is_err());
    assert!(ReadTarget::parse(&format!("{path}/history"), None).is_err());
}

#[test]
fn system_job_status_and_employee_onboarding_are_exact_reads() {
    let jobs = format!("/api/projects/{PROJECT}/system-jobs");
    let target = ReadTarget::parse(&jobs, None).unwrap();
    assert_eq!(target.path, format!("/v1/projects/{PROJECT}/system-jobs"));
    assert_eq!(target.body_limit, DETAIL_BODY_LIMIT);
    let onboarding = format!("/api/projects/{PROJECT}/employees/{TASK}/onboarding");
    let target = ReadTarget::parse(&onboarding, None).unwrap();
    assert_eq!(
        target.path,
        format!("/v1/projects/{PROJECT}/employees/{TASK}/onboarding")
    );
    assert_eq!(target.body_limit, DETAIL_BODY_LIMIT);
    for path in [&jobs, &onboarding] {
        assert!(ReadTarget::parse(path, Some("limit=20")).is_err());
        assert!(ReadTarget::parse(&format!("{path}/extra"), None).is_err());
    }
    assert!(ReadTarget::parse("/api/projects/not-a-uuid/system-jobs", None).is_err());
    assert!(
        ReadTarget::parse(
            "/api/projects/not-a-uuid/employees/not-a-uuid/onboarding",
            None
        )
        .is_err()
    );
}

#[test]
fn system_job_attempts_and_hook_invocations_are_scoped_bounded_reads() {
    let attempts = format!("/api/projects/{PROJECT}/system-jobs/{TASK}/attempts");
    let target = ReadTarget::parse(&attempts, Some(&format!("limit=20&cursor={TASK}"))).unwrap();
    assert_eq!(
        target.path,
        format!("/v1/projects/{PROJECT}/system-jobs/{TASK}/attempts?limit=20&cursor={TASK}")
    );
    assert_eq!(target.body_limit, DETAIL_BODY_LIMIT);
    assert!(target.cursor_conflict);
    for query in ["limit=0", "limit=51", "cursor=not-a-uuid", "actor=owner"] {
        assert!(ReadTarget::parse(&attempts, Some(query)).is_err());
    }
    let hooks = format!("/api/projects/{PROJECT}/hook-invocations");
    let target = ReadTarget::parse(&hooks, Some(&format!("limit=20&after={TASK}"))).unwrap();
    assert_eq!(
        target.path,
        format!("/v1/projects/{PROJECT}/hook-invocations?limit=20&after={TASK}")
    );
    assert_eq!(target.body_limit, DETAIL_BODY_LIMIT);
    for query in ["limit=0", "limit=101", "after=not-a-uuid", "actor=owner"] {
        assert!(ReadTarget::parse(&hooks, Some(query)).is_err());
    }
    for path in [&attempts, &hooks] {
        assert!(ReadTarget::parse(&format!("{path}/extra"), None).is_err());
    }
}

#[test]
fn employee_inbox_reads_are_exact_scoped_and_bounded() {
    let threads = format!("/api/projects/{PROJECT}/employees/{TASK}/threads");
    let target = ReadTarget::parse(&threads, Some(&format!("limit=20&after={TASK}"))).unwrap();
    assert_eq!(
        target.path,
        format!("/v1/projects/{PROJECT}/employees/{TASK}/threads?limit=20&after={TASK}")
    );
    assert_eq!(target.body_limit, CORE_BODY_LIMIT);
    let messages = format!("/api/projects/{PROJECT}/threads/{TASK}/messages");
    let target = ReadTarget::parse(&messages, Some("limit=20&after=42")).unwrap();
    assert_eq!(
        target.path,
        format!("/v1/projects/{PROJECT}/threads/{TASK}/messages?limit=20&after=42")
    );
    assert_eq!(target.body_limit, DETAIL_BODY_LIMIT);
    for path in [&threads, &messages] {
        assert!(
            ReadTarget::parse(path, None)
                .unwrap()
                .path
                .ends_with("?limit=20")
        );
        for query in [
            "limit=0",
            "limit=21",
            "limit=20&limit=1",
            "cursor=x",
            "after=",
            "after=bad",
        ] {
            assert!(matches!(
                ReadTarget::parse(path, Some(query)),
                Err(ApiError::BadRequest)
            ));
        }
        assert!(ReadTarget::parse(&format!("{path}/extra"), None).is_err());
    }
    for query in [
        "after=-1",
        "after=9223372036854775808",
        "after=1.0",
        "after=+1",
    ] {
        assert!(matches!(
            ReadTarget::parse(&messages, Some(query)),
            Err(ApiError::BadRequest)
        ));
    }
    assert!(ReadTarget::parse("/api/projects/bad/employees/bad/threads", None).is_err());
    assert!(ReadTarget::parse("/api/projects/bad/threads/bad/messages", None).is_err());
}

#[test]
fn dependency_reads_are_direction_scoped_and_paginated() {
    for direction in ["blocked_by", "blocks"] {
        let path = format!("/api/projects/{PROJECT}/tasks/{TASK}/dependencies/{direction}");
        let target = ReadTarget::parse(&path, None).expect("allowed");
        assert_eq!(
            target.path,
            format!("/v1/projects/{PROJECT}/tasks/{TASK}/dependencies/{direction}?limit=20")
        );
        assert!(target.cursor_conflict);
        assert_eq!(target.body_limit, CORE_BODY_LIMIT);
        for query in [
            "limit=0",
            "limit=101",
            "limit=1&limit=2",
            "cursor=",
            "after=x",
            "cursor=x&cursor=y",
        ] {
            assert!(ReadTarget::parse(&path, Some(query)).is_err());
        }
    }
    for path in [
        format!("/api/projects/{PROJECT}/tasks/{TASK}/dependencies"),
        format!("/api/projects/{PROJECT}/tasks/{TASK}/dependencies/unknown"),
        format!("/api/projects/{PROJECT}/tasks/not-a-task/dependencies/blocks"),
    ] {
        assert!(ReadTarget::parse(&path, None).is_err());
    }
}

#[test]
fn cancellation_catalog_rejects_queries_subpaths_and_invalid_scope() {
    let path = format!("/api/projects/{PROJECT}/cancellation-reasons");
    for query in ["", "limit=1", "actor=human", "retired=false"] {
        assert!(ReadTarget::parse(&path, Some(query)).is_err());
    }
    for invalid in [
        format!("{path}/"),
        format!("{path}/unspecified"),
        "/api/projects/not-a-uuid/cancellation-reasons".into(),
        "/api/projects/01988000-0000-4000-8000-000000000001/cancellation-reasons".into(),
    ] {
        assert!(ReadTarget::parse(&invalid, None).is_err());
    }
}

#[test]
fn scoped_reads_have_fixed_paths_limits_and_cursor_policy() {
    for (path, expected, limit, cursor_conflict) in [
        (
            format!("/api/projects/{PROJECT}/cancellation-reasons"),
            format!("/v1/projects/{PROJECT}/cancellation-reasons"),
            CORE_BODY_LIMIT,
            false,
        ),
        (
            "/api/health".into(),
            "/v1/health".into(),
            CORE_BODY_LIMIT,
            false,
        ),
        (
            format!("/api/projects/{PROJECT}"),
            format!("/v1/projects/{PROJECT}"),
            CORE_BODY_LIMIT,
            false,
        ),
        (
            format!("/api/projects/{PROJECT}/priority-scheme"),
            format!("/v1/projects/{PROJECT}/priority-scheme"),
            CORE_BODY_LIMIT,
            false,
        ),
        (
            format!("/api/projects/{PROJECT}/employees"),
            format!("/v1/projects/{PROJECT}/employees?limit=20"),
            CORE_BODY_LIMIT,
            true,
        ),
        (
            format!("/api/projects/{PROJECT}/tasks"),
            format!("/v1/projects/{PROJECT}/tasks?limit=20"),
            CORE_BODY_LIMIT,
            true,
        ),
        (
            format!("/api/projects/{PROJECT}/tasks/{TASK}"),
            format!("/v1/projects/{PROJECT}/tasks/{TASK}"),
            DETAIL_BODY_LIMIT,
            false,
        ),
        (
            format!("/api/projects/{PROJECT}/pipelines"),
            format!("/v1/projects/{PROJECT}/pipelines?limit=20"),
            DETAIL_BODY_LIMIT,
            true,
        ),
        (
            format!("/api/projects/{PROJECT}/pipelines/{TASK}"),
            format!("/v1/projects/{PROJECT}/pipelines/{TASK}"),
            DETAIL_BODY_LIMIT,
            false,
        ),
        (
            format!("/api/projects/{PROJECT}/runs"),
            format!("/v1/projects/{PROJECT}/runs?limit=20"),
            CORE_BODY_LIMIT,
            true,
        ),
        (
            format!("/api/projects/{PROJECT}/runs/{TASK}"),
            format!("/v1/projects/{PROJECT}/runs/{TASK}"),
            DETAIL_BODY_LIMIT,
            false,
        ),
    ] {
        let target = ReadTarget::parse(&path, None).unwrap();
        assert_eq!(target.path, expected);
        assert_eq!(target.body_limit, limit);
        assert_eq!(target.cursor_conflict, cursor_conflict);
    }
}

#[test]
fn pagination_decodes_then_reencodes_opaque_cursor_without_query_injection() {
    let target = ReadTarget::parse(
        &format!("/api/projects/{PROJECT}/tasks"),
        Some("cursor=%D1%82%D0%B5%D1%81%D1%82%2F%3F%26limit%3D100%2B+%25&limit=001"),
    )
    .unwrap();
    assert_eq!(
        target.path,
        format!(
            "/v1/projects/{PROJECT}/tasks?limit=1&cursor=%D1%82%D0%B5%D1%81%D1%82%2F%3F%26limit%3D100%2B%20%25"
        )
    );
}

#[test]
fn pagination_rejects_unknown_duplicate_empty_invalid_and_malformed_values() {
    for query in [
        "",
        "cursor",
        "limit=0",
        "limit=101",
        "limit=99999999999999999999999999999999999999",
        "limit=-1",
        "limit=+1",
        "limit=1.0",
        "limit=",
        "limit=1&limit=2",
        "limit=1&%6cimit=2",
        "cursor=a&cursor=b",
        "cursor=",
        "cursor=%",
        "cursor=%0",
        "cursor=%GG",
        "cursor=%FF",
        "cursor=%C0%80",
        "cur%FFsor=a",
        "limit=20&",
        "&limit=20",
        "token=x",
        "cursor=a&actor=owner",
        "cursor=a;limit=20&other=x",
    ] {
        assert!(
            matches!(
                ReadTarget::parse(&format!("/api/projects/{PROJECT}/tasks"), Some(query)),
                Err(ApiError::BadRequest)
            ),
            "unexpected acceptance: {query}"
        );
    }
}

#[test]
fn cursor_limit_counts_decoded_utf8_bytes_not_characters_or_encoded_length() {
    let path = format!("/api/projects/{PROJECT}/tasks");
    assert!(ReadTarget::parse(&path, Some(&format!("cursor={}", "%D1%8F".repeat(512)))).is_ok());
    assert!(matches!(
        ReadTarget::parse(&path, Some(&format!("cursor={}", "%D1%8F".repeat(513)))),
        Err(ApiError::BadRequest)
    ));
    assert!(ReadTarget::parse(&path, Some(&format!("cursor={}", "x".repeat(1024)))).is_ok());
    assert!(matches!(
        ReadTarget::parse(&path, Some(&format!("cursor={}", "x".repeat(1025)))),
        Err(ApiError::BadRequest)
    ));
}

#[test]
fn only_allowlisted_lists_accept_queries_and_other_resources_stay_closed() {
    for path in [
        "/api/health".into(),
        format!("/api/projects/{PROJECT}"),
        format!("/api/projects/{PROJECT}/priority-scheme"),
        format!("/api/projects/{PROJECT}/tasks/{TASK}"),
        format!("/api/projects/{PROJECT}/pipelines/{TASK}"),
        format!("/api/projects/{PROJECT}/pipelines/"),
        format!("/api/projects/{PROJECT}/pipelines/{TASK}/versions"),
        format!("/api/projects/{PROJECT}/tasks/"),
        format!("/api/projects/{PROJECT}/tasks/{TASK}/artifacts"),
        format!("/api/projects/{PROJECT}/employees/{TASK}"),
        format!("/api/projects/{PROJECT}/runs/{TASK}"),
        format!("/api/projects/{PROJECT}/runs/"),
        format!("/api/projects/{PROJECT}/tasks/{TASK}/runs"),
    ] {
        assert!(matches!(
            ReadTarget::parse(&path, Some("limit=20")),
            Err(ApiError::NotFound)
        ));
    }
}

#[test]
fn new_routes_require_both_identifiers_to_be_uuidv7() {
    for invalid in [
        "bad",
        "00000000-0000-0000-0000-000000000000",
        "01988000-0000-4000-8000-000000000001",
        "01988000-0000-7000-0000-000000000001",
        "01988000000070008000000000000001",
        "{01988000-0000-7000-8000-000000000001}",
        "%2e%2e",
    ] {
        for path in [
            format!("/api/projects/{invalid}/tasks"),
            format!("/api/projects/{invalid}/employees"),
            format!("/api/projects/{invalid}/employees/{TASK}"),
            format!("/api/projects/{PROJECT}/employees/{invalid}"),
            format!("/api/projects/{PROJECT}/employees/{invalid}/runs"),
            format!("/api/projects/{invalid}/priority-scheme"),
            format!("/api/projects/{invalid}/tasks/{TASK}"),
            format!("/api/projects/{PROJECT}/tasks/{invalid}"),
            format!("/api/projects/{PROJECT}/pipelines/{invalid}"),
            format!("/api/projects/{invalid}/pipelines"),
            format!("/api/projects/{invalid}/pipelines/{TASK}"),
            format!("/api/projects/{invalid}/runs"),
            format!("/api/projects/{invalid}/runs/{TASK}"),
            format!("/api/projects/{PROJECT}/runs/{invalid}"),
        ] {
            assert!(
                matches!(ReadTarget::parse(&path, None), Err(ApiError::NotFound)),
                "{path}"
            );
        }
    }
}

#[test]
fn priority_scheme_has_no_queries_subresources_or_trailing_slash() {
    let path = format!("/api/projects/{PROJECT}/priority-scheme");
    for query in ["", "limit=1", "cursor=a", "actor=owner", "retired=false"] {
        assert!(matches!(
            ReadTarget::parse(&path, Some(query)),
            Err(ApiError::NotFound)
        ));
    }
    for suffix in ["/", "/normal", "/levels", "/settings"] {
        assert!(matches!(
            ReadTarget::parse(&format!("{path}{suffix}"), None),
            Err(ApiError::NotFound)
        ));
    }
}

#[test]
fn pipeline_list_uses_full_definition_bound_and_strict_pagination() {
    let path = format!("/api/projects/{PROJECT}/pipelines");
    let target = ReadTarget::parse(&path, Some("limit=100&cursor=a%26limit%3D1%2B%25")).unwrap();
    assert_eq!(target.body_limit, DETAIL_BODY_LIMIT);
    assert!(target.cursor_conflict);
    assert_eq!(
        target.path,
        format!("/v1/projects/{PROJECT}/pipelines?limit=100&cursor=a%26limit%3D1%2B%25")
    );
    for query in [
        "",
        "limit=0",
        "limit=101",
        "limit=+1",
        "limit=1.0",
        "limit=1&%6cimit=2",
        "cursor=",
        "cursor=a&cursor=b",
        "cursor=%",
        "cursor=%GG",
        "cursor=%FF",
        "token=x",
        "pipeline_id=x",
        "deleted=false",
        "limit=20&",
        "cursor=a&actor=owner",
    ] {
        assert!(
            matches!(
                ReadTarget::parse(&path, Some(query)),
                Err(ApiError::BadRequest)
            ),
            "{query}"
        );
    }
    assert!(ReadTarget::parse(&path, Some(&format!("cursor={}", "%D1%8F".repeat(512)))).is_ok());
    assert!(matches!(
        ReadTarget::parse(&path, Some(&format!("cursor={}", "%D1%8F".repeat(513)))),
        Err(ApiError::BadRequest)
    ));
}

#[test]
fn run_pagination_uses_the_same_strict_bounded_parser_as_task_pagination() {
    let path = format!("/api/projects/{PROJECT}/runs");
    let target = ReadTarget::parse(
        &path,
        Some("cursor=%D1%8F%2F%3F%26limit%3D100%2B+%25&limit=001"),
    )
    .unwrap();
    assert_eq!(
        target.path,
        format!("/v1/projects/{PROJECT}/runs?limit=1&cursor=%D1%8F%2F%3F%26limit%3D100%2B%20%25")
    );
    for query in [
        "",
        "cursor",
        "limit=0",
        "limit=101",
        "limit=-1",
        "limit=1.0",
        "limit=1&%6cimit=2",
        "cursor=a&cursor=b",
        "cursor=",
        "cursor=%",
        "cursor=%GG",
        "cursor=%FF",
        "task_id=x",
        "purpose=hook",
        "token=x",
        "limit=20&",
        "cursor=a&actor=owner",
    ] {
        assert!(
            matches!(
                ReadTarget::parse(&path, Some(query)),
                Err(ApiError::BadRequest)
            ),
            "{query}"
        );
    }
    assert!(ReadTarget::parse(&path, Some(&format!("cursor={}", "%D1%8F".repeat(512)))).is_ok());
    assert!(matches!(
        ReadTarget::parse(&path, Some(&format!("cursor={}", "%D1%8F".repeat(513)))),
        Err(ApiError::BadRequest)
    ));
}
