//! Local command-line client boundary for Forge.

#![forbid(unsafe_code)]

mod client;
pub mod profile_template;
mod sse;
pub mod ui_login;

pub use client::{ClientError, LocalClient, default_socket_path};
