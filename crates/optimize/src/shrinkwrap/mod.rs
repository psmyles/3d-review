//! Replacing an object with one closed shell that hugs it.
//!
//! ## What it is for
//!
//! Game assets are rarely surfaces. A prop is a kitbash of interpenetrating
//! parts, a wall is a plane with no thickness, a scan has holes, and an
//! artist's mesh has inverted shells in the places nobody ever looks. None of
//! that stops a *renderer*, and none of it stops [`crate::remesh`]'s field
//! extraction either — but it stops every algorithm that needs to walk the
//! surface as a surface, and for a rebuild that wants one even shell rather
//! than a pile of overlapping parts.
//!
//! Shrinkwrap is the operation that makes such an object into one: sample a
//! signed distance field around it, extract the zero crossing, and hand back a
//! single watertight shell. The parts fuse, the holes close, the inverted shell
//! stops mattering, and what comes out is a manifold by construction.
//!
//! It is also useful on its own — a wrapped copy is a collision proxy, or the
//! lowest LOD of a silhouette — which is why the materials, UVs and colors come
//! back on it through the same projection a remesh uses.
//!
//! ## Why the sign is a winding number
//!
//! The one thing a distance field needs that a triangle soup cannot give it is
//! *inside*. Reading the nearest triangle's facing puts the boundary wherever
//! the nearest scrap of geometry happens to point; counting ray crossings needs
//! the closed surface we do not have. The generalized winding number needs
//! neither — see [`winding`].
//!
//! ## Layout
//!
//! [`grid`] is the lattice and its block-sparse storage, [`winding`] the inside
//! test, [`voxel`] one lattice point's value as a pure function of where it is,
//! [`sdf`] the pass that decides which blocks exist and fills them, and [`mc`]
//! the extraction. The attribute transfer and the per-material split are
//! [`crate::remesh`]'s, unchanged.

use std::collections::HashMap;

use review_model::{Bvh, ModelData, Vertex};

use crate::Warnings;
use crate::remesh::{self, proxy};
use crate::stack::{OpInstance, OpKind, OptStack, ShrinkwrapParams};
use crate::submesh::Submesh;

mod grid;
mod mc;
mod sdf;
mod voxel;
mod winding;

/// Fewest source triangles a node must have to be worth wrapping. Below this
/// there is no enclosed volume to speak of and the wrap would replace the object
/// with a blob.
const MIN_INPUT_TRIANGLES: usize = 8;

/// Wrap every eligible node, replacing its pieces in place.
///
/// Whole-scene like [`crate::remesh::remesh_submeshes`] and for the same reason:
/// a node's materials share one surface, and a wrap of one material's half of an
/// object is not a shell of anything.
pub(crate) fn shrinkwrap_submeshes(
    submeshes: &mut Vec<Submesh>,
    op: &OpInstance,
    stack: &OptStack,
    model: &ModelData,
    warnings: &mut Warnings,
    run: &crate::process::RunContext<'_>,
) {
    let _z = crate::prof::zone!("Shrinkwrap");

    let OpKind::Shrinkwrap(global) = &op.kind else {
        return;
    };

    let mut nodes: Vec<u32> = Vec::new();
    for piece in submeshes.iter() {
        if !nodes.contains(&piece.node) {
            nodes.push(piece.node);
        }
    }

    // Gathered before any wrap starts, for the reason [`crate::parallel`] gives:
    // the solves borrow `submeshes` immutably and the splice needs it back. A
    // node that cannot be wrapped is filtered out here rather than inside a
    // worker, so the warning for it is raised in stack order.
    let mut jobs: Vec<WrapJob<'_>> = Vec::new();
    for &node in &nodes {
        if crate::process::is_excluded(stack, node) {
            continue;
        }
        let pieces: Vec<&Submesh> = submeshes
            .iter()
            .filter(|piece| piece.node == node && !piece.is_empty())
            .collect();
        if pieces.is_empty() {
            continue;
        }
        let name = remesh::node_name(model, node);
        if pieces.iter().any(|piece| piece.has_rows()) {
            warnings.push(&format!(
                "Shrinkwrap: '{name}' is skinned or has blend shapes, so it was left as it \
                 is. Wrapping replaces the surface entirely, and no skin weight authored \
                 against the old vertices can follow it."
            ));
            continue;
        }
        let params = match crate::process::resolve_op(stack, op, node) {
            OpKind::Shrinkwrap(params) => *params,
            // An override can only ever hold the same kind as the operation it
            // overrides; fall back to the global settings if one somehow doesn't.
            _ => *global,
        };
        jobs.push(WrapJob {
            node,
            pieces,
            params,
            name,
        });
    }

    // One object per core, each result parked in its own slot so the warning
    // and piece order stay the stack's rather than the scheduler's.
    let total = jobs.len() as u32;
    let mut outcomes: Vec<Option<WrapOutcome>> = (0..jobs.len()).map(|_| None).collect();
    let mut done = 0u32;
    crate::parallel::solve_nodes(
        &jobs,
        run.token(),
        |job, _| {
            let mut notes = Warnings::default();
            let rebuilt = wrap_node(&job.pieces, &job.params, &job.name, &mut notes);
            WrapOutcome {
                rebuilt,
                notes: notes.into_vec(),
            }
        },
        |index, outcome| {
            // As each object lands, exactly as Remesh reports its own.
            done += 1;
            run.report(crate::process::OptProgress::object(
                &op.kind,
                &jobs[index].name,
                done,
                total,
            ));
            outcomes[index] = Some(outcome);
        },
        // A wrap has no half-finished state worth showing: it is one
        // extraction, not a sequence of improving ones.
        |_, (): ()| {},
    );

    let mut replacements: HashMap<u32, Vec<Submesh>> = HashMap::new();
    for (job, outcome) in jobs.iter().zip(&mut outcomes) {
        let Some(outcome) = outcome.take() else {
            continue;
        };
        for note in &outcome.notes {
            warnings.push(note);
        }
        if let Some(rebuilt) = outcome.rebuilt {
            replacements.insert(job.node, rebuilt);
        }
    }
    if replacements.is_empty() {
        return;
    }

    // Splice: a wrapped node's new pieces land where its first old piece was.
    let mut rebuilt: Vec<Submesh> = Vec::with_capacity(submeshes.len());
    let mut placed: Vec<u32> = Vec::new();
    for piece in submeshes.drain(..) {
        match replacements.get_mut(&piece.node) {
            Some(pieces) => {
                if !placed.contains(&piece.node) {
                    placed.push(piece.node);
                    rebuilt.append(pieces);
                }
            }
            None => rebuilt.push(piece),
        }
    }
    *submeshes = rebuilt;
}

