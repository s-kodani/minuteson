//! Shared inputs for native unit tests.

/// Reuse one function item across cancellation-free test calls so generic
/// storage and processing paths do not specialize for every `|| false` closure.
pub(crate) fn never_cancelled() -> bool {
    false
}

// Fixture construction only; no product retry entry point.
pub(crate) mod model_retry;
