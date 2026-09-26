//! What a quad rebuild actually produces.
//!
//! Three things are worth pinning, and only the first is obvious.
//!
//! **How many of the faces are quads.** A "quad-dominant" rebuild that came back
//! sixty per cent triangles would be a triangle rebuild with extra steps.
//!
//! **How flat they are.** A quad is a planar primitive and two triangles
//! generally are not, so every quad here is a claim that the two halves lie in
//! one plane closely enough for the claim to be harmless. The merge refuses a
//! pair that does not (`remesh::quads::MERGE_WARP`); this measures what got
//! through, from the *assembled level* rather than from inside the merge -
//! which is what makes it worth running, since between the two sit the
//! canonicalization, the fan and the weld, and any of them could have rotated a
//! quad onto its other diagonal.
//!
//! **That the surface did not move.** A quad is two triangles with the edge
//! between them rubbed out, so a level's index buffer must still hold exactly
//! two triangles for every quad and one for every triangle. The merge's own
//! suite pins the stronger form of this - that they are the *same* triangles -
//! because it can compare them directly.

#![cfg(has_meshopt)]

mod common;

use common::fixture;
use review_model::ModelData;
use review_optimize::{
    OpKind, OptStack, ProcessInput, RemeshDensity, RemeshParams, RemeshTopology, process,
};

/// A stand-in for the renderer's `SceneVertex` size.
const VERTEX_SIZE: usize = 80;

/// What one rebuild came back as.
#[derive(Debug, Default, Clone, Copy)]
struct Measured {
    faces: usize,
    quads: usize,
    triangles: usize,
    /// The worst of any quad's warp: how far a corner stands out of the plane
    /// of the other three, as a fraction of the quad's mean edge length.
    worst_warp: f32,
    /// The mean of the same.
    mean_warp: f32,
    /// Corners in the whole level, for the count against the budget.
    indices: usize,
}

impl Measured {
    fn quad_share(&self) -> f32 {
        if self.faces == 0 {
            return 0.0;
        }
        self.quads as f32 / self.faces as f32
    }
}

/// A Remesh stack at one topology and ratio.
fn stack(topology: RemeshTopology, ratio: f32) -> OptStack {
    let mut stack = OptStack::default();
    stack.push_op(OpKind::Remesh(RemeshParams {
        topology,
        density: RemeshDensity::Ratio,
        ratio,
        smooth_iterations: 2,
        sharp_edges: false,
        align_to_boundaries: true,
        adaptive_strength: 1.0,
        ..RemeshParams::default()
    }));
    stack
}

/// Run a stack and count what came out.
fn measure(model: &ModelData, stack: &OptStack) -> Option<Measured> {
    let result = process(ProcessInput {
        model,
        stack,
        render_vertex_size: VERTEX_SIZE,
        hidden_nodes: &[],
        extras: None,
    })
    .ok()?;
    let level = result.lod(0)?;
    Some(count(&level.model))
}

/// The faces of an assembled level, by degree and by how far from flat they are.
///
/// A level that carries polygons is in the **corner-run** layout: face `f` owns
/// `vertices[first_index .. first_index + index_count]` in winding order, so the
/// corners are read straight out of the vertex array rather than through the
/// index buffer.
fn count(model: &ModelData) -> Measured {
    let mut measured = Measured {
        indices: model.indices.len(),
        ..Measured::default()
    };
    if model.faces.is_empty() {
        // A pure triangle level publishes no face table at all - there is
        // nothing in one the index buffer does not already say.
        measured.faces = model.indices.len() / 3;
        measured.triangles = measured.faces;
        return measured;
    }
    let mut warps = Vec::new();
    for face in &model.faces {
        measured.faces += 1;
        match face.index_count {
            3 => measured.triangles += 1,
            4 => {
                measured.quads += 1;
                let at =
                    |corner: u32| model.vertices[(face.first_index + corner) as usize].position;
                // Read on the `0..2` diagonal exactly as written: that is the
                // one the merge built the quad on, and the corner order still
                // says so - canonicalization only rotates a quad by an even
                // number of places, which is the whole point of that rule.
                let corners = [at(0), at(1), at(2), at(3)];
                if let Some(warp) = warp_of(corners) {
                    warps.push(warp);
                }
            }
            _ => {}
        }
    }
    if !warps.is_empty() {
        measured.worst_warp = warps.iter().copied().fold(0.0f32, f32::max);
        measured.mean_warp = warps.iter().sum::<f32>() / warps.len() as f32;
    }
    measured
}

