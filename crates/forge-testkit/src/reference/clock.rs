//! Controllable wall clock; no process clock changes or timer sleeps.

use std::sync::{Arc, RwLock};

use forge_application::Clock;
use forge_domain::Timestamp;

/// Shared wall time supplied to the real application command boundary.
#[derive(Clone, Debug)]
pub struct ManualClock(Arc<RwLock<Timestamp>>);

impl ManualClock {
    /// Starts at an explicit instant; callers choose its precision.
    #[must_use]
    pub fn new(now: Timestamp) -> Self {
        Self(Arc::new(RwLock::new(now)))
    }

    /// Moves wall time in either direction, including across a lock wait.
    pub fn set(&self, now: Timestamp) {
        *self
            .0
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = now;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        *self
            .0
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
