//! The processing pipeline: stack in, LOD chain out.
//!
//! ## Shape of a run
//!
//! 1. Split the model into per-(node, material) submeshes ([`crate::submesh`]).
//! 2. Apply every enabled operation *above* the LOD operation, once.
//! 3. Fan out: level 0 is the mesh as it now stands; each configured LOD level
//!    re-simplifies **from that same level-0 mesh**, not from the level above.
//! 4. Apply every enabled operation *below* the LOD operation to each level.
//! 5. Compact, reassemble a [`ModelData`] per level, and measure it.
//!
//! Step 3 is the quality-over-speed choice the project's locked decisions call
//! for: simplifying each level from the full-detail mesh avoids compounding one
//! level's error into the next, at the cost of doing more work per run. That
//! work happens on a background thread, so the interactive loop doesn't feel it.
//!
//! ## Failure policy
//!
//! A failure inside one operation on one submesh is recorded as a warning and
//! that submesh is left as it was; the rest of the model still processes. A
//! preview that shows most of the asset plus a clear warning is far more useful
//! while tweaking sliders than no preview at all. Only a whole-run failure
//! (meshoptimizer missing, empty model) returns `Err`.

use std::time::{Duration, Instant};

use review_model::{ModelData, ModelStats, TriangleData, Vertex};

use crate::meshopt::{self, AnalysisCounters};
use crate::ops;
use crate::stack::{LodParams, OpInstance, OpKind, OptStack, SimplifyAlgorithm};
use crate::submesh::{self, Submesh, TagPresence};
use crate::{OptError, Warnings};

/// Everything a run needs. Borrowed rather than owned so the caller can hand in
/// an `Arc<ModelData>`'s contents without cloning the mesh.
#[derive(Debug, Clone, Copy)]
pub struct ProcessInput<'a> {
    pub model: &'a ModelData,
    pub stack: &'a OptStack,
    /// Size in bytes of one vertex *as the renderer uploads it*. Only the
    /// overfetch figure uses it, and using the real GPU vertex size is what
    /// makes that figure describe the actual draw rather than this crate's
    /// intermediate layout.
    pub render_vertex_size: usize,
}

/// One output level: the mesh, and what measuring it produced.
#[derive(Debug, Clone)]
pub struct ProcessedLod {
    /// 0 for the base mesh, then one per configured LOD level.
    pub level: usize,
    pub model: ModelData,
    pub metrics: AnalysisMetrics,
}

/// Measured figures for one level. Every field is a real measurement of the
/// mesh in the same struct — nothing here is estimated or carried over from the
/// source (invariant 5).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AnalysisMetrics {
    /// Average cache misses per triangle; lower is better (best case ~0.5).
    pub acmr: f32,
    /// Transformed vertices per vertex; 1.0 is ideal.
    pub atvr: f32,
    /// Shaded pixels / covered pixels; 1.0 means no overdraw.
    pub overdraw: f32,
    /// Fetched bytes / vertex buffer size; 1.0 means each byte is read once.
    pub overfetch: f32,
    /// The largest simplification error any submesh in this level reported, in
    /// the units the LOD settings asked for (a fraction of the mesh extent, or
    /// world units under the absolute-error flag). `0.0` for level 0.
    pub simplify_error: f32,
}

/// The size of a mesh as the GPU sees it: what is actually in the buffers.
///
/// Not the same thing as [`ModelStats::vertex_count`] for the *source*, which
/// carries the file's own DCC count (invariant 5) — a number import never
/// materialises, since it splits every face corner. The processed mesh can only
/// be compared against what the source really uploads, so a run measures that
/// too, the same way, and the overlay's deltas subtract like from like.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MeshCounts {
    pub triangles: usize,
    pub vertices: usize,
}

impl MeshCounts {
    fn of(model: &ModelData) -> Self {
        Self {
            triangles: model.indices.len() / 3,
            vertices: model.vertices.len(),
        }
    }
}

