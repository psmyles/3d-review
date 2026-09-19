//! Field-guided retopology: regenerate an object's surface as evenly sized,
//! curvature-aligned triangles or quads.
//!
//! ## Why this is not like the other operations
//!
//! Every meshoptimizer operation *edits* the mesh it is handed — it removes
//! triangles, merges vertices or reorders buffers, and each output triangle can
//! be traced back to an input one. Retopology cannot: the output shares no
//! vertex, no edge and no face with the input. It is a new surface that happens
//! to lie on the old one.
//!
//! Three things follow, and they shape this whole module:
//!
//! * **Attributes come back by projection.** For each new corner, the nearest
//!   point on the source surface is found ([`review_model::Bvh::closest_point`])
//!   and the material, UVs, colors and normal are read there ([`project`]).
//! * **The operation replaces a node's pieces** rather than rewriting them, so
//!   it takes the whole `Vec<Submesh>` and splices — a node that came in as
//!   three materials may come out as three different ones.
//! * **Quads are real.** The engine emits polygons, and they survive to the
//!   viewport, the stats card and the export as polygons, through a rebuilt
//!   [`crate::submesh::PolygonCarry`] ([`layout`]).
//!
//! ## Static meshes only
//!
//! A node carrying skin weights or blend shapes is left untouched with a
//! warning. Weights are per *vertex*, and the new mesh has none of the old
//! vertices; projecting them would mean inventing a binding the artist never
//! authored and that no later edit could check.
//!
//! ## Layout
//!
//! [`proxy`] builds the welded position-only mesh the engine solves over,
//! [`project`] is the attribute transfer, [`manifold`] the pre-flight report,
//! and [`layout`] the canonicalization plus the corner-run expansion
//! `process::assemble` uses. The engine call itself is behind
//! [`crate::remesh_ffi`], with [`unavailable`] as its twin for a build with no
//! vendored tree.

use std::collections::HashMap;

use glam::{Vec3, Vec4};
use review_model::{ModelData, Vertex};

use crate::stack::{OpInstance, OpKind, RemeshDensity, RemeshParams, WeldParams};
use crate::submesh::{NO_FACE, PolygonCarry, Submesh};
use crate::{OptError, Warnings, ops};

#[allow(
    dead_code,
    reason = "the solver that drives the collapse is the next stage"
)]
pub(crate) mod cleanup;
pub(crate) mod collapse;
#[allow(
    dead_code,
    reason = "the rebuild that reads the chains is the next stage"
)]
pub(crate) mod features;
pub(crate) mod layout;
#[allow(
    dead_code,
    reason = "the rebuild that relaxes the seeds is the next stage"
)]
pub(crate) mod lloyd;
#[allow(
    dead_code,
    reason = "the rebuild that reads the regions is the next stage"
)]
pub(crate) mod partition;
#[allow(
    dead_code,
    reason = "the progressive display that shows these lands in step 4"
)]
pub(crate) mod preview;
pub(crate) mod project;
pub(crate) mod proxy;
#[allow(dead_code, reason = "the collapse that reads these is the next stage")]
pub(crate) mod quadric;
#[allow(
    dead_code,
    reason = "the rebuild that reads the seeds is the next stage"
)]
pub(crate) mod seeds;
pub(crate) mod solve;
pub(crate) mod surface;
// Built and proven against the C++ it was ported from in this step; the solver
// that calls it lands in the next, and takes this allow with it.
#[allow(
    dead_code,
    reason = "the rebuild that reads the field is the next stage"
)]
pub(crate) mod size_field;
pub(crate) mod topology;

pub(crate) use project::Winding;

/// Whether this build can remesh at all.
///
/// Always, now: the rebuild is ordinary safe Rust over the same `glam` the rest
/// of the crate uses, with nothing vendored behind it to be missing. Kept as a
/// function because callers ask, and because `meshopt::available` beside it is
/// genuinely conditional.
pub fn available() -> bool {
    true
}

/// Fewest faces a node may be asked for. Below about this the field has no room
/// to align to anything and the result is noise rather than a low-poly mesh.
const MIN_FACES: u32 = 4;

/// Fewest source triangles a node must have to be worth remeshing. A handful of
/// triangles carries no surface to solve a field over, and the result would be
/// worse than what is already there.
const MIN_INPUT_TRIANGLES: usize = 16;

