//! Managed OpenCode server adapter and sandbox-local HTTP session driver.
#![forbid(unsafe_code)]

mod client;
pub mod driver;
mod events;
mod prepare;

pub use client::{OpenCodeClient, OpenCodeError, SessionId, SessionResult};
pub use events::{EventInterpreter, SessionEvent, parse_driver_event, write_driver_event};
pub use prepare::{OpenCodeAdapter, OpenCodeRunInput};

pub const PINNED_OPENCODE_VERSION: &str = "1.18.29";
pub const VIRTUAL_KEY_PATH: &str = "/run/forge/opencode/virtual-key";
