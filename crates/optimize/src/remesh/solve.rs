//! One object's rebuild, stage by stage.
//!
//! The order is fixed and each stage reads only what the ones before it wrote:
//!
//! 1. [`topology`](super::topology) — index the proxy once, so no later stage
//!    builds an adjacency of its own.
//! 2. [`size_field`](super::size_field) — how big a face should be, per vertex.
//! 3. [`features`](super::features) — the borders and creases that have to
//!    survive, walked into chains.
//! 4. [`seeds`](super::seeds) — one seed per output vertex, chains first.
//! 5. [`partition`](super::partition) + [`lloyd`](super::lloyd) — grow the seeds
//!    into even regions.
//! 6. [`collapse`](super::collapse) — merge each region down to its one vertex.
//! 7. [`cleanup`](super::cleanup) — even the faces out.
//! 8. [`quads`](super::quads) — pair the triangles up, when quads were asked
//!    for, guided by a [`cross_field`](super::cross_field) built here for it.
//!
//! Between every stage, and between the passes of the ones that iterate, the
//! run can be abandoned. That is the difference the user actually feels: the
//! engines this replaces took no cancel hook at all, so an edit mid-rebuild had
//! to wait out the object already in the solver.
//!
//! ## The target edge length
//!
//! The requested face count becomes a uniform spacing by area: a triangle of
//! side `s` covers `s^2 * sqrt(3) / 4`, so `s = 2 * sqrt(area / faces *
//! sqrt(1/3))`. The size field then redistributes that spacing without changing
//! the total, and the seed budget turns the count into vertices exactly
//! ([`seeds::vertex_budget`]) — so unlike the engines, nothing here has to be
//! run twice to land near the number in the box.

use crate::OptError;
use crate::cancel::{CancelToken, cancelled};

use super::proxy::Proxy;
use super::surface::Surface;
use super::topology::Topology;
use super::{
    RemeshOutput, cleanup, collapse, cross_field, features, lloyd, partition, preview, quads,
    seeds, size_field,
};
use crate::stack::{RemeshParams, RemeshTopology};

/// Triangles to aim for per quad asked for. See [`triangle_target`]; the value
/// is measured rather than derived, and `tests/remesh_quads.rs` is what pins it.
const QUAD_TRIANGLE_BUDGET: f64 = 1.85;

/// What a rebuild produced, beside the mesh.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Report {
    /// Regions that would not come down to a single vertex, so the output has
    /// more vertices than the budget asked for.
    pub(crate) stubborn: usize,
    /// Vertices the rebuild actually produced.
    pub(crate) vertices: usize,
    /// Faces that came out with four corners. Zero unless quads were asked for.
    pub(crate) quads: usize,
}

