//! Abandoning a run the user has already moved past.
//!
//! ## Why dropping the result is not enough
//!
//! `app` has always dropped a superseded result by generation, which keeps a
//! stale mesh off screen but recovers none of the work. That was affordable
//! while every operation was a buffer edit measured in milliseconds: the run
//! finished before the next slider frame either way.
//!
//! Retopology broke that. A Remesh over a dozen objects is tens of seconds even
//! spread across every core ([`crate::parallel`]), so an edit arriving mid-run
//! used to wait for the whole of a result nobody would ever see before the run
//! that replaced it could even start. Dragging a parameter meant one full
//! rebuild per drag, served in order.
//!
//! ## The token
//!
//! A verbatim twin of `review_import`'s `CancelToken`, and deliberately so: the
//! shape is proven and the two behave identically, so `app` reasons about a run
//! exactly as it reasons about a load. It is a *copy* rather than a shared type
//! because `optimize` depends on `review_model` alone — it must not grow a
//! dependency on the FBX reader to borrow fifteen lines (invariant 10's reason,
//! applied one crate over).
//!
//! The counter is the caller's own request generation, and the token is live
//! while it still reads the value it was made with. Cancellation is therefore a
//! consequence of the check `app` already performs rather than a second piece of
//! state to keep in step with it — there is no way to cancel a run without
//! superseding it, and no way to supersede one without cancelling it.
//!
//! ## How fine it can be
//!
//! Checks sit wherever the run is between units of work: between operations,
//! between LOD levels, between assembled levels, per submesh in a simplify, per
//! vertex chunk in an AO bake, per object in a rebuild, and between the stages
//! of one object's rebuild.
//!
//! Inside a Remesh object the floor is **one pass**. `remesh::solve` checks
//! between every stage, and the stages that iterate check again between their
//! own passes — the size field's smoothing, each partition sweep and Lloyd
//! iteration, each region the collapse merges, each cleanup round, the cross
//! field's smoothing — so a cancelled rebuild stops part way through the object
//! it is on rather than finishing it. The parallel sweeps a pass is built from
//! (`parallel::sweep`) do not check: each is one pass over a flat array and
//! short. Shrinkwrap checks only between its stages, so it can still wait out
//! one of them (the distance field or the extraction). Every other operation
//! stops almost at once.

pub use review_model::CancelToken;

/// Whether a run carrying `token` should stop. The `None` case is a run nobody
/// can cancel — a test, or a batch caller — so it never stops early.
pub(crate) fn cancelled(token: Option<&CancelToken>) -> bool {
    token.is_some_and(CancelToken::is_cancelled)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    #[test]
    fn a_token_is_live_until_its_generation_is_superseded() {
        let current = Arc::new(AtomicU64::new(7));
        let token = CancelToken::new(Arc::clone(&current), 7);
        assert!(!token.is_cancelled());

        current.store(8, Ordering::Relaxed);
        assert!(token.is_cancelled());
    }

    #[test]
    fn no_token_never_cancels() {
        assert!(!cancelled(None));
    }
}