/// The full result of a run.
#[derive(Debug, Clone)]
pub struct ProcessedResult {
    /// Level 0 first, then each configured LOD level in order. Never empty on
    /// success.
    pub lods: Vec<ProcessedLod>,
    /// The input mesh's own buffer counts, the baseline every level's change is
    /// measured against.
    pub source: MeshCounts,
    /// The input mesh's own cache / overdraw / fetch figures, measured exactly as
    /// each level's are so the overlay can show what an operation did to them.
    ///
    /// Measured on every run even though the source never changes between them:
    /// caching it would mean carrying a keyed baseline across the thread boundary
    /// for a figure that costs one more pass over a mesh the run has already
    /// walked several times.
    pub source_metrics: AnalysisMetrics,
    /// Non-fatal problems worth telling the user about, already de-duplicated.
    pub warnings: Vec<String>,
    /// Wall-clock time the run took, for the "this is taking a while" notice and
    /// the stats overlay.
    pub elapsed: Duration,
}

impl ProcessedResult {
    /// The level the viewport should show, clamped so a stale selection from a
    /// longer chain can't index past the end.
    pub fn lod(&self, level: usize) -> Option<&ProcessedLod> {
        self.lods.get(level.min(self.lods.len().saturating_sub(1)))
    }
}

/// Run `stack` over `model`, producing one mesh per LOD level.
pub fn process(input: ProcessInput<'_>) -> Result<ProcessedResult, OptError> {
    let _z = crate::prof::zone!("Process Stack");
    let started = Instant::now();

    if !meshopt::available() {
        return Err(OptError::Unavailable);
    }
    if input.model.indices.is_empty() || input.model.vertices.is_empty() {
        return Err(OptError::EmptyMesh);
    }

    // Nothing enabled: no mesh is produced, but the source is still measured.
    // The workspace shows those figures as the baseline — a user reads the
    // mesh's overdraw and cache behaviour *before* deciding what to add, and
    // every later run's change is quoted against them.
    if !input.stack.ops.iter().any(|op| op.enabled) {
        return Ok(ProcessedResult {
            lods: Vec::new(),
            source: MeshCounts::of(input.model),
            source_metrics: measure(input.model, input.render_vertex_size, 0.0),
            warnings: Vec::new(),
            elapsed: started.elapsed(),
        });
    }

    let mut warnings = Warnings::default();
    if input.model.skin.is_some() {
        warnings.push(
            "Source mesh is skinned. Opt processes static geometry only — skinning \
             is dropped from the processed mesh and from any export.",
        );
    }

    let (mut submeshes, tags) = submesh::partition(input.model);
    if submeshes.is_empty() {
        return Err(OptError::EmptyMesh);
    }

    index_mesh(&mut submeshes, input.stack, &mut warnings);

    // Operations split around the LOD operation: those above it shape the mesh
    // every level starts from, those below it tidy up each generated level.
    let lod_position = input
        .stack
        .ops
        .iter()
        .position(|op| op.enabled && matches!(op.kind, OpKind::SimplifyLod(_)));
    let (pre_ops, lod_op, post_ops) = match lod_position {
        Some(position) => (
            &input.stack.ops[..position],
            match &input.stack.ops[position].kind {
                OpKind::SimplifyLod(params) => Some(params),
                _ => None,
            },
            &input.stack.ops[position + 1..],
        ),
        None => (&input.stack.ops[..], None, &input.stack.ops[0..0]),
    };

    for op in pre_ops {
        apply_op(&mut submeshes, op, input.stack, &mut warnings);
    }

    // Level 0 is the mesh as the pre-operations left it; every LOD level
    // re-simplifies from this same snapshot.
    let mut levels: Vec<LevelState> = vec![LevelState {
        submeshes: submeshes.clone(),
        simplify_error: 0.0,
    }];

    if let Some(params) = lod_op {
        for level in &params.levels {
            levels.push(build_lod_level(
                &submeshes,
                params,
                level,
                input.stack,
                &mut warnings,
            ));
        }
    }

    for level in &mut levels {
        for op in post_ops {
            apply_op(&mut level.submeshes, op, input.stack, &mut warnings);
        }
        for piece in &mut level.submeshes {
            piece.compact_unreferenced();
        }
    }

    let geometry_changed = input
        .stack
        .ops
        .iter()
        .any(|op| op.enabled && op.kind.alters_geometry());

    // A weld that ignores normals merges vertices whose normals differ, and the
    // survivor keeps one of them arbitrarily — it then describes one incident face
    // rather than the surface, and the mesh shades as noise. Simplification needs
    // no such fix-up: it only ever *removes* vertices, so the ones that survive
    // still carry their authored normals.
    let normals_invalidated = input.stack.ops.iter().any(|op| {
        op.enabled && matches!(&op.kind, OpKind::Weld(params) if !params.compare_normals)
    });

    let mut lods = Vec::with_capacity(levels.len());
    for (index, level) in levels.into_iter().enumerate() {
        let model = assemble(
            &level.submeshes,
            input.model,
            tags,
            index,
            Rebuild {
                normals: normals_invalidated,
                tangents: geometry_changed,
            },
        );
        // A level can legitimately collapse to nothing when the target ratio and
        // error budget are aggressive enough. That is a real result, not a bug —
        // but it renders as an empty viewport, so say why rather than let the
        // user wonder whether processing failed.
        if model.indices.is_empty() {
            warnings.push(&format!(
                "LOD {index} simplified away completely. Raise its target ratio, or \
                 lower its error limit so the simplifier stops sooner."
            ));
        }
        let metrics = measure(&model, input.render_vertex_size, level.simplify_error);
        lods.push(ProcessedLod {
            level: index,
            model,
            metrics,
        });
    }

    Ok(ProcessedResult {
        lods,
        source: MeshCounts::of(input.model),
        source_metrics: measure(input.model, input.render_vertex_size, 0.0),
        warnings: warnings.into_vec(),
        elapsed: started.elapsed(),
    })
}