/// Rebuild one object's surface.
///
/// `after_pass` is handed a cheap intermediate mesh as the relaxation settles,
/// for a caller that wants to show the rebuild converging; returning `false`
/// stops the run. See [`preview`] for what those meshes are and are not.
pub(crate) fn rebuild(
    proxy: &Proxy,
    faces: u32,
    params: &RemeshParams,
    threads: usize,
    cancel: Option<&CancelToken>,
    mut after_pass: impl FnMut(RemeshOutput) -> bool,
) -> Result<(RemeshOutput, Report), OptError> {
    let _z = crate::prof::zone!("Remesh Solve");
    let vertices = proxy.positions.len() / 3;
    if vertices == 0 || proxy.indices.len() < 3 {
        return Err(OptError::EmptyMesh);
    }

    let topology = Topology::build(&proxy.indices, vertices, threads, cancel);
    if cancelled(cancel) {
        return Err(OptError::Cancelled);
    }

    let normals = size_field::vertex_normals(&proxy.positions, &proxy.indices, &topology, threads);
    let areas = size_field::dual_areas(&proxy.positions, &proxy.indices, &topology, threads);
    // The whole rebuild produces triangles; quads are pairs of them, made at the
    // very end. So a quad budget is twice as many triangles to begin with - see
    // [`triangle_target`], which is where the leftovers are accounted for too.
    let triangles = triangle_target(faces, params.topology);
    let target_edge = target_edge(proxy.area as f64, triangles);
    if target_edge <= 0.0 {
        return Err(OptError::EmptyMesh);
    }

    // The field is a *multiplier*; `None` means one size everywhere, which is
    // the same answer by a shorter road.
    let field = size_field::build(
        &size_field::FieldInput {
            positions: &proxy.positions,
            normals: &normals,
            areas: Some(&areas),
            indices: &proxy.indices,
            topology: &topology,
        },
        params.adaptive_strength.clamp(0.0, 1.0),
        target_edge,
        threads,
        cancel,
    );
    let sizes: Vec<f32> = match field {
        Some(field) => field
            .iter()
            .map(|multiplier| target_edge * multiplier)
            .collect(),
        None => vec![target_edge; vertices],
    };
    // Released here unless the cross field will read them: at ten million
    // vertices this is a hundred and twenty megabytes, and nothing else wants
    // them.
    let normals = (params.topology == RemeshTopology::Quads).then_some(normals);
    if cancelled(cancel) {
        return Err(OptError::Cancelled);
    }

    let surface = Surface {
        positions: &proxy.positions,
        indices: &proxy.indices,
        topology: &topology,
        sizes: &sizes,
        areas: &areas,
    };

    let crease_cosine = params
        .sharp_edges
        .then(|| params.crease_angle.clamp(0.0, 180.0).to_radians().cos());
    let features = features::build(surface, params.align_to_boundaries, crease_cosine, cancel);
    let mut seeds = seeds::place(surface, &features, triangles, cancel);
    if seeds.is_empty() {
        return Err(OptError::EmptyMesh);
    }
    let mut partition = partition::build(surface, &features, &seeds, threads, cancel);
    if cancelled(cancel) {
        return Err(OptError::Cancelled);
    }

    let mut stopped = false;
    lloyd::relax(
        surface,
        &features,
        &mut seeds,
        &mut partition,
        threads,
        cancel,
        |seeds, partition, _| {
            let carry_on = after_pass(preview::build(
                surface.positions,
                surface.indices,
                seeds,
                partition,
            ));
            stopped = !carry_on;
            carry_on
        },
    );
    if stopped || cancelled(cancel) {
        return Err(OptError::Cancelled);
    }

    let quadrics = collapse::build_quadrics(surface, threads);
    let state = collapse::run(surface, &features, &seeds, &partition, &quadrics, cancel);
    if cancelled(cancel) {
        return Err(OptError::Cancelled);
    }
    let carried = collapse::into_output(&state, &features);
    let collapse::Carried {
        mut output,
        pinned,
        source,
    } = carried;
    // What the collapse leaves is correct but lumpy — a region's vertex lands
    // wherever its last valid collapse put it. This is what makes the faces
    // even, which is most of what "retopology" means to look at.
    cleanup::run(&mut output, &pinned, params.smooth_iterations, cancel);
    // Checked here and after the merge: a cancelled pass returns early with a
    // half-tidied mesh, and nothing past this point should spend time on it.
    if cancelled(cancel) {
        return Err(OptError::Cancelled);
    }

    // Quads last, and only then. The field costs about a third of a rebuild to
    // build, and until this moment there is nothing that can spend it: every
    // stage before this one is choosing *where* vertices go, and a cross field
    // describes a square lattice that a triangulation cannot take (four
    // measurements of trying are in `cross_field`'s module doc). The merge is
    // the one consumer whose output is four-sided.
    let quads = match normals {
        Some(normals) => {
            let field = cross_field::build(surface, &normals, &features, threads, cancel);
            if cancelled(cancel) {
                return Err(OptError::Cancelled);
            }
            // The field is over the *input*, and every output vertex is an input
            // vertex - which is the one thing a collapse guarantees and an
            // extraction could not.
            let directions: Vec<[f32; 3]> = source
                .iter()
                .map(|&vertex| field.at(vertex).unwrap_or([0.0; 3]))
                .collect();
            quads::merge(&mut output, &directions, cancel).quads
        }
        None => 0,
    };
    if cancelled(cancel) {
        return Err(OptError::Cancelled);
    }
    output.validate()?;

    let report = Report {
        stubborn: state.stubborn,
        vertices: output.positions.len(),
        quads,
    };
    Ok((output, report))
}

/// How many *triangles* the rebuild must produce for `faces` finished faces.
///
/// A quad is two triangles, so a quad budget starts as twice the triangles -
/// except that the merge never pairs quite everything up, and each leftover
/// triangle is one more face than the arithmetic allowed for. The factor is
/// below two by the share the merge is measured to leave behind, which keeps the
/// finished count on the number in the box rather than ten per cent above it.
fn triangle_target(faces: u32, topology: RemeshTopology) -> u32 {
    let scale = match topology {
        RemeshTopology::Triangles => 1.0,
        RemeshTopology::Quads => QUAD_TRIANGLE_BUDGET,
    };
    ((faces as f64 * scale).round() as u32).max(1)
}

