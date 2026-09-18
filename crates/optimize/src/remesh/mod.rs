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

use crate::stack::{OpInstance, OpKind, RemeshDensity, RemeshParams, RemeshTopology, WeldParams};
use crate::submesh::{NO_FACE, PolygonCarry, Submesh};
use crate::{OptError, Warnings, ops};

pub(crate) mod layout;
pub(crate) mod manifold;
pub(crate) mod project;
pub(crate) mod proxy;
mod unavailable;

pub(crate) use project::Winding;

/// The engine entry point: the real one when a retopologizer is vendored, the
/// `RemeshUnavailable` stub otherwise. Selected here rather than at each call
/// site so there is exactly one `cfg` for it.
#[cfg(has_instant_meshes)]
use crate::remesh_ffi::run as run_engine;
#[cfg(not(has_instant_meshes))]
use unavailable::run as run_engine;

/// Whether this build can remesh at all — false without
/// `third_party/instant-meshes`, exactly as [`crate::meshopt::available`] is
/// false without meshoptimizer.
pub fn available() -> bool {
    cfg!(has_instant_meshes)
}

/// `RVO_REMESH_ENGINE_INSTANT_MESHES` in `remesh_bridge.h`. Here rather than in
/// [`crate::remesh_ffi`], which does not exist in a build with no vendored tree.
const ENGINE_INSTANT_MESHES: u32 = 0;

/// `RVO_REMESH_ENGINE_QUADRIFLOW`.
const ENGINE_QUADRIFLOW: u32 = 1;

/// Whether this build carries the quad solver behind
/// [`RemeshTopology::PureQuads`]. `third_party/quadriflow` is optional on top of
/// the retopologizer itself, so "Only quads" falls back to "Mostly quads" with a
/// warning rather than disappearing from the menu — which would make a preset
/// that names it silently mean something else.
pub fn pure_quads_available() -> bool {
    cfg!(has_quadriflow)
}

/// Fewest faces a node may be asked for. Below about this the field has no room
/// to align to anything and the result is noise rather than a low-poly mesh.
const MIN_FACES: u32 = 4;

/// Fewest source triangles a node must have to be worth remeshing. A handful of
/// triangles carries no surface to solve a field over, and the result would be
/// worse than what is already there.
const MIN_INPUT_TRIANGLES: usize = 16;

/// How far the face count may land from what was asked for before it is worth
/// asking the engine again.
///
/// Neither engine is *told* a face count — the number becomes a target edge
/// length, and how many faces that turns into is whatever the extraction makes
/// of it. Two things make the answer miss. A varying face size
/// ([`RemeshParams::adaptive_strength`]) pins the count only in prediction: a
/// region the field made fine comes back with more faces than its area bought,
/// because the extraction snaps and collapses at that size too. And "Mostly
/// quads" converts the count as though every face were a quad, while what it
/// emits is quads *and* the triangles at the singularities — measured at 16%
/// over on a sculpted pedestal, uniform field and all.
///
/// So the answer is measured and the request corrected. It is what makes the
/// number in the box the number you get.
const BUDGET_TOLERANCE: f64 = 0.05;

/// How many times a node's solve may be repeated to land inside
/// [`BUDGET_TOLERANCE`]. Most nodes take one or two; the fourth is what the
/// bracketing in [`solve_to_budget`] needs to close on an engine that answers
/// in steps. The best attempt is kept, so stopping early never returns a worse
/// mesh than a shorter budget would have.
///
/// This is the operation's worst case, and it is worth it: a face count 16% out
/// is one the user has to guess around on every slider drag, and guessing costs
/// them more runs than this costs the machine. It is also the *worst* case, not
/// the usual one — a node already inside the tolerance is solved once.
const BUDGET_ATTEMPTS: u32 = 4;

/// How far from a new corner the source surface may be and still be projected
/// from, as a multiple of the proxy's bounding-sphere radius. Generous on
/// purpose: the retopologized surface sits within a fraction of an edge length
/// of the original, so this only ever rejects a corner the engine put somewhere
/// impossible.
const PROJECTION_RANGE: f32 = 0.25;

