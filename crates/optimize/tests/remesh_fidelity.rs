//! How far a rebuilt surface sits from the one it was built from.
//!
//! The suite beside this one asks whether the rebuild is *valid* — closed where
//! the source was closed, wound one way, within its face budget. None of that
//! notices a rebuild that is perfectly valid and the wrong shape: a bark strip
//! that comes back narrower so a gap opens against the trunk, a leaf tip cut to
//! a chord. That is what this measures.
//!
//! **Two directions, and they see different faults.** Output vertex to source
//! surface catches a vertex placed off the surface — the collapse is free to put
//! a region's survivor at its quadric's optimum, which on a nearly flat patch can
//! be a long way from anything. Source vertex to output surface catches *lost*
//! shape: a tip that was cut off has no output surface near where it used to be,
//! and nothing in the other direction reports that, because every output vertex
//! is still sitting neatly on the source. The second is the one that matches what
//! a person sees.
//!
//! Distances are in units of the target edge length `h`, so a figure means the
//! same thing at every density: 0.5 h is "half a face away", which is roughly the
//! most a correct rebuild can be off by, since that is where the surface between
//! two output vertices genuinely runs.
//!
//! Reported at P99 rather than max. A max is one stubborn sliver on one object
//! and moves for reasons that have nothing to do with a change under test.

#![cfg(has_meshopt)]

mod common;

use common::fixture;
use review_model::{ModelData, SceneBvh};
use review_optimize::{
    OpKind, OptStack, ProcessInput, RemeshDensity, RemeshParams, RemeshTopology, process,
};

/// A stand-in for the renderer's `SceneVertex` size.
const VERTEX_SIZE: usize = 80;

/// The spread of one set of distances, in units of `h`.
#[derive(Debug, Clone, Copy, Default)]
struct Spread {
    median: f32,
    p99: f32,
    max: f32,
    count: usize,
}

impl Spread {
    /// `distances` is consumed sorted; empty gives every figure zero.
    fn of(distances: &mut [f32]) -> Self {
        if distances.is_empty() {
            return Self::default();
        }
        distances.sort_by(f32::total_cmp);
        let at = |fraction: f64| {
            let last = distances.len() - 1;
            distances[((last as f64 * fraction).round() as usize).min(last)]
        };
        Self {
            median: at(0.5),
            p99: at(0.99),
            max: distances[distances.len() - 1],
            count: distances.len(),
        }
    }
}

impl std::fmt::Display for Spread {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            out,
            "p50 {:.2} p99 {:.2} max {:.2} h ({} pts)",
            self.median, self.p99, self.max, self.count
        )
    }
}

/// The spread of edge lengths: P90 over P10, which is what "even faces" means
/// as a number. A field extraction that lays vertices on a lattice reaches
/// about 1.4x; an unstructured triangulation of evenly spaced points cannot.
fn edge_spread(model: &ModelData) -> f32 {
    let mut lengths = Vec::new();
    for corners in model.indices.as_chunks::<3>().0 {
        let p = corners.map(|c| model.vertices[c as usize].position);
        for at in 0..3 {
            let length = p[at].distance(p[(at + 1) % 3]);
            if length > 0.0 {
                lengths.push(length);
            }
        }
    }
    if lengths.is_empty() {
        return 0.0;
    }
    lengths.sort_by(f32::total_cmp);
    let at = |fraction: f64| lengths[((lengths.len() - 1) as f64 * fraction) as usize];
    at(0.9) / at(0.1).max(f32::MIN_POSITIVE)
}

/// The triangles of `model` owned by `node`, and their total area.
fn node_area(model: &ModelData, node: u32) -> (usize, f64) {
    let mut faces = 0;
    let mut area = 0.0f64;
    for (triangle, corners) in model.indices.as_chunks::<3>().0.iter().enumerate() {
        if model.triangles.node.get(triangle).copied() != Some(node) {
            continue;
        }
        faces += 1;
        let [a, b, c] = corners.map(|corner| model.vertices[corner as usize].position);
        area += f64::from((b - a).cross(c - a).length()) * 0.5;
    }
    (faces, area)
}