/// One node's whole wrap, gathered before the parallel pass so a worker touches
/// nothing it shares with another. Remesh's `NodeJob` twin.
struct WrapJob<'a> {
    node: u32,
    pieces: Vec<&'a Submesh>,
    params: ShrinkwrapParams,
    name: String,
}

/// What one node's wrap produced, warnings and all - see Remesh's `NodeOutcome`
/// for why they ride back rather than going into the shared [`Warnings`].
struct WrapOutcome {
    rebuilt: Option<Vec<Submesh>>,
    notes: Vec<String>,
}

/// Wrap one node, or `None` when it was left as it is (with a warning already
/// recorded).
fn wrap_node(
    pieces: &[&Submesh],
    params: &ShrinkwrapParams,
    name: &str,
    warnings: &mut Warnings,
) -> Option<Vec<Submesh>> {
    let proxy = proxy::build(pieces);
    if proxy.triangle_count() < MIN_INPUT_TRIANGLES {
        warnings.push(&format!(
            "Shrinkwrap: '{name}' has too few triangles to wrap ({}), so it was left as it is.",
            proxy.triangle_count()
        ));
        return None;
    }
    if proxy.bounds.is_empty() {
        return None;
    }

    // A lattice fine enough to be asked for but small enough to hold. Halving
    // the resolution divides the point count by eight, so this converges in a
    // step or two whatever was asked for.
    let mut resolution = params.resolution.clamp(16, 1024);
    let mut desc = grid::GridDesc::cover(proxy.bounds, resolution, params.offset);
    let requested = resolution;
    while desc.point_count() > grid::MAX_POINTS && resolution > 16 {
        resolution = (resolution / 2).max(16);
        desc = grid::GridDesc::cover(proxy.bounds, resolution, params.offset);
    }
    if resolution != requested {
        warnings.push(&format!(
            "Shrinkwrap: '{name}' would need more memory than a wrap is worth at \
             resolution {requested}, so it ran at {resolution} instead."
        ));
    }

    // The proxy as something the hierarchies can be queried against. Positions
    // only: the sign and the distance are all the field reads, and the
    // attributes come back afterwards from the *unwelded* source.
    let mut sampled = ModelData {
        indices: proxy.indices.clone(),
        ..ModelData::default()
    };
    for values in proxy.positions.as_chunks::<3>().0 {
        sampled.vertices.push(Vertex {
            position: glam::Vec3::new(values[0], values[1], values[2]),
            ..Vertex::default()
        });
    }
    let bvh = Bvh::build(&sampled);
    let winding = winding::Tree::build(&proxy.positions, &proxy.indices);
    if winding.is_empty() {
        return None;
    }

    let field = sdf::build(desc, &sampled, &bvh, &winding, params.offset);
    let mut surface = mc::extract(&field);
    if params.keep_largest_shell {
        mc::keep_largest_shell(&mut surface);
    }
    if surface.is_empty() {
        warnings.push(&format!(
            "Shrinkwrap: '{name}' came back empty — at resolution {resolution} the object \
             is thinner than one voxel. Raise the resolution, or add a small offset to \
             give it some thickness."
        ));
        return None;
    }

    // The extraction is a polygon soup of triangles, which is exactly what the
    // remesh path already knows how to project attributes onto and split by
    // material. No polygon carry: a wrap produces triangles, and a face table
    // over them would put the whole level into the corner-run layout for nothing.
    let face_offsets = (0..=surface.triangle_count() as u32)
        .map(|face| face * 3)
        .collect();
    let output = remesh::RemeshOutput {
        positions: surface.positions,
        face_offsets,
        corners: surface.indices,
    };
    Some(remesh::build_pieces_from(
        &output,
        pieces,
        &proxy,
        remesh::Winding::Keep,
    ))
}

