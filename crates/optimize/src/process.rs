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
//! Only [`OpKind::SimplifyLod`] fans out. [`OpKind::Reduce`] runs the same
//! simplifier as an ordinary step of 2 or 4, so what it leaves behind *is* the
//! mesh — the one every later operation works on, every level starts from, and
//! the export writes in the source mesh's place.
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
use crate::stack::{
    LodLevel, LodParams, OpInstance, OpKind, OptStack, SimplifyAlgorithm, SimplifySettings,
};
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
    /// Outliner-hidden mesh nodes (sorted node indices). Only the AO bake
    /// consults these: a hidden object neither occludes nor receives the bake.
    /// Game assets routinely carry their whole LOD chain and a collision shell
    /// as co-located sibling nodes, and baking against those invisible,
    /// near-coincident surfaces shreds the result — what you see occluding is
    /// what occludes. Every other operation still processes hidden geometry
    /// (it is exported either way).
    pub hidden_nodes: &'a [u32],
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
    /// the units the simplifier was asked for (a fraction of the mesh extent, or
    /// world units under the absolute-error flag). `0.0` for level 0 unless the
    /// stack reduced the mesh in place, whose error every level inherits.
    pub simplify_error: f32,
}

/// The size of a mesh as the GPU sees it: what an engine's vertex/index buffers
/// would actually hold.
///
/// Not the same thing as [`ModelStats::vertex_count`] for the *source*, which
/// carries the file's own DCC count (invariant 5), and not the corner-split
/// buffer either — see the field docs on [`ProcessedResult::source`]. A run
/// measures the source and each level the same way, so the overlay's deltas
/// subtract like from like.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MeshCounts {
    pub triangles: usize,
    pub vertices: usize,
}

impl MeshCounts {
    fn of_submeshes(submeshes: &[Submesh]) -> Self {
        Self {
            triangles: submeshes.iter().map(Submesh::triangle_count).sum(),
            vertices: submeshes.iter().map(|piece| piece.vertices.len()).sum(),
        }
    }
}

/// The full result of a run.
#[derive(Debug, Clone)]
pub struct ProcessedResult {
    /// Level 0 first, then each configured LOD level in order. Never empty on
    /// success.
    pub lods: Vec<ProcessedLod>,
    /// The input mesh's counts **after the lossless index pass** — the baseline
    /// every level's change is measured against.
    ///
    /// Deliberately not the raw corner-split buffer import hands over: every
    /// engine importer performs the same lossless indexing, so quoting changes
    /// against the un-indexed mesh would credit the user's operations with an
    /// "-82%" any cooker gets for free, and describe a cost the asset never has.
    pub source: MeshCounts,
    /// The indexed input mesh's own cache / overdraw / fetch figures, measured
    /// exactly as each level's are so the overlay can show what an operation did
    /// to them. Same baseline rule as [`Self::source`]: the raw corner-split
    /// buffer always measures ACMR 3.0 (no vertex is ever shared), which is a
    /// property of the import path, not of the asset.
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

    // The baseline is the mesh as it stands *now* — partitioned and losslessly
    // indexed, before any of the user's operations touch it (they mutate the
    // submeshes in place below). See the field docs on [`ProcessedResult`] for
    // why the raw corner-split buffer would be the wrong thing to quote against.
    let source = MeshCounts::of_submeshes(&submeshes);
    let source_metrics =
        measure_submeshes(&submeshes, input.render_vertex_size, 0.0, &mut warnings);

    // Nothing enabled: no mesh is produced, but the baseline above still comes
    // back — a user reads the mesh's overdraw and cache behaviour *before*
    // deciding what to add, and every later run's change is quoted against it.
    if !input.stack.ops.iter().any(|op| op.enabled) {
        return Ok(ProcessedResult {
            lods: Vec::new(),
            source,
            source_metrics,
            warnings: warnings.into_vec(),
            elapsed: started.elapsed(),
        });
    }

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

    // A Reduce among the pre-operations simplifies the mesh every level then
    // starts from, so its error is part of level 0's — and of every level below
    // it, which inherits the reduced mesh.
    let mut base_error = 0.0f32;
    for op in pre_ops {
        base_error = base_error.max(apply_op(
            &mut submeshes,
            op,
            input.stack,
            input.model,
            input.hidden_nodes,
            &mut warnings,
        ));
    }

    // Level 0 is the mesh as the pre-operations left it; every LOD level
    // re-simplifies from this same snapshot.
    let mut levels: Vec<LevelState> = vec![LevelState {
        submeshes: submeshes.clone(),
        simplify_error: base_error,
    }];