/// How far from a new corner the source surface may be and still be projected
/// from, as a multiple of the proxy's bounding-sphere radius. Generous on
/// purpose: the retopologized surface sits within a fraction of an edge length
/// of the original, so this only ever rejects a corner the engine put somewhere
/// impossible.
const PROJECTION_RANGE: f32 = 0.25;

/// What one rebuild produced: a polygon soup over its own vertices.
///
/// Triangles today; the offsets table is what lets a quad-dominant topology
/// hand back n-gons later without changing anything that reads this.
pub(crate) struct RemeshOutput {
    pub positions: Vec<Vec3>,
    /// `face_count + 1` starts into `corners`.
    pub face_offsets: Vec<u32>,
    pub corners: Vec<u32>,
}

impl RemeshOutput {
    pub(crate) fn face_count(&self) -> usize {
        self.face_offsets.len().saturating_sub(1)
    }

    pub(crate) fn face(&self, face: usize) -> &[u32] {
        let first = self.face_offsets[face] as usize;
        let end = self.face_offsets[face + 1] as usize;
        &self.corners[first..end]
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.face_count() == 0 || self.positions.is_empty()
    }

    /// Re-check what the bridge handed back before any of it is indexed.
    ///
    /// The bridge validates its own output, but this is the boundary the rest of
    /// the crate trusts: an offset table that is not monotone, or a corner past
    /// the vertex array, would otherwise surface as a panic in the projection
    /// loop with nothing to point at.
    pub(crate) fn validate(&self) -> Result<(), OptError> {
        let Some(&first) = self.face_offsets.first() else {
            return Err(OptError::Remesh(
                "the remesher returned no face table".to_owned(),
            ));
        };
        if first != 0 {
            return Err(OptError::Remesh(
                "the remesher's face table does not start at zero".to_owned(),
            ));
        }
        for window in self.face_offsets.windows(2) {
            if window[1] < window[0] || (window[1] - window[0]) < 3 {
                return Err(OptError::Remesh(
                    "the remesher returned a face with fewer than three corners".to_owned(),
                ));
            }
        }
        if self.face_offsets[self.face_offsets.len() - 1] as usize != self.corners.len() {
            return Err(OptError::Remesh(
                "the remesher's face table does not cover its corners".to_owned(),
            ));
        }
        if let Some(&corner) = self
            .corners
            .iter()
            .find(|&&corner| corner as usize >= self.positions.len())
        {
            return Err(OptError::IndexRange {
                index: corner,
                vertex_count: self.positions.len(),
            });
        }
        Ok(())
    }
}

/// One node's measured input to the density split.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BudgetInput {
    pub node: u32,
    /// The node's triangle count as the stack has left it.
    pub triangles: usize,
    /// Its surface area, in world units squared.
    pub area: f32,
    pub params: RemeshParams,
    /// Whether `params` came from this node's own override rather than the
    /// operation's global settings. An overridden node takes exactly what it
    /// asks for and leaves the shared pool, which is the only reading of an
    /// absolute count that lets one object be pinned while the rest share.
    pub overridden: bool,
}

/// What one node is asked to produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NodeBudget {
    pub node: u32,
    pub faces: u32,
}

/// Turn each node's settings into a face count.
///
/// [`RemeshDensity::Ratio`] is per node and needs no coordination: the fraction
/// is read against the node's *triangles* (so it matches the Tris figure on the
/// stats card) and scaled by what one output face is worth in triangles.
///
/// [`RemeshDensity::Absolute`] is one budget for the whole selection, split by
/// surface area — the only split that gives a uniform face size across objects,
/// which is what "3000 faces for this asset" means. A node whose own override
/// names a count takes it verbatim and is removed from the pool first, so
/// pinning one object does not silently re-weight the rest.
pub(crate) fn resolve_budgets(inputs: &[BudgetInput]) -> Vec<NodeBudget> {
    // The shared pool is whatever the non-overridden absolute-density nodes ask
    // for. They all carry the same global settings, so any of them names it.
    let shared: Vec<&BudgetInput> = inputs
        .iter()
        .filter(|input| input.params.density == RemeshDensity::Absolute && !input.overridden)
        .collect();
    let shared_area: f64 = shared.iter().map(|input| input.area.max(0.0) as f64).sum();
    let shared_faces = shared
        .first()
        .map_or(0.0, |input| input.params.faces.max(MIN_FACES) as f64);

    inputs
        .iter()
        .map(|input| {
            let faces = match input.params.density {
                RemeshDensity::Ratio => {
                    let ratio = input.params.ratio.max(0.0) as f64;
                    let worth = input.params.topology.faces_per_triangle() as f64;
                    (input.triangles as f64 * ratio * worth).round()
                }
                RemeshDensity::Absolute if input.overridden => {
                    input.params.faces.max(MIN_FACES) as f64
                }
                // A selection with no area at all (every node degenerate) falls
                // back to an even split rather than dividing by zero.
                RemeshDensity::Absolute if shared_area > 0.0 => {
                    shared_faces * (input.area.max(0.0) as f64 / shared_area)
                }
                RemeshDensity::Absolute => shared_faces / shared.len().max(1) as f64,
            };
            NodeBudget {
                node: input.node,
                faces: faces.clamp(MIN_FACES as f64, u32::MAX as f64) as u32,
            }
        })
        .collect()
}