/// How far a quad's corners stand out of each other's plane, over its mean edge
/// length - the same reading `remesh::quads::warp` takes, from the outside.
fn warp_of(corners: [glam::Vec3; 4]) -> Option<f32> {
    let [a, b, c, d] = corners;
    let first = (b - a).cross(c - a);
    let second = (c - a).cross(d - a);
    let smaller = first.length().min(second.length());
    let size = ((b - a).length() + (c - b).length() + (d - c).length() + (a - d).length()) / 4.0;
    if smaller <= 0.0 || size <= 0.0 {
        return None;
    }
    Some(first.dot(d - a).abs() / smaller / size)
}

/// The fixtures this suite reads, from a thin open shell to a chunky closed one.
const FIXTURES: [&str; 4] = [
    "stylized_palm_plant_04.fbx",
    "rock_pillar_03.fbx",
    "cpg_pedestal_pebbles.fbx",
    "SM_column04.fbx",
];

/// The share of faces each fixture must come back as quads, at half density.
///
/// One floor per object rather than one for all of them, because the spread is
/// the finding. What a surface can be paired into quads at all depends on how
/// faceted it is at the density asked for, and `SM_column04` is the end of that
/// range: a hard-surface model whose rebuild is nearly all creases, where most
/// pairs are a genuine angle and a quad across one would be a lie. The organic
/// fixtures sit where a quadrangulation is normally expected to.
///
/// Each floor is the measured figure less about a tenth.
const QUAD_SHARE_FLOOR: [(&str, f32); 4] = [
    ("stylized_palm_plant_04.fbx", 0.73),
    ("rock_pillar_03.fbx", 0.67),
    ("cpg_pedestal_pebbles.fbx", 0.78),
    ("SM_column04.fbx", 0.42),
];

#[test]
fn most_of_a_quad_rebuild_is_quads() {
    for (name, floor) in QUAD_SHARE_FLOOR {
        let Some(model) = fixture(name) else { return };
        let Some(measured) = measure(&model, &stack(RemeshTopology::Quads, 0.5)) else {
            panic!("{name} did not rebuild");
        };

        assert!(
            measured.quad_share() > floor,
            "{name} came back only {:.0}% quads ({} of {} faces), against a floor of {:.0}%",
            measured.quad_share() * 100.0,
            measured.quads,
            measured.faces,
            floor * 100.0
        );
    }
}

#[test]
fn a_quad_is_nearly_flat() {
    for name in FIXTURES {
        let Some(model) = fixture(name) else { return };
        let Some(measured) = measure(&model, &stack(RemeshTopology::Quads, 0.5)) else {
            panic!("{name} did not rebuild");
        };

        // The merge's own guard, read back from the assembled level: a quad past
        // it would mean the merge measured a different pair of triangles from
        // the ones that ended up in the file.
        assert!(
            measured.worst_warp <= 0.401,
            "{name} has a quad whose corner stands {:.2} of an edge out of plane",
            measured.worst_warp
        );
    }
}

#[test]
fn a_quad_is_still_two_triangles_to_the_renderer() {
    // The polygon table and the index buffer describe one mesh, and the whole
    // "a merge does not move anything" claim depends on them agreeing: two
    // triangles per quad, one per triangle, and nothing over.
    for name in FIXTURES {
        let Some(model) = fixture(name) else { return };
        let Some(measured) = measure(&model, &stack(RemeshTopology::Quads, 0.5)) else {
            panic!("{name} did not rebuild");
        };

        assert!(measured.quads > 0, "{name} asked for quads and got none");
        assert_eq!(
            measured.indices / 3,
            measured.quads * 2 + measured.triangles,
            "{name}: {} quads and {} triangles against {} triangles drawn",
            measured.quads,
            measured.triangles,
            measured.indices / 3
        );
    }
}

#[test]
fn asking_for_triangles_still_gives_only_triangles() {
    for name in FIXTURES {
        let Some(model) = fixture(name) else { return };
        let Some(measured) = measure(&model, &stack(RemeshTopology::Triangles, 0.5)) else {
            panic!("{name} did not rebuild");
        };

        assert_eq!(
            measured.quads, 0,
            "{name} asked for triangles and got {} quads",
            measured.quads
        );
        // And publishes no face table at all, which is what the count above
        // falls back on - see `count`.
        assert_eq!(measured.faces, measured.triangles);
    }
}

