//! Pinned Claude Code subscription adapter. Core owns authorization and outcomes;
//! the Supervisor owns process isolation, stream delivery, and termination.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod events;
mod input;
mod prepare;

pub use events::{ClaudeEvent, ClaudeTurnEnd, parse_jsonl_event};
pub use input::{encode_user_message, validate_setup_token};
pub use prepare::{ClaudeAdapter, ClaudeRunInput};

/// Version whose headless flags and protocol fixtures this adapter targets.
/// Fixture conformance does not establish live provider availability.
pub const PINNED_CLAUDE_VERSION: &str = "2.1.263";
/// Fresh per-Run configuration directory, never the operator's Claude home.
pub const CLAUDE_CONFIG_DIR: &str = "/run/forge/claude-home";
/// Fixed private copy of the explicitly enrolled subscription setup token.
pub const CLAUDE_TOKEN_PATH: &str = "/run/forge/claude-home/setup-token";
/// Bounded machine event/input size, excluding one line delimiter.
pub const MAX_EVENT_BYTES: usize = 1024 * 1024;
