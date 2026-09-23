//! Local HTTP/JSON and bounded SSE transport for the canonical Core.

mod cancellation_reasons;
mod candidate_review;
mod communication;
mod dependency_reads;
mod error;
mod file_snapshots;
mod finding;
mod git_integration;
mod git_source_policy;
mod handlers;
mod knowledge;
mod management_reads;
mod memory;
mod priority_scheme;
mod project_hooks;
mod reads;
mod run_activity;
mod runtime_metadata;
mod system_jobs;
mod task_handoffs;
mod views;

pub use handlers::router;