/// Merge vertices that are identical in *every* attribute, before any of the
/// user's operations run.
///
/// FBX import splits each face corner into its own vertex, so the mesh arrives
/// with no shared vertices at all — a real asset measured here imports as 10006
/// triangles over 30018 vertices, one per corner, which makes every single edge
/// an attribute discontinuity. Every meshoptimizer operation works through the
/// index buffer, so on a mesh like that they are all no-ops: there is nothing for
/// the vertex cache to reuse, and no edge a topology-preserving collapse is
/// allowed to cross. Simplification in particular removes *nothing*, which reads
/// as the tool being broken.
///
/// Merging only vertices that match in position, normal, UV and color is
/// lossless — each survivor is byte-for-byte what it replaced, so the mesh looks
/// and shades exactly as it did — and it is the precondition meshoptimizer's own
/// pipeline assumes ("generate a vertex remap first"). On that same asset it
/// yields 30018 → 5284 vertices, after which a 50% LOD target is hit exactly.
///
/// It is deliberately not a stack operation: it changes nothing the user can see
/// and there is no case where skipping it is useful. The Weld operation remains
/// for the *lossy* merges — dropping normals or UVs from the comparison, or
/// merging within a tolerance — which do change the mesh.
fn index_mesh(submeshes: &mut [Submesh], stack: &OptStack, warnings: &mut Warnings) {
    let _z = crate::prof::zone!("Index Mesh");
    for piece in submeshes.iter_mut() {
        // An excluded object is left exactly as it was, down to its vertex order.
        if is_excluded(stack, piece.node) {
            continue;
        }
        if let Err(error) = ops::weld(piece, &crate::stack::WeldParams::default()) {
            warnings.push(&format!("Couldn't index the mesh for processing: {error}"));
        }
    }
}

/// One level under construction.
struct LevelState {
    submeshes: Vec<Submesh>,
    simplify_error: f32,
}

