//! What Remesh and Shrinkwrap share around their per-object solves: which
//! objects there are, how much of the machine each one may use, and how the
//! rebuilt ones are put back in place of the old.
//!
//! Both operations throw a node's pieces away and replace them, one node per
//! core ([`crate::parallel::solve_nodes`]), and all of this used to be written
//! once in each - with the second copy looking nodes up by `Vec::contains`,
//! which is quadratic in exactly the scene a wrap is pointed at: a node of a
//! thousand leaf cards.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::Warnings;
use crate::notice::OptWarning;
use crate::process::{OptProgress, RunContext};
use crate::stack::OpKind;
use crate::submesh::Submesh;

/// Every node's non-empty pieces, nodes in first-seen order.
///
/// First-seen so the output's piece order follows the input's and two runs over
/// the same stack produce the same buffer layout. One pass over the pieces,
/// whatever the number of nodes. A node whose pieces are all empty has nothing
/// to rebuild and is left out.
pub(crate) fn pieces_by_node(submeshes: &[Submesh]) -> Vec<(u32, Vec<&Submesh>)> {
    let mut slot_of: HashMap<u32, usize> = HashMap::new();
    let mut groups: Vec<(u32, Vec<&Submesh>)> = Vec::new();
    for piece in submeshes.iter().filter(|piece| !piece.is_empty()) {
        let slot = *slot_of.entry(piece.node).or_insert_with(|| {
            groups.push((piece.node, Vec::new()));
            groups.len() - 1
        });
        groups[slot].1.push(piece);
    }
    groups
}

/// What one node's solve produced. The warnings ride back with the mesh rather
/// than going into the shared [`Warnings`] as they happen: a worker cannot
/// reach it, and folding them in afterwards in job order is what keeps the list
/// the same on every run.
pub(crate) struct NodeOutcome {
    /// The node's new pieces, or `None` when it is left as it was.
    pub(crate) rebuilt: Option<Vec<Submesh>>,
    pub(crate) notes: Vec<OptWarning>,
}

/// Each job's outcome parked in a slot of its own as it lands, rather than
/// folded in, so the warning order and the piece order are the stack's and not
/// the scheduler's.
pub(crate) struct Outcomes {
    slots: Vec<Option<NodeOutcome>>,
    done: u32,
}

impl Outcomes {
    pub(crate) fn new(jobs: usize) -> Self {
        Self {
            slots: (0..jobs).map(|_| None).collect(),
            done: 0,
        }
    }

    /// Park job `index`'s outcome, and say so.
    ///
    /// Reported as each object *lands*: with several in flight there is no
    /// single object being waited on any more, so the count is what carries the
    /// news, and the name says which one just finished. A solve is minutes on a
    /// dense object, so this is what separates "slow" from "wedged" - the
    /// question a user staring at the notice actually has.
    pub(crate) fn land(
        &mut self,
        index: usize,
        outcome: NodeOutcome,
        run: &RunContext<'_>,
        op: &OpKind,
        name: &str,
    ) {
        self.done += 1;
        run.report(OptProgress::object(
            op,
            name,
            self.done,
            self.slots.len() as u32,
        ));
        self.slots[index] = Some(outcome);
    }

    /// Job `index`'s finished mesh, once it has landed and if it rebuilt one.
    pub(crate) fn rebuilt(&self, index: usize) -> Option<&[Submesh]> {
        self.slots[index].as_ref()?.rebuilt.as_deref()
    }

    /// Fold every outcome in, in job order - `nodes` names each job's node - and
    /// splice the rebuilt nodes into place.
    pub(crate) fn finish(
        self,
        submeshes: &mut Vec<Submesh>,
        nodes: impl IntoIterator<Item = u32>,
        warnings: &mut Warnings,
    ) {
        let mut replacements: HashMap<u32, Vec<Submesh>> = HashMap::new();
        for (node, outcome) in nodes.into_iter().zip(self.slots) {
            // A job a cancelled run never claimed has no outcome; its node is
            // left as it was, and the run it belongs to is discarded anyway.
            let Some(outcome) = outcome else {
                continue;
            };
            for note in outcome.notes {
                warnings.push(note);
            }
            if let Some(rebuilt) = outcome.rebuilt {
                replacements.insert(node, rebuilt);
            }
        }
        splice(submeshes, replacements);
    }
}

/// Replace each rebuilt node's pieces: its new pieces land where its first old
/// piece was, and its other old pieces are dropped. Everything else passes
/// through untouched, in order.
pub(crate) fn splice(submeshes: &mut Vec<Submesh>, mut replacements: HashMap<u32, Vec<Submesh>>) {
    if replacements.is_empty() {
        return;
    }
    let mut rebuilt: Vec<Submesh> = Vec::with_capacity(submeshes.len());
    let mut placed = HashSet::new();
    for piece in submeshes.drain(..) {
        match replacements.get_mut(&piece.node) {
            Some(pieces) => {
                if placed.insert(piece.node) {
                    rebuilt.append(pieces);
                }
            }
            None => rebuilt.push(piece),
        }
    }
    *submeshes = rebuilt;
}

/// [`splice`] by borrow, for a preview: the same layout, so a preview and the
/// mesh that replaces it are laid out the same way and the viewport does not
/// jump, and nothing is cloned to show it.
pub(crate) fn spliced<'a>(
    submeshes: &'a [Submesh],
    replacements: &HashMap<u32, &'a [Submesh]>,
) -> Vec<&'a Submesh> {
    let mut spliced: Vec<&Submesh> = Vec::with_capacity(submeshes.len());
    let mut placed = HashSet::new();
    for piece in submeshes {
        match replacements.get(&piece.node) {
            Some(pieces) => {
                if placed.insert(piece.node) {
                    spliced.extend(pieces.iter());
                }
            }
            None => spliced.push(piece),
        }
    }
    spliced
}