/// Regenerate every eligible node's surface, replacing its pieces in place.
///
/// Runs over the whole `Vec` rather than one piece at a time — like the AO bake
/// and unlike the per-piece operations — because a node's materials share one
/// surface: the field has to be solved over all of them at once or the seams
/// between them come back as holes.
pub(crate) fn remesh_submeshes(
    submeshes: &mut Vec<Submesh>,
    op: &OpInstance,
    stack: &crate::stack::OptStack,
    model: &ModelData,
    warnings: &mut Warnings,
    run: &crate::process::RunContext<'_>,
) {
    let _z = crate::prof::zone!("Remesh");

    let OpKind::Remesh(global) = &op.kind else {
        return;
    };
    // Nodes in first-seen order, so the output's piece order follows the input's
    // and two runs over the same stack produce the same buffer layout.
    let mut nodes: Vec<u32> = Vec::new();
    for piece in submeshes.iter() {
        if !nodes.contains(&piece.node) {
            nodes.push(piece.node);
        }
    }

    let mut budget_inputs: Vec<BudgetInput> = Vec::new();
    let mut proxies: HashMap<u32, proxy::Proxy> = HashMap::new();
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
        if pieces.iter().any(|piece| piece.has_rows()) {
            warnings.push(&format!(
                "Remesh: '{}' is skinned or has blend shapes, so it was left as it is. \
                 Remeshing rebuilds the surface from scratch, and no skin weight \
                 authored against the old vertices can follow it.",
                node_name(model, node)
            ));
            continue;
        }
        let resolved = crate::process::resolve_op(stack, op, node);
        let params = match resolved {
            OpKind::Remesh(params) => *params,
            // An override can only ever hold the same kind as the operation it
            // overrides; fall back to the global settings if one somehow doesn't.
            _ => *global,
        };
        let proxy = proxy::build(&pieces);
        if proxy.triangle_count() < MIN_INPUT_TRIANGLES {
            warnings.push(&format!(
                "Remesh: '{}' has too few triangles to rebuild ({}), so it was left as it is.",
                node_name(model, node),
                proxy.triangle_count()
            ));
            continue;
        }
        budget_inputs.push(BudgetInput {
            node,
            triangles: proxy.source_triangles,
            area: proxy.area,
            params,
            overridden: !std::ptr::eq(resolved, &op.kind),
        });
        proxies.insert(node, proxy);
    }
    if budget_inputs.is_empty() {
        return;
    }

    let budgets = resolve_budgets(&budget_inputs);

    // Everything each solve needs, gathered before any of them starts: the
    // parallel pass below borrows `submeshes` immutably, and the splice at the
    // end needs it back.
    let jobs: Vec<NodeJob<'_>> = budget_inputs
        .iter()
        .zip(&budgets)
        .filter_map(|(input, budget)| {
            Some(NodeJob {
                node: input.node,
                pieces: submeshes
                    .iter()
                    .filter(|piece| piece.node == input.node && !piece.is_empty())
                    .collect(),
                proxy: proxies.get(&input.node)?,
                faces: budget.faces,
                params: input.params,
                name: node_name(model, input.node),
            })
        })
        .collect();

    // One object per core (see [`crate::parallel`]). Each result is parked in
    // its own slot rather than folded in as it lands, so the warning order and
    // the piece order are the stack's, not the scheduler's.
    let total = jobs.len() as u32;
    let mut outcomes: Vec<Option<NodeOutcome>> = (0..jobs.len()).map(|_| None).collect();
    let mut done = 0u32;
    // The token alone, never the whole context: a worker may ask whether the run
    // is still wanted, but the progress sink posts to the event loop and stays
    // on the thread that owns the scope.
    let token = run.token();
    // What each object may spend inside its own rebuild, so the per-object
    // parallelism and the parallelism *within* an object do not oversubscribe
    // each other: N objects across N cores get one thread each, one object gets
    // the machine.
    let cores = crate::parallel::default_threads();
    let workers = cores.min(jobs.len().max(1));
    let threads = (cores / workers).max(1);
    crate::parallel::solve_nodes(
        &jobs,
        token,
        |job| {
            let mut notes = Warnings::default();
            let rebuilt = rebuild_node(job, &mut notes, token, threads);
            NodeOutcome {
                rebuilt,
                notes: notes.into_vec(),
            }
        },
        |index, outcome| {
            // Reported as each object *lands*: with several in flight there is
            // no single object being waited on any more, so the count is what
            // carries the news, and the name says which one just finished. A
            // solve is minutes on a dense object, so this is what separates
            // "slow" from "wedged" — the question a user staring at the notice
            // actually has.
            done += 1;
            run.report(crate::process::OptProgress::object(
                &op.kind,
                &jobs[index].name,
                done,
                total,
            ));
            outcomes[index] = Some(outcome);
        },
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

    // Splice: a replaced node's new pieces land where its first old piece was,
    // and its other old pieces are dropped. Everything else passes through
    // untouched, in order.
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

/// One node's whole solve, gathered before the parallel pass so a worker
/// touches nothing it shares with another.
struct NodeJob<'a> {
    node: u32,
    pieces: Vec<&'a Submesh>,
    proxy: &'a proxy::Proxy,
    faces: u32,
    params: RemeshParams,
    name: String,
}

