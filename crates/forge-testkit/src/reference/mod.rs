//! Services-free persistence adapter for the production command engine.
//!
//! This is a test reference, not an alternative Forge deployment. Runtime
//! observations may be seeded explicitly as fixtures; this module runs no
//! scheduler, provider, delivery consumer or physical executor.

mod audit;
mod clock;
mod fault;
mod model;
mod store;
mod transaction;

pub use clock::ManualClock;
pub use fault::{FaultPoint, FaultTransaction};
pub use model::*;
pub use store::{MemoryStore, MemoryTransaction};

#[cfg(test)]
mod tests;