    if let Some(params) = lod_op {
        for level in &params.levels {
            let mut state = build_lod_level(&submeshes, params, level, input.stack, &mut warnings);
            state.simplify_error = state.simplify_error.max(base_error);
            levels.push(state);
        }
    }

    for level in &mut levels {
        for op in post_ops {
            let error = apply_op(
                &mut level.submeshes,
                op,
                input.stack,
                input.model,
                input.hidden_nodes,
                &mut warnings,
            );
            level.simplify_error = level.simplify_error.max(error);
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

    // A bake above a simplifier isn't wrong — surviving vertices keep their
    // colors — but the AO then describes the pre-simplified geometry, which is
    // rarely what the user meant. Advise rather than reorder.
    let first_bake = input
        .stack
        .ops
        .iter()
        .position(|op| op.enabled && matches!(op.kind, OpKind::BakeAo(_)));
    let last_simplify = input.stack.ops.iter().rposition(|op| {
        op.enabled && matches!(op.kind, OpKind::Reduce(_) | OpKind::SimplifyLod(_))
    });
    if let (Some(bake), Some(simplify)) = (first_bake, last_simplify)
        && bake < simplify
    {
        warnings.push(
            "Bake AO runs before a simplifier, so the baked occlusion describes \
             the pre-simplified geometry. Move the bake below the simplifier — or \
             below Generate LODs to bake every level.",
        );
    }

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
        let metrics = measure(
            &model,
            input.render_vertex_size,
            level.simplify_error,
            &mut warnings,
        );
        lods.push(ProcessedLod {
            level: index,
            model,
            metrics,
        });
    }

    Ok(ProcessedResult {
        lods,
        source,
        source_metrics,
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
    let simplify_error =
        simplify_submeshes(&mut submeshes, stack, "Generate LODs", warnings, |_| {
            (params.simplify, *level)
        });

    LevelState {
        submeshes,
        simplify_error,
    }
}

/// Simplify every non-excluded submesh toward the settings and target
/// `settings_for` resolves for it, returning the worst error any of them
/// reported.
///
/// Shared by the LOD fan-out and the in-place Reduce operation: the two differ
/// only in what their caller does with the submeshes afterwards, so the ratio
/// interpretation and the stall diagnosis below stay one implementation. The
/// resolver is what lets Reduce honour per-node parameter overrides, which the
/// LOD chain has no equivalent of.
fn simplify_submeshes(
    submeshes: &mut [Submesh],
    stack: &OptStack,
    label: &str,
    warnings: &mut Warnings,
    mut settings_for: impl FnMut(&Submesh) -> (SimplifySettings, LodLevel),
) -> f32 {
    let mut worst_error = 0.0f32;
    let mut requested = 0usize;
    let mut produced = 0usize;
    // Whether any piece was simplified by a topology-preserving collapse, which
    // is the only case the seam diagnosis below applies to.
    let mut seam_bound = false;

    for piece in submeshes.iter_mut() {
        if is_excluded(stack, piece.node) {
            continue;
        }
        let (settings, target) = settings_for(piece);
        seam_bound |= settings.algorithm != SimplifyAlgorithm::Sloppy && !settings.flags.permissive;

        // The ratio is applied per submesh, which is the only interpretation
        // consistent with processing them independently: a 50% target means each
        // object keeps half its triangles, not that the scene total halves while
        // one object vanishes.
        let ratio = target.target_ratio.clamp(0.0, 1.0);
        let triangles = ((piece.triangle_count() as f32) * ratio).round() as usize;
        let before = piece.triangle_count();

        match ops::simplify(piece, &settings, triangles, target.target_error.max(0.0)) {
            Ok(error) => worst_error = worst_error.max(error),
            Err(error) => warnings.push(&format!("{label}: {error}")),
        }
        requested += before.saturating_sub(triangles);
        produced += before.saturating_sub(piece.triangle_count());
    }

    // A topology-preserving collapse cannot cross an attribute discontinuity, and
    // FBX import splits every face corner into its own vertex — so a mesh whose
    // normals or UVs differ at every corner (a scan with generated per-face
    // normals, say) presents *every* edge as a seam and the simplifier stalls.
    // That looks identical to the tool being broken, so name the two ways out
    // rather than let the user rediscover them.
    let stalled = requested > 0 && produced * 10 < requested;
    if stalled && seam_bound {
        warnings.push(
            "The simplifier removed almost nothing: this mesh has an attribute seam \
             at nearly every edge, which a topology-preserving collapse cannot cross. \
             Add a Weld operation with 'Compare normals' off, or turn on 'Collapse \
             across seams' in the simplifier options.",
        );
    }

    worst_error
}

/// Apply one stack operation across every submesh, honouring per-node
/// exclusions and parameter overrides. Returns the worst simplification error it
/// caused, which is non-zero only for [`OpKind::Reduce`].
fn apply_op(
    submeshes: &mut [Submesh],
    op: &OpInstance,
    stack: &OptStack,
    model: &ModelData,
    hidden_nodes: &[u32],
    warnings: &mut Warnings,
) -> f32 {
    if !op.enabled {
        return 0.0;
    }

    // Reduce runs the simplifier, whose stall diagnosis reads the whole mesh
    // rather than one submesh at a time — so it goes through the same helper the
    // LOD fan-out uses instead of the per-piece loop below. Unlike the fan-out it
    // rewrites the submeshes in place, which is the whole difference between the
    // two: what it leaves behind is what every later operation sees, what each LOD
    // level starts from, and what the export writes in the source mesh's place.
    if let OpKind::Reduce(params) = &op.kind {
        return simplify_submeshes(submeshes, stack, op.kind.label(), warnings, |piece| {
            let params = match resolve_op(stack, op, piece.node) {
                OpKind::Reduce(resolved) => resolved,
                // An override can only ever hold the same kind as the operation
                // it overrides; fall back to the global settings if one somehow
                // doesn't.
                _ => params,
            };
            (params.simplify, params.target)
        });
    }

    // The AO bake needs the whole scene as occluders — excluded pieces still
    // occlude, they just aren't written — so like Reduce it takes every submesh
    // at once rather than the per-piece loop below. Outliner-hidden nodes are
    // out of the scene entirely (see [`ProcessInput::hidden_nodes`]), and the
    // scene partitions by `_LOD<n>` name suffix so co-located LOD copies never
    // shadow each other; `model` supplies the node names for that.
    if matches!(op.kind, OpKind::BakeAo(_)) {
        crate::ao::bake_submeshes(submeshes, op, stack, model, hidden_nodes);
        return 0.0;
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
            // caller; Reduce and the AO bake are handled above. None reaches
            // this path.
            OpKind::Reduce(_) | OpKind::SimplifyLod(_) | OpKind::BakeAo(_) => Ok(()),
        };
        if let Err(error) = outcome {
            warnings.push(&format!("{}: {error}", kind.label()));
        }
    }
    0.0
}

/// Whether the user excluded `node` from processing entirely.
pub(crate) fn is_excluded(stack: &OptStack, node: u32) -> bool {
    stack
        .node_override(node as usize)
        .is_some_and(|entry| entry.exclude)
}

/// The settings to use for `op` on `node`: the node's override when it has one
/// for this operation, otherwise the global settings.
pub(crate) fn resolve_op<'a>(stack: &'a OptStack, op: &'a OpInstance, node: u32) -> &'a OpKind {
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
        gpu_vertex_count: model.count_gpu_vertices(),
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
fn measure(
    model: &ModelData,
    render_vertex_size: usize,
    simplify_error: f32,
    warnings: &mut Warnings,
) -> AnalysisMetrics {
    let (submeshes, _) = submesh::partition(model);
    measure_submeshes(&submeshes, render_vertex_size, simplify_error, warnings)
}

/// [`measure`] over submeshes already in hand — the shape the pipeline holds
/// mid-run, so the baseline can be measured without assembling a `ModelData`.
///
/// A submesh that cannot be analyzed is left out of the totals and reported: the
/// figures would otherwise describe part of the mesh while being presented as
/// the whole of it (invariant 5).
fn measure_submeshes(
    submeshes: &[Submesh],
    render_vertex_size: usize,
    simplify_error: f32,
    warnings: &mut Warnings,
) -> AnalysisMetrics {
    let _z = crate::prof::zone!("Measure Level");

    let mut counters = AnalysisCounters::default();

    for piece in submeshes {
        if piece.is_empty() {
            continue;
        }
        let positions = piece.positions();
        match meshopt::analyze(
            &piece.indices,
            &positions,
            piece.vertices.len(),
            render_vertex_size,
        ) {
            Ok(measured) => counters.accumulate(measured),
            Err(error) => warnings.push(&format!(
                "Couldn't measure part of the mesh: {error}. The cache, overdraw \
                 and fetch figures cover only the parts that measured."
            )),
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
    use crate::stack::{
        AoTarget, BakeAoParams, LodLevel, LodParams, OpInstance, OpKind, ReduceParams,
        SimplifyAlgorithm, SimplifySettings, WeldParams,
    };
    use glam::{Vec3, Vec4};
    use review_model::{TriangleData, Vertex, demo_cube_model};

    /// A stand-in for the renderer's `SceneVertex` size in tests.
    const VERTEX_SIZE: usize = 48;

    fn run(stack: &OptStack) -> ProcessedResult {
        let model = demo_cube_model();
        process(ProcessInput {
            model: &model,
            stack,
            render_vertex_size: VERTEX_SIZE,
            hidden_nodes: &[],
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
        let result = run_model(&model, &stack);

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
            simplify: SimplifySettings {
                algorithm: SimplifyAlgorithm::Sloppy,
                ..SimplifySettings::default()
            },
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
            simplify: SimplifySettings {
                algorithm: SimplifyAlgorithm::Sloppy,
                ..SimplifySettings::default()
            },
            levels: vec![LodLevel {
                target_ratio: 0.5,
                target_error: 1.0,
            }],
        }));

        let result = run(&stack);
        assert_eq!(result.lods[0].model.name, "Demo Cube");
        assert_eq!(result.lods[1].model.name, "Demo Cube_LOD1");
    }

    /// A Reduce with no LOD operation leaves exactly one mesh — the source one,
    /// simplified. That single-mesh output is the whole point of the operation:
    /// it stands in for the source asset rather than adding levels beside it.
    #[test]
    fn reduce_replaces_the_base_mesh_instead_of_adding_a_level() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::Weld(WeldParams {
            attribute_tolerance: 0.0,
            compare_normals: false,
            compare_uvs: false,
            compare_colors: false,
        }));
        stack.push_op(OpKind::Reduce(ReduceParams {
            simplify: SimplifySettings {
                algorithm: SimplifyAlgorithm::Sloppy,
                ..SimplifySettings::default()
            },
            target: LodLevel {
                target_ratio: 0.5,
                target_error: 1.0,
            },
        }));

        let result = run(&stack);
        assert_eq!(result.lods.len(), 1, "no chain, just the reduced mesh");
        assert_eq!(result.lods[0].level, 0);
        assert_eq!(
            result.lods[0].model.name, "Demo Cube",
            "the reduced mesh keeps the source name, carrying no LOD suffix"
        );
        assert!(
            result.lods[0].model.indices.len() / 3 < 12,
            "the cube was actually simplified: {} triangles",
            result.lods[0].model.indices.len() / 3
        );
        assert!(
            result.lods[0].metrics.simplify_error > 0.0,
            "level 0 reports the error the reduce cost: {:?}",
            result.lods[0].metrics
        );
    }

    /// A Reduce above the LOD operation reshapes the mesh the whole chain is
    /// built from — level 0 included, which is what separates it from a level.
    #[test]
    fn reduce_above_the_lod_operation_feeds_every_level() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::Weld(WeldParams {
            attribute_tolerance: 0.0,
            compare_normals: false,
            compare_uvs: false,
            compare_colors: false,
        }));
        stack.push_op(OpKind::Reduce(ReduceParams {
            simplify: SimplifySettings {
                algorithm: SimplifyAlgorithm::Sloppy,
                ..SimplifySettings::default()
            },
            target: LodLevel {
                target_ratio: 0.5,
                target_error: 1.0,
            },
        }));
        stack.push_op(OpKind::SimplifyLod(LodParams {
            levels: vec![LodLevel {
                target_ratio: 0.5,
                target_error: 0.01,
            }],
            ..LodParams::default()
        }));

        let result = run(&stack);
        assert_eq!(result.lods.len(), 2, "base plus one level");
        let base_triangles = result.lods[0].model.indices.len() / 3;
        assert!(
            base_triangles < 12,
            "level 0 is the reduced mesh, not the source: {base_triangles} triangles"
        );
        assert!(
            result.lods[1].model.indices.len() / 3 <= base_triangles,
            "the level starts from the reduced mesh"
        );
    }

