use super::EvidenceError;
use zeroize::Zeroize;

/// Exact-byte secret redaction policy, pinned before collection begins.
/// Secrets are not Debug/Serialize; only the policy reference enters receipts.
/// It covers supplied literal secrets, not unknown secrets or encoded variants.
pub struct EvidenceRedaction {
    pub(super) reference: String,
    secrets: Vec<Vec<u8>>,
    pending: Vec<u8>,
}

impl Drop for EvidenceRedaction {
    fn drop(&mut self) {
        self.secrets.zeroize();
        self.pending.zeroize();
    }
}

impl EvidenceRedaction {
    /// Creates a bounded redactor. A caller must supply every secret exposed to
    /// this runtime; prompts and credentials themselves are never diagnostic inputs.
    pub fn new(reference: String, secrets: Vec<Vec<u8>>) -> Result<Self, EvidenceError> {
        if reference.is_empty()
            || reference.len() > 256
            || !reference
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._:/-".contains(&b))
            || secrets.len() > 128
            || secrets
                .iter()
                .any(|value| value.is_empty() || value.len() > 16_384)
            || secrets.iter().map(Vec::len).sum::<usize>() > 128 * 1024
        {
            return Err(EvidenceError::InvalidConfiguration);
        }
        let mut secrets = secrets;
        secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        Ok(Self {
            reference,
            secrets,
            pending: Vec::new(),
        })
    }

    pub(super) fn push(&mut self, bytes: &[u8], finish: bool) -> Vec<u8> {
        self.pending.extend_from_slice(bytes);
        let mut output = Vec::new();
        let mut cursor = 0;
        while cursor < self.pending.len() {
            let remaining = &self.pending[cursor..];
            // Delay a possible longer match until the next read. Redacting
            // after independently writing chunks could persist a split secret.
            if !finish
                && self
                    .secrets
                    .iter()
                    .any(|secret| secret.len() > remaining.len() && secret.starts_with(remaining))
            {
                break;
            }
            if let Some(secret) = self
                .secrets
                .iter()
                .find(|secret| remaining.starts_with(secret))
            {
                output.extend_from_slice(b"[redacted]");
                cursor += secret.len();
            } else {
                output.push(self.pending[cursor]);
                cursor += 1;
            }
        }
        self.pending.drain(..cursor);
        output
    }
}