#[cfg(test)]
mod tests {
    use review_model::demo_cube_model;

    use super::*;
    use crate::submesh::partition;

    /// The demo cube, subdivided enough to be past [`MIN_INPUT_TRIANGLES`].
    fn cube_pieces() -> Vec<Submesh> {
        let model = demo_cube_model();
        let (pieces, _) = partition(&model, None);
        pieces
    }

    #[test]
    fn a_wrapped_cube_is_a_closed_shell_around_it() {
        let pieces = cube_pieces();
        let borrowed: Vec<&Submesh> = pieces.iter().collect();
        let mut warnings = Warnings::default();
        let params = ShrinkwrapParams {
            resolution: 32,
            ..ShrinkwrapParams::default()
        };

        let wrapped =
            wrap_node(&borrowed, &params, "cube", &mut warnings).expect("the demo cube wraps");

        assert!(!wrapped.is_empty(), "the wrap produced geometry");
        let triangles: usize = wrapped.iter().map(Submesh::triangle_count).sum();
        assert!(triangles > 100, "a shell at 32 voxels has real geometry");
        for piece in &wrapped {
            assert!(
                piece.polygons.is_none(),
                "a wrap emits triangles and carries no face table"
            );
        }

        // The shell hugs the cube: every vertex within a voxel or so of it.
        let source = proxy::build(&borrowed);
        let bounds = source.bounds;
        let slack = bounds.size().max_element() / 32.0 * 3.0;
        for piece in &wrapped {
            for vertex in &piece.vertices {
                let outside = (bounds.min - vertex.position)
                    .max(vertex.position - bounds.max)
                    .max_element();
                assert!(
                    outside < slack,
                    "a shell vertex at {:?} is {outside} outside the object",
                    vertex.position
                );
            }
        }
    }

    #[test]
    fn a_node_with_almost_no_triangles_is_left_alone() {
        let model = demo_cube_model();
        let (mut pieces, _) = partition(&model, None);
        pieces[0].indices.truncate(9); // three triangles
        let borrowed: Vec<&Submesh> = pieces.iter().collect();
        let mut warnings = Warnings::default();

        let wrapped = wrap_node(
            &borrowed,
            &ShrinkwrapParams::default(),
            "sliver",
            &mut warnings,
        );

        assert!(wrapped.is_none());
        assert!(!warnings.is_empty(), "and the user is told why");
    }

    #[test]
    fn a_positive_offset_grows_the_shell() {
        let pieces = cube_pieces();
        let borrowed: Vec<&Submesh> = pieces.iter().collect();
        let mut warnings = Warnings::default();
        let source = proxy::build(&borrowed);
        let offset = source.bounds.size().max_element() * 0.1;

        let mut extent_of = |params: &ShrinkwrapParams| -> f32 {
            let wrapped = wrap_node(&borrowed, params, "cube", &mut warnings).expect("wraps");
            let mut bounds = review_model::Bounds::EMPTY;
            for piece in &wrapped {
                for vertex in &piece.vertices {
                    bounds.include_point(vertex.position);
                }
            }
            bounds.size().max_element()
        };

        let plain = extent_of(&ShrinkwrapParams {
            resolution: 32,
            ..ShrinkwrapParams::default()
        });
        let grown = extent_of(&ShrinkwrapParams {
            resolution: 32,
            offset,
            ..ShrinkwrapParams::default()
        });

        assert!(
            grown > plain + offset,
            "an offset of {offset} grew the shell from {plain} to {grown}"
        );
    }
}