/// Simplify a fresh copy of the base submeshes down to one LOD level's target.
fn build_lod_level(
    base: &[Submesh],
    params: &LodParams,
    level: &crate::stack::LodLevel,
    stack: &OptStack,
    warnings: &mut Warnings,
) -> LevelState {
    let mut submeshes = base.to_vec();
    let mut worst_error = 0.0f32;

    let mut requested = 0usize;
    let mut produced = 0usize;

    for piece in &mut submeshes {
        if is_excluded(stack, piece.node) {
            continue;
        }
        // The ratio is applied per submesh, which is the only interpretation
        // consistent with processing them independently: a 50% target means each
        // object keeps half its triangles, not that the scene total halves while
        // one object vanishes.
        let ratio = level.target_ratio.clamp(0.0, 1.0);
        let target = ((piece.triangle_count() as f32) * ratio).round() as usize;
        let before = piece.triangle_count();

        match ops::simplify_level(piece, params, target, level.target_error.max(0.0)) {
            Ok(error) => worst_error = worst_error.max(error),
            Err(error) => warnings.push(&format!("Generate LODs: {error}")),
        }
        requested += before.saturating_sub(target);
        produced += before.saturating_sub(piece.triangle_count());
    }

    // A topology-preserving collapse cannot cross an attribute discontinuity, and
    // FBX import splits every face corner into its own vertex — so a mesh whose
    // normals or UVs differ at every corner (a scan with generated per-face
    // normals, say) presents *every* edge as a seam and the simplifier stalls.
    // That looks identical to the tool being broken, so name the two ways out
    // rather than let the user rediscover them.
    let stalled = requested > 0 && produced * 10 < requested;
    if stalled && params.algorithm != SimplifyAlgorithm::Sloppy && !params.flags.permissive {
        warnings.push(
            "The simplifier removed almost nothing: this mesh has an attribute seam \
             at nearly every edge, which a topology-preserving collapse cannot cross. \
             Add a Weld operation with 'Compare normals' off, or turn on 'Collapse \
             across seams' in the LOD options.",
        );
    }

    LevelState {
        submeshes,
        simplify_error: worst_error,
    }
}

/// Apply one stack operation across every submesh, honouring per-node
/// exclusions and parameter overrides.
fn apply_op(submeshes: &mut [Submesh], op: &OpInstance, stack: &OptStack, warnings: &mut Warnings) {
    if !op.enabled {
        return;
    }

    for piece in submeshes.iter_mut() {
        if is_excluded(stack, piece.node) {
            continue;
        }
        let kind = resolve_op(stack, op, piece.node);
        let outcome = match kind {
            OpKind::Weld(params) => ops::weld(piece, params),
            OpKind::FilterTriangles => ops::filter_triangles(piece),
            OpKind::PruneComponents { error } => ops::prune_components(piece, *error),
            OpKind::VertexCache => ops::optimize_vertex_cache(piece),
            OpKind::Overdraw { threshold } => ops::optimize_overdraw(piece, *threshold),
            OpKind::VertexFetch => ops::optimize_vertex_fetch(piece),
            // The LOD operation is the pipeline's fan-out point, handled by the
            // caller; it never reaches the per-submesh path.
            OpKind::SimplifyLod(_) => Ok(()),
        };
        if let Err(error) = outcome {
            warnings.push(&format!("{}: {error}", kind.label()));
        }
    }
}

/// Whether the user excluded `node` from processing entirely.
fn is_excluded(stack: &OptStack, node: u32) -> bool {
    stack
        .node_override(node as usize)
        .is_some_and(|entry| entry.exclude)
}

/// The settings to use for `op` on `node`: the node's override when it has one
/// for this operation, otherwise the global settings.
fn resolve_op<'a>(stack: &'a OptStack, op: &'a OpInstance, node: u32) -> &'a OpKind {
    stack
        .node_override(node as usize)
        .and_then(|entry| entry.ops.iter().find(|candidate| candidate.id == op.id))
        .map_or(&op.kind, |candidate| &candidate.kind)
}

/// Rebuild a [`ModelData`] from processed submeshes.
///
/// The output is a pure triangle mesh, so it carries **no** face topology:
/// `faces` and `triangles.to_face` are both left empty. That is not a gap — the
/// original polygon table describes contiguous corner runs in the *source*
/// vertex array, a layout welding and simplification necessarily destroy, and
/// every consumer in `render` already falls back to per-triangle behaviour when
/// the table is absent (the wireframe draws triangle edges, face normals get one
/// slot per triangle, UV islands fall back to the solid fill). Synthesizing a
/// plausible-looking table instead would draw a wireframe that is simply wrong.
/// Which derived per-vertex bases the assembled mesh has to rebuild. Both are
/// computed from the geometry, so recomputing one that is still valid would just
/// overwrite the source file's authored values with synthesized ones.
#[derive(Debug, Clone, Copy)]
struct Rebuild {
    normals: bool,
    tangents: bool,
}

