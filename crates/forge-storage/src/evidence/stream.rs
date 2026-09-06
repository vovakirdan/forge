use forge_domain::{EvidenceScope, EvidenceStream};

use super::{EvidenceError, EvidenceRedaction, EvidenceSpool, PendingEvidence};

/// Reason this stream cannot provide complete retained evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpoolIncident {
    CapacityExhausted,
    WriteFailed,
}

/// Collection signals consumed by the runtime, never inferred Task transitions.
#[derive(Default, Debug)]
pub struct SpoolAppend {
    pub chunks: Vec<PendingEvidence>,
    pub evidence_incomplete: bool,
    pub stop_required: bool,
    pub incident: Option<SpoolIncident>,
    /// Bytes read from the pipe, including discarded bytes after failure.
    pub consumed_bytes: u64,
}

/// Bounded per-stream collector. `push` never stops draining after an incident;
/// its caller must forward the stop signal while continuing to read the pipe.
pub struct EvidenceCollector {
    spool: EvidenceSpool,
    scope: EvidenceScope,
    stream: EvidenceStream,
    redaction: EvidenceRedaction,
    buffer: Vec<u8>,
    sequence: u64,
    incident: Option<SpoolIncident>,
}

impl EvidenceCollector {
    pub(super) fn new(
        spool: EvidenceSpool,
        scope: EvidenceScope,
        stream: EvidenceStream,
        redaction: EvidenceRedaction,
    ) -> Self {
        Self {
            spool,
            scope,
            stream,
            redaction,
            buffer: Vec::new(),
            sequence: 0,
            incident: None,
        }
    }

    /// Consumes any input size using bounded intermediate buffers. Raw bytes
    /// never touch the filesystem before secret redaction has completed.
    pub fn push(&mut self, bytes: &[u8]) -> SpoolAppend {
        let mut output = SpoolAppend {
            consumed_bytes: bytes.len() as u64,
            ..SpoolAppend::default()
        };
        if self.incident.is_none() {
            for slice in bytes.chunks(self.spool.chunk_bytes()) {
                let redacted = self.redaction.push(slice, false);
                self.persist(&redacted, false, &mut output);
                if self.incident.is_some() {
                    break;
                }
            }
        }
        self.signals(&mut output);
        output
    }

    /// Flushes the final redaction suffix and returns any remaining safe chunk.
    pub fn finish(mut self) -> SpoolAppend {
        let mut output = SpoolAppend::default();
        if self.incident.is_none() {
            let final_bytes = self.redaction.push(&[], true);
            self.persist(&final_bytes, true, &mut output);
        }
        self.signals(&mut output);
        output
    }

    fn persist(&mut self, bytes: &[u8], finish: bool, output: &mut SpoolAppend) {
        self.buffer.extend_from_slice(bytes);
        while self.buffer.len() >= self.spool.chunk_bytes() || (finish && !self.buffer.is_empty()) {
            let size = self.buffer.len().min(self.spool.chunk_bytes());
            self.sequence += 1;
            match self.spool.write_chunk(
                self.scope,
                self.stream,
                self.sequence,
                &self.redaction.reference,
                &self.buffer[..size],
            ) {
                Ok(chunk) => output.chunks.push(chunk),
                Err(error) => {
                    self.incident = Some(if matches!(error, EvidenceError::SpoolCapacity) {
                        SpoolIncident::CapacityExhausted
                    } else {
                        SpoolIncident::WriteFailed
                    });
                    self.buffer.clear();
                    return;
                }
            }
            self.buffer.drain(..size);
        }
    }

    fn signals(&self, output: &mut SpoolAppend) {
        output.evidence_incomplete = self.incident.is_some();
        output.stop_required = self.incident.is_some();
        output.incident = self.incident;
    }
}