/// Nothing reaches the engine's input and output types in a build with no
/// vendored retopologizer — but they still have to exist, because the stub in
/// [`unavailable`] must have the same signature as the real call. The same
/// "declared on both paths, reached on one" shape `meshopt::unavailable` has.
#[cfg_attr(not(has_instant_meshes), allow(dead_code))]
/// The proxy mesh handed to the engine: positions and a triangle index buffer,
/// and nothing else.
pub(crate) struct RemeshInput<'a> {
    /// Three floats per vertex.
    pub positions: &'a [f32],
    pub indices: &'a [u32],
}

#[cfg_attr(not(has_instant_meshes), allow(dead_code))]
impl RemeshInput<'_> {
    pub(crate) fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }
}

/// Nothing reaches the engine's input and output types in a build with no
/// vendored retopologizer — but they still have to exist, because the stub in
/// [`unavailable`] must have the same signature as the real call. The same
/// "declared on both paths, reached on one" shape `meshopt::unavailable` has.
#[cfg_attr(not(has_instant_meshes), allow(dead_code))]
/// What to ask the engine for. Mirrors `rvo_remesh_options` in
/// `remesh_bridge.h`; [`crate::remesh_ffi`] is what converts between them.
pub(crate) struct RemeshOptions {
    pub engine: u32,
    pub rosy: u32,
    pub posy: u32,
    pub face_count: u32,
    /// Negative disables crease detection.
    pub crease_angle_deg: f32,
    pub extrinsic: bool,
    pub align_to_boundaries: bool,
    pub smooth_iterations: u32,
    pub pure_quad: bool,
    pub deterministic: bool,
    pub adaptive_strength: f32,
    pub min_cost_flow: bool,
}

/// Nothing reaches the engine's input and output types in a build with no
/// vendored retopologizer — but they still have to exist, because the stub in
/// [`unavailable`] must have the same signature as the real call. The same
/// "declared on both paths, reached on one" shape `meshopt::unavailable` has.
#[cfg_attr(not(has_instant_meshes), allow(dead_code))]
/// What the engine produced: a polygon soup over its own vertices.
pub(crate) struct RemeshOutput {
    pub positions: Vec<Vec3>,
    /// `face_count + 1` starts into `corners`.
    pub face_offsets: Vec<u32>,
    pub corners: Vec<u32>,
}

