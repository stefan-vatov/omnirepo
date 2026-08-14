//! Declared transitions (explicit names for the callers).

use super::InitialResult;

pub fn acquired(_source_identity: &str) -> InitialResult {
    // The transition carries no result; the caller supplies the
    // synchronized result separately.
    InitialResult::Unchanged
}

pub fn synchronized(result: InitialResult) -> InitialResult {
    result
}

pub fn failed(reason: &str) -> InitialResult {
    InitialResult::Failed {
        reason: reason.to_owned(),
    }
}

pub fn cancelled() -> InitialResult {
    InitialResult::Cancelled
}