/// What one object may spend inside its own solve, asked again before every
/// stage.
///
/// So that the parallelism across objects and the parallelism *within* one do
/// not oversubscribe each other: N objects across N cores get one thread each,
/// one object gets the machine. Fixing that share when the jobs start left the
/// object that takes longest - the trunk of a plant, minutes against the
/// leaves' seconds - on one thread long after every other core had gone idle.
/// Asked per stage, the share grows as the objects around it finish.
///
/// Handing an object more threads part way through changes nothing in what it
/// produces: every stage is the same at any thread count, because the work is
/// split into fixed chunks rather than into one per thread
/// ([`crate::parallel::CHUNK`]). That is what makes it free to give the cores
/// to whoever is still running.
pub(crate) struct ThreadBudget {
    cores: usize,
    /// How many objects run at once at most: one per core, or one per object.
    workers: usize,
    /// Objects not finished yet, running or still waiting for a core.
    unfinished: AtomicUsize,
}

impl ThreadBudget {
    /// The budget for `jobs` objects solved by [`crate::parallel::solve_nodes`],
    /// which runs one per core.
    pub(crate) fn for_jobs(jobs: usize) -> Self {
        let cores = crate::parallel::default_threads();
        let jobs = jobs.max(1);
        Self {
            cores,
            workers: cores.min(jobs),
            unfinished: AtomicUsize::new(jobs),
        }
    }

    /// The same share for as long as it is asked, for a stage tested alone.
    #[cfg(test)]
    pub(crate) fn fixed(threads: usize) -> Self {
        Self {
            cores: threads.max(1),
            workers: 1,
            unfinished: AtomicUsize::new(1),
        }
    }

    /// Threads one object may use for the stage it is about to start.
    ///
    /// While objects are still waiting for a core every worker is busy, so the
    /// objects running are the workers; once fewer remain than there are
    /// workers, the ones that remain are all that is running.
    pub(crate) fn now(&self) -> usize {
        let running = self
            .workers
            .min(self.unfinished.load(Ordering::Relaxed))
            .max(1);
        (self.cores / running).max(1)
    }

    /// One object has finished, so the ones still running may spread into the
    /// cores it leaves.
    pub(crate) fn finished_one(&self) {
        let _ = self
            .unfinished
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                left.checked_sub(1)
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::submesh::partition;

    #[test]
    fn a_share_grows_as_the_objects_around_it_finish() {
        let budget = ThreadBudget {
            cores: 24,
            workers: 13,
            unfinished: AtomicUsize::new(13),
        };
        assert_eq!(budget.now(), 1, "thirteen objects on 24 cores: one each");
        for _ in 0..9 {
            budget.finished_one();
        }
        assert_eq!(budget.now(), 6, "four left share the machine");
        for _ in 0..3 {
            budget.finished_one();
        }
        assert_eq!(budget.now(), 24, "the last one gets all of it");
        budget.finished_one();
        assert_eq!(budget.now(), 24, "and finishing past zero is harmless");
    }

    #[test]
    fn a_fixed_budget_is_fixed() {
        let budget = ThreadBudget::fixed(4);
        budget.finished_one();
        assert_eq!(budget.now(), 4);
        assert_eq!(ThreadBudget::fixed(0).now(), 1);
    }

    #[test]
    fn pieces_are_grouped_by_node_in_first_seen_order() {
        let (mut pieces, _) = partition(&review_model::demo_cube_model(), None);
        let template = pieces[0].clone();
        pieces.clear();
        for node in [4u32, 2, 4, 7, 2] {
            pieces.push(Submesh {
                node,
                ..template.clone()
            });
        }
        pieces.push(Submesh {
            node: 9,
            indices: Vec::new(),
            ..template.clone()
        });

        let groups = pieces_by_node(&pieces);

        let shape: Vec<(u32, usize)> = groups
            .iter()
            .map(|(node, pieces)| (*node, pieces.len()))
            .collect();
        assert_eq!(
            shape,
            vec![(4, 2), (2, 2), (7, 1)],
            "and a node with nothing in it is not a job"
        );
    }

    #[test]
    fn a_splice_puts_a_node_where_its_first_piece_was() {
        let (template, _) = partition(&review_model::demo_cube_model(), None);
        let piece = |node: u32, material: u32| Submesh {
            node,
            material,
            ..template[0].clone()
        };
        let mut submeshes = vec![piece(1, 0), piece(2, 0), piece(1, 1), piece(3, 0)];
        let replacements = HashMap::from([(1u32, vec![piece(1, 7)])]);

        let borrowed: HashMap<u32, &[Submesh]> = replacements
            .iter()
            .map(|(&node, pieces)| (node, pieces.as_slice()))
            .collect();
        let preview: Vec<(u32, u32)> = spliced(&submeshes, &borrowed)
            .iter()
            .map(|piece| (piece.node, piece.material))
            .collect();
        splice(&mut submeshes, replacements);

        let result: Vec<(u32, u32)> = submeshes
            .iter()
            .map(|piece| (piece.node, piece.material))
            .collect();
        assert_eq!(result, vec![(1, 7), (2, 0), (3, 0)]);
        assert_eq!(preview, result, "a preview is laid out as the result is");
    }
}
