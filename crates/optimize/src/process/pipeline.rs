//! The steps a stack is walked in: index, then one operation at a time.
//!
//! [`index_mesh`] runs before any stack operation and is deliberately not one:
//! import splits every face corner into its own vertex, so a mesh reaches this
//! crate with no shared vertices at all and every meshoptimizer operation is a
//! no-op on it. Skipping it is never useful.

use review_model::ModelData;

use crate::Warnings;
use crate::ops;
use crate::stack::{
    LodLevel, LodParams, OpInstance, OpKind, OptStack, SimplifyAlgorithm, SimplifySettings,
};
use crate::submesh::Submesh;

use super::*;

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
pub(crate) fn index_mesh(submeshes: &mut [Submesh], stack: &OptStack, warnings: &mut Warnings) {
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
pub(crate) struct LevelState {
    pub(crate) submeshes: Vec<Submesh>,
    pub(crate) simplify_error: f32,
}

/// Simplify a fresh copy of the base submeshes down to one LOD level's target.
pub(crate) fn build_lod_level(
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
pub(crate) fn simplify_submeshes(
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
pub(crate) fn apply_op(
    submeshes: &mut Vec<Submesh>,
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

    // Remesh does not edit pieces, it *replaces* them: a node's materials share
    // one surface, and the regenerated mesh may use a different set of them. So
    // like the two above it takes the whole scene at once — and unlike them it
    // takes the `Vec` itself, which is why `apply_op` does.
    if matches!(op.kind, OpKind::Remesh(_)) {
        crate::remesh::remesh_submeshes(submeshes, op, stack, model, warnings);
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
            // caller; Reduce, the AO bake and Remesh are handled above. None
            // reaches this path.
            OpKind::Reduce(_) | OpKind::SimplifyLod(_) | OpKind::BakeAo(_) | OpKind::Remesh(_) => {
                Ok(())
            }
        };
        if let Err(error) = outcome {
            warnings.push(&format!("{}: {error}", kind.label()));
        }
    }
    0.0
}