    /// The per-object overrides that apply to every other operation apply to a
    /// Reduce's target too, so one object can be reduced harder than the rest.
    #[test]
    fn a_node_override_retargets_a_reduce() {
        let untouched = ReduceParams {
            simplify: SimplifySettings {
                algorithm: SimplifyAlgorithm::Sloppy,
                ..SimplifySettings::default()
            },
            target: LodLevel {
                target_ratio: 1.0,
                target_error: 0.0,
            },
        };
        let mut stack = OptStack::default();
        stack.push_op(OpKind::Weld(WeldParams {
            attribute_tolerance: 0.0,
            compare_normals: false,
            compare_uvs: false,
            compare_colors: false,
        }));
        let id = stack.push_op(OpKind::Reduce(untouched));
        assert_eq!(
            run(&stack).lods[0].model.indices.len() / 3,
            12,
            "a full-ratio reduce is the control: it removes nothing"
        );

        stack.node_override_mut(0).ops.push(OpInstance {
            id,
            enabled: true,
            kind: OpKind::Reduce(ReduceParams {
                target: LodLevel {
                    target_ratio: 0.5,
                    target_error: 1.0,
                },
                ..untouched
            }),
        });

        assert!(
            run(&stack).lods[0].model.indices.len() / 3 < 12,
            "the only node's override is what reduced it"
        );
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

    fn run_model(model: &ModelData, stack: &OptStack) -> ProcessedResult {
        run_model_hiding(model, stack, &[])
    }

    fn run_model_hiding(model: &ModelData, stack: &OptStack, hidden: &[u32]) -> ProcessedResult {
        process(ProcessInput {
            model,
            stack,
            render_vertex_size: VERTEX_SIZE,
            hidden_nodes: hidden,
        })
        .expect("processing succeeds")
    }

    /// A model of named, upward-facing horizontal quads for the AO tests — one
    /// scene node per `(name, half-extent, height)` entry. Every vertex carries
    /// a distinctive source color so the tests can tell "written as fully open"
    /// apart from "never written".
    fn named_quads_model(quads: &[(&str, f32, f32)]) -> ModelData {
        let placed: Vec<(&str, f32, Vec3)> = quads
            .iter()
            .map(|&(name, half, y)| (name, half, Vec3::new(0.0, y, 0.0)))
            .collect();
        placed_quads_model(&placed)
    }

    /// The general form of [`named_quads_model`]: each quad centered anywhere,
    /// not only on the y-axis.
    fn placed_quads_model(quads: &[(&str, f32, Vec3)]) -> ModelData {
        use glam::Mat4;
        use review_model::{NodeKind, SceneNode};

        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut node_tags = Vec::new();
        let mut nodes = Vec::new();
        for (index, &(name, half, center)) in quads.iter().enumerate() {
            let base = vertices.len() as u32;
            for position in [
                center + Vec3::new(-half, 0.0, -half),
                center + Vec3::new(half, 0.0, -half),
                center + Vec3::new(half, 0.0, half),
                center + Vec3::new(-half, 0.0, half),
            ] {
                vertices.push(Vertex {
                    position,
                    normal: Vec3::Y,
                    vertex_color: Vec4::new(0.2, 0.4, 0.6, 0.8),
                    ..Vertex::default()
                });
            }
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            node_tags.extend_from_slice(&[index as u32, index as u32]);
            nodes.push(SceneNode {
                name: name.to_owned(),
                parent: None,
                mesh_part: Some(index),
                source_vertex_count: 0,
                transform: Mat4::IDENTITY,
                kind: NodeKind::Mesh,
                bone: None,
            });
        }
        ModelData {
            vertices,
            indices,
            triangles: TriangleData {
                node: node_tags,
                ..TriangleData::default()
            },
            nodes,
            ..ModelData::default()
        }
    }

    /// Two nodes with no LOD identity: node 0 is a 1×1 quad at y = 0 facing
    /// +Y, node 1 a 2×2 cover hovering at y = 0.5 above it. The geometry is
    /// sized so node 0's vertices end up *partially* occluded (strictly
    /// between 0 and 1), which is what the target/encode/intensity comparisons
    /// need.
    fn covered_quad_model() -> ModelData {
        named_quads_model(&[("lower", 0.5, 0.0), ("cover", 1.0, 0.5)])
    }

    /// The processed vertices of the lower (covered) quad / the upper cover,
    /// identified by height rather than order so the tests survive reordering.
    fn split_by_height(model: &ModelData) -> (Vec<Vertex>, Vec<Vertex>) {
        model
            .vertices
            .iter()
            .partition(|vertex| vertex.position.y < 0.25)
    }

    /// A convex mesh occludes none of its own hemispheres: every vertex must
    /// come out fully open — any value below 1 would be a self-intersection
    /// artifact the ray-origin bias exists to prevent.
    #[test]
    fn bake_ao_leaves_a_convex_mesh_fully_open() {
        let mut baseline = OptStack::default();
        baseline.push_op(OpKind::FilterTriangles);
        let mut baked = OptStack::default();
        baked.push_op(OpKind::FilterTriangles);
        baked.push_op(OpKind::BakeAo(BakeAoParams::default()));

        let baseline = run(&baseline);
        let baked = run(&baked);
        for (before, after) in baseline.lods[0]
            .model
            .vertices
            .iter()
            .zip(&baked.lods[0].model.vertices)
        {
            assert!(
                after.vertex_color.w > 0.99,
                "a convex surface is fully open, got {}",
                after.vertex_color.w
            );
            assert_eq!(
                after.vertex_color.truncate(),
                before.vertex_color.truncate(),
                "the alpha target leaves RGB untouched"
            );
        }
    }

    #[test]
    fn bake_ao_darkens_a_covered_surface() {
        let model = covered_quad_model();
        let mut stack = OptStack::default();
        stack.push_op(OpKind::BakeAo(BakeAoParams::default()));

        let result = run_model(&model, &stack);
        let (covered, cover) = split_by_height(&result.lods[0].model);
        assert!(!covered.is_empty() && !cover.is_empty());
        for vertex in &covered {
            let ao = vertex.vertex_color.w;
            assert!(
                ao < 0.5 && ao > 0.0,
                "a covered vertex is mostly but not fully occluded, got {ao}"
            );
        }
        for vertex in &cover {
            assert!(
                vertex.vertex_color.w > 0.99,
                "nothing hangs over the cover, got {}",
                vertex.vertex_color.w
            );
        }
    }

    #[test]
    fn bake_ao_max_distance_releases_distant_occluders() {
        let model = covered_quad_model();
        let mut stack = OptStack::default();
        stack.push_op(OpKind::BakeAo(BakeAoParams {
            // The cover hangs 0.5 above; a shorter reach never finds it.
            max_distance: 0.4,
            ..BakeAoParams::default()
        }));

        let (covered, _) = split_by_height(&run_model(&model, &stack).lods[0].model);
        for vertex in &covered {
            assert!(
                vertex.vertex_color.w > 0.99,
                "the cover is out of reach, got {}",
                vertex.vertex_color.w
            );
        }
    }

    /// Exclusion means "don't touch this object's data" — not "pretend it
    /// isn't there": an excluded object still occludes its neighbours.
    #[test]
    fn an_excluded_node_still_occludes_but_is_not_written() {
        let model = covered_quad_model();
        let mut stack = OptStack::default();
        stack.push_op(OpKind::BakeAo(BakeAoParams {
            target: AoTarget::Rgb,
            ..BakeAoParams::default()
        }));
        stack.node_override_mut(1).exclude = true;

        let (covered, cover) = split_by_height(&run_model(&model, &stack).lods[0].model);
        for vertex in &covered {
            assert!(
                vertex.vertex_color.x < 0.5,
                "the excluded cover still darkens the quad below, got {}",
                vertex.vertex_color.x
            );
        }
        for vertex in &cover {
            assert_eq!(
                vertex.vertex_color,
                Vec4::new(0.2, 0.4, 0.6, 0.8),
                "the excluded object's own colors are untouched"
            );
        }
    }

    /// A hidden object is out of the bake's scene entirely: it neither occludes
    /// (unlike an *excluded* one, which does) nor receives colors. This is what
    /// keeps assets carrying hidden co-located LOD copies and collision shells
    /// bakeable at all — those invisible near-coincident surfaces would
    /// otherwise shadow every vertex of the visible mesh.
    #[test]
    fn a_hidden_node_neither_occludes_nor_is_baked() {
        let model = covered_quad_model();
        let mut stack = OptStack::default();
        stack.push_op(OpKind::BakeAo(BakeAoParams::default()));

        // Hiding the cover releases the quad below it...
        let (covered, cover) =
            split_by_height(&run_model_hiding(&model, &stack, &[1]).lods[0].model);
        for vertex in &covered {
            assert!(
                vertex.vertex_color.w > 0.99,
                "a hidden cover casts no occlusion, got {}",
                vertex.vertex_color.w
            );
        }
        // ...and the hidden cover itself keeps its source colors.
        for vertex in &cover {
            assert_eq!(
                vertex.vertex_color,
                Vec4::new(0.2, 0.4, 0.6, 0.8),
                "a hidden object is not baked"
            );
        }
    }

    /// The classic per-vertex-AO failure this bake explicitly guards against:
    /// a face whose corners sit in tight contact gaps used to bake black
    /// across its whole area, because occlusion was sampled exactly at the
    /// buried corner point (visibility ≈ 0.03 under a ±0.06 cap hovering
    /// 0.005 above — measured 0.0156 before the fix). With neighborhood
    /// sampling the ray origins are inset onto the incident faces — out from
    /// under the caps — so the mostly-open floor stays open. The caps sit on
    /// the floor's *diagonal* corners too, the ones with a single incident
    /// triangle, so this also pins the low-valence case the two-ring inset
    /// exists for.
    #[test]
    fn a_corner_buried_under_a_tight_cap_stays_open() {
        let model = placed_quads_model(&[
            ("floor", 0.5, Vec3::ZERO),
            ("cap0", 0.06, Vec3::new(-0.5, 0.005, -0.5)),
            ("cap1", 0.06, Vec3::new(0.5, 0.005, -0.5)),
            ("cap2", 0.06, Vec3::new(0.5, 0.005, 0.5)),
            ("cap3", 0.06, Vec3::new(-0.5, 0.005, 0.5)),
        ]);
        let mut stack = OptStack::default();
        stack.push_op(OpKind::BakeAo(BakeAoParams::default()));

        for vertex in &run_model(&model, &stack).lods[0].model.vertices {
            let ao = vertex.vertex_color.w;
            if vertex.position.y < 0.005 {
                assert!(
                    ao > 0.85,
                    "a corner buried under a tight cap samples its open \
                     neighborhood, got {ao}"
                );
            } else {
                assert!(ao > 0.99, "nothing hangs over a cap, got {ao}");
            }
        }
    }

    /// Same geometry as [`covered_quad_model`], but the nodes carry LOD
    /// suffixes — the co-located-LOD-chain layout every game FBX ships. Each
    /// LOD bakes only against its own group, so the "cover" (a different LOD)
    /// casts nothing on the quad below it and both come out fully open. This
    /// is what lets an artist keep the whole chain visible and bake every LOD
    /// in one run.
    #[test]
    fn co_located_lod_copies_do_not_shadow_each_other() {
        let model = named_quads_model(&[("Thing_LOD0", 0.5, 0.0), ("Thing_lod1", 1.0, 0.5)]);
        let mut stack = OptStack::default();
        stack.push_op(OpKind::BakeAo(BakeAoParams::default()));

        for vertex in &run_model(&model, &stack).lods[0].model.vertices {
            assert!(
                vertex.vertex_color.w > 0.99,
                "a different LOD never occludes, got {}",
                vertex.vertex_color.w
            );
        }
    }

    /// A suffix-less mesh has no LOD variants, so it exists at every level: it
    /// occludes each LOD group, and the LOD quads still ignore each other.
    #[test]
    fn an_unsuffixed_object_occludes_every_lod() {
        let model = named_quads_model(&[
            ("Q_LOD0", 0.5, 0.0),
            ("Q_LOD1", 0.5, 0.05),
            ("Cover", 2.0, 0.5),
        ]);
        let mut stack = OptStack::default();
        stack.push_op(OpKind::BakeAo(BakeAoParams::default()));

        for vertex in &run_model(&model, &stack).lods[0].model.vertices {
            let ao = vertex.vertex_color.w;
            if vertex.position.y < 0.2 {
                assert!(
                    ao < 0.5,
                    "the shared cover darkens both LOD quads, got {ao}"
                );
            } else {
                assert!(ao > 0.99, "nothing hangs over the cover, got {ao}");
            }
        }
    }

    /// Suffix-less meshes bake against the lowest LOD present — the
    /// ground-truth geometry — so a floor under a LOD'd canopy still darkens.
    #[test]
    fn an_unsuffixed_object_is_occluded_by_the_lowest_lod() {
        let model = named_quads_model(&[
            ("Floor", 0.5, 0.0),
            ("Canopy_LOD0", 1.0, 0.5),
            ("Canopy_LOD1", 1.5, 0.6),
        ]);
        let mut stack = OptStack::default();
        stack.push_op(OpKind::BakeAo(BakeAoParams::default()));

        let baked = run_model(&model, &stack);
        for vertex in &baked.lods[0].model.vertices {
            if vertex.position.y < 0.2 {
                assert!(
                    vertex.vertex_color.w < 0.5,
                    "the LOD0 canopy darkens the suffix-less floor, got {}",
                    vertex.vertex_color.w
                );
            }
        }
    }

    #[test]
    fn bake_ao_is_deterministic() {
        let model = covered_quad_model();
        let mut stack = OptStack::default();
        stack.push_op(OpKind::BakeAo(BakeAoParams::default()));

        let first = run_model(&model, &stack);
        let second = run_model(&model, &stack);
        for (a, b) in first.lods[0]
            .model
            .vertices
            .iter()
            .zip(&second.lods[0].model.vertices)
        {
            assert_eq!(
                a.vertex_color.to_array(),
                b.vertex_color.to_array(),
                "two runs are bit-identical"
            );
        }
    }

    /// Every write target touches exactly its own channels, leaving the
    /// authored color in the rest.
    #[test]
    fn each_bake_target_touches_only_its_channels() {
        let model = covered_quad_model();
        let bake = |target: AoTarget| {
            let mut stack = OptStack::default();
            stack.push_op(OpKind::BakeAo(BakeAoParams {
                target,
                ..BakeAoParams::default()
            }));
            let (covered, _) = split_by_height(&run_model(&model, &stack).lods[0].model);
            covered[0].vertex_color
        };
        let source = Vec4::new(0.2, 0.4, 0.6, 0.8);

        let alpha = bake(AoTarget::Alpha);
        assert_eq!(alpha.truncate(), source.truncate());
        assert!(alpha.w < 0.5, "alpha carries the occlusion");

        let rgb = bake(AoTarget::Rgb);
        assert!(rgb.x == rgb.y && rgb.y == rgb.z, "grayscale");
        assert!(rgb.x < 0.5);
        assert_eq!(rgb.w, source.w);

        let multiplied = bake(AoTarget::MultiplyRgb);
        assert!(multiplied.x < source.x && multiplied.y < source.y);
        assert_eq!(multiplied.w, source.w);
        let ratio_x = multiplied.x / source.x;
        let ratio_y = multiplied.y / source.y;
        assert!(
            (ratio_x - ratio_y).abs() < 1.0e-6,
            "one factor multiplies every channel: {ratio_x} vs {ratio_y}"
        );

        let red = bake(AoTarget::Red);
        assert!(red.x < 0.5);
        assert_eq!(red.y, source.y);
        assert_eq!(red.z, source.z);
        assert_eq!(red.w, source.w);

        let green = bake(AoTarget::Green);
        assert_eq!(green.x, source.x);
        assert!(green.y < 0.5);
        assert_eq!(green.w, source.w);

        let blue = bake(AoTarget::Blue);
        assert_eq!(blue.x, source.x);
        assert!(blue.z < 0.5);
        assert_eq!(blue.w, source.w);
    }

    /// The sRGB option encodes the RGB-family writes (brightening any value in
    /// (0, 1)) and never applies to the alpha target.
    #[test]
    fn bake_ao_srgb_encodes_rgb_but_never_alpha() {
        let model = covered_quad_model();
        let bake = |target: AoTarget, srgb: bool| {
            let mut stack = OptStack::default();
            stack.push_op(OpKind::BakeAo(BakeAoParams {
                target,
                srgb,
                ..BakeAoParams::default()
            }));
            let (covered, _) = split_by_height(&run_model(&model, &stack).lods[0].model);
            covered[0].vertex_color
        };

        let linear = bake(AoTarget::Rgb, false);
        let encoded = bake(AoTarget::Rgb, true);
        assert!(
            encoded.x > linear.x,
            "sRGB encoding brightens a mid value: {} vs {}",
            encoded.x,
            linear.x
        );

        assert_eq!(
            bake(AoTarget::Alpha, true).w,
            bake(AoTarget::Alpha, false).w,
            "alpha is always written linear"
        );
    }

    #[test]
    fn bake_ao_intensity_darkens() {
        let model = covered_quad_model();
        let bake = |intensity: f32| {
            let mut stack = OptStack::default();
            stack.push_op(OpKind::BakeAo(BakeAoParams {
                intensity,
                ..BakeAoParams::default()
            }));
            let (covered, _) = split_by_height(&run_model(&model, &stack).lods[0].model);
            covered[0].vertex_color.w
        };

        assert!(bake(2.0) < bake(1.0), "a higher power darkens mid values");
    }

    #[test]
    fn a_bake_above_a_simplifier_gets_an_advisory() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::BakeAo(BakeAoParams::default()));
        stack.push_op(OpKind::Reduce(ReduceParams {
            simplify: SimplifySettings {
                algorithm: SimplifyAlgorithm::Sloppy,
                ..SimplifySettings::default()
            },
            target: LodLevel {
                target_ratio: 0.5,
                target_error: 1.0,
            },
        }));
        assert!(
            run(&stack)
                .warnings
                .iter()
                .any(|warning| warning.contains("Bake AO runs before a simplifier")),
            "the ordering advisory is raised"
        );

        stack.reorder(0, 1);
        assert!(
            !run(&stack)
                .warnings
                .iter()
                .any(|warning| warning.contains("Bake AO runs before a simplifier")),
            "baking after the simplifier is the recommended order"
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
            simplify: SimplifySettings {
                algorithm: SimplifyAlgorithm::Standard,
                ..SimplifySettings::default()
            },
            levels: vec![LodLevel {
                target_ratio: 0.5,
                target_error: 0.01,
            }],
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
            simplify: SimplifySettings {
                algorithm: SimplifyAlgorithm::Sloppy,
                ..SimplifySettings::default()
            },
            levels: vec![LodLevel {
                target_ratio: 0.05,
                target_error: 1.0,
            }],
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

        let result = run_model(&model, &stack);

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

        let result = run_model(&model, &stack);

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
