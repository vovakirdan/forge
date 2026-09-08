//! Local HTTP/JSON and bounded SSE transport for the canonical Core.

mod candidate_review;
mod communication;
mod error;
mod finding;
mod git_integration;
mod handlers;
mod project_hooks;
mod reads;
mod views;

pub use handlers::router;
