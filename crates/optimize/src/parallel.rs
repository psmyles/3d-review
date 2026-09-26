//! Solving one object per core, for the operations that rebuild a surface.
//!
//! ## Why this exists
//!
//! Remesh and Shrinkwrap are the only operations whose unit of work is a whole
//! object: each one throws a node's surface away and regenerates it, so two
//! nodes share nothing and can be solved at the same time. Every other
//! operation edits a buffer in place and is already fast.
//!
//! Left serial they were the slowest thing in the workspace by a wide margin: a
//! rebuild is seconds on a dense object, and a scene of a dozen of them ran one
//! at a time while the rest of the machine sat idle. Measured on a stylized
//! palm: thirteen objects, one core busy, the other twenty-three not.
//!
//! ## Why it changes no output
//!
//! An object's rebuild reads its own surface and writes its own mesh; nothing
//! is shared with the object beside it. And each stage inside one is itself
//! independent of how it was scheduled (see [`sweep`]), so the whole thing is a
//! pure function of its input and the parallelism is invisible in the result.
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
pub(crate) fn solve_nodes<J, R, P>(
    jobs: &[J],
    cancel: Option<&CancelToken>,
    solve: impl Fn(&J, &dyn Fn(P)) -> R + Sync,
    mut landed: impl FnMut(usize, R),
    mut previewed: impl FnMut(usize, P),
) where
    J: Sync,
    R: Send,
    P: Send,
{
    if jobs.is_empty() {
        return;
    }
    // One job is the common case for a single-object edit, and spawning for it
    // would cost a thread and a channel to do the same work.
    if jobs.len() == 1 {
        if !cancelled(cancel) {
            // Handed straight on as each one is made, exactly as the threaded
            // path below does: holding them until the solve returned meant a
            // single-object rebuild - the common edit - showed no progress at
            // all, then replayed every stale preview at the end. `solve` takes
            // an `Fn` and `previewed` is an `FnMut`, hence the `RefCell`; not a
            // lock, because there is exactly one thread here.
            let previewed = std::cell::RefCell::new(&mut previewed);
            let result = solve(&jobs[0], &|preview| (previewed.borrow_mut())(0, preview));
            landed(0, result);
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
                    // rebuild at the one object already in flight rather than
                    // at the whole scene.
                    if cancelled(cancel) {
                        break;
                    }
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(job) = jobs.get(index) else {
                        break;
                    };
                    // A part-finished object goes down the same channel as a
                    // finished one, so the thread below sees both in the order
                    // they actually happened.
                    let emit = |preview| {
                        let _ = sender.send(Message::Preview(index, preview));
                    };
                    let result = solve(job, &emit);
                    // A closed channel means the receiver below is gone, which
                    // only happens once every job has been claimed.
                    if sender.send(Message::Done(index, result)).is_err() {
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
        while let Ok(message) = receiver.recv() {
            match message {
                Message::Done(index, result) => landed(index, result),
                Message::Preview(index, preview) => previewed(index, preview),
            }
        }
    });
}

/// What a worker sends back: an object finished, or one part way through.
enum Message<R, P> {
    Done(usize, R),
    Preview(usize, P),
}

/// How many elements one worker claims at a time in a [`sweep`].
///
/// **Fixed, never derived from the thread count.** Every reduction below folds
/// per chunk and merges in chunk order, so the arithmetic a sweep performs is a
/// function of this constant alone — change the thread count and the answer is
/// bit-for-bit the same, change this and it may not be. That is the whole
/// determinism argument for the stages built on it, and it is why a "tune the
/// chunk to the core count" optimization must never be added here.
pub(crate) const CHUNK: usize = 65_536;

/// Threads to use when nothing has budgeted them. The per-object budget
/// (`solve_nodes`) is the real source; this is for a lone call.
pub(crate) fn default_threads() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
}

/// Fill `out` in parallel, one [`CHUNK`] at a time: `body(base, chunk)` writes
/// `chunk[i]` for the element whose global index is `base + i`.
///
/// The Jacobi shape every field pass in [`crate::remesh`] takes — read the
/// previous buffer, write this one — so the chunks are disjoint `&mut` slices
/// and the borrow checker is the proof that the parallelism is sound. Since a
/// chunk's contents depend only on its own indices and on immutable input,
/// scheduling cannot reach the result.
pub(crate) fn sweep<T, F>(out: &mut [T], threads: usize, body: F)
where
    T: Send,
    F: Fn(usize, &mut [T]) + Sync,
{
    if out.is_empty() {
        return;
    }
    let chunks = out.len().div_ceil(CHUNK);
    let workers = threads.max(1).min(chunks);
    if workers <= 1 {
        for (index, chunk) in out.chunks_mut(CHUNK).enumerate() {
            body(index * CHUNK, chunk);
        }
        return;
    }

    // Dealt round-robin before the threads start, as `ao.rs` does: the chunks
    // are a fixed size, so a static split is already balanced and needs no
    // shared counter.
    let mut buckets: Vec<Vec<(usize, &mut [T])>> = (0..workers).map(|_| Vec::new()).collect();
    for (index, chunk) in out.chunks_mut(CHUNK).enumerate() {
        buckets[index % workers].push((index * CHUNK, chunk));
    }
    std::thread::scope(|scope| {
        for bucket in buckets {
            let body = &body;
            scope.spawn(move || {
                for (base, chunk) in bucket {
                    body(base, chunk);
                }
            });
        }
    });
}

