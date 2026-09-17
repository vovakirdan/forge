//! Closed browser read allowlist and bounded pagination, never an arbitrary proxy URL.

use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

use crate::http::ApiError;

pub(crate) const CORE_BODY_LIMIT: usize = 64 * 1024;
pub(crate) const DETAIL_BODY_LIMIT: usize = 1024 * 1024;

#[derive(Debug)]
pub(crate) struct ReadTarget {
    pub path: String,
    pub body_limit: usize,
    pub cursor_conflict: bool,
}

impl ReadTarget {
    pub fn parse(path: &str, query: Option<&str>) -> Result<Self, ApiError> {
        let mut target = Self {
            path: String::new(),
            body_limit: CORE_BODY_LIMIT,
            cursor_conflict: false,
        };
        if path == "/api/health" && query.is_none() {
            target.path = "/v1/health".into();
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
            [project, resource @ ("tasks" | "runs")] => {
                let project = uuid_v7(project)?;
                target.path = format!("/v1/projects/{project}/{resource}?{}", pagination(query)?);
                target.cursor_conflict = true;
            }
            [project, resource @ ("tasks" | "pipelines" | "runs"), id] if query.is_none() => {
                let project = uuid_v7(project)?;
                let id = uuid_v7(id)?;
                target.path = format!("/v1/projects/{project}/{resource}/{id}");
                target.body_limit = DETAIL_BODY_LIMIT;
            }
            _ => return Err(ApiError::NotFound),
        }
        Ok(target)
    }
}

fn uuid_v7(value: &str) -> Result<uuid::Uuid, ApiError> {
    let id = uuid::Uuid::parse_str(value).map_err(|_| ApiError::NotFound)?;
    if value.len() != 36 || id.get_version_num() != 7 || id.get_variant() != uuid::Variant::RFC4122
    {
        return Err(ApiError::NotFound);
    }
    Ok(id)
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
