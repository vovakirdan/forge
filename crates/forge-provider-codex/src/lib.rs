//! Pinned native Codex CLI adapter; execution belongs to the sandbox Supervisor.
#![forbid(unsafe_code)]

pub mod app_server;
pub mod driver;
mod events;
mod prepare;

pub use events::parse_jsonl_event;
pub use prepare::{CodexAdapter, CodexRunInput};

/// CLI flags, feature names and JSONL shapes are verified against this release:
/// <https://github.com/openai/codex/tree/rust-v0.153.2>.
/// Live provider and relay conformance are separate opt-in acceptance gates.
pub const PINNED_CODEX_VERSION: &str = "0.153.2";
pub const CODEX_HOME: &str = "/run/forge/codex-home";
