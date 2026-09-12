//! The operation stack: what the user assembles in the Opt workspace, and the
//! single value that a preset round-trips.
//!
//! The stack is deliberately a plain data description — it holds *what* to do,
//! never any mesh or GPU state — so it is cheap to clone into an undo snapshot,
//! cheap to hand to a worker thread, and serializable as-is. [`crate::process`]
//! is what interprets it.//!
//! ## Layout
//!
//! [`OptStack`] and the operations it holds are here; [`params`] carries each
//! operation's parameters, [`ao`] the bake's settings and [`export`] the export
//! options. Everything is `serde`-derived, so a move changes no preset: the
//! names on the wire are per type, not per module.

use serde::{Deserialize, Serialize};

mod ao;
mod export;
mod params;

pub use ao::*;
pub use export::*;
pub use params::*;

/// A whole optimization setup: the ordered operations, any per-object
/// deviations from them, and the settings the export step will use.
///
/// Order is meaning: operations apply top to bottom. The single
/// [`OpKind::SimplifyLod`] entry (if present) is where the pipeline fans out
/// into the LOD chain — operations above it run once on the base mesh,
/// operations below it run on every generated level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OptStack {
    pub ops: Vec<OpInstance>,
    /// Per-object deviations, keyed by index into `ModelData::nodes`.
    pub overrides: Vec<NodeOverride>,
    /// Settings for the final export step. Part of the stack (rather than
    /// separate UI state) so they are undoable and travel with a preset — the
    /// packaging and hierarchy choices are as much a part of "the setup" as the
    /// operations are.
    pub export: ExportOptions,
    /// Source of stable ids for new operations. Ids must stay unique for the
    /// life of a stack: the UI addresses the selected operation by id, so
    /// reusing one after a delete would silently move the selection.
    next_id: u64,
}

impl Default for OptStack {
    fn default() -> Self {
        Self {
            ops: Vec::new(),
            overrides: Vec::new(),
            export: ExportOptions::default(),
            next_id: 1,
        }
    }
}

impl OptStack {
    /// Append an operation, assigning it a fresh id (which is returned so the
    /// caller can select the new row).
    pub fn push_op(&mut self, kind: OpKind) -> u64 {
        let id = self.allocate_id();
        self.ops.push(OpInstance {
            id,
            enabled: true,
            kind,
        });
        id
    }

    /// Remove the operation with `id`, along with any per-node override that
    /// referenced it — an override for an operation that no longer exists would
    /// be invisible in the UI but still serialize into presets.
    pub fn remove_op(&mut self, id: u64) {
        self.ops.retain(|op| op.id != id);
        for entry in &mut self.overrides {
            entry.ops.retain(|op| op.id != id);
        }
        self.overrides
            .retain(|entry| entry.exclude || !entry.ops.is_empty());
    }

    /// Move the operation at `index` by `offset` positions, clamped to the
    /// stack. Returns whether anything actually moved.
    pub fn reorder(&mut self, index: usize, offset: isize) -> bool {
        if index >= self.ops.len() {
            return false;
        }
        let target = index.saturating_add_signed(offset).min(self.ops.len() - 1);
        if target == index {
            return false;
        }
        let op = self.ops.remove(index);
        self.ops.insert(target, op);
        true
    }

    pub fn op(&self, id: u64) -> Option<&OpInstance> {
        self.ops.iter().find(|op| op.id == id)
    }

    pub fn op_mut(&mut self, id: u64) -> Option<&mut OpInstance> {
        self.ops.iter_mut().find(|op| op.id == id)
    }

    /// True when the stack already carries a LOD operation. Only one is allowed:
    /// a second would have to fan out an already-fanned-out chain, which has no
    /// meaningful interpretation.
    pub fn has_lod(&self) -> bool {
        self.ops
            .iter()
            .any(|op| matches!(op.kind, OpKind::SimplifyLod(_)))
    }

    /// The LOD operation's parameters, if the stack has an *enabled* one.
    pub fn lod_params(&self) -> Option<&LodParams> {
        self.ops.iter().find_map(|op| match &op.kind {
            OpKind::SimplifyLod(params) if op.enabled => Some(params),
            _ => None,
        })
    }