/// The vertices of `model` on a *rim* of `node`: where the surface folds back on
/// itself, which on a thin shell is its silhouette.
///
/// Found from the spread of the incident face normals rather than from any one
/// edge's dihedral, because a rim is usually bevelled: the palm's leaves turn
/// their full 180 degrees over two or three rings, ~60 degrees at a time, so no
/// single edge reads as a fold and only the accumulated turn does.
fn rim_vertices(model: &ModelData, node: u32) -> Vec<glam::Vec3> {
    let mut slot: std::collections::HashMap<[u32; 3], u32> = std::collections::HashMap::new();
    let mut normals: Vec<Vec<glam::Vec3>> = Vec::new();
    let mut point_of: Vec<glam::Vec3> = Vec::new();
    for (triangle, corners) in model.indices.as_chunks::<3>().0.iter().enumerate() {
        if model.triangles.node.get(triangle).copied() != Some(node) {
            continue;
        }
        let points = corners.map(|corner| model.vertices[corner as usize].position);
        let normal = (points[1] - points[0]).cross(points[2] - points[0]);
        if normal.length_squared() == 0.0 {
            continue;
        }
        let normal = normal.normalize();
        for point in points {
            let key = [point.x.to_bits(), point.y.to_bits(), point.z.to_bits()];
            let next = slot.len() as u32;
            let at = *slot.entry(key).or_insert(next);
            if at as usize == normals.len() {
                normals.push(Vec::new());
                point_of.push(point);
            }
            normals[at as usize].push(normal);
        }
    }
    // A rim vertex has two incident faces pointing more than 120 degrees apart.
    normals
        .iter()
        .enumerate()
        .filter(|(_, fan)| {
            fan.iter()
                .enumerate()
                .any(|(at, one)| fan[at + 1..].iter().any(|other| one.dot(*other) < -0.5))
        })
        .map(|(at, _)| point_of[at])
        .collect()
}

/// The vertices of `model` that `node`'s triangles actually use.
fn node_vertices(model: &ModelData, node: u32) -> Vec<glam::Vec3> {
    let mut seen = vec![false; model.vertices.len()];
    let mut out = Vec::new();
    for (triangle, corners) in model.indices.as_chunks::<3>().0.iter().enumerate() {
        if model.triangles.node.get(triangle).copied() != Some(node) {
            continue;
        }
        for &corner in corners {
            if !std::mem::replace(&mut seen[corner as usize], true) {
                out.push(model.vertices[corner as usize].position);
            }
        }
    }
    out
}

/// Both directions between one object's source and rebuilt surfaces, in `h`.
///
/// `h` is the edge length the rebuild was aiming for on this object, derived the
/// same way [`review_optimize`]'s solver derives it — from the object's area and
/// the faces it actually produced, so a rebuild that missed its budget is judged
/// against what it built rather than what it was asked for.
fn two_sided(
    source: &ModelData,
    source_bvh: &SceneBvh,
    level: &ModelData,
    level_bvh: &SceneBvh,
    node: u32,
) -> Option<Measured> {
    let (_, area) = node_area(source, node);
    let (faces, built) = node_area(level, node);
    if faces == 0 || area <= 0.0 {
        return None;
    }
    // An equilateral triangle of side `s` covers `s^2 * sqrt(3) / 4`.
    let h = (2.0 * (area / faces as f64 * (1.0f64 / 3.0).sqrt()).sqrt()) as f32;
    if h <= 0.0 || !h.is_finite() {
        return None;
    }
    // Generous: the point of the cap is to reject a miss, not to bound a hit.
    let range = h * 64.0;

    let mut out_to_source: Vec<f32> = node_vertices(level, node)
        .into_iter()
        .map(|point| {
            source_bvh
                .closest_point(source, point, range, |owner| owner == node)
                .map_or(range, |(_, hit)| hit.distance_squared.sqrt())
                / h
        })
        .collect();
    let mut source_to_out: Vec<f32> = node_vertices(source, node)
        .into_iter()
        .map(|point| {
            level_bvh
                .closest_point(level, point, range, |owner| owner == node)
                .map_or(range, |(_, hit)| hit.distance_squared.sqrt())
                / h
        })
        .collect();

    // The silhouette on its own. A strip that comes back narrower loses its rim
    // first, and a handful of rim vertices is invisible in a percentile over
    // every vertex the object has.
    let mut rim_to_out: Vec<f32> = rim_vertices(source, node)
        .into_iter()
        .map(|point| {
            level_bvh
                .closest_point(level, point, range, |owner| owner == node)
                .map_or(range, |(_, hit)| hit.distance_squared.sqrt())
                / h
        })
        .collect();

    Some(Measured {
        out_to_source: Spread::of(&mut out_to_source),
        source_to_out: Spread::of(&mut source_to_out),
        rim_to_out: Spread::of(&mut rim_to_out),
        // Surface area is the bluntest reading of "is this the same shape" and
        // the hardest to argue with: a rebuild that eats a rim loses area, and
        // one that merely re-triangulates does not.
        area_ratio: (built / area) as f32,
        h,
    })
}