/// The uniform edge length `faces` triangles come to over `area`.
fn target_edge(area: f64, faces: u32) -> f32 {
    if area <= 0.0 || faces == 0 {
        return 0.0;
    }
    let per_face = area / faces as f64;
    // An equilateral triangle of side `s` covers `s^2 * sqrt(3) / 4`.
    let edge = 2.0 * (per_face * (1.0f64 / 3.0).sqrt()).sqrt();
    if edge.is_finite() && edge > 0.0 {
        edge as f32
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stack::RemeshTopology;

    /// A sphere, closed and curved: the shape every stage has something to do
    /// on.
    fn sphere(rings: u32, segments: u32) -> Proxy {
        let mut positions = Vec::new();
        for ring in 0..=rings {
            let phi = ring as f32 / rings as f32 * std::f32::consts::PI;
            for segment in 0..segments {
                let theta = segment as f32 / segments as f32 * std::f32::consts::TAU;
                positions.extend_from_slice(&[
                    phi.sin() * theta.cos(),
                    phi.cos(),
                    phi.sin() * theta.sin(),
                ]);
            }
        }
        let mut indices = Vec::new();
        for ring in 0..rings {
            for segment in 0..segments {
                let next = (segment + 1) % segments;
                let a = ring * segments + segment;
                let b = ring * segments + next;
                let c = (ring + 1) * segments + segment;
                let d = (ring + 1) * segments + next;
                indices.extend_from_slice(&[a, c, b]);
                indices.extend_from_slice(&[b, c, d]);
            }
        }
        proxy_of(positions, indices)
    }

    fn proxy_of(positions: Vec<f32>, indices: Vec<u32>) -> Proxy {
        let mut bounds = review_model::Bounds::EMPTY;
        let mut area = 0.0f64;
        for point in positions.as_chunks::<3>().0 {
            bounds.include_point(glam::Vec3::new(point[0], point[1], point[2]));
        }
        for corners in indices.as_chunks::<3>().0 {
            let at = |vertex: u32| {
                let base = vertex as usize * 3;
                glam::Vec3::new(positions[base], positions[base + 1], positions[base + 2])
            };
            let (a, b, c) = (at(corners[0]), at(corners[1]), at(corners[2]));
            area += ((b - a).cross(c - a).length() * 0.5) as f64;
        }
        let triangles = indices.len() / 3;
        Proxy {
            positions,
            indices,
            source_triangles: triangles,
            area: area as f32,
            bounds,
        }
    }

    fn params() -> RemeshParams {
        RemeshParams {
            topology: RemeshTopology::Triangles,
            adaptive_strength: 0.5,
            ..RemeshParams::default()
        }
    }

    #[test]
    fn a_sphere_rebuilds_to_about_the_face_count_asked_for() {
        let proxy = sphere(24, 32);

        for asked in [200u32, 600, 1500] {
            let (output, _) =
                rebuild(&proxy, asked, &params(), 1, None, |_| true).expect("a sphere rebuilds");
            let produced = output.face_count() as f64;
            let miss = (produced - asked as f64).abs() / asked as f64;
            assert!(
                miss < 0.20,
                "asked {asked}, produced {produced}: {:.0}% out",
                miss * 100.0
            );
        }
    }

    #[test]
    fn the_rebuild_is_the_same_at_every_thread_count() {
        let proxy = sphere(20, 28);

        let run = |threads: usize| {
            rebuild(&proxy, 500, &params(), threads, None, |_| true)
                .expect("a sphere rebuilds")
                .0
        };

        let one = run(1);
        let many = run(8);
        assert_eq!(one.positions, many.positions);
        assert_eq!(one.corners, many.corners);
    }

    #[test]
    fn the_relaxation_offers_a_preview_before_the_result() {
        let proxy = sphere(18, 24);

        let mut previews = 0;
        let (output, _) = rebuild(&proxy, 400, &params(), 1, None, |preview: RemeshOutput| {
            assert!(preview.validate().is_ok(), "a preview is a valid soup");
            previews += 1;
            true
        })
        .expect("a sphere rebuilds");

        assert!(previews > 0, "the relaxation reports as it settles");
        assert!(output.face_count() > 0);
    }

    #[test]
    fn refusing_a_preview_cancels_the_run() {
        let proxy = sphere(18, 24);

        let result = rebuild(&proxy, 400, &params(), 1, None, |_| false);

        assert!(matches!(result, Err(OptError::Cancelled)));
    }

    #[test]
    fn a_dead_token_stops_the_rebuild() {
        use std::sync::Arc;
        use std::sync::atomic::AtomicU64;

        let proxy = sphere(16, 20);
        let token = CancelToken::new(Arc::new(AtomicU64::new(9)), 1);

        let result = rebuild(&proxy, 300, &params(), 1, Some(&token), |_| true);

        assert!(matches!(result, Err(OptError::Cancelled)));
    }

    #[test]
    fn an_empty_proxy_is_an_empty_mesh() {
        let proxy = proxy_of(Vec::new(), Vec::new());

        assert!(matches!(
            rebuild(&proxy, 100, &params(), 1, None, |_| true),
            Err(OptError::EmptyMesh)
        ));
    }

    #[test]
    fn the_target_edge_follows_the_area_and_the_count() {
        // Four times the faces over the same area is half the edge length.
        let coarse = target_edge(100.0, 100);
        let fine = target_edge(100.0, 400);

        assert!(
            (coarse / fine - 2.0).abs() < 1.0e-5,
            "{coarse} against {fine}"
        );
        assert_eq!(target_edge(0.0, 100), 0.0);
        assert_eq!(target_edge(100.0, 0), 0.0);
    }
}
