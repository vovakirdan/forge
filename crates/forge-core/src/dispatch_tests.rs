use super::*;

// Infer the opaque future type without constructing Core, connecting to PG,
// polling the scheduler, or configuring a larger test-thread stack.
fn future_size<F>(_: impl FnOnce(&'static CoreService, ProjectId) -> F) -> usize {
    std::mem::size_of::<F>()
}

#[test]
fn scheduler_boundary_does_not_embed_all_purpose_futures_in_callers() {
    let size = future_size(CoreService::dispatch_available);
    // The M3 unboxed debug future measured 27,680 bytes. This generous bound
    // protects callers without coupling the test to an exact compiler layout.
    assert!(size <= 4_096, "scheduler boundary future is {size} bytes");
}