/// Everything measured about one object's rebuild.
struct Measured {
    out_to_source: Spread,
    source_to_out: Spread,
    rim_to_out: Spread,
    area_ratio: f32,
    h: f32,
}

/// Run one rebuild and print both directions for every object it touched.
fn report(name: &str, ratio: f32, smoothing: u32) {
    let Some(model) = fixture(name) else {
        return;
    };
    let mut stack = OptStack::default();
    stack.push_op(OpKind::Remesh(RemeshParams {
        topology: RemeshTopology::Triangles,
        density: RemeshDensity::Ratio,
        ratio,
        smooth_iterations: smoothing,
        // The settings the shape loss was reported at: organic asset, so no
        // creases, and the borders followed.
        sharp_edges: false,
        align_to_boundaries: true,
        adaptive_strength: 0.5,
        ..RemeshParams::default()
    }));
    let Ok(result) = process(ProcessInput {
        model: &model,
        stack: &stack,
        render_vertex_size: VERTEX_SIZE,
        hidden_nodes: &[],
        extras: None,
    }) else {
        println!("{name} @ {ratio} smoothing {smoothing}: the run failed");
        return;
    };
    let Some(level) = result.lod(0) else {
        return;
    };

    println!("\n=== {name} @ ratio {ratio}, smoothing {smoothing} ===");
    let source_bvh = SceneBvh::build(&model);
    let level_bvh = SceneBvh::build(&level.model);
    let mut worst_rim = 0.0f32;
    let mut least_area = 1.0f32;
    for node in 0..model.nodes.len() as u32 {
        let Some(measured) = two_sided(&model, &source_bvh, &level.model, &level_bvh, node) else {
            continue;
        };
        worst_rim = worst_rim.max(measured.rim_to_out.p99);
        least_area = least_area.min(measured.area_ratio);
        println!(
            "  {:<20} h {:.4}  area {:>6.1}%\n      out->src  {}\n      src->out  {}\n      rim->out  {}",
            model.nodes[node as usize].name,
            measured.h,
            measured.area_ratio * 100.0,
            measured.out_to_source,
            measured.source_to_out,
            measured.rim_to_out
        );
    }
    println!(
        "  worst rim->out p99: {worst_rim:.2} h, least area kept: {:.1}%, edge spread {:.2}x",
        least_area * 100.0,
        edge_spread(&level.model)
    );
}

/// Run one rebuild and hand back the worst figures over every object.
fn worst(name: &str, ratio: f32, smoothing: u32) -> Option<(f32, f32)> {
    let model = fixture(name)?;
    let mut stack = OptStack::default();
    stack.push_op(OpKind::Remesh(RemeshParams {
        topology: RemeshTopology::Triangles,
        density: RemeshDensity::Ratio,
        ratio,
        smooth_iterations: smoothing,
        sharp_edges: false,
        align_to_boundaries: true,
        adaptive_strength: 0.5,
        ..RemeshParams::default()
    }));
    let result = process(ProcessInput {
        model: &model,
        stack: &stack,
        render_vertex_size: VERTEX_SIZE,
        hidden_nodes: &[],
        extras: None,
    })
    .expect("the stack runs");
    let level = result.lod(0).expect("the stack produced a level");
    let source_bvh = SceneBvh::build(&model);
    let level_bvh = SceneBvh::build(&level.model);

    let mut least_area = 1.0f32;
    let mut worst_rim = 0.0f32;
    let mut checked = 0;
    for node in 0..model.nodes.len() as u32 {
        let Some(measured) = two_sided(&model, &source_bvh, &level.model, &level_bvh, node) else {
            continue;
        };
        checked += 1;
        least_area = least_area.min(measured.area_ratio);
        worst_rim = worst_rim.max(measured.rim_to_out.p99);
    }
    (checked > 0).then_some((least_area, worst_rim))
}