    /// How many mesh levels processing will produce: the base mesh plus one per
    /// configured LOD level, or just the base mesh with no enabled LOD op.
    pub fn output_level_count(&self) -> usize {
        1 + self.lod_params().map_or(0, |params| params.levels.len())
    }

    pub fn node_override(&self, node: usize) -> Option<&NodeOverride> {
        self.overrides.iter().find(|entry| entry.node == node)
    }

    /// The override entry for `node`, created if absent.
    pub fn node_override_mut(&mut self, node: usize) -> &mut NodeOverride {
        if let Some(position) = self.overrides.iter().position(|entry| entry.node == node) {
            return &mut self.overrides[position];
        }
        self.overrides.push(NodeOverride {
            node,
            exclude: false,
            ops: Vec::new(),
        });
        let last = self.overrides.len() - 1;
        &mut self.overrides[last]
    }

    /// Drop override entries that no longer say anything, so an empty entry left
    /// behind by unchecking every box doesn't persist into a preset.
    pub fn prune_overrides(&mut self) {
        self.overrides
            .retain(|entry| entry.exclude || !entry.ops.is_empty());
    }

    /// Discard overrides pointing past `node_count`. Called when a preset is
    /// loaded against a different model than the one it was authored on —
    /// stale node indices would otherwise silently apply to the wrong objects.
    pub fn clamp_to_model(&mut self, node_count: usize) {
        self.overrides.retain(|entry| entry.node < node_count);
    }

    /// Re-key every operation id from a fresh sequence. Applied after loading a
    /// preset so ids can never collide with those of the stack being replaced.
    pub fn reassign_ids(&mut self) {
        // The whole old→new mapping is built before a single override is
        // touched. Rewriting them as the walk goes lets an id that has already
        // been rewritten collide with a later operation's *old* id and be
        // rewritten a second time, silently reattaching the override to the
        // wrong operation — which is reachable whenever a reorder has left the
        // ids out of ascending order.
        let mut remap: Vec<(u64, u64)> = Vec::with_capacity(self.ops.len());
        for (position, op) in self.ops.iter_mut().enumerate() {
            let new = position as u64 + 1;
            remap.push((op.id, new));
            op.id = new;
        }
        for entry in &mut self.overrides {
            for override_op in &mut entry.ops {
                if let Some(&(_, new)) = remap.iter().find(|(old, _)| *old == override_op.id) {
                    override_op.id = new;
                }
            }
        }
        self.next_id = self.ops.len() as u64 + 1;
    }

    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }
}

/// One configured operation in the stack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpInstance {
    /// Stable identity for UI selection; unique within a stack.
    pub id: u64,
    /// Unchecked operations stay in the stack but are skipped — the way to A/B
    /// a single step without losing its settings.
    pub enabled: bool,
    pub kind: OpKind,
}

/// The operations the Opt workspace exposes, one variant per meshoptimizer
/// capability the tool surfaces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OpKind {
    /// Merge vertices the importer split apart. FBX import expands every face
    /// corner into its own vertex, so this is usually the first thing worth
    /// doing — a cube arrives with 24 vertices and leaves with 8.
    Weld(WeldParams),
    /// Drop degenerate and duplicate triangles.
    FilterTriangles,
    /// Remove small disconnected components (stray shells, orphaned faces).
    PruneComponents { error: f32 },
    /// Simplify the mesh in place, replacing it — the same simplifier
    /// [`OpKind::SimplifyLod`] runs, applied as an ordinary stack step rather
    /// than as a fan-out.
    Reduce(ReduceParams),
    /// Generate the LOD chain. At most one per stack.
    SimplifyLod(LodParams),
    /// Bake raycast ambient occlusion into the vertex-color set.
    BakeAo(BakeAoParams),
    /// Reorder triangles for the GPU's post-transform vertex cache.
    VertexCache,
    /// Reorder triangles front-to-back to reduce overdraw.
    Overdraw { threshold: f32 },
    /// Reorder (and compact) vertices for linear vertex-buffer reads.
    VertexFetch,
}