fn assemble(
    submeshes: &[Submesh],
    source: &ModelData,
    tags: TagPresence,
    level: usize,
    rebuild: Rebuild,
) -> ModelData {
    let _z = crate::prof::zone!("Assemble Model");

    let total_vertices: usize = submeshes.iter().map(|piece| piece.vertices.len()).sum();
    let total_indices: usize = submeshes.iter().map(|piece| piece.indices.len()).sum();
    let channel_count = source.uv_channels.len();

    let mut vertices: Vec<Vertex> = Vec::with_capacity(total_vertices);
    let mut indices: Vec<u32> = Vec::with_capacity(total_indices);
    let mut uv_channels: Vec<Vec<glam::Vec2>> =
        vec![Vec::with_capacity(total_vertices); channel_count];
    let mut triangle_node: Vec<u32> = Vec::new();
    let mut triangle_material: Vec<u32> = Vec::new();

    for piece in submeshes {
        if piece.is_empty() {
            continue;
        }
        let base = vertices.len() as u32;
        vertices.extend_from_slice(&piece.vertices);
        indices.extend(piece.indices.iter().map(|&index| index + base));

        for (channel, destination) in uv_channels.iter_mut().enumerate() {
            match piece.uv_channels.get(channel) {
                Some(source_uvs) => destination.extend_from_slice(source_uvs),
                // A submesh built before this channel existed can't happen today,
                // but padding keeps the arrays parallel rather than silently short.
                None => destination.resize(vertices.len(), glam::Vec2::ZERO),
            }
        }

        let triangles = piece.triangle_count();
        if tags.node {
            triangle_node.extend(std::iter::repeat_n(piece.node, triangles));
        }
        if tags.material {
            triangle_material.extend(std::iter::repeat_n(piece.material, triangles));
        }
    }

    let mut model = ModelData {
        name: level_name(&source.name, level),
        vertices,
        indices,
        // Deliberately empty — see the doc comment above.
        faces: Vec::new(),
        triangles: TriangleData {
            to_face: Vec::new(),
            material: triangle_material,
            node: triangle_node,
        },
        nodes: source.nodes.clone(),
        uv_channels,
        uv_set_names: source.uv_set_names.clone(),
        bounds: None,
        stats: ModelStats::default(),
        materials: source.materials.clone(),
        // Static-mesh tool: skinning is dropped, with a warning raised by the
        // caller so the user knows rather than discovers it at export.
        skin: None,
    };

    // Normals first: tangents are orthonormalized against them, so rebuilding
    // tangents from stale normals would bake the staleness into both.
    if rebuild.normals {
        model.generate_normals();
    }
    // Tangents are derived from positions, UVs and normals, so any geometry
    // change invalidates them. Regenerating unconditionally would be wasted work
    // on a reorder-only stack, and would also overwrite the source file's
    // authored tangents with synthesized ones for no reason.
    if rebuild.tangents {
        model.generate_tangents();
    }

    model.recompute_bounds();
    model.stats = measured_stats(&model, source);
    model
}

/// `"Asset"` for level 0, `"Asset_LOD1"` and up for the rest — the same naming
/// the suffixed-siblings export uses, so what the viewport labels matches what
/// lands on disk.
fn level_name(source_name: &str, level: usize) -> String {
    if level == 0 {
        source_name.to_owned()
    } else {
        format!("{source_name}_LOD{level}")
    }
}

/// Stats measured off the processed mesh itself.
///
/// Two figures deliberately differ in meaning from their source counterparts:
/// `polygon_count` equals the triangle count (the output *is* triangulated, so
/// its polygons are its triangles), and `vertex_count` is the real length of the
/// vertex buffer rather than the source file's logical DCC count. Both are
/// honest measurements of the mesh in hand, which is what the overlay must show.
fn measured_stats(model: &ModelData, source: &ModelData) -> ModelStats {
    let triangle_count = model.indices.len() / 3;
    ModelStats {
        polygon_count: triangle_count,
        triangle_count,
        vertex_count: model.vertices.len(),
        uv_set_count: source.stats.uv_set_count,
        material_count: model.materials.len(),
        draw_count: model.material_draw_count(),
        // The node graph is carried through unchanged, so its bone count still
        // describes this model — even though the skin binding itself is dropped.
        bone_count: source.stats.bone_count,
        source_unit_meters: source.stats.source_unit_meters,
    }
}

