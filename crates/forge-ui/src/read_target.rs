//! Closed browser read allowlist and bounded pagination, never an arbitrary proxy URL.

use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

use crate::http::ApiError;

pub(crate) const CORE_BODY_LIMIT: usize = 64 * 1024;
pub(crate) const DETAIL_BODY_LIMIT: usize = 1024 * 1024;
pub(crate) const EVENT_BODY_LIMIT: usize = 8 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct ReadTarget {
    pub path: String,
    pub body_limit: usize,
    pub cursor_conflict: bool,
    pub event_stream: bool,
}

impl ReadTarget {
    pub fn parse(path: &str, query: Option<&str>) -> Result<Self, ApiError> {
        let mut target = Self {
            path: String::new(),
            body_limit: CORE_BODY_LIMIT,
            cursor_conflict: false,
            event_stream: false,
        };
        if path == "/api/health" && query.is_none() {
            target.path = "/v1/health".into();
            return Ok(target);
        }
        if path == "/api/projects" {
            target.path = format!("/v1/projects?{}", pagination(query)?);
            target.cursor_conflict = true;
            return Ok(target);
        }
        let tail = path
            .strip_prefix("/api/projects/")
            .ok_or(ApiError::NotFound)?;
        let parts: Vec<_> = tail.split('/').collect();
        match parts.as_slice() {
            [project] if query.is_none() => {
                // Keep the existing Project route contract; new routes require UUIDv7.
                let project = uuid::Uuid::parse_str(project).map_err(|_| ApiError::NotFound)?;
                target.path = format!("/v1/projects/{project}");
            }
            [project, "priority-scheme"] if query.is_none() => {
                let project = uuid_v7(project)?;
                target.path = format!("/v1/projects/{project}/priority-scheme");
            }
            [project, "cancellation-reasons"] if query.is_none() => {
                let project = uuid_v7(project)?;
                target.path = format!("/v1/projects/{project}/cancellation-reasons");
            }
            [project, "events"] => {
                let project = uuid_v7(project)?;
                let after = event_after(query)?;
                target.path = match after {
                    Some(after) => format!("/v1/projects/{project}/events?after={after}"),
                    None => format!("/v1/projects/{project}/events"),
                };
                target.body_limit = EVENT_BODY_LIMIT;
                target.event_stream = true;
            }
            [project, "knowledge"] => {
                let project = uuid_v7(project)?;
                target.path = format!(
                    "/v1/projects/{project}/knowledge?{}",
                    scoped_page(query, false, false)?
                );
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            [project, "knowledge", page] if query.is_none() => {
                let project = uuid_v7(project)?;
                let page = uuid_v7(page)?;
                target.path = format!("/v1/projects/{project}/knowledge/{page}");
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            [project, "knowledge", page, "history"] => {
                let project = uuid_v7(project)?;
                let page = uuid_v7(page)?;
                target.path = format!(
                    "/v1/projects/{project}/knowledge/{page}/history?{}",
                    scoped_page(query, true, false)?
                );
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            [project, "memory", "status"] if query.is_none() => {
                let project = uuid_v7(project)?;
                target.path = format!("/v1/projects/{project}/memory/status");
            }
            [project, "system-jobs"] if query.is_none() => {
                let project = uuid_v7(project)?;
                target.path = format!("/v1/projects/{project}/system-jobs");
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            [project, "system-jobs", job, "attempts"] => {
                let project = uuid_v7(project)?;
                let job = uuid_v7(job)?;
                target.path = format!(
                    "/v1/projects/{project}/system-jobs/{job}/attempts?{}",
                    run_evidence_page(query)?
                );
                target.body_limit = DETAIL_BODY_LIMIT;
                target.cursor_conflict = true;
            }
            [project, "resources"] if query.is_none() => {
                let project = uuid_v7(project)?;
                target.path = format!("/v1/projects/{project}/resources");
            }
            [project, "pipeline-catalog"] => {
                let project = uuid_v7(project)?;
                target.path = format!(
                    "/v1/projects/{project}/pipeline-catalog?{}",
                    catalog_page(query)?
                );
            }
            [project, "hook-versions"] => {
                let project = uuid_v7(project)?;
                target.path = format!(
                    "/v1/projects/{project}/hook-versions?{}",
                    scoped_page(query, false, false)?
                );
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            [project, "hook-invocations"] => {
                let project = uuid_v7(project)?;
                target.path = format!(
                    "/v1/projects/{project}/hook-invocations?{}",
                    scoped_page(query, false, false)?
                );
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            [project, "task-property-schema"] if query.is_none() => {
                let project = uuid_v7(project)?;
                target.path = format!("/v1/projects/{project}/task-property-schema");
            }
            [
                project,
                resource @ ("resume-schedules"
                | "escalations"
                | "next-run-constraints"
                | "recovery-runs"),
            ] => {
                let project = uuid_v7(project)?;
                target.path = format!(
                    "/v1/projects/{project}/{resource}?{}",
                    management_page(query)?
                );
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            [project, "escalations", escalation] if query.is_none() => {
                let project = uuid_v7(project)?;
                let escalation = uuid_v7(escalation)?;
                target.path = format!("/v1/projects/{project}/escalations/{escalation}");
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            [project, "resolver-routes"] => {
                let project = uuid_v7(project)?;
                target.path = format!(
                    "/v1/projects/{project}/resolver-routes?{}",
                    resolver_route_page(query)?
                );
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            [project, "recovery"] if query.is_none() => {
                let project = uuid_v7(project)?;
                target.path = format!("/v1/projects/{project}/recovery");
            }
            [project, "recovery-runs", run, "assessment-readiness"] if query.is_none() => {
                let project = uuid_v7(project)?;
                let run = uuid_v7(run)?;
                target.path =
                    format!("/v1/projects/{project}/recovery-runs/{run}/assessment-readiness");
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            [project, "memory", "search"] => {
                let project = uuid_v7(project)?;
                target.path = format!(
                    "/v1/projects/{project}/memory/search?{}",
                    memory_search(query)?
                );
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            [project, "memory"] => {
                let project = uuid_v7(project)?;
                target.path = format!(
                    "/v1/projects/{project}/memory?{}",
                    scoped_page(query, false, true)?
                );
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            [project, "memory", entry] if query.is_none() => {
                let project = uuid_v7(project)?;
                let entry = uuid_v7(entry)?;
                target.path = format!("/v1/projects/{project}/memory/{entry}");
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            [project, "memory", entry, "history"] => {
                let project = uuid_v7(project)?;
                let entry = uuid_v7(entry)?;
                target.path = format!(
                    "/v1/projects/{project}/memory/{entry}/history?{}",
                    scoped_page(query, true, false)?
                );
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            [project, "employees", employee, "onboarding"] if query.is_none() => {
                let project = uuid_v7(project)?;
                let employee = uuid_v7(employee)?;
                target.path = format!("/v1/projects/{project}/employees/{employee}/onboarding");
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            [project, "employees", employee, "runtime-metadata"] if query.is_none() => {
                let project = uuid_v7(project)?;
                let employee = uuid_v7(employee)?;
                target.path =
                    format!("/v1/projects/{project}/employees/{employee}/runtime-metadata");
            }
            [project, "tasks", task, "handoffs"] => {
                let project = uuid_v7(project)?;
                let task = uuid_v7(task)?;
                target.path = format!(
                    "/v1/projects/{project}/tasks/{task}/handoffs?{}",
                    management_page(query)?
                );
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            [
                project,
                "tasks",
                task,
                resource @ ("git-source-policy" | "file-inputs"),
            ] if query.is_none() => {
                let project = uuid_v7(project)?;
                let task = uuid_v7(task)?;
                target.path = format!("/v1/projects/{project}/tasks/{task}/{resource}");
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            [
                project,
                "tasks",
                task,
                resource @ ("file-snapshots" | "reviews" | "integrations"),
            ] => {
                let project = uuid_v7(project)?;
                let task = uuid_v7(task)?;
                target.path = format!(
                    "/v1/projects/{project}/tasks/{task}/{resource}?{}",
                    scoped_page(query, false, false)?
                );
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            [
                project,
                "tasks",
                task,
                "dependencies",
                direction @ ("blocked_by" | "blocks"),
            ] => {
                let project = uuid_v7(project)?;
                let task = uuid_v7(task)?;
                target.path = format!(
                    "/v1/projects/{project}/tasks/{task}/dependencies/{direction}?{}",
                    pagination(query)?
                );
                target.cursor_conflict = true;
            }
            [project, "employees", employee, "runs"] => {
                let project = uuid_v7(project)?;
                let employee = uuid_v7(employee)?;
                target.path = format!(
                    "/v1/projects/{project}/employees/{employee}/runs?{}",
                    pagination(query)?
                );
                target.cursor_conflict = true;
            }
            [project, "runs", run, "context"] if query.is_none() => {
                let project = uuid_v7(project)?;
                let run = uuid_v7(run)?;
                target.path = format!("/v1/projects/{project}/runs/{run}/context");
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            [project, "runs", run, "evidence"] => {
                let project = uuid_v7(project)?;
                let run = uuid_v7(run)?;
                target.path = format!(
                    "/v1/projects/{project}/runs/{run}/evidence?{}",
                    run_evidence_page(query)?
                );
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            [project, "employees", employee, "threads"] => {
                let project = uuid_v7(project)?;
                let employee = uuid_v7(employee)?;
                target.path = format!(
                    "/v1/projects/{project}/employees/{employee}/threads?{}",
                    inbox_page(query, false)?
                );
            }
            [
                project,
                "threads",
                thread,
                resource @ ("messages" | "delivery"),
            ] => {
                let project = uuid_v7(project)?;
                let thread = uuid_v7(thread)?;
                target.path = format!(
                    "/v1/projects/{project}/threads/{thread}/{resource}?{}",
                    inbox_page(query, true)?
                );
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            [
                project,
                resource @ ("tasks" | "pipelines" | "runs" | "employees"),
            ] => {
                let project = uuid_v7(project)?;
                target.path = format!("/v1/projects/{project}/{resource}?{}", pagination(query)?);
                target.cursor_conflict = true;
                // Pipeline pages contain full definitions, not compact summaries.
                if *resource == "pipelines" {
                    target.body_limit = DETAIL_BODY_LIMIT;
                }
            }
            [project, resource @ ("tasks" | "pipelines" | "runs"), id] if query.is_none() => {
                let project = uuid_v7(project)?;
                let id = uuid_v7(id)?;
                target.path = format!("/v1/projects/{project}/{resource}/{id}");
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            [project, "employees", employee] if query.is_none() => {
                let project = uuid_v7(project)?;
                let employee = uuid_v7(employee)?;
                target.path = format!("/v1/projects/{project}/employees/{employee}");
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            [project, "employees", employee, "operations"] if query.is_none() => {
                let project = uuid_v7(project)?;
                let employee = uuid_v7(employee)?;
                target.path = format!("/v1/projects/{project}/employees/{employee}/operations");
            }
            _ => return Err(ApiError::NotFound),
        }
        Ok(target)
    }
}

fn event_after(query: Option<&str>) -> Result<Option<u64>, ApiError> {
    let Some(query) = query else { return Ok(None) };
    let Some(value) = query.strip_prefix("after=") else {
        return Err(ApiError::BadRequest);
    };
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ApiError::BadRequest);
    }
    Ok(Some(value.parse().map_err(|_| ApiError::BadRequest)?))
}

fn fields(query: Option<&str>) -> Result<Vec<(String, String)>, ApiError> {
    let mut result = Vec::new();
    if let Some(query) = query {
        if query.len() > 32 * 1024 {
            return Err(ApiError::BadRequest);
        }
        for field in query.split('&') {
            let (key, value) = field.split_once('=').ok_or(ApiError::BadRequest)?;
            let key = decode_component(key)?;
            if result.iter().any(|(existing, _)| existing == &key) {
                return Err(ApiError::BadRequest);
            }
            result.push((key, decode_component(value)?));
        }
    }
    Ok(result)
}

fn scoped_page(query: Option<&str>, revision: bool, personal: bool) -> Result<String, ApiError> {
    let mut after = None;
    let mut limit = 20;
    let mut employee = None;
    for (key, value) in fields(query)? {
        match key.as_str() {
            "after" => {
                after = Some(if revision {
                    value
                        .parse::<i64>()
                        .ok()
                        .filter(|value| *value >= 0)
                        .ok_or(ApiError::BadRequest)?
                        .to_string()
                } else {
                    uuid_v7(&value)?.to_string()
                });
            }
            "limit" => {
                limit = value
                    .parse::<u32>()
                    .ok()
                    .filter(|value| (1..=100).contains(value))
                    .ok_or(ApiError::BadRequest)?;
            }
            "employee_id" if personal => {
                employee = Some(uuid_v7(&value)?.to_string());
            }
            _ => return Err(ApiError::BadRequest),
        }
    }
    let mut result = format!("limit={limit}");
    if let Some(after) = after {
        result.push_str("&after=");
        result.push_str(&after);
    }
    if let Some(employee) = employee {
        result.push_str("&employee_id=");
        result.push_str(&employee);
    }
    Ok(result)
}

fn memory_search(query: Option<&str>) -> Result<String, ApiError> {
    let mut term = None;
    let mut limit = 20;
    let mut employee = None;
    for (key, value) in fields(query)? {
        match key.as_str() {
            "query" if !value.is_empty() && value.len() <= 8192 => term = Some(value),
            "limit" => {
                limit = value
                    .parse::<u32>()
                    .ok()
                    .filter(|value| (1..=100).contains(value))
                    .ok_or(ApiError::BadRequest)?;
            }
            "employee_id" => employee = Some(uuid_v7(&value)?.to_string()),
            _ => return Err(ApiError::BadRequest),
        }
    }
    let term = term.ok_or(ApiError::BadRequest)?;
    let mut result = format!(
        "limit={limit}&query={}",
        utf8_percent_encode(&term, NON_ALPHANUMERIC)
    );
    if let Some(employee) = employee {
        result.push_str("&employee_id=");
        result.push_str(&employee);
    }
    Ok(result)
}

fn catalog_page(query: Option<&str>) -> Result<String, ApiError> {
    let mut cursor = None;
    let mut limit = 20;
    for (key, value) in fields(query)? {
        match key.as_str() {
            "cursor" => cursor = Some(uuid_v7(&value)?.to_string()),
            "limit" => {
                limit = value
                    .parse::<u32>()
                    .ok()
                    .filter(|value| (1..=100).contains(value))
                    .ok_or(ApiError::BadRequest)?;
            }
            _ => return Err(ApiError::BadRequest),
        }
    }
    let mut result = format!("limit={limit}");
    if let Some(cursor) = cursor {
        result.push_str("&cursor=");
        result.push_str(&cursor);
    }
    Ok(result)
}

fn management_page(query: Option<&str>) -> Result<String, ApiError> {
    let mut cursor = None;
    let mut limit = 20;
    for (key, value) in fields(query)? {
        match key.as_str() {
            "cursor" => cursor = Some(uuid_v7(&value).map_err(|_| ApiError::BadRequest)?),
            "limit" => {
                limit = value
                    .parse::<u32>()
                    .ok()
                    .filter(|value| (1..=20).contains(value))
                    .ok_or(ApiError::BadRequest)?;
            }
            _ => return Err(ApiError::BadRequest),
        }
    }
    Ok(match cursor {
        Some(cursor) => format!("limit={limit}&cursor={cursor}"),
        None => format!("limit={limit}"),
    })
}

fn resolver_route_page(query: Option<&str>) -> Result<String, ApiError> {
    let mut cursor = None;
    let mut limit = 20;
    for (key, value) in fields(query)? {
        match key.as_str() {
            "cursor"
                if (1..=64).contains(&value.len())
                    && value.as_bytes()[0].is_ascii_lowercase()
                    && value.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                    }) =>
            {
                cursor = Some(value)
            }
            "limit" => {
                limit = value
                    .parse::<u32>()
                    .ok()
                    .filter(|value| (1..=20).contains(value))
                    .ok_or(ApiError::BadRequest)?
            }
            _ => return Err(ApiError::BadRequest),
        }
    }
    Ok(match cursor {
        Some(cursor) => format!("limit={limit}&cursor={cursor}"),
        None => format!("limit={limit}"),
    })
}

fn run_evidence_page(query: Option<&str>) -> Result<String, ApiError> {
    let mut cursor = None;
    let mut limit = 20;
    for (key, value) in fields(query)? {
        match key.as_str() {
            "cursor" => cursor = Some(uuid_v7(&value).map_err(|_| ApiError::BadRequest)?),
            "limit" => {
                limit = value
                    .parse::<u32>()
                    .ok()
                    .filter(|value| (1..=50).contains(value))
                    .ok_or(ApiError::BadRequest)?
            }
            _ => return Err(ApiError::BadRequest),
        }
    }
    Ok(match cursor {
        Some(cursor) => format!("limit={limit}&cursor={cursor}"),
        None => format!("limit={limit}"),
    })
}

fn uuid_v7(value: &str) -> Result<uuid::Uuid, ApiError> {
    let id = uuid::Uuid::parse_str(value).map_err(|_| ApiError::NotFound)?;
    if value.len() != 36 || id.get_version_num() != 7 || id.get_variant() != uuid::Variant::RFC4122
    {
        return Err(ApiError::NotFound);
    }
    Ok(id)
}

fn inbox_page(query: Option<&str>, sequence_cursor: bool) -> Result<String, ApiError> {
    let mut limit = 20;
    let mut after = None;
    for (key, value) in fields(query)? {
        match key.as_str() {
            "limit" => {
                if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(ApiError::BadRequest);
                }
                limit = value
                    .parse::<u32>()
                    .ok()
                    .filter(|value| (1..=20).contains(value))
                    .ok_or(ApiError::BadRequest)?;
            }
            "after" => {
                after = Some(if sequence_cursor {
                    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                        return Err(ApiError::BadRequest);
                    }
                    value
                        .parse::<u64>()
                        .ok()
                        .filter(|value| *value <= i64::MAX as u64)
                        .ok_or(ApiError::BadRequest)?
                        .to_string()
                } else {
                    uuid_v7(&value)
                        .map_err(|_| ApiError::BadRequest)?
                        .to_string()
                });
            }
            _ => return Err(ApiError::BadRequest),
        }
    }
    let mut result = format!("limit={limit}");
    if let Some(after) = after {
        result.push_str("&after=");
        result.push_str(&after);
    }
    Ok(result)
}

fn pagination(query: Option<&str>) -> Result<String, ApiError> {
    let mut limit = None;
    let mut cursor = None;
    if let Some(query) = query {
        for field in query.split('&') {
            let (key, value) = field.split_once('=').ok_or(ApiError::BadRequest)?;
            let key = decode_component(key)?;
            let value = decode_component(value)?;
            match key.as_str() {
                "limit" if limit.is_none() => {
                    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                        return Err(ApiError::BadRequest);
                    }
                    let parsed = value.parse::<usize>().map_err(|_| ApiError::BadRequest)?;
                    if !(1..=100).contains(&parsed) {
                        return Err(ApiError::BadRequest);
                    }
                    limit = Some(parsed);
                }
                "cursor" if cursor.is_none() => {
                    if value.is_empty() || value.len() > 1024 {
                        return Err(ApiError::BadRequest);
                    }
                    cursor = Some(value);
                }
                _ => return Err(ApiError::BadRequest),
            }
        }
    }
    let mut encoded = format!("limit={}", limit.unwrap_or(20));
    if let Some(cursor) = cursor {
        encoded.push_str("&cursor=");
        encoded.push_str(&utf8_percent_encode(&cursor, NON_ALPHANUMERIC).to_string());
    }
    Ok(encoded)
}

fn decode_component(value: &str) -> Result<String, ApiError> {
    // The library preserves malformed escapes and replaces invalid UTF-8 in its lossy API;
    // neither is allowed at this boundary. Decode strictly before any route is forwarded.
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%'
            && (!bytes.next().is_some_and(|byte| byte.is_ascii_hexdigit())
                || !bytes.next().is_some_and(|byte| byte.is_ascii_hexdigit()))
        {
            return Err(ApiError::BadRequest);
        }
    }
    let value = value.replace('+', " ");
    percent_decode_str(&value)
        .decode_utf8()
        .map(|value| value.into_owned())
        .map_err(|_| ApiError::BadRequest)
}