/// What one node's solve produced. The warnings ride back with the mesh rather
/// than going into the shared [`Warnings`] as they happen: a worker cannot
/// reach it, and folding them in afterwards in job order is what keeps the list
/// the same on every run.
struct NodeOutcome {
    rebuilt: Option<Vec<Submesh>>,
    notes: Vec<String>,
}

/// One node through the in-house rebuild.
///
/// Unlike the engine path there is no budget search: the face count becomes a
/// vertex count by Euler's formula and the seeds are placed to match, so the
/// number in the box is produced once rather than converged on.
fn rebuild_node(
    job: &NodeJob<'_>,
    warnings: &mut Warnings,
    cancel: Option<&crate::CancelToken>,
    threads: usize,
) -> Option<Vec<Submesh>> {
    let _z = crate::prof::zone!("Remesh Node");
    let (output, report) =
        match solve::rebuild(job.proxy, job.faces, &job.params, threads, cancel, |_| true) {
            Ok(result) => result,
            // A cancelled rebuild is the user having moved on, and the run it
            // belongs to is discarded whole — so it is not worth a warning.
            Err(OptError::Cancelled) => return None,
            Err(error) => {
                warnings.push(&format!(
                    "Remesh: '{}' was left as it is — {error}",
                    job.name
                ));
                return None;
            }
        };
    // A rebuild works by merging the mesh it was given, so it can never hand
    // back more faces than went in. Asking for more is a reasonable thing to
    // try - the engines this replaced *could*, because they built a surface
    // instead of reducing one - so it is worth saying plainly rather than
    // quietly handing back the input.
    if job.faces as usize > job.proxy.source_triangles {
        warnings.push(&format!(
            "Remesh: '{}' already has fewer faces ({}) than the {} asked for, and a rebuild              only ever merges - it cannot add detail that is not there. It came back at              about its current density.",
            job.name, job.proxy.source_triangles, job.faces
        ));
    }

    if output.is_empty() {
        warnings.push(&format!(
            "Remesh: '{}' came back empty. Ask for more faces, or check that the object is              not a handful of disconnected slivers.",
            job.name
        ));
        return None;
    }

    // A region that would not come down to one vertex kept the ones it had, so
    // the result is denser than asked for rather than broken. Worth saying when
    // it is a large share, since the face count is otherwise exact.
    let regions = report.vertices.max(1);
    if report.stubborn * 20 > regions {
        warnings.push(&format!(
            "Remesh: {} parts of '{}' could not be simplified as far as asked without              breaking the surface, so it came back denser there.",
            report.stubborn, job.name
        ));
    }

    let mut output = output;
    layout::canonicalize(&mut output);
    Some(build_pieces_from(
        &output,
        &job.pieces,
        job.proxy,
        project::Winding::Keep,
    ))
}