#[cfg_attr(not(has_instant_meshes), allow(dead_code))]
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
) {
    let _z = crate::prof::zone!("Remesh");

    let OpKind::Remesh(global) = &op.kind else {
        return;
    };
    if !available() {
        warnings.push(&format!(
            "{}: {}",
            op.kind.label(),
            OptError::RemeshUnavailable
        ));
        return;
    }

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

    let mut replacements: HashMap<u32, Vec<Submesh>> = HashMap::new();
    for (input, budget) in budget_inputs.iter().zip(&budgets) {
        let Some(proxy) = proxies.get(&input.node) else {
            continue;
        };
        let pieces: Vec<&Submesh> = submeshes
            .iter()
            .filter(|piece| piece.node == input.node && !piece.is_empty())
            .collect();
        let name = node_name(model, input.node);
        if let Some(rebuilt) =
            remesh_node(&pieces, proxy, budget.faces, &input.params, &name, warnings)
        {
            replacements.insert(input.node, rebuilt);
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

/// Rebuild one node's surface, or `None` when it was left as it is (with a
/// warning already recorded).
fn remesh_node(
    pieces: &[&Submesh],
    proxy: &proxy::Proxy,
    faces: u32,
    params: &RemeshParams,
    name: &str,
    warnings: &mut Warnings,
) -> Option<Vec<Submesh>> {
    let report = manifold::report(&proxy.indices);

    // "Only quads" is a solver over a half-edge structure, so a surface that
    // branches or pinches is not something it can decline politely — it is
    // something it cannot represent. Checked here rather than left to the
    // engine, so the user gets a reason and a mesh instead of a failure.
    let mut topology = params.topology;
    if topology == RemeshTopology::PureQuads {
        if !pure_quads_available() {
            warnings.push(&format!(
                "Remesh: this build has no quad solver, so 'Only quads' fell back to \
                 'Mostly quads' on '{name}'."
            ));
            topology = RemeshTopology::QuadDominant;
        } else if !report.is_manifold() {
            warnings.push(&format!(
                "Remesh: '{name}' is not a manifold mesh ({} edges shared by more than two \
                 faces, {} pinched vertices), so 'Only quads' fell back to 'Mostly quads'. \
                 Add a Shrinkwrap above the Remesh to fuse it into one closed shell first.",
                report.nonmanifold_edges, report.nonmanifold_vertices
            ));
            topology = RemeshTopology::QuadDominant;
        }
    }

    // Advisory for the field extraction, not a gate: Instant Meshes completes on
    // a non-manifold input, it just cannot guarantee the result closes where the
    // input did not. Skipped when a fall-back above already said the same thing
    // in more useful words.
    if params.topology != RemeshTopology::PureQuads && report.nonmanifold_edges > 0 {
        warnings.push(&format!(
            "Remesh: '{name}' is not a manifold mesh ({} edges are shared by more than two \
             faces), so the rebuilt surface may leave holes there.",
            report.nonmanifold_edges
        ));
    }

    let input = RemeshInput {
        positions: &proxy.positions,
        indices: &proxy.indices,
    };
    let mut output = match solve_to_budget(&input, topology, faces, params) {
        Ok(output) => output,
        Err(error) if topology == RemeshTopology::PureQuads => {
            // The solve can fail on geometry that passed the manifold check —
            // `ComputeIndexMap` gives up on layouts it cannot make consistent.
            // One retry through the field extraction, which has no such failure
            // mode, rather than handing back nothing.
            warnings.push(&format!(
                "Remesh: the quad solver could not lay out '{name}' ({}), so it fell back \
                 to 'Mostly quads'.",
                engine_reason(&error)
            ));
            match solve_to_budget(&input, RemeshTopology::QuadDominant, faces, params) {
                Ok(output) => output,
                Err(error) => {
                    warnings.push(&format!("Remesh: '{name}' was left as it is — {error}"));
                    return None;
                }
            }
        }
        Err(error) => {
            warnings.push(&format!("Remesh: '{name}' was left as it is — {error}"));
            return None;
        }
    };
    if output.is_empty() {
        warnings.push(&format!(
            "Remesh: '{name}' came back empty. Ask for more faces, or check that the \
             object is not a handful of disconnected slivers."
        ));
        return None;
    }

    layout::canonicalize(&mut output);
    Some(build_pieces_from(
        &output,
        pieces,
        proxy,
        CarryPolygons::Yes,
        project::Winding::FromSource,
    ))
}

/// Run the engine, and keep running it until the face count is the one that was
/// asked for.
///
/// The engine's own arithmetic decides how many faces a target edge length ends
/// up producing, and nothing on this side can do better than measure the miss
/// and ask again. See [`BUDGET_TOLERANCE`] for why there is a miss at all.
///
/// Asking again is not a matter of scaling the request by the miss, though —
/// that assumes the engine answers proportionally, and the quad solver does
/// not. Measured on one object: 1817 asked gave 1338, so a proportional
/// correction asked 2467 and got 3544, and the next correction landed back
/// under. An integer quad layout moves in steps, and a request between two of
/// them resolves whichever way the solve goes.
///
/// So the correction is damped, and as soon as one attempt has come back short
/// and another long the search **brackets**: every later ask is the midpoint
/// between them, which cannot oscillate. The closest attempt is what comes
/// back, not the last one.
fn solve_to_budget(
    input: &RemeshInput,
    topology: RemeshTopology,
    faces: u32,
    params: &RemeshParams,
) -> Result<RemeshOutput, OptError> {
    let target = faces.max(MIN_FACES) as f64;
    let mut asked = faces.max(MIN_FACES) as f64;
    let mut best: Option<(RemeshOutput, f64)> = None;
    // The largest ask that came back short, and the smallest that came back
    // long. Once both exist the answer is between them.
    let mut short: Option<f64> = None;
    let mut long: Option<f64> = None;

    for attempt in 0..BUDGET_ATTEMPTS {
        let requested = asked.clamp(MIN_FACES as f64, u32::MAX as f64).round();
        let output = run_engine(input, &engine_options(topology, requested as u32, params))?;
        let produced = output.face_count();
        if produced == 0 {
            // Nothing to correct against, and the caller reports an empty result
            // in words the user can act on.
            return Ok(output);
        }
        let produced = produced as f64;
        let miss = (produced - target).abs() / target;
        if best.as_ref().is_none_or(|(_, previous)| miss < *previous) {
            best = Some((output, miss));
        }
        if miss <= BUDGET_TOLERANCE || attempt + 1 == BUDGET_ATTEMPTS {
            break;
        }

        if produced < target {
            short = Some(short.map_or(requested, |previous: f64| previous.max(requested)));
        } else {
            long = Some(long.map_or(requested, |previous: f64| previous.min(requested)));
        }
        let next = match (short, long) {
            (Some(low), Some(high)) => 0.5 * (low + high),
            // Not bracketed yet: move toward the target, but only half as far
            // in logs as a proportional correction would, so an engine that
            // answers in steps is not chased past the step it is on.
            _ => requested * (target / produced).sqrt(),
        };
        if (next - requested).abs() < 1.0 {
            break;
        }
        asked = next;
    }

    best.map(|(output, _)| output)
        .ok_or_else(|| OptError::Remesh("the remesher was never asked for a face count".to_owned()))
}

/// Whether the rebuilt pieces publish a polygon table.
///
/// A field extraction's quads have to: they are the point of the operation, and
/// a `PolygonCarry` marked `rebuilt` is what carries them to the viewport and
/// the export. A [`crate::shrinkwrap`] shell's triangles must not: a face table
/// over triangles says nothing, and publishing one would put the whole level
/// into the corner-run layout for no gain (see `process::assemble`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CarryPolygons {
    Yes,
    No,
}

/// An engine failure in the engine's own words.
///
/// [`OptError::Remesh`]'s `Display` prefixes "the remesher could not rebuild
/// this object", which reads as a contradiction inside a sentence that goes on
/// to say what it did instead.
fn engine_reason(error: &OptError) -> String {
    match error {
        OptError::Remesh(reason) => reason.clone(),
        other => other.to_string(),
    }
}

/// What to ask the engine for, for one topology.
fn engine_options(topology: RemeshTopology, faces: u32, params: &RemeshParams) -> RemeshOptions {
    let (rosy, posy) = topology.rosy_posy();
    RemeshOptions {
        engine: match topology {
            RemeshTopology::PureQuads => ENGINE_QUADRIFLOW,
            _ => ENGINE_INSTANT_MESHES,
        },
        rosy,
        posy,
        face_count: faces.max(MIN_FACES),
        crease_angle_deg: if params.sharp_edges {
            params.crease_angle.clamp(0.0, 180.0)
        } else {
            -1.0
        },
        extrinsic: true,
        align_to_boundaries: params.align_to_boundaries,
        smooth_iterations: params.smooth_iterations,
        // Instant Meshes' own pure-quad pass subdivides everything, which
        // quadruples the face count against the budget the user asked for.
        // "Only quads" is the solver's job, not its.
        pure_quad: false,
        deterministic: params.deterministic,
        adaptive_strength: params.adaptive_strength.clamp(0.0, 1.0),
        min_cost_flow: params.min_cost_flow,
    }
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
    carry_polygons: CarryPolygons,
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
        match carry_polygons {
            CarryPolygons::Yes => {
                if let Some(carry) = &mut piece.polygons {
                    carry.rebuilt = true;
                }
            }
            CarryPolygons::No => piece.polygons = None,
        }
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
        let quads = RemeshParams {
            topology: RemeshTopology::QuadDominant,
            ratio: 1.0,
            ..RemeshParams::default()
        };
        let triangles = RemeshParams {
            topology: RemeshTopology::Triangles,
            ..quads
        };

        let budgets = resolve_budgets(&[
            input(0, 10_000, 1.0, quads),
            input(1, 10_000, 9.0, triangles),
        ]);

        // A quad is worth two triangles, so "100%" of 10 000 triangles is 5 000
        // quads — and area is irrelevant to a ratio.
        assert_eq!(budgets[0].faces, 5_000);
        assert_eq!(budgets[1].faces, 10_000);
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