/// Measure a finished level against the cache / overdraw / fetch models.
///
/// Submeshes are measured separately (they are what the GPU draws) and their raw
/// counters summed before the ratios are re-derived — averaging per-submesh
/// ratios would let a ten-triangle part outweigh a hundred-thousand-triangle one
/// and report a number the mesh never exhibits.
fn measure(model: &ModelData, render_vertex_size: usize, simplify_error: f32) -> AnalysisMetrics {
    let _z = crate::prof::zone!("Measure Level");

    let (submeshes, _) = submesh::partition(model);
    let mut counters = AnalysisCounters::default();

    for piece in &submeshes {
        if piece.is_empty() {
            continue;
        }
        let positions = piece.positions();
        if let Ok(measured) = meshopt::analyze(
            &piece.indices,
            &positions,
            piece.vertices.len(),
            render_vertex_size,
        ) {
            counters.accumulate(measured);
        }
    }

    AnalysisMetrics {
        acmr: counters.acmr(),
        atvr: counters.atvr(),
        overdraw: counters.overdraw(),
        overfetch: counters.overfetch(),
        simplify_error,
    }
}

#[cfg(all(test, has_meshopt))]
mod tests {
    use super::*;
    use crate::stack::{LodLevel, LodParams, OpKind, SimplifyAlgorithm, WeldParams};
    use review_model::demo_cube_model;

    /// A stand-in for the renderer's `SceneVertex` size in tests.
    const VERTEX_SIZE: usize = 48;

    fn run(stack: &OptStack) -> ProcessedResult {
        let model = demo_cube_model();
        process(ProcessInput {
            model: &model,
            stack,
            render_vertex_size: VERTEX_SIZE,
        })
        .expect("the demo cube always processes")
    }

    /// Nothing enabled produces no mesh — but the source is still measured, so
    /// the workspace can show what it costs before anything is asked of it.
    #[test]
    fn an_empty_stack_measures_the_source_and_produces_no_levels() {
        let result = run(&OptStack::default());

        assert!(result.lods.is_empty(), "nothing was asked for");
        assert_eq!(result.source.triangles, 12);
        assert_eq!(result.source.vertices, 24);
        assert!(
            result.source_metrics.acmr > 0.0,
            "the source is measured anyway: {:?}",
            result.source_metrics
        );
    }

