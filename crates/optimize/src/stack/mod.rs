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

use review_model::SceneNode;
use serde::{Deserialize, Serialize};

mod ao;
mod export;
pub mod limits;
mod normals;
mod params;
mod remesh;
mod shrinkwrap;

pub use ao::*;
pub use export::*;
pub use normals::*;
pub use params::*;
pub use remesh::*;
pub use shrinkwrap::*;

/// What [`OptStack::rebind_to_model`] did with a loaded preset's per-object
/// overrides — three counts the caller reports, since each means something
/// different to the user: `rebound` found the object elsewhere, `by_position`
/// could only trust the index (a preset older than node names), and `dropped`
/// had nothing to attach to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RebindReport {
    pub rebound: usize,
    pub by_position: usize,
    pub dropped: usize,
}

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
            // Stamped only on the way into a preset: at runtime the index is the
            // identity, and carrying a name here would have to be kept in step
            // with every model swap.
            name: String::new(),
            exclude: false,
            ops: Vec::new(),
        });
        let last = self.overrides.len() - 1;
        &mut self.overrides[last]
    }

    /// Drop override entries that no longer say anything, so an empty entry left
    /// behind by unchecking every box doesn't persist into a preset.
    /// Clamp every numeric setting into the range the inspector offers
    /// ([`limits`]) and cap the LOD chain at [`limits::MAX_LOD_LEVELS`], the
    /// per-object overrides included.
    ///
    /// For a stack that came from outside the chrome: a preset is a file anyone
    /// can edit, and the operations assume the inspector's ranges. Unchecked, a
    /// hand-written `levels` list builds one full copy of the mesh per entry,
    /// and a face count of four billion is a seeding loop that never ends.
    pub fn sanitize(&mut self) {
        let overridden = self
            .overrides
            .iter_mut()
            .flat_map(|entry| entry.ops.iter_mut());
        for op in self.ops.iter_mut().chain(overridden) {
            op.kind.sanitize();
        }
    }

    pub fn prune_overrides(&mut self) {
        self.overrides
            .retain(|entry| entry.exclude || !entry.ops.is_empty());
    }

    /// Drop every per-object override. Called when the model is replaced: an
    /// override names one object of the *old* model, and no amount of bounds
    /// checking makes index 2 of another file the same thing. The operations
    /// themselves describe the setup, not the asset, so they stay.
    pub fn clear_overrides(&mut self) {
        self.overrides.clear();
    }

    /// Record each override's node name from `nodes`, so a preset written from
    /// this stack can be rebound to whatever model it is loaded against.
    ///
    /// Called on a clone on the way into a preset, never on the live stack: the
    /// index is the runtime identity and the name would only be one more thing
    /// to keep in step.
    pub fn stamp_node_names(&mut self, nodes: &[SceneNode]) {
        for entry in &mut self.overrides {
            entry.name = nodes
                .get(entry.node)
                .map(|node| node.name.clone())
                .unwrap_or_default();
        }
    }

    /// Re-point every override at the node of `nodes` that it actually names,
    /// dropping the ones this model has no unambiguous answer for.
    ///
    /// Called when a preset is loaded. An override carries both an index and the
    /// name the node had when it was written, and the name is what decides:
    ///
    /// * the index still holds that name — kept as is;
    /// * exactly one node elsewhere has it — rebound to that index;
    /// * the name is empty (a preset written before names were recorded) and the
    ///   index is in range — kept by position, which is all such a preset can
    ///   say;
    /// * no match, or several — dropped, since applying it would exclude or
    ///   re-parameterize an object the user never chose.
    pub fn rebind_to_model(&mut self, nodes: &[SceneNode]) -> RebindReport {
        let mut report = RebindReport::default();
        self.overrides.retain_mut(|entry| {
            if entry.name.is_empty() {
                // Legacy preset: position is the only identity it carries.
                if entry.node < nodes.len() {
                    report.by_position += 1;
                    return true;
                }
                report.dropped += 1;
                return false;
            }
            if nodes
                .get(entry.node)
                .is_some_and(|node| node.name == entry.name)
            {
                return true;
            }
            let mut matches = nodes
                .iter()
                .enumerate()
                .filter(|(_, node)| node.name == entry.name);
            match (matches.next(), matches.next()) {
                (Some((index, _)), None) => {
                    entry.node = index;
                    report.rebound += 1;
                    true
                }
                // Nothing named that, or several things are — either way this
                // model gives no answer to which object was meant.
                _ => {
                    report.dropped += 1;
                    false
                }
            }
        });
        report
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
        // An override naming no operation is dropped rather than kept: it overrides
        // nothing, and left at its old id it could match one of the fresh ids
        // above and attach itself to an operation it was never written for.
        for entry in &mut self.overrides {
            entry.ops.retain_mut(|override_op| {
                match remap.iter().find(|(old, _)| *old == override_op.id) {
                    Some(&(_, new)) => {
                        override_op.id = new;
                        true
                    }
                    None => false,
                }
            });
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
    /// Regenerate each object's surface as evenly sized, curvature-aligned
    /// triangles or quads, with the materials, UVs and colors projected back on.
    Remesh(RemeshParams),
    /// Replace each object with one closed shell that hugs it, fusing
    /// interpenetrating parts into a single watertight surface.
    Shrinkwrap(ShrinkwrapParams),
    /// Regenerate the normals from the geometry, with a crease angle deciding
    /// which edges stay hard.
    RecalculateNormals(NormalParams),
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
    /// Clamp this operation's settings into [`limits`]; see
    /// [`OptStack::sanitize`].
    pub fn sanitize(&mut self) {
        use limits::*;
        fn normals(params: &mut NormalParams) {
            let default = NormalParams::default();
            fit(
                &mut params.crease_angle,
                NORMAL_CREASE_MIN,
                NORMAL_CREASE_MAX,
                default.crease_angle,
            );
            fit(
                &mut params.smoothing,
                NORMAL_SMOOTHING_MIN,
                NORMAL_SMOOTHING_MAX,
                default.smoothing,
            );
        }
        fn simplify(settings: &mut SimplifySettings) {
            let default = AttributeWeights::default();
            let weights = &mut settings.attribute_weights;
            fit(
                &mut weights.normal,
                ATTRIBUTE_WEIGHT_MIN,
                ATTRIBUTE_WEIGHT_MAX,
                default.normal,
            );
            fit(
                &mut weights.uv,
                ATTRIBUTE_WEIGHT_MIN,
                ATTRIBUTE_WEIGHT_MAX,
                default.uv,
            );
            fit(
                &mut weights.color,
                ATTRIBUTE_WEIGHT_MIN,
                ATTRIBUTE_WEIGHT_MAX,
                default.color,
            );
        }
        fn level(level: &mut LodLevel) {
            let default = LodLevel::default();
            fit(
                &mut level.target_ratio,
                LOD_RATIO_MIN,
                LOD_RATIO_MAX,
                default.target_ratio,
            );
            fit(
                &mut level.target_error,
                LOD_ERROR_MIN,
                LOD_ERROR_MAX,
                default.target_error,
            );
        }
        match self {
            OpKind::Weld(params) => fit(
                &mut params.attribute_tolerance,
                WELD_TOLERANCE_MIN,
                WELD_TOLERANCE_MAX,
                WeldParams::default().attribute_tolerance,
            ),
            OpKind::PruneComponents { error } => {
                fit(
                    error,
                    PRUNE_THRESHOLD_MIN,
                    PRUNE_THRESHOLD_MAX,
                    PRUNE_THRESHOLD_MIN,
                );
            }
            OpKind::Reduce(params) => {
                simplify(&mut params.simplify);
                level(&mut params.target);
            }
            OpKind::Remesh(params) => {
                let default = RemeshParams::default();
                fit(
                    &mut params.ratio,
                    REMESH_RATIO_MIN,
                    REMESH_RATIO_MAX,
                    default.ratio,
                );
                params.faces = params.faces.clamp(REMESH_FACES_MIN, REMESH_FACES_MAX);
                fit(
                    &mut params.crease_angle,
                    REMESH_CREASE_MIN,
                    REMESH_CREASE_MAX,
                    default.crease_angle,
                );
                params.smooth_iterations = params.smooth_iterations.min(REMESH_SMOOTH_MAX);
                fit(
                    &mut params.adaptive_strength,
                    REMESH_ADAPTIVE_MIN,
                    REMESH_ADAPTIVE_MAX,
                    default.adaptive_strength,
                );
                normals(&mut params.normal_params);
            }
            OpKind::Shrinkwrap(params) => {
                let default = ShrinkwrapParams::default();
                params.resolution = params
                    .resolution
                    .clamp(SHRINKWRAP_RESOLUTION_MIN, SHRINKWRAP_RESOLUTION_MAX);
                fit(
                    &mut params.offset,
                    SHRINKWRAP_OFFSET_MIN,
                    SHRINKWRAP_OFFSET_MAX,
                    default.offset,
                );
                params.voxel_resolution = params.voxel_resolution.clamp(
                    SHRINKWRAP_VOXEL_RESOLUTION_MIN,
                    SHRINKWRAP_VOXEL_RESOLUTION_MAX,
                );
                fit(
                    &mut params.voxel_ratio,
                    SHRINKWRAP_TARGET_RATIO_MIN,
                    SHRINKWRAP_TARGET_RATIO_MAX,
                    default.voxel_ratio,
                );
                params.voxel_triangles = params.voxel_triangles.clamp(
                    SHRINKWRAP_TARGET_TRIANGLES_MIN,
                    SHRINKWRAP_TARGET_TRIANGLES_MAX,
                );
                normals(&mut params.normal_params);
            }
            OpKind::RecalculateNormals(params) => normals(params),
            OpKind::SimplifyLod(params) => {
                simplify(&mut params.simplify);
                params.levels.truncate(MAX_LOD_LEVELS);
                params.levels.iter_mut().for_each(level);
            }
            OpKind::BakeAo(params) => {
                let default = BakeAoParams::default();
                fit(
                    &mut params.max_distance,
                    AO_BAKE_DISTANCE_MIN,
                    AO_BAKE_DISTANCE_MAX,
                    default.max_distance,
                );
                fit(
                    &mut params.intensity,
                    AO_BAKE_INTENSITY_MIN,
                    AO_BAKE_INTENSITY_MAX,
                    default.intensity,
                );
            }
            OpKind::Overdraw { threshold } => {
                fit(
                    threshold,
                    OVERDRAW_THRESHOLD_MIN,
                    OVERDRAW_THRESHOLD_MAX,
                    OVERDRAW_THRESHOLD_MIN,
                );
            }
            OpKind::FilterTriangles | OpKind::VertexCache | OpKind::VertexFetch => {}
        }
    }

    /// Every operation the "Add" menu offers, in menu order — cleanup first
    /// (what you almost always want before anything else), then the LOD
    /// generator, then the GPU reorder passes that belong at the end.
    pub const ALL: [fn() -> OpKind; 12] = [
        || OpKind::Weld(WeldParams::default()),
        || OpKind::FilterTriangles,
        || OpKind::PruneComponents { error: 0.01 },
        || OpKind::Shrinkwrap(ShrinkwrapParams::default()),
        || OpKind::Reduce(ReduceParams::default()),
        || OpKind::Remesh(RemeshParams::default()),
        || OpKind::RecalculateNormals(NormalParams::default()),
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
            OpKind::Remesh(_) => "Remesh",
            OpKind::Shrinkwrap(_) => "Shrinkwrap",
            OpKind::RecalculateNormals(_) => "Recalculate Normals",
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
                "Simplify the mesh in place. The same simplifier the LOD chain uses, \
                 but it replaces the mesh instead of generating extra ones - so the \
                 reduced geometry is what the rest of the stack works on and what the \
                 export writes in the source mesh's place."
            }
            OpKind::Remesh(_) => {
                "Rebuild each object's surface as evenly sized, curvature-aligned \
                 triangles or quads. Unlike a simplifier it does not remove what is \
                 there - it regenerates the surface from scratch and projects the \
                 materials, UVs and colors back on, which is what turns a scan or a \
                 CAD import into geometry an engine can use. For fewer triangles with \
                 the silhouette kept, a Reduce does better at the same count. Static \
                 meshes only."
            }
            OpKind::Shrinkwrap(_) => {
                "Replace each object with one closed shell that hugs it. The surface is \
                 voxelized into a distance field (or, with the Voxel method, remeshed \
                 on a voxel grid that keeps thin sheets) and re-extracted, which fuses \
                 a kitbash of interpenetrating parts into a single watertight mesh — and is what \
                 a Remesh below it can even out. Materials, UVs and \
                 colors are projected back on. Static meshes only."
            }
            OpKind::RecalculateNormals(_) => {
                "Regenerate the normals from the shape itself. Edges sharper than the \
                 crease angle stay hard and the rest are smoothed, and the export's \
                 hard/soft edge flags are rewritten to match. Objects with blend \
                 shapes are left as they are. Changes no geometry."
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
    /// reordering it). Drives whether processing has to recompute bounds, and
    /// whether the "geometry changed" warning applies.
    pub fn alters_geometry(&self) -> bool {
        match self {
            OpKind::Weld(_)
            | OpKind::FilterTriangles
            | OpKind::PruneComponents { .. }
            | OpKind::Reduce(_)
            | OpKind::Remesh(_)
            | OpKind::Shrinkwrap(_)
            | OpKind::SimplifyLod(_) => true,
            OpKind::BakeAo(_)
            | OpKind::RecalculateNormals(_)
            | OpKind::VertexCache
            | OpKind::Overdraw { .. }
            | OpKind::VertexFetch => false,
        }
    }

    /// Whether this operation leaves the tangents stale, so processing has to
    /// rebuild them: anything that changes the shape, and anything that rewrites
    /// the normals they are built perpendicular to.
    pub fn invalidates_tangents(&self) -> bool {
        self.alters_geometry() || matches!(self, OpKind::RecalculateNormals(_))
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

    /// An override whose operation is gone must not survive re-keying: at its
    /// old id it could match a fresh one and attach itself to a stranger.
    #[test]
    fn reassigning_ids_drops_an_override_that_names_no_operation() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::FilterTriangles);
        stack.push_op(OpKind::VertexCache);
        // One override for an id that never existed, and one for an operation
        // that has since been removed.
        stack.node_override_mut(0).ops.push(OpInstance {
            id: 999,
            enabled: true,
            kind: OpKind::FilterTriangles,
        });
        stack
            .ops
            .retain(|op| !matches!(op.kind, OpKind::VertexCache));
        stack.node_override_mut(0).ops.push(OpInstance {
            id: 2,
            enabled: true,
            kind: OpKind::VertexCache,
        });

        stack.reassign_ids();

        assert!(
            stack.overrides[0].ops.is_empty(),
            "overrides of missing operations are dropped: {:?}",
            stack.overrides[0].ops
        );
    }

    #[test]
    fn the_add_menu_offers_twelve_distinct_operations() {
        let labels: Vec<&str> = OpKind::ALL.iter().map(|build| build().label()).collect();
        assert_eq!(labels.len(), 12);
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

    /// A scene of `names`, which is all the rebind reads.
    fn nodes(names: &[&str]) -> Vec<SceneNode> {
        names
            .iter()
            .map(|name| SceneNode {
                name: (*name).to_owned(),
                ..SceneNode::default()
            })
            .collect()
    }

    #[test]
    fn clearing_overrides_keeps_the_operations() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::VertexCache);
        stack.node_override_mut(1).exclude = true;

        stack.clear_overrides();
        assert!(stack.overrides.is_empty());
        assert_eq!(stack.ops.len(), 1, "the setup is not the asset");
    }

    #[test]
    fn a_preset_records_the_names_of_the_objects_it_excludes() {
        let mut stack = OptStack::default();
        stack.node_override_mut(1).exclude = true;
        // Out of range at save time (nothing should produce this, but a preset
        // must not carry a name it invented).
        stack.node_override_mut(7).exclude = true;

        stack.stamp_node_names(&nodes(&["root", "hero_prop"]));
        assert_eq!(stack.overrides[0].name, "hero_prop");
        assert_eq!(stack.overrides[1].name, "");
    }

    #[test]
    fn an_override_follows_its_object_to_a_new_index() {
        let mut stack = OptStack::default();
        stack.node_override_mut(1).exclude = true;
        stack.stamp_node_names(&nodes(&["root", "hero_prop"]));

        // The same object, two positions later.
        let report = stack.rebind_to_model(&nodes(&["root", "wall", "floor", "hero_prop"]));
        assert_eq!(
            report,
            RebindReport {
                rebound: 1,
                by_position: 0,
                dropped: 0
            }
        );
        assert_eq!(stack.overrides[0].node, 3);
    }

    #[test]
    fn an_override_stays_put_when_its_index_still_names_it() {
        let mut stack = OptStack::default();
        stack.node_override_mut(1).exclude = true;
        stack.stamp_node_names(&nodes(&["root", "hero_prop"]));

        let report = stack.rebind_to_model(&nodes(&["root", "hero_prop", "extra"]));
        assert_eq!(report, RebindReport::default(), "nothing to report");
        assert_eq!(stack.overrides[0].node, 1);
    }

    #[test]
    fn an_override_is_dropped_rather_than_applied_to_a_different_object() {
        let mut stack = OptStack::default();
        stack.node_override_mut(1).exclude = true;
        stack.stamp_node_names(&nodes(&["root", "hero_prop"]));

        // Same node count, different objects: the old bounds check kept this.
        let report = stack.rebind_to_model(&nodes(&["head", "torso"]));
        assert_eq!(report.dropped, 1);
        assert!(stack.overrides.is_empty());
    }

    #[test]
    fn an_ambiguous_name_is_dropped() {
        let mut stack = OptStack::default();
        stack.node_override_mut(1).exclude = true;
        stack.stamp_node_names(&nodes(&["root", "leaf"]));

        let report = stack.rebind_to_model(&nodes(&["trunk", "branch", "leaf", "leaf"]));
        assert_eq!(report.dropped, 1, "which of the two leaves was meant?");
        assert!(stack.overrides.is_empty());
    }

    #[test]
    fn a_preset_without_names_still_loads_by_position() {
        let mut stack = OptStack::default();
        stack.node_override_mut(1).exclude = true;
        stack.node_override_mut(9).exclude = true;
        // No `stamp_node_names`: this is what a preset written before the name
        // field deserializes to.

        let report = stack.rebind_to_model(&nodes(&["root", "a", "b", "c"]));
        assert_eq!(
            report,
            RebindReport {
                rebound: 0,
                by_position: 1,
                dropped: 1
            }
        );
        assert_eq!(stack.overrides.len(), 1);
        assert_eq!(stack.overrides[0].node, 1);
    }
}
