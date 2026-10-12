//! Abandoning work the user has already moved past.
//!
//! One shared token for every long-running job that reads a model — an import,
//! an Opt run, an audit. The counter is the caller's own request generation, and
//! the token is live while it still reads the value it was made with, so
//! cancellation is a consequence of the generation check the caller already
//! performs rather than a second piece of state to keep in step with it.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// A shared "is this run still wanted?" flag, checked from inside a run.
#[derive(Debug, Clone)]
pub struct CancelToken {
    current: Arc<AtomicU64>,
    mine: u64,
}

impl CancelToken {
    /// A token for request `generation`, live until `current` moves past it.
    pub fn new(current: Arc<AtomicU64>, generation: u64) -> Self {
        Self {
            current,
            mine: generation,
        }
    }

    /// Whether this request has been superseded.
    pub fn is_cancelled(&self) -> bool {
        self.current.load(Ordering::Relaxed) != self.mine
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_live_until_its_generation_is_superseded() {
        let current = Arc::new(AtomicU64::new(7));
        let token = CancelToken::new(Arc::clone(&current), 7);
        assert!(!token.is_cancelled());
        current.store(8, Ordering::Relaxed);
        assert!(token.is_cancelled());
    }
}