    #[test]
    fn an_operation_produces_a_mesh_carrying_no_polygon_topology() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::FilterTriangles);

        let output = &run(&stack).lods[0].model;
        assert_eq!(output.indices.len(), 36);
        assert!(
            output.faces.is_empty() && output.triangles.to_face.is_empty(),
            "processed meshes carry no polygon topology"
        );
    }

    #[test]
    fn weld_ignoring_split_attributes_collapses_the_cube_to_eight_corners() {
        // The demo cube is flat-shaded with per-face UVs, so all 24 corners are
        // genuinely distinct until normals and UVs are excluded from the test.
        let mut stack = OptStack::default();
        stack.push_op(OpKind::Weld(WeldParams {
            attribute_tolerance: 0.0,
            compare_normals: false,
            compare_uvs: false,
            compare_colors: false,
        }));

        let result = run(&stack);
        let output = &result.lods[0].model;
        assert_eq!(output.vertices.len(), 8, "a cube has eight corners");
        assert_eq!(output.indices.len(), 36, "welding removes no triangles");
        assert_eq!(output.stats.vertex_count, 8);
        assert_eq!(output.stats.triangle_count, 12);
    }

    #[test]
    fn weld_comparing_split_attributes_merges_nothing_on_a_flat_shaded_cube() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::Weld(WeldParams::default()));

        let result = run(&stack);
        assert_eq!(
            result.lods[0].model.vertices.len(),
            24,
            "every corner has a distinct normal and UV"
        );
    }

    #[test]
    fn tolerance_weld_reaches_the_same_eight_corners() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::Weld(WeldParams {
            attribute_tolerance: 1.0e-4,
            compare_normals: false,
            compare_uvs: false,
            compare_colors: false,
        }));

        assert_eq!(run(&stack).lods[0].model.vertices.len(), 8);
    }

    #[test]
    fn filter_triangles_leaves_a_clean_mesh_alone() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::FilterTriangles);

        assert_eq!(run(&stack).lods[0].model.indices.len(), 36);
    }

    #[test]
    fn filter_triangles_removes_an_injected_duplicate_and_degenerate() {
        let mut model = demo_cube_model();
        let [a, b, c] = [model.indices[0], model.indices[1], model.indices[2]];
        // A duplicate of triangle 0 (same winding) and a degenerate sliver.
        model.indices.extend_from_slice(&[a, b, c, a, a, b]);
        model.triangles.to_face.extend_from_slice(&[0, 0]);
        model.triangles.material.extend_from_slice(&[0, 0]);
        model.triangles.node.extend_from_slice(&[0, 0]);

        let mut stack = OptStack::default();
        stack.push_op(OpKind::FilterTriangles);
        let result = process(ProcessInput {
            model: &model,
            stack: &stack,
            render_vertex_size: VERTEX_SIZE,
        })
        .expect("processing succeeds");

        assert_eq!(
            result.lods[0].model.indices.len(),
            36,
            "both added triangles are redundant"
        );
    }

    #[test]
    fn reorder_operations_preserve_the_triangle_set() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::VertexCache);
        stack.push_op(OpKind::Overdraw { threshold: 1.05 });
        stack.push_op(OpKind::VertexFetch);

        let result = run(&stack);
        let output = &result.lods[0].model;
        assert_eq!(output.indices.len(), 36, "reordering removes no triangles");
        assert_eq!(output.vertices.len(), 24, "and drops no referenced vertex");
    }

    #[test]
    fn a_lod_chain_produces_one_model_per_level_with_shrinking_triangle_counts() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::Weld(WeldParams {
            attribute_tolerance: 0.0,
            compare_normals: false,
            compare_uvs: false,
            compare_colors: false,
        }));
        stack.push_op(OpKind::SimplifyLod(LodParams {
            algorithm: SimplifyAlgorithm::Sloppy,
            levels: vec![
                LodLevel {
                    target_ratio: 0.5,
                    target_error: 1.0,
                },
                LodLevel {
                    target_ratio: 0.25,
                    target_error: 1.0,
                },
            ],
            ..LodParams::default()
        }));

        let result = run(&stack);
        assert_eq!(result.lods.len(), 3, "base plus two levels");

        let counts: Vec<usize> = result
            .lods
            .iter()
            .map(|lod| lod.model.stats.triangle_count)
            .collect();
        assert_eq!(counts[0], 12);
        assert!(
            counts.windows(2).all(|pair| pair[1] <= pair[0]),
            "triangle counts never grow along the chain: {counts:?}"
        );
    }

    #[test]
    fn every_level_is_named_for_its_lod() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::SimplifyLod(LodParams {
            algorithm: SimplifyAlgorithm::Sloppy,
            levels: vec![LodLevel {
                target_ratio: 0.5,
                target_error: 1.0,
            }],
            ..LodParams::default()
        }));

        let result = run(&stack);
        assert_eq!(result.lods[0].model.name, "Demo Cube");
        assert_eq!(result.lods[1].model.name, "Demo Cube_LOD1");
    }

    #[test]
    fn an_excluded_node_keeps_its_full_detail() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::Weld(WeldParams {
            attribute_tolerance: 0.0,
            compare_normals: false,
            compare_uvs: false,
            compare_colors: false,
        }));
        stack.node_override_mut(0).exclude = true;

        assert_eq!(
            run(&stack).lods[0].model.vertices.len(),
            24,
            "the only node is excluded, so nothing is welded"
        );
    }

    #[test]
    fn a_disabled_operation_does_nothing() {
        let mut stack = OptStack::default();
        let id = stack.push_op(OpKind::Weld(WeldParams {
            attribute_tolerance: 0.0,
            compare_normals: false,
            compare_uvs: false,
            compare_colors: false,
        }));
        stack.op_mut(id).expect("just pushed").enabled = false;

        assert!(
            run(&stack).lods.is_empty(),
            "a stack with nothing enabled is an empty stack"
        );
    }

    #[test]
    fn processed_models_stay_internally_consistent() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::Weld(WeldParams::default()));
        // A tight error budget on the topology-preserving simplifier: a cube has
        // nothing it can collapse without visible deformation, so every level
        // keeps real geometry and the consistency checks have something to check.
        stack.push_op(OpKind::SimplifyLod(LodParams {
            algorithm: SimplifyAlgorithm::Standard,
            levels: vec![LodLevel {
                target_ratio: 0.5,
                target_error: 0.01,
            }],
            ..LodParams::default()
        }));
        stack.push_op(OpKind::VertexFetch);

        for lod in run(&stack).lods {
            let model = &lod.model;
            let triangle_count = model.indices.len() / 3;
            model
                .triangles
                .validate(
                    triangle_count,
                    model.faces.len(),
                    model.materials.len(),
                    model.nodes.len(),
                )
                .expect("per-triangle arrays stay in lockstep");
            assert!(
                model
                    .indices
                    .iter()
                    .all(|&i| (i as usize) < model.vertices.len()),
                "every index addresses a real vertex"
            );
            assert!(!model.vertices.is_empty(), "the level keeps geometry");
            assert!(model.bounds.is_some(), "bounds are recomputed");
        }
    }

    #[test]
    fn a_level_that_simplifies_away_completely_says_so() {
        let mut stack = OptStack::default();
        // Sloppy simplification with a 100%-of-extent error budget and a hard
        // triangle target is free to collapse the cube to nothing.
        stack.push_op(OpKind::SimplifyLod(LodParams {
            algorithm: SimplifyAlgorithm::Sloppy,
            levels: vec![LodLevel {
                target_ratio: 0.05,
                target_error: 1.0,
            }],
            ..LodParams::default()
        }));

        let result = run(&stack);
        let last = result.lods.last().expect("the chain has levels");
        if last.model.indices.is_empty() {
            assert!(
                result
                    .warnings
                    .iter()
                    .any(|warning| warning.contains("simplified away")),
                "an empty level is explained: {:?}",
                result.warnings
            );
        }
        assert_eq!(
            result.lods[0].model.indices.len(),
            36,
            "level 0 is never simplified"
        );
    }

    #[test]
    fn multi_channel_uvs_stay_parallel_through_a_weld() {
        let mut model = demo_cube_model();
        let count = model.vertices.len();
        model.uv_channels = vec![
            model.vertices.iter().map(|vertex| vertex.uv).collect(),
            (0..count)
                .map(|index| glam::Vec2::splat(index as f32 * 0.01))
                .collect(),
        ];
        model.uv_set_names = vec!["UVMap".to_owned(), "Lightmap".to_owned()];
        model.stats.uv_set_count = 2;

        let mut stack = OptStack::default();
        stack.push_op(OpKind::Weld(WeldParams {
            attribute_tolerance: 0.0,
            compare_normals: false,
            compare_uvs: false,
            compare_colors: false,
        }));

        let result = process(ProcessInput {
            model: &model,
            stack: &stack,
            render_vertex_size: VERTEX_SIZE,
        })
        .expect("processing succeeds");

        let output = &result.lods[0].model;
        assert_eq!(output.uv_channels.len(), 2);
        for channel in &output.uv_channels {
            assert_eq!(
                channel.len(),
                output.vertices.len(),
                "each UV channel stays parallel to the vertex array"
            );
        }
    }

    #[test]
    fn a_skinned_source_is_reported_and_stripped() {
        let mut model = demo_cube_model();
        model.skin = Some(review_model::SkinData::default());
        let mut stack = OptStack::default();
        stack.push_op(OpKind::FilterTriangles);

        let result = process(ProcessInput {
            model: &model,
            stack: &stack,
            render_vertex_size: VERTEX_SIZE,
        })
        .expect("processing succeeds");

        assert!(result.lods[0].model.skin.is_none());
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("skinned")),
            "the user is told: {:?}",
            result.warnings
        );
    }

    #[test]
    fn metrics_are_measured_for_every_level() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::VertexCache);

        for lod in run(&stack).lods {
            assert!(lod.metrics.acmr > 0.0, "ACMR is measured");
            assert!(lod.metrics.atvr > 0.0, "ATVR is measured");
            assert!(
                lod.metrics.overfetch >= 1.0,
                "overfetch is at best 1.0, got {}",
                lod.metrics.overfetch
            );
        }
    }

    #[test]
    fn vertex_cache_optimization_does_not_worsen_acmr() {
        // The source's own figure, which is what the overlay quotes the change
        // against — measured by the same run that produces the optimized one.
        let mut stack = OptStack::default();
        stack.push_op(OpKind::VertexCache);
        let result = run(&stack);
        let plain = result.source_metrics.acmr;
        let optimized = result.lods[0].metrics.acmr;

        assert!(
            optimized <= plain + f32::EPSILON,
            "ACMR should not regress: {plain} -> {optimized}"
        );
    }
}
