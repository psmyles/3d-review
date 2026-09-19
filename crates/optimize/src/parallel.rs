//! Solving one object per core, for the operations that rebuild a surface.
//!
//! ## Why this exists
//!
//! Remesh and Shrinkwrap are the only operations whose unit of work is a whole
//! object: each one throws a node's surface away and regenerates it, so two
//! nodes share nothing and can be solved at the same time. Every other
//! operation edits a buffer in place and is already fast.
//!
//! Left serial they were the slowest thing in the workspace by a wide margin.
//! A field solve is seconds to minutes on a dense object; "Only quads" is
//! slower again (QuadriFlow is entirely serial as vendored) and may be run up
//! to four times on one node (`remesh::BUDGET_ATTEMPTS`) to land the face
//! count. Measured on a stylized palm: thirteen objects, one core busy, the
//! other twenty-three idle, minutes end to end.
//!
//! ## Why it is safe, and why the result does not move
//!
//! Instant Meshes reaches a process-wide pool through the vendored TBB shim,
//! and the shim is already built for concurrent callers: `serial_here` is
//! `thread_local`, so one node's `SerialScope` cannot silence another's, and
//! `ThreadPool::run` takes the batch lock with `try_lock` and runs the body in
//! order on the calling thread when it cannot get it. So the first node to
//! reach a `parallel_for` takes the pool and the rest run serially — never a
//! race, never a deadlock. QuadriFlow has no shared state at all.
//!
//! That also means the intra-solve pool stops mattering once several nodes are
//! in flight, and **that is the better trade**: the pool parallelized phases
//! within one solve, where the shim's own notes record that thread count
//! changes the answer. Node-level work does not — a node's bytes are a function
//! of its own input — so with `Reproducible` on (the default), where every node
//! opens a `SerialScope` and solves in order on its own thread, this is exactly
//! as reproducible as the serial loop was, and faster by the number of objects.
//!
//! ## Shape
//!
//! A shared claim counter rather than buckets dealt up front, because these
//! jobs are wildly uneven — a trunk and a leaf are the same job to a scheduler
//! and minutes apart in fact — so a static split leaves cores idle behind the
//! one long node. (`ao.rs` deals its buckets up front for the opposite reason:
//! its jobs are fixed-size vertex chunks.)
//!
//! Results come back down a channel to the thread that opened the scope, which
//! is what keeps the progress sink single-threaded — it is a `&dyn Fn`, and it
//! renders a line and posts an event, neither of which belongs on a worker.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use crate::cancel::{CancelToken, cancelled};

/// Run `solve` over every job, on every core, and hand each result to `landed`
/// on *this* thread as it arrives.
///
/// `landed` takes the job's index, so a caller that needs a deterministic order
/// can park the results in a slot each and merge them afterwards — completion
/// order is whatever the machine made of it, and nothing user-visible may
/// depend on that.
///
/// A cancelled run stops claiming jobs, so `landed` is called for some prefix
/// of them and the caller is left with holes. Every caller already treats a
/// missing result as "this node was left as it is", and the run it belongs to
/// is discarded whole anyway.
pub(crate) fn solve_nodes<J, R>(
    jobs: &[J],
    cancel: Option<&CancelToken>,
    solve: impl Fn(&J) -> R + Sync,
    mut landed: impl FnMut(usize, R),
) where
    J: Sync,
    R: Send,
{
    if jobs.is_empty() {
        return;
    }
    // One job is the common case for a single-object edit, and spawning for it
    // would cost a thread and a channel to do the same work.
    if jobs.len() == 1 {
        if !cancelled(cancel) {
            landed(0, solve(&jobs[0]));
        }
        return;
    }

    let workers = std::thread::available_parallelism()
        .map_or(1, std::num::NonZero::get)
        .min(jobs.len());
    let next = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel();

    std::thread::scope(|scope| {
        for _ in 0..workers {
            let sender = sender.clone();
            let next = &next;
            let solve = &solve;
            scope.spawn(move || {
                loop {
                    // Claimed but not started: a superseded run stops without
                    // solving anything else, which is what bounds a cancelled
                    // rebuild at the one engine call already in flight rather
                    // than at the whole scene.
                    if cancelled(cancel) {
                        break;
                    }
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(job) = jobs.get(index) else {
                        break;
                    };
                    // A closed channel means the receiver below is gone, which
                    // only happens once every job has been claimed.
                    if sender.send((index, solve(job))).is_err() {
                        break;
                    }
                }
            });
        }
        // The last live sender, dropped so the loop below ends when the workers
        // do rather than blocking forever on a channel nobody can write to.
        drop(sender);

        // This thread does no solving: it blocks here handing results on, which
        // is what lets `landed` report progress the moment a node finishes
        // instead of after the whole scope.
        while let Ok((index, result)) = receiver.recv() {
            landed(index, result);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_job_lands_exactly_once_under_its_own_index() {
        let jobs: Vec<usize> = (0..64).collect();
        let mut seen = vec![None; jobs.len()];
        solve_nodes(
            &jobs,
            None,
            |job| job * 2,
            |index, result| {
                assert!(seen[index].is_none(), "job {index} landed twice");
                seen[index] = Some(result);
            },
        );
        assert_eq!(
            seen,
            (0..64).map(|job| Some(job * 2)).collect::<Vec<_>>(),
            "each result must come back under the index of the job that made it"
        );
    }

    /// The single-job shortcut is a different code path, so it gets its own
    /// check rather than riding on the one above.
    #[test]
    fn a_single_job_still_lands() {
        let mut seen = None;
        solve_nodes(
            &[7usize],
            None,
            |job| job + 1,
            |index, result| seen = Some((index, result)),
        );
        assert_eq!(seen, Some((0, 8)));
    }

    /// A token that is already dead stops the run before any job is claimed.
    #[test]
    fn a_cancelled_run_solves_nothing() {
        let current = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(2));
        let dead = CancelToken::new(current, 1);
        let jobs: Vec<usize> = (0..32).collect();
        let mut landed = 0;
        solve_nodes(&jobs, Some(&dead), |job| *job, |_, _| landed += 1);
        assert_eq!(landed, 0);
    }

    #[test]
    fn no_jobs_is_not_an_error() {
        let jobs: [usize; 0] = [];
        solve_nodes(&jobs, None, |job| *job, |_, _| panic!("nothing to land"));
    }
}
