//! The steps a stack is walked in: index, then one operation at a time.
//!
//! [`index_mesh`] runs before any stack operation and is deliberately not one:
//! import splits every face corner into its own vertex, so a mesh reaches this
//! crate with no shared vertices at all and every meshoptimizer operation is a
//! no-op on it. Skipping it is never useful.

use review_model::naming::{self, LodRole};
use review_model::{ModelData, SourceExtras};

use crate::Warnings;
use crate::cancel::{CancelToken, cancelled};
use crate::notice::OptWarning;
use crate::ops;
use crate::quality::{self, Broken, Surface};
use crate::stack::{
    LodLevel, LodParams, OpInstance, OpKind, OptStack, SimplifyAlgorithm, SimplifySettings,
    TargetMode,
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
/// Merging only vertices that match in everything they carry — position,
/// normal, every UV and colour set, crease, deform row and an authored tangent
/// — is lossless: in everything the source authored, each survivor is
/// byte-for-byte what it replaced, so the mesh looks, shades and exports exactly
/// as it did. (The one thing not compared is a tangent the import synthesized,
/// which no file ever carried; [`ops::index_losslessly`] says why.) It is also
/// the precondition meshoptimizer's own pipeline assumes ("generate a vertex
/// remap first"). On that same asset it yields 30018 → 5284 vertices, after
/// which a 50% LOD target is hit exactly.
///
/// It is deliberately not a stack operation: it changes nothing the user can see
/// and there is no case where skipping it is useful. The Weld operation remains
/// for the *lossy* merges — dropping normals or UVs from the comparison, or
/// merging within a tolerance — which do change the mesh.
///
/// Excluded objects are indexed too. Exclusion keeps the user's operations off
/// an object, and this is not one of them: it moves no triangle and changes no
/// value. Left corner-split, an excluded object was counted one vertex per face
/// corner in the baseline and in every level's stats (invariant 5), measured an
/// ACMR of exactly 3.0, and exported one control point per corner — a polygon
/// soup with no shared vertex, which is about as far from "untouched" as a file
/// can get.
///
/// `extras` says which pieces authored their tangents; `None` (no capture, or a
/// model without node tags, where no piece can be matched to its mesh) treats
/// every tangent as synthesized.
pub(crate) fn index_mesh(
    submeshes: &mut [Submesh],
    extras: Option<&SourceExtras>,
    warnings: &mut Warnings,
) {
    let _z = crate::prof::zone!("Index Mesh");
    for piece in submeshes.iter_mut() {
        let tangents_authored = extras
            .and_then(|extras| extras.mesh_of_node(piece.node))
            .is_some_and(|part| part.tangents_authored);
        if let Err(error) = ops::index_losslessly(piece, tangents_authored) {
            warnings.push(OptWarning::IndexFailed {
                detail: error.to_string(),
            });
        }
    }
}

/// Remove every piece of an authored LOD level the generated chain stands in
/// for, returning how many objects went.
///
/// A file carrying `Wall_LOD0` … `Wall_LOD2` already *is* a chain, and running
/// Generate LODs over all of it would simplify the hand-made `Wall_LOD1` into
/// levels of its own and export it beside the generated `Wall_LOD1`. So the
/// chain is generated from its most detailed level alone and the rest are
/// replaced ([`naming::lod_roles`] says which is which) — excluded or not,
/// since an excluded copy would collide with the generated level just the same.
/// Objects outside any chain are untouched.
///
/// `tagged` is whether the pieces carry real node tags; without them every
/// piece claims node 0 and nothing can be told apart.
pub(crate) fn drop_authored_lods(
    submeshes: &mut Vec<Submesh>,
    model: &ModelData,
    tagged: bool,
) -> usize {
    if !tagged {
        return 0;
    }
    let roles = naming::lod_roles(model);
    let replaced = |node: u32| matches!(roles.get(node as usize), Some(LodRole::Replaced));
    let mut dropped: Vec<u32> = submeshes
        .iter()
        .map(|piece| piece.node)
        .filter(|&node| replaced(node))
        .collect();
    dropped.sort_unstable();
    dropped.dedup();
    submeshes.retain(|piece| !replaced(piece.node));
    dropped.len()
}

/// One level under construction.
pub(crate) struct LevelState {
    pub(crate) submeshes: Vec<Submesh>,
    pub(crate) simplify_error: f32,
    /// Whether any simplifier ran on this level (see
    /// [`AnalysisMetrics::simplified`](super::AnalysisMetrics::simplified)).
    pub(crate) simplified: bool,
    /// What the level's last simplify aimed at and reached.
    pub(crate) aim: Option<Aim>,
}

/// What one simplify pass over a mesh did: its worst error, and what it was aimed
/// at against what it reached (`None` when it simplified no object).
pub(crate) struct SimplifyRun {
    pub(crate) error: f32,
    pub(crate) aim: Option<Aim>,
}

impl SimplifyRun {
    /// What an operation that simplified nothing did.
    pub(crate) const NONE: SimplifyRun = SimplifyRun {
        error: 0.0,
        aim: None,
    };
}

/// Simplify a fresh copy of the base submeshes down to one LOD level's target.
pub(crate) fn build_lod_level(
    base: &[Submesh],
    source: &ModelData,
    params: &LodParams,
    level: &crate::stack::LodLevel,
    stack: &OptStack,
    warnings: &mut Warnings,
    cancel: Option<&CancelToken>,
) -> LevelState {
    let mut submeshes = base.to_vec();
    let operation = OpKind::SimplifyLod(params.clone());
    let run = simplify_submeshes(
        &mut submeshes,
        source,
        stack,
        &operation,
        warnings,
        cancel,
        |_| {
            ObjectSettings {
                simplify: params.simplify,
                target: *level,
                // The chain has no per-object overrides: every object shares its
                // levels' counts.
                overridden: false,
            }
        },
    );

    LevelState {
        submeshes,
        simplify_error: run.error,
        simplified: true,
        aim: run.aim,
    }
}

/// What one object is simplified with: the settings and target its operation
/// resolves to for it, and whether they came from its own override.
pub(crate) struct ObjectSettings {
    pub(crate) simplify: SimplifySettings,
    pub(crate) target: LodLevel,
    /// The node's own override supplied them, so a triangle count is the object's
    /// alone rather than its share of the operation's pool (see [`TargetMode`]).
    pub(crate) overridden: bool,
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
    source: &ModelData,
    stack: &OptStack,
    operation: &OpKind,
    warnings: &mut Warnings,
    cancel: Option<&CancelToken>,
    mut settings_for: impl FnMut(&Submesh) -> ObjectSettings,
) -> SimplifyRun {
    let mut worst_error = 0.0f32;
    let mut requested = 0usize;
    let mut produced = 0usize;
    // Whether any piece was simplified by a topology-preserving collapse, which
    // is the only case the seam diagnosis below applies to.
    let mut seam_bound = false;

    // One object at a time: a node's material pieces are one surface, and the
    // simplifier spends the object's budget across all of them by error (see
    // `ops::simplify_object`). Nodes in the order their first piece appears.
    let mut nodes: Vec<u32> = Vec::new();
    for piece in submeshes.iter() {
        if !nodes.contains(&piece.node) {
            nodes.push(piece.node);
        }
    }
    // Each object's settings and triangles first, so a count the objects share
    // (`TargetMode::Triangles`) can be split before any of them is cut. Settings
    // resolve per node (an override names a node), so any piece answers for all of
    // a node's.
    let mut plan: Vec<(u32, ObjectSettings, usize)> = Vec::new();
    for node in nodes {
        if is_excluded(stack, node) {
            continue;
        }
        let Some(first) = submeshes.iter().find(|piece| piece.node == node) else {
            continue;
        };
        let settings = settings_for(first);
        let before = submeshes
            .iter()
            .filter(|piece| piece.node == node)
            .map(Submesh::triangle_count)
            .sum();
        plan.push((node, settings, before));
    }
    // The pool: every object whose count is the operation's own. They all carry
    // the same target, so any of them names the count.
    let pooled = |settings: &ObjectSettings| {
        settings.target.target_mode == TargetMode::Triangles && !settings.overridden
    };
    let pool = plan
        .iter()
        .find(|(_, settings, _)| pooled(settings))
        .map(|(_, settings, _)| {
            let triangles: usize = plan
                .iter()
                .filter(|(_, settings, _)| pooled(settings))
                .map(|&(_, _, before)| before)
                .sum();
            (settings.target.target_triangles, triangles)
        });

    let mut aim: Option<Aim> = None;
    for (node, object, before) in plan {
        if cancelled(cancel) {
            return SimplifyRun {
                error: worst_error,
                aim,
            };
        }
        let mut pieces: Vec<&mut Submesh> = submeshes
            .iter_mut()
            .filter(|piece| piece.node == node)
            .collect();
        let (settings, target) = (object.simplify, object.target);
        seam_bound |= settings.algorithm != SimplifyAlgorithm::Sloppy && !settings.flags.permissive;
        let triangles = object_target(before, &object, pool);

        let error_limit = target.target_error.max(0.0);
        // The object as it came, kept only when a quality limit is set: what each
        // candidate is measured against, and started from.
        let original: Option<Vec<Submesh>> = settings
            .limits
            .any()
            .then(|| pieces.iter().map(|piece| (**piece).clone()).collect());
        let mut outcome = ops::simplify_object(&mut pieces, &settings, triangles, error_limit);
        let mut held: Option<Broken> = None;
        if let Some(original) = &original {
            // Measured against the object as the run found it, so the limit means
            // what the report's figures do: no object strays further than this
            // from the source, whatever a Reduce above the chain already cost.
            let reference = Surface::of_node(source, node);
            let breaks = |pieces: &[&mut Submesh]| {
                settings.limits.broken_by(&quality::compare(
                    &reference,
                    &Surface::of_pieces(pieces.iter().map(|piece| &**piece)),
                    quality::LIMIT_SAMPLES,
                ))
            };
            held = breaks(&pieces);
            if held.is_some() {
                // The fewest triangles between the target and all of them that
                // keep to the limits, found by halving: fewer triangles never
                // change an object less, near enough to search on. Each candidate
                // starts again from the object as it came.
                let (mut low, mut high) = (triangles, before);
                while high - low > (before / 256).max(1) {
                    let middle = low + (high - low) / 2;
                    let mut attempt = restore(submeshes, node, original);
                    ops::simplify_object(&mut attempt, &settings, middle, error_limit);
                    if breaks(&attempt).is_none() {
                        high = middle;
                    } else {
                        low = middle;
                    }
                }
                pieces = restore(submeshes, node, original);
                outcome = if high < before {
                    ops::simplify_object(&mut pieces, &settings, high, error_limit)
                } else {
                    ops::ObjectSimplified::default()
                };
            }
        }
        worst_error = worst_error.max(outcome.error);
        // Each failure left only its own piece as it was; the rest of the
        // object is simplified.
        for error in outcome.failures {
            warnings.push(OptWarning::OperationFailed {
                operation: operation.clone(),
                detail: error.to_string(),
            });
        }
        let after: usize = pieces.iter().map(|piece| piece.triangle_count()).sum();
        requested += before.saturating_sub(triangles);
        produced += before.saturating_sub(after);

        // Above its target by more than rounding: ask why, while the object is
        // still small (see `ops::reachable_without_limit`). The sloppy simplifier
        // ignores topology, so for it only the limit can be the reason.
        // A quality limit that held the object back is the reason, whatever else.
        let stopped_by = match held {
            Some(Broken::Volume) => Some(Shortfall::VolumeLimit),
            Some(Broken::Distance) => Some(Shortfall::DistanceLimit),
            None => (after.saturating_sub(triangles) * 100 > triangles.max(1)).then(|| {
                let further = settings.algorithm == SimplifyAlgorithm::Sloppy
                    || ops::reachable_without_limit(&pieces, &settings, triangles)
                        .is_ok_and(|reach| reach < after);
                if further {
                    Shortfall::ErrorLimit
                } else {
                    Shortfall::Topology
                }
            }),
        };
        let sum = aim.get_or_insert(Aim {
            target: 0,
            reached: 0,
            error_limit: target.target_error,
            limits: settings.limits,
            stopped_by: None,
        });
        sum.target += triangles;
        sum.reached += after;
        // What the level names is the most actionable reason any object had: a
        // limit the user set, then the error limit, then the mesh itself. The
        // limits named are the object's that had it, as an override can differ.
        if let Some(reason) = stopped_by
            && sum
                .stopped_by
                .is_none_or(|already| already.rank() < reason.rank())
        {
            sum.stopped_by = Some(reason);
            sum.error_limit = target.target_error;
            sum.limits = settings.limits;
        }
    }

    // A topology-preserving collapse cannot cross an attribute discontinuity, and
    // FBX import splits every face corner into its own vertex — so a mesh whose
    // normals or UVs differ at every corner (a scan with generated per-face
    // normals, say) presents *every* edge as a seam and the simplifier stalls.
    // That looks identical to the tool being broken, so name the two ways out
    // rather than let the user rediscover them.
    let stalled = requested > 0 && produced * 10 < requested;
    if stalled && seam_bound {
        warnings.push(OptWarning::SimplifierStalledOnSeams);
    }

    SimplifyRun {
        error: worst_error,
        aim,
    }
}

/// An object's pieces, with their contents put back as they were in `original`:
/// the starting point of each candidate a quality limit's search tries.
fn restore<'s>(
    submeshes: &'s mut [Submesh],
    node: u32,
    original: &[Submesh],
) -> Vec<&'s mut Submesh> {
    let mut pieces: Vec<&mut Submesh> = submeshes
        .iter_mut()
        .filter(|piece| piece.node == node)
        .collect();
    for (piece, kept) in pieces.iter_mut().zip(original) {
        (**piece).clone_from(kept);
    }
    pieces
}

