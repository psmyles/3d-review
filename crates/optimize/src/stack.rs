//! The operation stack: what the user assembles in the Opt workspace, and the
//! single value that a preset round-trips.
//!
//! The stack is deliberately a plain data description — it holds *what* to do,
//! never any mesh or GPU state — so it is cheap to clone into an undo snapshot,
//! cheap to hand to a worker thread, and serializable as-is. [`crate::process`]
//! is what interprets it.

use serde::{Deserialize, Serialize};

use crate::ffi::simplify_options;

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
        let mut next = 1u64;
        for op in &mut self.ops {
            let old = op.id;
            op.id = next;
            for entry in &mut self.overrides {
                for override_op in &mut entry.ops {
                    if override_op.id == old {
                        override_op.id = next;
                    }
                }
            }
            next += 1;
        }
        self.next_id = next;
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
    /// Generate the LOD chain. At most one per stack.
    SimplifyLod(LodParams),
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
    pub const ALL: [fn() -> OpKind; 7] = [
        || OpKind::Weld(WeldParams::default()),
        || OpKind::FilterTriangles,
        || OpKind::PruneComponents { error: 0.01 },
        || OpKind::SimplifyLod(LodParams::default()),
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
            OpKind::SimplifyLod(_) => "Generate LODs",
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
            OpKind::SimplifyLod(_) => {
                "Generate the LOD chain. Each level is simplified independently from \
                 the mesh as it stands at this point in the stack."
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
            | OpKind::SimplifyLod(_) => true,
            OpKind::VertexCache | OpKind::Overdraw { .. } | OpKind::VertexFetch => false,
        }
    }
}

/// Vertex welding settings.
///
/// meshoptimizer always requires positions to match *exactly* to merge two
/// vertices; the tolerance applies to the other attributes. That is the useful
/// knob in practice: it rejoins a seam that was split only because a normal or
/// UV differs in the last few digits, without moving any geometry.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WeldParams {
    /// Maximum per-component attribute difference still considered equal.
    /// `0.0` means exact binary equality (the fast path).
    pub attribute_tolerance: f32,
    /// Include normals in the equality test. Turning this off merges across
    /// hard edges, which flattens their shading — but it is what you want when
    /// normals will be recomputed downstream.
    pub compare_normals: bool,
    /// Include UVs in the equality test. Off merges across UV seams.
    pub compare_uvs: bool,
    /// Include vertex colors in the equality test.
    pub compare_colors: bool,
}

impl Default for WeldParams {
    fn default() -> Self {
        Self {
            attribute_tolerance: 0.0,
            compare_normals: true,
            compare_uvs: true,
            compare_colors: true,
        }
    }
}

/// Which simplifier runs, and what it is asked to preserve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SimplifyAlgorithm {
    /// Topology-preserving, position error only. The safe default.
    #[default]
    Standard,
    /// Topology-preserving, and additionally penalizes attribute drift so
    /// normals / UVs / colors stay closer to the original.
    WithAttributes,
    /// Ignores topology: much faster and hits the target far more reliably, but
    /// can close holes and merge nearby shells. Best for the smallest levels.
    Sloppy,
}

impl SimplifyAlgorithm {
    pub const ALL: [SimplifyAlgorithm; 3] = [
        SimplifyAlgorithm::Standard,
        SimplifyAlgorithm::WithAttributes,
        SimplifyAlgorithm::Sloppy,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SimplifyAlgorithm::Standard => "Standard",
            SimplifyAlgorithm::WithAttributes => "Preserve Attributes",
            SimplifyAlgorithm::Sloppy => "Sloppy (fast)",
        }
    }
}

/// How much each attribute resists being distorted, used only by
/// [`SimplifyAlgorithm::WithAttributes`]. Higher weights preserve the attribute
/// harder at the cost of geometric accuracy; `0.0` ignores it entirely.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AttributeWeights {
    pub normal: f32,
    pub uv: f32,
    pub color: f32,
}

impl Default for AttributeWeights {
    fn default() -> Self {
        // meshoptimizer's own demo uses weights around 1.0 for normals and
        // noticeably lower for UVs; colors matter least for silhouette.
        Self {
            normal: 1.0,
            uv: 0.5,
            color: 0.0,
        }
    }
}

/// LOD chain settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LodParams {
    pub algorithm: SimplifyAlgorithm,
    pub attribute_weights: AttributeWeights,
    pub flags: SimplifyFlags,
    /// One entry per generated level, in order. Level 0 is always the
    /// unsimplified mesh and is not listed here.
    pub levels: Vec<LodLevel>,
}

impl Default for LodParams {
    fn default() -> Self {
        Self {
            algorithm: SimplifyAlgorithm::default(),
            attribute_weights: AttributeWeights::default(),
            flags: SimplifyFlags::default(),
            levels: vec![
                LodLevel {
                    target_ratio: 0.5,
                    target_error: 0.01,
                },
                LodLevel {
                    target_ratio: 0.25,
                    target_error: 0.02,
                },
                LodLevel {
                    target_ratio: 0.125,
                    target_error: 0.05,
                },
            ],
        }
    }
}