#[test]
fn a_quad_rebuild_lands_near_the_face_count_asked_for() {
    // `solve::QUAD_TRIANGLE_BUDGET` turns a quad budget into a triangle one, and
    // it is a measured constant rather than a derived one: a quad is two
    // triangles, less however many the merge leaves unpaired. That share is not
    // the same on every object, so the count cannot be exact the way the
    // triangle path's is - and this bound is set by `SM_column04`, which pairs
    // the fewest and so carries the most leftover triangles. The ignored report
    // below prints the figure for every fixture and ratio.
    for name in FIXTURES {
        let Some(model) = fixture(name) else { return };
        let source: usize = model.faces.len().max(model.indices.len() / 3);
        let Some(measured) = measure(&model, &stack(RemeshTopology::Quads, 0.5)) else {
            panic!("{name} did not rebuild");
        };

        // Half the source triangles, halved again because a quad is worth two.
        let asked = (source as f32 * 0.5 * 0.5).round();
        let miss = (measured.faces as f32 - asked).abs() / asked;
        assert!(
            miss < 0.35,
            "{name} asked for {asked} faces and came back with {} ({:.0}% out)",
            measured.faces,
            miss * 100.0
        );
    }
}

/// `cargo test -p review-optimize --test remesh_quads -- --ignored --nocapture`
#[test]
#[ignore = "a measurement, not a threshold"]
fn how_well_a_rebuild_quadrangulates() {
    println!(
        "{:<34} {:>6} {:>6} {:>6} {:>7} {:>8} {:>8}",
        "object", "ratio", "faces", "quads", "share", "mean warp", "worst warp"
    );
    for name in FIXTURES {
        let Some(model) = fixture(name) else { return };
        for ratio in [0.25f32, 0.5, 1.0] {
            let Some(measured) = measure(&model, &stack(RemeshTopology::Quads, ratio)) else {
                continue;
            };
            println!(
                "{name:<34} {ratio:>6.2} {:>6} {:>6} {:>6.1}% {:>9.3} {:>10.3}",
                measured.faces,
                measured.quads,
                measured.quad_share() * 100.0,
                measured.mean_warp,
                measured.worst_warp
            );
        }
    }
}

/// What the face budget actually lands on, which is where
/// `solve::QUAD_TRIANGLE_BUDGET` comes from.
#[test]
#[ignore = "a measurement, not a threshold"]
fn how_close_a_quad_budget_lands() {
    println!(
        "{:<34} {:<9} {:>6} {:>8} {:>8} {:>8}",
        "object", "topology", "ratio", "asked", "got", "out"
    );
    for name in FIXTURES {
        let Some(model) = fixture(name) else { return };
        let source = model.faces.len().max(model.indices.len() / 3);
        for ratio in [0.25f32, 0.5, 1.0] {
            for topology in RemeshTopology::ALL {
                let Some(measured) = measure(&model, &stack(topology, ratio)) else {
                    continue;
                };
                let worth = if topology == RemeshTopology::Quads {
                    0.5
                } else {
                    1.0
                };
                let asked = source as f32 * ratio * worth;
                println!(
                    "{name:<34} {:<9} {ratio:>6.2} {asked:>8.0} {:>8} {:>7.1}%",
                    topology.label(),
                    measured.faces,
                    (measured.faces as f32 - asked) / asked * 100.0
                );
            }
        }
    }
}

/// Does the triangle rebuild itself land on an absolute budget at the finer
/// density the quad path asks it for?
#[test]
#[ignore = "a measurement, not a threshold"]
fn how_close_an_absolute_budget_lands() {
    let Some(model) = fixture("cpg_pedestal_pebbles.fbx") else {
        return;
    };
    for faces in [3_000u32, 5_550] {
        for strength in [0.0f32, 0.5, 1.0] {
            for topology in RemeshTopology::ALL {
                let mut stack = OptStack::default();
                stack.push_op(OpKind::Remesh(RemeshParams {
                    topology,
                    density: RemeshDensity::Absolute,
                    faces,
                    adaptive_strength: strength,
                    ..RemeshParams::default()
                }));
                let Some(measured) = measure(&model, &stack) else {
                    continue;
                };
                println!(
                    "{faces:>6} s{strength:<4} {:<9} -> {:>6} faces ({:>5} quads, {:>5} tris) {:>6.1}%",
                    topology.label(),
                    measured.faces,
                    measured.quads,
                    measured.triangles,
                    (measured.faces as f32 - faces as f32) / faces as f32 * 100.0
                );
            }
        }
    }
}
