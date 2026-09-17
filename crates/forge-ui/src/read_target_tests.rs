use crate::{
    http::ApiError,
    read_target::{CORE_BODY_LIMIT, DETAIL_BODY_LIMIT, ReadTarget},
};

const PROJECT: &str = "01988000-0000-7000-8000-000000000001";
const TASK: &str = "01988000-0000-7000-8000-000000000002";

#[test]
fn scoped_reads_have_fixed_paths_limits_and_cursor_policy() {
    for (path, expected, limit, cursor_conflict) in [
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
        format!("/api/projects/{PROJECT}/tasks/{TASK}"),
        format!("/api/projects/{PROJECT}/pipelines/{TASK}"),
        format!("/api/projects/{PROJECT}/pipelines/"),
        format!("/api/projects/{PROJECT}/pipelines/{TASK}/versions"),
        format!("/api/projects/{PROJECT}/tasks/"),
        format!("/api/projects/{PROJECT}/tasks/{TASK}/artifacts"),
        format!("/api/projects/{PROJECT}/employees"),
        format!("/api/projects/{PROJECT}/runs/{TASK}"),
        format!("/api/projects/{PROJECT}/runs/"),
        format!("/api/projects/{PROJECT}/runs/{TASK}/evidence"),
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