/// One level of the LOD chain.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LodLevel {
    /// Target triangle count as a fraction of the level-0 mesh, in `0.0..=1.0`.
    pub target_ratio: f32,
    /// Error the simplifier may not exceed, relative to the mesh extent (or in
    /// world units when [`SimplifyFlags::error_absolute`] is set). The
    /// simplifier stops short of `target_ratio` rather than exceed this.
    pub target_error: f32,
}

impl Default for LodLevel {
    fn default() -> Self {
        Self {
            target_ratio: 0.5,
            target_error: 0.01,
        }
    }
}

/// The `meshopt_Simplify*` option flags, as individually-labelled toggles. Kept
/// as named booleans rather than a raw bitmask so a preset stays readable and
/// survives upstream renumbering the bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SimplifyFlags {
    /// Never move vertices on an open boundary — keeps pieces that must stay
    /// aligned with neighbouring meshes from pulling away.
    pub lock_border: bool,
    /// Interpret `target_error` in world units instead of as a fraction of the
    /// mesh extent.
    pub error_absolute: bool,
    /// Let the simplifier delete disconnected parts as it goes.
    pub prune: bool,
    /// Even out triangle size and shape, at some cost to accuracy.
    pub regularize: bool,
    /// A gentler [`Self::regularize`].
    pub regularize_light: bool,
    /// Allow collapses across attribute discontinuities (UV/normal seams).
    pub permissive: bool,
}

impl SimplifyFlags {
    /// The bitmask meshoptimizer expects.
    pub fn bits(self) -> u32 {
        let mut bits = 0;
        if self.lock_border {
            bits |= simplify_options::LOCK_BORDER;
        }
        if self.error_absolute {
            bits |= simplify_options::ERROR_ABSOLUTE;
        }
        if self.prune {
            bits |= simplify_options::PRUNE;
        }
        if self.regularize {
            bits |= simplify_options::REGULARIZE;
        }
        if self.regularize_light {
            bits |= simplify_options::REGULARIZE_LIGHT;
        }
        if self.permissive {
            bits |= simplify_options::PERMISSIVE;
        }
        bits
    }
}

/// A per-object deviation from the global stack.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NodeOverride {
    /// Index into `ModelData::nodes`.
    pub node: usize,
    /// Pass this object through untouched. It still appears in every output
    /// level, at full detail — the way to keep a hero prop or a collision shell
    /// out of the optimization.
    pub exclude: bool,
    /// Replacement settings for specific operations, matched by
    /// [`OpInstance::id`]. Operations not listed here use the global settings.
    pub ops: Vec<OpInstance>,
}

/// How the LOD chain is laid out on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LodPackaging {
    /// One FBX holding `MeshName_LOD0` … `MeshName_LODn` as sibling nodes — the
    /// naming convention most engines auto-detect.
    #[default]
    SingleFileSuffixed,
    /// `Asset_LOD0.fbx`, `Asset_LOD1.fbx`, … one file per level.
    FilePerLod,
}

impl LodPackaging {
    pub const ALL: [LodPackaging; 2] = [LodPackaging::SingleFileSuffixed, LodPackaging::FilePerLod];

    pub fn label(self) -> &'static str {
        match self {
            LodPackaging::SingleFileSuffixed => "Single file, suffixed nodes",
            LodPackaging::FilePerLod => "One file per LOD",
        }
    }
}

/// Whether the export reconstructs the source scene graph or flattens it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HierarchyMode {
    /// Re-emit the original node hierarchy, moving geometry back into each
    /// node's local space. The export round-trips like the source asset.
    #[default]
    Rebuild,
    /// Emit one root-level node per mesh with world-space geometry and an
    /// identity transform.
    FlatBaked,
}

impl HierarchyMode {
    pub const ALL: [HierarchyMode; 2] = [HierarchyMode::Rebuild, HierarchyMode::FlatBaked];

    pub fn label(self) -> &'static str {
        match self {
            HierarchyMode::Rebuild => "Rebuild original hierarchy",
            HierarchyMode::FlatBaked => "Flat, world-baked meshes",
        }
    }
}

/// FBX container format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FbxFormat {
    #[default]
    Binary,
    Ascii,
}

impl FbxFormat {
    pub const ALL: [FbxFormat; 2] = [FbxFormat::Binary, FbxFormat::Ascii];

    pub fn label(self) -> &'static str {
        match self {
            FbxFormat::Binary => "Binary",
            FbxFormat::Ascii => "ASCII",
        }
    }
}

/// Settings for the export step, edited through the stack panel's
/// "Export settings" row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportOptions {
    pub packaging: LodPackaging,
    pub hierarchy: HierarchyMode,
    pub format: FbxFormat,
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
    fn clamp_to_model_drops_stale_node_overrides() {
        let mut stack = OptStack::default();
        stack.node_override_mut(1).exclude = true;
        stack.node_override_mut(9).exclude = true;

        stack.clamp_to_model(4);
        assert_eq!(stack.overrides.len(), 1);
        assert_eq!(stack.overrides[0].node, 1);
    }
}
