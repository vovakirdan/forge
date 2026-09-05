use forge_protocol::wire::CommandName;
use thiserror::Error;

/// A safe refusal while decoding an untrusted named command.
#[derive(Debug, Error)]
pub enum ApplicationError {
    /// The caller omitted or malformed its idempotency identity.
    #[error("invalid idempotency key: {reason}")]
    InvalidIdempotencyKey {
        /// Safe explanation for a local operator.
        reason: String,
    },

    /// The body does not match the command selected by the HTTP path.
    #[error("invalid {command:?} payload: {reason}")]
    InvalidPayload {
        /// Named command selected by the request path.
        command: CommandName,
        /// Safe structural validation explanation.
        reason: String,
    },

    /// A field intended as a Forge identifier did not parse or validate.
    #[error("invalid {field}: {reason}")]
    InvalidIdentifier {
        /// Field name safe to show to callers.
        field: &'static str,
        /// Safe validation explanation.
        reason: String,
    },
}