/// Turn a polygon soup into one [`Submesh`] per material, with every attribute
/// projected from the source surface.
///
/// Shared with [`crate::shrinkwrap`], which produces a soup of its own and needs
/// exactly this: the projection, the per-material split and the weld that turns
/// the corner stream back into an indexed mesh.
pub(crate) fn build_pieces_from(
    output: &RemeshOutput,
    pieces: &[&Submesh],
    proxy: &proxy::Proxy,
    winding: project::Winding,
) -> Vec<Submesh> {
    let source = &project::ProjectionSource::build(pieces);
    let node = pieces.first().map_or(0, |piece| piece.node);
    let uv_channel_count = pieces
        .iter()
        .map(|piece| piece.uv_channels.len())
        .max()
        .unwrap_or(0);
    let color_channel_count = pieces
        .iter()
        .map(|piece| piece.color_channels.len())
        .max()
        .unwrap_or(0);
    let range = proxy.radius() * PROJECTION_RANGE;

    let samples = project::project_faces(
        output,
        source,
        range,
        uv_channel_count,
        color_channel_count,
        winding,
    );

    // One piece per material actually used, in first-seen face order so the
    // output is a deterministic function of the canonicalized soup.
    let mut order: Vec<u32> = Vec::new();
    for face in &samples {
        if !order.contains(&face.material) {
            order.push(face.material);
        }
    }

    let mut built = Vec::with_capacity(order.len());
    for material in order {
        let mut piece = Submesh {
            node,
            material,
            vertices: Vec::new(),
            uv_channels: vec![Vec::new(); uv_channel_count],
            color_channels: vec![Vec::new(); color_channel_count],
            vertex_crease: Vec::new(),
            indices: Vec::new(),
            polygons: Some(PolygonCarry {
                face_offsets: vec![0],
                ..PolygonCarry::default()
            }),
            skin: Default::default(),
            extra_skins: Vec::new(),
            dq_weight: Default::default(),
            morph: Default::default(),
            source_corner: Vec::new(),
        };
        let carry = piece.polygons.as_mut().expect("just built");

        for face in samples.iter().filter(|face| face.material == material) {
            // One vertex per corner, contiguous and in face order — the import's
            // own layout, and what `layout::corner_run` later re-establishes at
            // the level scale. The weld below is what turns it back into a
            // shared-vertex mesh.
            let base = piece.vertices.len() as u32;
            for corner in &face.corners {
                piece.vertices.push(Vertex {
                    position: corner.position,
                    normal: corner.normal,
                    uv: corner.uv,
                    // Rebuilt by `assemble` — geometry changed, so any tangent
                    // carried here would describe the old surface.
                    tangent: Vertex::default().tangent,
                    vertex_color: corner.color,
                });
                for (channel, destination) in piece.uv_channels.iter_mut().enumerate() {
                    destination.push(corner.uvs.get(channel).copied().unwrap_or(corner.uv));
                }
                for (channel, destination) in piece.color_channels.iter_mut().enumerate() {
                    destination.push(corner.colors.get(channel).copied().unwrap_or(Vec4::ONE));
                }
            }
            let degree = face.corners.len() as u32;
            carry
                .corners
                .extend((0..degree).map(|corner| base + corner));
            carry.face_offsets.push(carry.corners.len() as u32);
            carry.source_face.push(NO_FACE);

            let local_face = carry.face_count() as u32 - 1;
            for triangle in fan(&face.corners, base) {
                piece.indices.extend_from_slice(&triangle);
                carry.triangle_face.push(local_face);
            }
        }

        // The exact weld merges the corners that projected to the same source
        // point (two faces meeting inside one attribute region) and leaves the
        // ones that did not (a real UV or material seam) split, which is what
        // puts every seam on an output edge.
        if let Some(error) = ops::weld(&mut piece, &WeldParams::default()).err() {
            // A weld failure leaves the piece as an unshared corner mesh, which
            // still draws and exports correctly — only larger.
            debug_assert!(false, "welding a rebuilt piece failed: {error}");
        }
        // A triangle mesh publishes no polygon table: a face table over
        // triangles says nothing the index buffer does not, and carrying one
        // would put the whole level into the corner-run layout for no gain. A
        // quad-dominant topology would want the other answer, and `assemble`
        // still knows how to take it - see `PolygonCarry::rebuilt`.
        piece.polygons = None;
        built.push(piece);
    }
    built
}

