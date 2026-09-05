//! Local command-line client boundary for Forge.

#![forbid(unsafe_code)]

mod client;
mod sse;

pub use client::{ClientError, LocalClient, default_socket_path};
