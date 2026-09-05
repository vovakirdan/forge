//! Local HTTP/JSON and bounded SSE transport for the canonical Core.

mod error;
mod handlers;
mod reads;
mod views;

pub use handlers::router;