/// A rebuilt object is still the shape it was.
///
/// The thresholds are **measured, then given a margin** — not chosen and then
/// argued for. At the time they were set the plant's worst object kept 96.9 %
/// of its surface area with no tidying and 96.6 % with two rounds, and its rims
/// sat 0.15 h from the rebuilt surface; the stones, which are closed and chunky
/// and have no rims at all, kept 99.9 %.
///
/// Both figures matter and they fail differently. Area catches an object that
/// came back *smaller* — a strip of bark narrowed until a gap opens against the
/// trunk, which is what prompted all of this and what the validity suite beside
/// this one cannot see, since a narrowed strip is a perfectly good mesh. The rim
/// figure catches the same thing earlier and more locally: a silhouette is a few
/// hundred vertices out of fifteen thousand, so it can move a long way before it
/// shows up in an area total.
#[test]
fn a_rebuilt_object_keeps_its_shape() {
    // Thin shells with rims — the hard case, at the density it was reported at.
    for smoothing in [0, 2] {
        let Some((area, rim)) = worst("stylized_palm_plant_04.fbx", 0.25, smoothing) else {
            return;
        };
        assert!(
            area > 0.92,
            "at {smoothing} rounds of tidying an object of the plant kept only {:.1}% of its \
             surface area",
            area * 100.0
        );
        assert!(
            rim < 0.35,
            "at {smoothing} rounds of tidying the plant's rims moved {rim:.2} h"
        );
    }

    // Closed and chunky: nothing here should cost anything at all, and a change
    // that buys the plant its shape back by blunting everything else fails here.
    let Some((area, _)) = worst("cpg_pedestal_pebbles.fbx", 0.25, 2) else {
        return;
    };
    assert!(
        area > 0.97,
        "a stone kept only {:.1}% of its surface area",
        area * 100.0
    );
}

/// What the rebuild currently costs in shape, object by object.
#[test]
#[ignore = "a measurement, not a check"]
fn how_far_a_rebuild_moves_the_surface() {
    for (ratio, smoothing) in [(0.25, 0), (0.3, 0), (0.25, 2)] {
        report("stylized_palm_plant_04.fbx", ratio, smoothing);
    }
    for smoothing in [0, 1, 2, 4] {
        report("SM_column04.fbx", 0.25, smoothing);
    }
    report("cpg_pedestal_pebbles.fbx", 0.25, 0);
    report("cpg_pedestal_pebbles.fbx", 0.25, 2);
    report("stylized_palm_plant_04.fbx", 0.25, 1);
    report("stylized_palm_plant_04.fbx", 0.25, 4);
}

/// How sharply the source's own surfaces fold.
///
/// This decides whether a "fold" feature can see the rims at all. A leaf or a
/// bark strip modelled as a single sharp edge turns through ~180 degrees there
/// and shows up near -1; one bevelled over two or three rings turns 60 degrees at
/// a time and never reads as a fold, in which case holding its silhouette is a
/// job for the projection and the size field instead.
#[test]
#[ignore = "a measurement, not a check"]
fn how_sharply_the_source_folds() {
    let Some(model) = fixture("stylized_palm_plant_04.fbx") else {
        return;
    };
    // Welded by position, because import splits every face corner and an
    // unwelded buffer shares no edges at all.
    let mut slot: std::collections::HashMap<[u32; 3], u32> = std::collections::HashMap::new();
    let mut normals: std::collections::HashMap<(u32, u32), Vec<glam::Vec3>> =
        std::collections::HashMap::new();
    for (triangle, corners) in model.indices.as_chunks::<3>().0.iter().enumerate() {
        let node = model.triangles.node.get(triangle).copied().unwrap_or(0);
        let points = corners.map(|corner| model.vertices[corner as usize].position);
        let normal = (points[1] - points[0]).cross(points[2] - points[0]);
        if normal.length_squared() == 0.0 {
            continue;
        }
        let welded = points.map(|point| {
            let key = [
                point.x.to_bits() ^ node.wrapping_mul(0x9E37_79B9),
                point.y.to_bits(),
                point.z.to_bits(),
            ];
            let next = slot.len() as u32;
            *slot.entry(key).or_insert(next)
        });
        for corner in 0..3 {
            let (a, b) = (welded[corner], welded[(corner + 1) % 3]);
            if a == b {
                continue;
            }
            let key = if a < b { (a, b) } else { (b, a) };
            normals.entry(key).or_default().push(normal.normalize());
        }
    }

    let mut buckets = [0usize; 8];
    let mut interior = 0usize;
    for pair in normals.values() {
        if pair.len() != 2 {
            continue;
        }
        interior += 1;
        let dot = pair[0].dot(pair[1]).clamp(-1.0, 1.0);
        // -1 .. 1 in eight bands; bucket 0 is a fold back on itself.
        let bucket = (((dot + 1.0) * 0.5 * 8.0) as usize).min(7);
        buckets[bucket] += 1;
    }
    println!("interior edges: {interior}");
    for (bucket, count) in buckets.iter().enumerate() {
        let low = -1.0 + bucket as f32 * 0.25;
        println!(
            "  dot {:+.2}..{:+.2} ({:>3.0}..{:>3.0} deg): {:>7} ({:.2}%)",
            low,
            low + 0.25,
            (low.clamp(-1.0, 1.0)).acos().to_degrees(),
            ((low + 0.25).clamp(-1.0, 1.0)).acos().to_degrees(),
            count,
            100.0 * *count as f64 / interior.max(1) as f64
        );
    }
}