impl OpKind {
    /// Every operation the "Add" menu offers, in menu order — cleanup first
    /// (what you almost always want before anything else), then the LOD
    /// generator, then the GPU reorder passes that belong at the end.
    pub const ALL: [fn() -> OpKind; 9] = [
        || OpKind::Weld(WeldParams::default()),
        || OpKind::FilterTriangles,
        || OpKind::PruneComponents { error: 0.01 },
        || OpKind::Reduce(ReduceParams::default()),
        || OpKind::SimplifyLod(LodParams::default()),
        || OpKind::BakeAo(BakeAoParams::default()),
        || OpKind::VertexCache,
        || OpKind::Overdraw { threshold: 1.05 },
        || OpKind::VertexFetch,
    ];

    /// Row label in the stack panel.
    pub fn label(&self) -> &'static str {
        match self {
            OpKind::Weld(_) => "Weld Vertices",
            OpKind::FilterTriangles => "Filter Triangles",
            OpKind::PruneComponents { .. } => "Prune Components",
            OpKind::Reduce(_) => "Reduce",
            OpKind::SimplifyLod(_) => "Generate LODs",
            OpKind::BakeAo(_) => "Bake AO to Vertex Colors",
            OpKind::VertexCache => "Optimize Vertex Cache",
            OpKind::Overdraw { .. } => "Optimize Overdraw",
            OpKind::VertexFetch => "Optimize Vertex Fetch",
        }
    }

    /// One-line explanation, shown as the row tooltip and above the parameters
    /// in the Inspector.
    pub fn description(&self) -> &'static str {
        match self {
            OpKind::Weld(_) => {
                "Merge vertices across a seam. Every run already merges vertices \
                 that match in every attribute; this widens what counts as a match \
                 — dropping normals or UVs from the comparison, or allowing a \
                 tolerance — which does change the mesh."
            }
            OpKind::FilterTriangles => {
                "Remove degenerate triangles (two corners at one position) and exact \
                 duplicates. Opposite-winding duplicates are kept for double-sided geometry."
            }
            OpKind::PruneComponents { .. } => {
                "Remove disconnected pieces smaller than the error threshold — stray \
                 shells and orphaned faces left behind by modelling."
            }
            OpKind::Reduce(_) => {
                "Simplify the mesh in place. The same simplifier the LOD chain uses,                  but it replaces the mesh instead of generating extra ones - so the                  reduced geometry is what the rest of the stack works on and what the                  export writes in the source mesh's place."
            }
            OpKind::SimplifyLod(_) => {
                "Generate the LOD chain. Each level is simplified independently from \
                 the mesh as it stands at this point in the stack."
            }
            OpKind::BakeAo(_) => {
                "Raycast ambient occlusion at each vertex and write it into the \
                 vertex-color set. Objects named *_LOD<n> bake only against their \
                 own LOD's geometry, so a whole visible LOD chain bakes correctly \
                 in one run; hidden objects don't take part — hide collision \
                 shells first. Bakes the mesh as it stands at this point in the \
                 stack. Changes no geometry."
            }
            OpKind::VertexCache => {
                "Reorder triangles so the GPU's post-transform vertex cache hits more \
                 often. Changes no geometry; watch ACMR/ATVR in the stats."
            }
            OpKind::Overdraw { .. } => {
                "Reorder triangles front-to-back within cache-friendly clusters so the \
                 GPU shades fewer hidden pixels. Changes no geometry."
            }
            OpKind::VertexFetch => {
                "Reorder vertices into the order the index buffer reads them, and drop \
                 any vertex nothing references. Changes no geometry."
            }
        }
    }

    /// Whether this operation can change the mesh's shape (as opposed to only
    /// reordering it). Drives whether processing has to regenerate tangents and
    /// recompute bounds, and whether the "geometry changed" warning applies.
    pub fn alters_geometry(&self) -> bool {
        match self {
            OpKind::Weld(_)
            | OpKind::FilterTriangles
            | OpKind::PruneComponents { .. }
            | OpKind::Reduce(_)
            | OpKind::SimplifyLod(_) => true,
            OpKind::BakeAo(_)
            | OpKind::VertexCache
            | OpKind::Overdraw { .. }
            | OpKind::VertexFetch => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_not_reused_after_removal() {
        let mut stack = OptStack::default();
        let first = stack.push_op(OpKind::FilterTriangles);
        let second = stack.push_op(OpKind::VertexCache);
        assert_ne!(first, second);

        stack.remove_op(first);
        let third = stack.push_op(OpKind::VertexFetch);
        assert_ne!(third, first, "a removed id must not come back");
        assert_ne!(third, second);
    }

    #[test]
    fn removing_an_op_drops_its_overrides() {
        let mut stack = OptStack::default();
        let id = stack.push_op(OpKind::Weld(WeldParams::default()));
        stack.node_override_mut(3).ops.push(OpInstance {
            id,
            enabled: true,
            kind: OpKind::Weld(WeldParams::default()),
        });

        stack.remove_op(id);
        assert!(
            stack.overrides.is_empty(),
            "an override with nothing left to say should not persist"
        );
    }

    #[test]
    fn reorder_clamps_at_the_ends() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::FilterTriangles);
        stack.push_op(OpKind::VertexCache);

        assert!(!stack.reorder(0, -1), "already first");
        assert!(!stack.reorder(1, 1), "already last");
        assert!(stack.reorder(0, 1));
        assert_eq!(stack.ops[0].kind, OpKind::VertexCache);
    }

    #[test]
    fn output_level_count_follows_the_lod_op() {
        let mut stack = OptStack::default();
        assert_eq!(stack.output_level_count(), 1);

        let id = stack.push_op(OpKind::SimplifyLod(LodParams::default()));
        assert_eq!(stack.output_level_count(), 4, "base plus three levels");

        stack.op_mut(id).expect("just pushed").enabled = false;
        assert_eq!(
            stack.output_level_count(),
            1,
            "a disabled LOD op generates nothing"
        );
    }

    #[test]
    fn reassigning_ids_keeps_every_override_on_its_own_operation() {
        let mut stack = OptStack::default();
        let weld = stack.push_op(OpKind::Weld(WeldParams::default()));
        stack.push_op(OpKind::FilterTriangles);
        stack.push_op(OpKind::VertexCache);
        // Move the last operation to the front: the ids are now [3, 1, 2], so a
        // rewrite that consumed the id space as it walked would hand the weld's
        // override an id it has yet to visit.
        assert!(stack.reorder(2, -2));
        stack.node_override_mut(0).ops.push(OpInstance {
            id: weld,
            enabled: true,
            kind: OpKind::Weld(WeldParams {
                compare_normals: false,
                ..WeldParams::default()
            }),
        });

        stack.reassign_ids();

        let weld = stack
            .ops
            .iter()
            .find(|op| matches!(op.kind, OpKind::Weld(_)))
            .expect("the weld is still in the stack");
        assert_eq!(
            stack.overrides[0].ops[0].id, weld.id,
            "the override still addresses the operation it was attached to"
        );
        assert!(
            stack.ops.iter().all(|op| op.id < stack.next_id),
            "the id source stays ahead of every re-keyed operation"
        );
    }

    #[test]
    fn the_add_menu_offers_nine_distinct_operations() {
        let labels: Vec<&str> = OpKind::ALL.iter().map(|build| build().label()).collect();
        assert_eq!(labels.len(), 9);
        for (index, label) in labels.iter().enumerate() {
            assert!(
                !labels[index + 1..].contains(label),
                "duplicate menu label: {label}"
            );
        }
    }

    #[test]
    fn the_ao_bake_reports_attribute_only_changes() {
        assert!(
            !OpKind::BakeAo(BakeAoParams::default()).alters_geometry(),
            "the bake writes colors, never shape — no tangent rebuild, no warning"
        );
    }

    #[test]
    fn clamp_to_model_drops_stale_node_overrides() {
        let mut stack = OptStack::default();
        stack.node_override_mut(1).exclude = true;
        stack.node_override_mut(9).exclude = true;

        stack.clamp_to_model(4);
        assert_eq!(stack.overrides.len(), 1);
        assert_eq!(stack.overrides[0].node, 1);
    }
}
