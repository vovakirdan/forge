//! Minimal parser for the bounded `forge.event` SSE replay format.

use forge_protocol::wire::EventEnvelope;

use crate::client::ClientError;

pub(crate) fn parse_events(body: &[u8]) -> Result<Vec<EventEnvelope>, ClientError> {
    let text = std::str::from_utf8(body).map_err(|_| ClientError::Sse("body is not UTF-8"))?;
    text.split("\n\n")
        .filter(|record| !record.trim().is_empty())
        .filter_map(event_data)
        .map(|data| serde_json::from_str(data).map_err(ClientError::from))
        .collect()
}

fn event_data(record: &str) -> Option<&str> {
    let mut event_name = None;
    let mut data = None;
    for line in record.lines().map(|line| line.trim_end_matches('\r')) {
        if let Some(value) = line.strip_prefix("event:") {
            event_name = Some(value.trim());
        }
        if let Some(value) = line.strip_prefix("data:") {
            data = Some(value.trim_start());
        }
    }
    if event_name == Some("forge.event") {
        data
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::parse_events;

    #[test]
    fn parser_ignores_non_forge_keepalive_records() {
        let events = parse_events(b": keepalive\n\n").expect("valid keepalive");

        assert!(events.is_empty());
    }
}
