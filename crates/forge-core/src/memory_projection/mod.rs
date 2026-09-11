//! Retrieval is a disposable projection; its IDs require canonical Core hydration.

pub(crate) mod agentmemory;
mod canonical;
mod delivery;
mod search;

pub use canonical::MemorySearchDocument;
pub use delivery::ProjectionDrainReport;
pub use search::{MemorySearchDegradation, MemorySearchHit, MemorySearchMode, MemorySearchResult};
