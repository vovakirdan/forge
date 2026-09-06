//! Strict W3C version-00 trace context. Untrusted header text is never logged.

/// Validated correlation metadata only; not an authentication or authority token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TraceContext {
    trace_id: u128,
    span_id: u64,
    sampled: bool,
}

impl TraceContext {
    /// Parses the supported W3C traceparent version, rejecting zero identities.
    #[must_use]
    pub fn parse(header: &str) -> Option<Self> {
        if header.len() != 55
            || !header.starts_with("00-")
            || header.as_bytes()[35] != b'-'
            || header.as_bytes()[52] != b'-'
            || !header.bytes().enumerate().all(|(index, byte)| {
                [2, 35, 52].contains(&index)
                    || byte.is_ascii_digit()
                    || (b'a'..=b'f').contains(&byte)
            })
        {
            return None;
        }
        let trace_id = u128::from_str_radix(&header[3..35], 16).ok()?;
        let span_id = u64::from_str_radix(&header[36..52], 16).ok()?;
        let flags = u8::from_str_radix(&header[53..], 16).ok()?;
        Self::new(trace_id, span_id, flags & 1 == 1)
    }

    /// Creates a context from caller-generated nonzero identities.
    #[must_use]
    pub fn new(trace_id: u128, span_id: u64, sampled: bool) -> Option<Self> {
        (trace_id != 0 && span_id != 0).then_some(Self {
            trace_id,
            span_id,
            sampled,
        })
    }

    /// Creates a downstream span while retaining the trace and sampling decision.
    #[must_use]
    pub fn child(self, span_id: u64) -> Option<Self> {
        Self::new(self.trace_id, span_id, self.sampled)
    }
    #[must_use]
    pub fn trace_id(self) -> String {
        format!("{:032x}", self.trace_id)
    }
    #[must_use]
    pub fn span_id(self) -> String {
        format!("{:016x}", self.span_id)
    }
    #[must_use]
    pub fn header(self) -> String {
        format!(
            "00-{:032x}-{:016x}-{:02x}",
            self.trace_id,
            self.span_id,
            u8::from(self.sampled)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trace_context_validates_and_preserves_only_bounded_correlation_fields() {
        let value = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        assert_eq!(TraceContext::parse(value).unwrap().header(), value);
        for invalid in [
            "secret-value",
            "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01",
            "00-4BF92F3577B34DA6A3CE929D0E0E4736-00f067aa0ba902b7-01",
        ] {
            assert!(TraceContext::parse(invalid).is_none());
        }
    }
}