/// A polygon's fan triangulation over corners numbered from `base`.
///
/// A quad is split on its shorter diagonal by the caller's corner order — the
/// canonicalization rotates each face to its lowest vertex, so the split is a
/// deterministic function of the face rather than of how the engine emitted it.
fn fan(corners: &[project::CornerSample], base: u32) -> Vec<[u32; 3]> {
    let degree = corners.len();
    if degree < 3 {
        return Vec::new();
    }
    if degree == 4 {
        // Split on the shorter diagonal: the longer one is the more likely to
        // cut outside a concave quad.
        let d02 = corners[0].position.distance_squared(corners[2].position);
        let d13 = corners[1].position.distance_squared(corners[3].position);
        if d13 < d02 {
            return vec![[base + 1, base + 2, base + 3], [base + 1, base + 3, base]];
        }
    }
    (1..degree - 1)
        .map(|corner| [base, base + corner as u32, base + corner as u32 + 1])
        .collect()
}

/// A node's display name for a warning, falling back to its index.
pub(crate) fn node_name(model: &ModelData, node: u32) -> String {
    model
        .nodes
        .get(node as usize)
        .filter(|entry| !entry.name.is_empty())
        .map(|entry| entry.name.clone())
        .unwrap_or_else(|| format!("object {node}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stack::RemeshTopology;

    fn input(node: u32, triangles: usize, area: f32, params: RemeshParams) -> BudgetInput {
        BudgetInput {
            node,
            triangles,
            area,
            params,
            overridden: false,
        }
    }

    #[test]
    fn a_ratio_budget_reads_against_the_node_s_own_triangles() {
        let triangles = RemeshParams {
            topology: RemeshTopology::Triangles,
            ratio: 1.0,
            ..RemeshParams::default()
        };
        let half = RemeshParams {
            ratio: 0.5,
            ..triangles
        };

        let budgets = resolve_budgets(&[
            input(0, 10_000, 1.0, triangles),
            // Nine times the area, to show a ratio does not read it.
            input(1, 10_000, 9.0, half),
        ]);

        assert_eq!(budgets[0].faces, 10_000, "a triangle is worth a triangle");
        assert_eq!(budgets[1].faces, 5_000, "and area is irrelevant to a ratio");
    }

    #[test]
    fn an_absolute_budget_splits_by_area() {
        let params = RemeshParams {
            density: RemeshDensity::Absolute,
            faces: 4_000,
            ..RemeshParams::default()
        };

        let budgets = resolve_budgets(&[input(0, 100, 1.0, params), input(1, 100, 3.0, params)]);

        assert_eq!(budgets[0].faces, 1_000);
        assert_eq!(budgets[1].faces, 3_000);
    }

    #[test]
    fn an_overridden_node_takes_its_count_and_leaves_the_pool() {
        let shared = RemeshParams {
            density: RemeshDensity::Absolute,
            faces: 4_000,
            ..RemeshParams::default()
        };
        let pinned = RemeshParams {
            faces: 999,
            ..shared
        };

        let mut inputs = vec![
            input(0, 100, 1.0, shared),
            input(1, 100, 3.0, shared),
            input(2, 100, 100.0, pinned),
        ];
        inputs[2].overridden = true;
        let budgets = resolve_budgets(&inputs);

        assert_eq!(budgets[2].faces, 999, "the pinned node takes its own count");
        // Its area does not dilute the pool the other two share.
        assert_eq!(budgets[0].faces, 1_000);
        assert_eq!(budgets[1].faces, 3_000);
    }

    #[test]
    fn a_budget_never_falls_below_the_floor() {
        let params = RemeshParams {
            ratio: 0.0,
            ..RemeshParams::default()
        };

        let budgets = resolve_budgets(&[input(0, 10_000, 1.0, params)]);

        assert_eq!(budgets[0].faces, MIN_FACES);
    }
}