/// How many triangles one object is asked for.
///
/// A ratio is applied per object, which is the only interpretation consistent with
/// processing them independently: a 50% target means each object keeps half its
/// triangles, not that the scene total halves while one object vanishes. A count
/// is the object's own when its override names one; otherwise it is the pool's
/// (`pool`: the count and the pooled objects' triangles together), shared out in
/// proportion to each object's triangles — so every pooled object keeps the same
/// share, exactly as under a ratio. Never more than the object has.
fn object_target(before: usize, object: &ObjectSettings, pool: Option<(u32, usize)>) -> usize {
    let target = &object.target;
    match target.target_mode {
        // Kept to the arithmetic it has always had, so a ratio's output is the
        // same bytes it was (D27).
        TargetMode::Ratio => {
            let ratio = target.target_ratio.clamp(0.0, 1.0);
            ((before as f32) * ratio).round() as usize
        }
        TargetMode::Triangles if object.overridden => {
            (target.target_triangles as usize).min(before)
        }
        TargetMode::Triangles => match pool {
            Some((count, pooled)) if pooled > 0 => {
                let share = f64::from(count) / pooled as f64;
                ((before as f64 * share).round() as usize).min(before)
            }
            _ => before,
        },
    }
}

/// Apply one stack operation across every submesh, honouring per-node
/// exclusions and parameter overrides. Returns the worst simplification error it
/// caused and what it aimed at, which only [`OpKind::Reduce`] has.
pub(crate) fn apply_op(
    submeshes: &mut Vec<Submesh>,
    op: &OpInstance,
    stack: &OptStack,
    model: &ModelData,
    hidden_nodes: &[u32],
    warnings: &mut Warnings,
    run: &RunContext<'_>,
) -> SimplifyRun {
    // The LOD operation is the fan-out point the caller handles, and only the
    // first enabled one is: a second would have to fan out a chain that has
    // already fanned out. Neither reaches this function to run, so neither is
    // reported as running. (A preset carrying a second is repaired on load —
    // `preset::from_json` — and the chrome never adds one.)
    if !op.enabled || matches!(op.kind, OpKind::SimplifyLod(_)) {
        return SimplifyRun::NONE;
    }

    // Named once here, before the branches: every arm below is the same
    // operation starting, and the three that work object by object go on to
    // report which object through the same sink.
    run.report(OptProgress::operation(&op.kind));

    // Reduce runs the simplifier, whose stall diagnosis reads the whole mesh
    // rather than one submesh at a time — so it goes through the same helper the
    // LOD fan-out uses instead of the per-piece loop below. Unlike the fan-out it
    // rewrites the submeshes in place, which is the whole difference between the
    // two: what it leaves behind is what every later operation sees, what each LOD
    // level starts from, and what the export writes in the source mesh's place.
    if let OpKind::Reduce(params) = &op.kind {
        return simplify_submeshes(
            submeshes,
            model,
            stack,
            &op.kind,
            warnings,
            run.token(),
            |piece| {
                let resolved = resolve_op(stack, op, piece.node);
                let (params, overridden) = match resolved {
                    OpKind::Reduce(own) => (own, !std::ptr::eq(resolved, &op.kind)),
                    // An override can only ever hold the same kind as the operation
                    // it overrides; fall back to the global settings if one somehow
                    // doesn't.
                    _ => (params, false),
                };
                ObjectSettings {
                    simplify: params.simplify,
                    target: params.target,
                    overridden,
                }
            },
        );
    }

    // The AO bake needs the whole scene as occluders — excluded pieces still
    // occlude, they just aren't written — so like Reduce it takes every submesh
    // at once rather than the per-piece loop below. Outliner-hidden nodes are
    // out of the scene entirely (see [`ProcessInput::hidden_nodes`]), and the
    // scene partitions by `_LOD<n>` name suffix so co-located LOD copies never
    // shadow each other; `model` supplies the node names for that.
    //
    // Either bake may measure the model as it came in instead of the level, and
    // carry the values across (`transfer`).
    if matches!(op.kind, OpKind::BakeAo(_)) {
        crate::transfer::bake(
            submeshes,
            op,
            stack,
            model,
            hidden_nodes,
            run,
            |pieces, op, stack| {
                crate::ao::bake_submeshes(pieces, op, stack, model, hidden_nodes, run.token());
            },
        );
        return SimplifyRun::NONE;
    }

    // Edge wear is per object, so a material border is no edge.
    if matches!(op.kind, OpKind::BakeWear(_)) {
        crate::transfer::bake(
            submeshes,
            op,
            stack,
            model,
            hidden_nodes,
            run,
            |pieces, op, stack| {
                crate::wear::bake_wear_submeshes(pieces, op, stack, model, run);
            },
        );
        return SimplifyRun::NONE;
    }

    // Remesh does not edit pieces, it *replaces* them: a node's materials share
    // one surface, and the regenerated mesh may use a different set of them. So
    // like the two above it takes the whole scene at once — and unlike them it
    // takes the `Vec` itself, which is why `apply_op` does.
    if matches!(op.kind, OpKind::Remesh(_)) {
        crate::remesh::remesh_submeshes(submeshes, op, stack, model, warnings, run);
        return SimplifyRun::NONE;
    }

    // Shrinkwrap replaces a node's pieces the same way, and for the same reason:
    // a wrap of one material's half of an object is not a shell of anything.
    if matches!(op.kind, OpKind::Shrinkwrap(_)) {
        crate::shrinkwrap::shrinkwrap_submeshes(submeshes, op, stack, model, warnings, run);
        return SimplifyRun::NONE;
    }

    // A hull is of a whole object, whatever materials it is in.
    if matches!(op.kind, OpKind::ConvexHull(_)) {
        crate::hull::hull_submeshes(submeshes, op, stack, model, warnings, run);
        return SimplifyRun::NONE;
    }

    // Normals are generated per object, not per piece: a node's material pieces
    // share one surface, and each alone would come out hard along the border.
    if matches!(op.kind, OpKind::RecalculateNormals(_)) {
        crate::shading::recalculate_normals_submeshes(submeshes, op, stack, model, warnings, run);
        return SimplifyRun::NONE;
    }

    // Unwrap reads the model's custom attribute layout (its seam attribute)
    // and runs its pieces in parallel, each in an atlas of its own.
    if matches!(op.kind, OpKind::Unwrap(_)) {
        crate::unwrap::unwrap_submeshes(submeshes, op, stack, model, warnings, run);
        return SimplifyRun::NONE;
    }

    for piece in submeshes.iter_mut() {
        if run.cancelled() {
            return SimplifyRun::NONE;
        }
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
            // The LOD operation is refused above; Reduce, the AO bake, Remesh,
            // Shrinkwrap, Recalculate Normals and Unwrap are handled above. None
            // reaches this path.
            OpKind::Reduce(_)
            | OpKind::SimplifyLod(_)
            | OpKind::BakeAo(_)
            | OpKind::BakeWear(_)
            | OpKind::Remesh(_)
            | OpKind::Shrinkwrap(_)
            | OpKind::ConvexHull(_)
            | OpKind::RecalculateNormals(_)
            | OpKind::Unwrap(_) => Ok(()),
        };
        if let Err(error) = outcome {
            warnings.push(OptWarning::OperationFailed {
                operation: kind.clone(),
                detail: error.to_string(),
            });
        }
    }
    SimplifyRun::NONE
}
