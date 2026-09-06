//! Core-side LiteLLM administration, never linked into the Run driver.
#![forbid(unsafe_code)]

mod client;
mod model;
mod route;
pub use client::LiteLlmClient;
pub use model::{KeyProvisioned, LiteLlmError, RunKeySpec, SpendObservation, generate_virtual_key};
pub use route::{RouteProvisioned, RouteSpec};
pub const PINNED_LITELLM_VERSION: &str = "1.99.0";
