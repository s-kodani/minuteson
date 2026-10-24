//! Shared inputs for native unit tests.

/// Reuse one function item across cancellation-free test calls so generic
/// storage and processing paths do not specialize for every `|| false` closure.
pub(crate) fn never_cancelled() -> bool {
    false
}
/// Absolute fixture identity without assuming a Unix root on Windows.
pub(crate) fn absolute(path: &str) -> String {
    if cfg!(windows) {
        format!("C:{path}")
    } else {
        path.into()
    }
}

// Fixture construction only; no product retry entry point.
pub(crate) mod model_retry;