/// Fold `[0, len)` one [`CHUNK`] at a time, returning one partial per chunk
/// **in chunk order**.
///
/// The caller merges them itself, in that order, which is what keeps a
/// floating-point reduction independent of the thread count: the partials are
/// always the same values combined in always the same sequence.
#[allow(
    dead_code,
    reason = "read by the size field, which the next stage wires in"
)]
pub(crate) fn sweep_reduce<A, F>(len: usize, threads: usize, fold: F) -> Vec<A>
where
    A: Send,
    F: Fn(std::ops::Range<usize>) -> A + Sync,
{
    if len == 0 {
        return Vec::new();
    }
    let chunks = len.div_ceil(CHUNK);
    let range_of = |chunk: usize| {
        let start = chunk * CHUNK;
        start..((start + CHUNK).min(len))
    };
    let workers = threads.max(1).min(chunks);
    if workers <= 1 {
        return (0..chunks).map(|chunk| fold(range_of(chunk))).collect();
    }

    let mut out: Vec<Option<A>> = (0..chunks).map(|_| None).collect();
    let mut buckets: Vec<Vec<(usize, &mut Option<A>)>> = (0..workers).map(|_| Vec::new()).collect();
    for (index, slot) in out.iter_mut().enumerate() {
        buckets[index % workers].push((index, slot));
    }
    std::thread::scope(|scope| {
        for bucket in buckets {
            let fold = &fold;
            scope.spawn(move || {
                for (index, slot) in bucket {
                    *slot = Some(fold(range_of(index)));
                }
            });
        }
    });
    out.into_iter()
        .map(|partial| partial.expect("every chunk is folded exactly once"))
        .collect()
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
            |job, _| job * 2,
            |index, result| {
                assert!(seen[index].is_none(), "job {index} landed twice");
                seen[index] = Some(result);
            },
            |_, (): ()| panic!("nothing previews here"),
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
            |job, _| job + 1,
            |index, result| seen = Some((index, result)),
            |_, (): ()| panic!("nothing previews here"),
        );
        assert_eq!(seen, Some((0, 8)));
    }

    /// A single job's previews reach the caller while it is still solving - the
    /// whole point of a preview. They used to be held until the solve returned
    /// and then replayed, all stale, just before the result.
    #[test]
    fn a_single_jobs_previews_arrive_while_it_solves() {
        // Atomic only because `solve` must be `Sync`; one thread touches it.
        let delivered = std::sync::atomic::AtomicU32::new(0);
        let read = || delivered.load(Ordering::Relaxed);
        let mut landed_after = None;
        solve_nodes(
            &[()],
            None,
            |_, emit| {
                for pass in 1..=3 {
                    emit(pass);
                    assert_eq!(read(), pass, "preview {pass} was held back");
                }
            },
            |_, ()| landed_after = Some(read()),
            |_, pass: u32| {
                assert_eq!(pass, read() + 1, "previews arrive in order");
                delivered.store(pass, Ordering::Relaxed);
            },
        );
        assert_eq!(landed_after, Some(3));
    }

    /// A token that is already dead stops the run before any job is claimed.
    #[test]
    fn a_cancelled_run_solves_nothing() {
        let current = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(2));
        let dead = CancelToken::new(current, 1);
        let jobs: Vec<usize> = (0..32).collect();
        let mut landed = 0;
        solve_nodes(
            &jobs,
            Some(&dead),
            |job, _| *job,
            |_, _| landed += 1,
            |_, (): ()| panic!("nothing previews here"),
        );
        assert_eq!(landed, 0);
    }

    #[test]
    fn no_jobs_is_not_an_error() {
        let jobs: [usize; 0] = [];
        solve_nodes(
            &jobs,
            None,
            |job, _| *job,
            |_, _| panic!("nothing to land"),
            |_, (): ()| panic!("nothing to preview"),
        );
    }

    /// The property every field stage rests on: a sweep writes the same bytes
    /// however many threads ran it.
    #[test]
    fn a_sweep_writes_the_same_bytes_at_every_thread_count() {
        let len = CHUNK * 3 + 17;
        let fill = |threads: usize| {
            let mut out = vec![0u64; len];
            sweep(&mut out, threads, |base, chunk| {
                for (offset, slot) in chunk.iter_mut().enumerate() {
                    *slot = ((base + offset) as u64).wrapping_mul(2_654_435_761);
                }
            });
            out
        };

        let one = fill(1);
        assert_eq!(one, fill(2));
        assert_eq!(one, fill(7));
        assert_eq!(one[len - 1], ((len - 1) as u64).wrapping_mul(2_654_435_761));
    }

    /// A float reduction is order-sensitive, so the partials must come back in
    /// chunk order regardless of which worker produced them.
    #[test]
    fn sweep_reduce_partials_are_in_chunk_order_at_every_thread_count() {
        let len = CHUNK * 4 + 3;
        let fold = |threads: usize| {
            sweep_reduce(len, threads, |range| {
                range.map(|index| 1.0 / (index as f64 + 1.0)).sum::<f64>()
            })
        };

        let one = fold(1);
        assert_eq!(one.len(), len.div_ceil(CHUNK));
        assert_eq!(one, fold(3));
        assert_eq!(one, fold(8));
        // And the merged total, which is what a caller actually uses.
        let total: f64 = one.iter().sum();
        assert_eq!(total, fold(5).iter().sum::<f64>());
    }

    #[test]
    fn an_empty_sweep_is_not_an_error() {
        let mut out: Vec<u32> = Vec::new();
        sweep(&mut out, 4, |_, _| panic!("nothing to fill"));
        assert!(sweep_reduce(0, 4, |_| 1u32).is_empty());
    }
}
