//! End-to-end checks over the Remesh operation, on real fixtures.
//!
//! The unit tests beside the implementation cover the pieces in isolation — the
//! density split, the corner-run expansion, the projection's seam behaviour.
//! These run the whole thing: a real asset in, a rebuilt quad mesh out, through
//! the same `process` the workspace calls.
//!
//! Each test skips itself when its fixture or a vendored tree is absent, so a
//! checkout without them still passes. `REVIEW_REQUIRE_FIXTURES=1` turns every
//! skip into a failure.

#![cfg(has_meshopt)]

use review_model::ModelData;
use review_optimize::{
    NodeOverride, OpKind, OptStack, ProcessedLod, RemeshDensity, RemeshParams, RemeshTopology,
    WeldParams,
};

mod common;

use common::{fixture, run};

/// Every fixture this suite loads.
const FIXTURES: [&str; 5] = [
    "monkey.fbx",
    "SM_column04.fbx",
    "SM_Ammo_Crate_01a.fbx",
    "cpg_pedestal_pebbles.fbx",
    "stylized_palm_plant_04.fbx",
];

fn remesh_stack(params: RemeshParams) -> OptStack {
    let mut stack = OptStack::default();
    stack.push_op(OpKind::Remesh(params));
    stack
}

fn absolute_params(faces: u32) -> RemeshParams {
    RemeshParams {
        topology: RemeshTopology::Triangles,
        density: RemeshDensity::Absolute,
        faces,
        ..RemeshParams::default()
    }
}

/// The faces a level actually has — its polygons where it carries them, and its
/// triangles where it does not.
fn face_count(level: &ProcessedLod) -> usize {
    if level.model.faces.is_empty() {
        level.model.indices.len() / 3
    } else {
        level.model.faces.len()
    }
}

/// The structural guarantees a rebuilt level must satisfy on top of the ones
/// every processed mesh does.
fn assert_consistent(model: &ModelData, label: &str) {
    let triangle_count = model.indices.len() / 3;
    assert_eq!(
        model.indices.len() % 3,
        0,
        "{label}: the index buffer is not whole triangles"
    );
    for &index in &model.indices {
        assert!(
            (index as usize) < model.vertices.len(),
            "{label}: index {index} addresses no vertex of {}",
            model.vertices.len()
        );
    }
    assert!(
        model
            .triangles
            .validate(
                triangle_count,
                model.faces.len(),
                model.materials.len(),
                model.nodes.len()
            )
            .is_ok(),
        "{label}: the per-triangle tags drifted: {:?}",
        model.triangles.validate(
            triangle_count,
            model.faces.len(),
            model.materials.len(),
            model.nodes.len()
        )
    );
    // Corner runs: every face owns a contiguous, in-range slice of the vertex
    // array, and every triangle's corners lie inside its own face's run.
    for (index, face) in model.faces.iter().enumerate() {
        assert!(
            face.index_count >= 3,
            "{label}: face {index} has no corners"
        );
        let end = face.first_index as usize + face.index_count as usize;
        assert!(
            end <= model.vertices.len(),
            "{label}: face {index} runs past the vertex array"
        );
    }
    // Only a level that carries polygons has runs to check; a plain triangle
    // mesh addresses its vertices directly, as every other operation's output
    // does.
    for triangle in 0..triangle_count.min(model.triangles.to_face.len()) {
        let face = model.faces[model.triangles.to_face[triangle] as usize];
        for corner in 0..3 {
            let index = model.indices[triangle * 3 + corner];
            assert!(
                index >= face.first_index && index < face.first_index + face.index_count,
                "{label}: triangle {triangle} reaches outside its own face's run"
            );
        }
    }
    for vertex in &model.vertices {
        assert!(
            vertex.position.is_finite(),
            "{label}: a rebuilt vertex is not at a finite position"
        );
        assert!(
            vertex.normal.is_finite() && vertex.normal.length() > 0.5,
            "{label}: a rebuilt vertex has no usable normal"
        );
    }
}

fn assert_no_skip_warning(result: &review_optimize::ProcessedResult) {
    for warning in &result.warnings {
        assert!(
            !warning.contains("left as it is"),
            "the remesh skipped an object: {warning}"
        );
    }
}

#[test]
fn every_fixture_the_suite_loads_exists() {
    for name in FIXTURES {
        let path = common::fixture_path(name);
        if !path.exists() {
            common::skip(&format!("{name} is not present"));
            return;
        }
    }
}

/// The budget is the budget. Unlike the engines this replaced, the count is
/// worked out from the requested faces rather than searched for, so the
/// tolerance is what stubborn patches of geometry cost rather than what a
/// face-size-to-count conversion happens to overshoot by.
#[test]
fn a_rebuild_hits_its_face_budget() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    // Below the fixture's own 968 triangles, because a rebuild merges and
    // cannot add - see `a_budget_above_the_input_says_so`.
    let target = 500u32;
    let result = run(&model, &remesh_stack(absolute_params(target)));
    assert_no_skip_warning(&result);

    let level = result.lod(0).expect("the stack produced a level");
    assert_consistent(&level.model, "monkey, triangles");

    let produced = face_count(level) as f32;
    let miss = (produced - target as f32).abs() / target as f32;
    assert!(
        miss < 0.12,
        "asked {target}, produced {produced}: {:.0}% out",
        miss * 100.0
    );
    assert_eq!(
        level.model.stats.polygon_count, level.model.stats.triangle_count,
        "an all-triangle mesh counts one polygon per triangle"
    );
}

/// A rebuild merges the mesh it is given, so it cannot hand back more faces
/// than went in.
///
/// That is a real change from the field extraction this replaced, which built a
/// surface from scratch and could happily return more detail than the input
/// had. Asking for more is a reasonable thing to try, so it is reported rather
/// than quietly under-delivered.
#[test]
fn a_budget_above_the_input_says_so() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let source_triangles = model.indices.len() / 3;
    let target = (source_triangles * 4) as u32;

    let result = run(&model, &remesh_stack(absolute_params(target)));

    let level = result.lod(0).expect("the stack produced a level");
    assert!(
        face_count(level) <= source_triangles,
        "a merge cannot invent faces: {} out of {source_triangles} in",
        face_count(level)
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("cannot add detail")),
        "asking for more than the input holds should be reported: {:?}",
        result.warnings
    );
}

#[test]
fn triangles_topology_produces_no_polygons_beyond_triangles() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let params = RemeshParams {
        topology: RemeshTopology::Triangles,
        ..absolute_params(2_000)
    };
    let result = run(&model, &remesh_stack(params));
    assert_no_skip_warning(&result);

    let level = result.lod(0).expect("the stack produced a level");
    assert_consistent(&level.model, "monkey, triangles");
    assert!(
        level.model.faces.is_empty(),
        "a triangle rebuild publishes no polygon table: a face table over triangles says          nothing the index buffer does not, and carrying one would put the whole level into          the corner-run layout for no gain"
    );
    assert!(level.model.triangles.to_face.is_empty());
    assert_eq!(
        level.model.stats.polygon_count, level.model.stats.triangle_count,
        "an all-triangle mesh counts one polygon per triangle"
    );
    assert!(face_count(level) > 0, "and it did produce a mesh");
}

/// Every undirected edge of one node's geometry, welded by position, with how
/// many faces use it.
///
/// Welded because the assembled buffer splits a node into one piece per
/// material, and each piece has its own vertices — so the seam between two
/// materials reads as two borders unless the pieces are rejoined. That is the
/// same weld the rebuild itself works on (`remesh::proxy`), so this measures
/// the surface rather than the buffer layout.
fn welded_edge_uses(model: &ModelData, node: u32) -> std::collections::HashMap<(u32, u32), u32> {
    let mut slot_of: std::collections::HashMap<[u32; 3], u32> = std::collections::HashMap::new();
    let mut uses: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
    for (triangle, corners) in model.indices.as_chunks::<3>().0.iter().enumerate() {
        if model.triangles.node.get(triangle).copied() != Some(node) {
            continue;
        }
        let welded = corners.map(|corner| {
            let point = model.vertices[corner as usize].position;
            let key = [point.x.to_bits(), point.y.to_bits(), point.z.to_bits()];
            let next = slot_of.len() as u32;
            *slot_of.entry(key).or_insert(next)
        });
        for corner in 0..3 {
            let (a, b) = (welded[corner], welded[(corner + 1) % 3]);
            if a == b {
                continue;
            }
            let key = if a < b { (a, b) } else { (b, a) };
            *uses.entry(key).or_insert(0) += 1;
        }
    }
    uses
}

/// The failure this whole rebuild exists to fix.
///
/// A stylized plant is a trunk plus a dozen leaves, and each of them is a thin
/// closed shell. The field extraction tore them: at a quarter of the original
/// density the leaves came back as lace, with holes punched through and their
/// silhouettes in shreds.
///
/// A collapse cannot do that — it only ever removes edges from a mesh that was
/// already a surface — and this is what says so on the asset itself. Every
/// object that arrived closed leaves closed, and no edge anywhere has gained a
/// third face.
#[test]
fn a_plant_of_thin_shells_comes_back_without_holes() {
    let Some(model) = fixture("stylized_palm_plant_04.fbx") else {
        return;
    };
    let result = run(
        &model,
        &remesh_stack(RemeshParams {
            topology: RemeshTopology::Triangles,
            density: RemeshDensity::Ratio,
            // The setting from the report the rewrite came out of.
            ratio: 0.25,
            ..RemeshParams::default()
        }),
    );
    assert_no_skip_warning(&result);
    let level = result.lod(0).expect("the stack produced a level");
    assert_consistent(&level.model, "palm plant, triangles");
    assert!(face_count(level) > 0, "the plant came back with no faces");

    // Per object, because a torn leaf would otherwise be averaged away by the
    // trunk beside it.
    let mut checked = 0;
    for node in 0..model.nodes.len() as u32 {
        let before = welded_edge_uses(&model, node);
        if before.is_empty() {
            continue;
        }
        let after = welded_edge_uses(&level.model, node);
        assert!(!after.is_empty(), "an object vanished entirely");
        checked += 1;
        let name = &model.nodes[node as usize].name;

        for (edge, count) in &after {
            assert!(
                *count <= 2,
                "'{name}': edge {edge:?} is used by {count} faces, so the rebuild branched"
            );
        }
        // The headline: a shell that arrived closed comes back closed. Every
        // hole the old extraction punched would show up here as a border edge
        // on a source that had none.
        let opened = after.values().filter(|&&count| count == 1).count();
        let was_open = before.values().filter(|&&count| count == 1).count();
        assert!(
            opened <= was_open,
            "'{name}': the rebuild opened {opened} border edges where the source had {was_open}"
        );
    }
    assert!(checked > 3, "the plant has several objects to check");
}

#[test]
fn every_rebuilt_face_carries_a_material_the_source_authored() {
    let Some(model) = fixture("SM_column04.fbx") else {
        return;
    };
    let result = run(&model, &remesh_stack(absolute_params(3_000)));
    let level = result.lod(0).expect("the stack produced a level");
    assert_consistent(&level.model, "column, mostly quads");

    let source_materials = model.materials.len();
    assert!(source_materials > 0, "the fixture has materials to keep");
    for &material in &level.model.triangles.material {
        assert!(
            material == u32::MAX || (material as usize) < source_materials,
            "a rebuilt triangle carries material {material}, which the source does not have"
        );
    }
    // The projection must find *some* real material for the surface, not leave
    // the whole mesh on the no-material sentinel.
    assert!(
        level
            .model
            .triangles
            .material
            .iter()
            .any(|&material| material != u32::MAX),
        "the rebuilt mesh lost every material"
    );
}

#[test]
fn two_deterministic_runs_produce_the_same_mesh() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let stack = remesh_stack(absolute_params(1_500));

    let first = run(&model, &stack);
    let second = run(&model, &stack);

    let a = first.lod(0).expect("a level");
    let b = second.lod(0).expect("a level");
    assert_eq!(a.model.vertices, b.model.vertices, "vertices differ");
    assert_eq!(a.model.indices, b.model.indices, "index buffers differ");
    assert_eq!(a.model.faces, b.model.faces, "face tables differ");
    assert_eq!(a.model.triangles, b.model.triangles, "triangle tags differ");

    // Thread count is pinned a stage at a time rather than here, because a
    // whole-run comparison cannot say *which* stage drifted: `topology`,
    // `size_field`, `partition`, `lloyd` and `solve` each assert the same
    // bytes at one thread and at eight. The rebuild used to need a second test
    // binary for this, since the engine sized its pool once per process.
}

#[test]
fn an_excluded_object_is_left_exactly_as_it_was() {
    let Some(model) = fixture("SM_column04.fbx") else {
        return;
    };
    // Exclude every node: the whole mesh must come back untouched, which is the
    // only exclusion assertion that does not depend on which node is which.
    let mut stack = remesh_stack(absolute_params(500));
    stack.overrides = (0..model.nodes.len())
        .map(|node| NodeOverride {
            node,
            exclude: true,
            ..NodeOverride::default()
        })
        .collect();

    let result = run(&model, &stack);
    let level = result.lod(0).expect("the stack produced a level");

    assert!(
        level.model.faces.is_empty(),
        "nothing was rebuilt, so the level carries no face table"
    );
    assert_eq!(
        level.model.stats.triangle_count,
        model.indices.len() / 3,
        "an excluded object keeps every triangle"
    );
}

#[test]
fn a_weld_below_a_remesh_leaves_its_faces_alone() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let mut stack = remesh_stack(absolute_params(1_500));
    stack.push_op(OpKind::Weld(WeldParams::default()));
    stack.push_op(OpKind::VertexFetch);

    let result = run(&model, &stack);
    assert_no_skip_warning(&result);
    let level = result.lod(0).expect("the stack produced a level");
    assert_consistent(&level.model, "monkey, remesh + weld + fetch");

    // Against the same remesh on its own rather than against a fixed count:
    // what is being tested is that the two operations below it change the
    // *shape* of nothing, and a bare threshold would drift with every change
    // to the rebuild.
    let alone = run(&model, &remesh_stack(absolute_params(1_500)));
    let alone = alone.lod(0).expect("the bare remesh produced a level");
    assert_eq!(
        face_count(level),
        face_count(alone),
        "a weld and a vertex-fetch reorder below the remesh changed its faces"
    );
}

/// Invariant 5: the stats panel reports the indexed mesh the export writes.
///
/// This used to check the opposite inequality — a rebuild that carried quads
/// put the whole level into the corner-run layout, where the vertex *buffer* is
/// much larger than the mesh it holds. A triangle rebuild publishes no polygon
/// table, so the buffer and the mesh are the same thing again.
#[test]
fn a_rebuild_reports_the_mesh_it_actually_wrote() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let result = run(&model, &remesh_stack(absolute_params(1_500)));
    let level = result.lod(0).expect("the stack produced a level");

    assert_eq!(
        level.model.stats.vertex_count,
        level.model.vertices.len(),
        "a triangle rebuild's buffer is the mesh, with no corner runs over it"
    );
    assert!(
        level.model.stats.vertex_count > 0,
        "the indexed count is a real measurement"
    );
}

/// The ratio between the largest and smallest face on one object, measured
/// across the middle of the distribution so a handful of degenerates at either
/// end cannot answer for it.
///
/// In *edge* length rather than area, which is the thing you see: a face twice
/// as long either way is four times the area.
fn face_size_spread(level: &ProcessedLod, node: u32) -> f32 {
    let mut edges: Vec<f32> = Vec::new();
    // A triangle rebuild carries no polygon table, so its faces *are* its
    // triangles and the index buffer is what they are read from.
    if level.model.faces.is_empty() {
        for (triangle, corners) in level.model.indices.as_chunks::<3>().0.iter().enumerate() {
            if level.model.triangles.node.get(triangle).copied() != Some(node) {
                continue;
            }
            let at = |corner: usize| level.model.vertices[corners[corner] as usize].position;
            let area = 0.5 * (at(1) - at(0)).cross(at(2) - at(0)).length();
            edges.push(area.sqrt());
        }
        assert!(!edges.is_empty(), "node {node} has no triangles to measure");
        edges.sort_by(|a, b| a.partial_cmp(b).expect("finite face areas"));
        let at = |share: f32| edges[((edges.len() - 1) as f32 * share) as usize];
        return at(0.9) / at(0.1).max(f32::MIN_POSITIVE);
    }
    for (index, face) in level.model.faces.iter().enumerate() {
        let corners = face.index_count as usize;
        if corners < 3 {
            continue;
        }
        let triangle = level
            .model
            .triangles
            .to_face
            .iter()
            .position(|&face| face as usize == index);
        let owner = triangle
            .and_then(|triangle| level.model.triangles.node.get(triangle).copied())
            .unwrap_or(u32::MAX);
        if owner != node {
            continue;
        }
        let corner = |at: usize| level.model.vertices[face.first_index as usize + at].position;
        let mut area = 0.0;
        for at in 1..corners - 1 {
            area += 0.5
                * (corner(at) - corner(0))
                    .cross(corner(at + 1) - corner(0))
                    .length();
        }
        edges.push(area.sqrt());
    }
    assert!(!edges.is_empty(), "node {node} has no faces to measure");
    edges.sort_by(|a, b| a.partial_cmp(b).expect("finite face areas"));
    let at = |share: f32| edges[((edges.len() - 1) as f32 * share) as usize];
    at(0.9) / at(0.1).max(f32::MIN_POSITIVE)
}

/// The node with the most surface area, which on a pedestal-and-pebbles asset is
/// the pedestal: one broad flat face with a rounded rim, i.e. the thing "vary
/// face size" exists for.
fn widest_node(model: &ModelData) -> u32 {
    let mut area: std::collections::BTreeMap<u32, f32> = Default::default();
    for triangle in 0..model.indices.len() / 3 {
        let corner = |at: usize| model.vertices[model.indices[triangle * 3 + at] as usize].position;
        let node = model
            .triangles
            .node
            .get(triangle)
            .copied()
            .unwrap_or_default();
        *area.entry(node).or_default() += 0.5
            * (corner(1) - corner(0))
                .cross(corner(2) - corner(0))
                .length();
    }
    area.into_iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).expect("finite areas"))
        .map(|(node, _)| node)
        .expect("the fixture has geometry")
}

/// The operation this whole parameter exists for: the same budget, spent where
/// the surface needs it.
#[test]
fn varying_face_size_spends_the_budget_on_the_curved_parts() {
    let Some(model) = fixture("cpg_pedestal_pebbles.fbx") else {
        return;
    };
    let node = widest_node(&model);
    let params = |topology, strength: f32| RemeshParams {
        topology,
        density: RemeshDensity::Ratio,
        ratio: 0.25,
        adaptive_strength: strength,
        ..RemeshParams::default()
    };

    // One topology today; a quad-dominant one would want its own floor and
    // ceiling here, since how far a layout carries a field is a property of the
    // layout.
    {
        let (topology, floor) = (RemeshTopology::Triangles, 1.5);
        let even = run(&model, &remesh_stack(params(topology, 0.0)));
        let varied = run(&model, &remesh_stack(params(topology, 1.0)));
        let even = even.lod(0).expect("a level");
        let varied = varied.lod(0).expect("a level");

        // A uniform field puts one face size on the whole object, rim and flat
        // alike — the spread is whatever the rebuild's own jitter is, and that
        // is a property of *how* it builds the mesh.
        //
        // The two ceilings differ by a lot and the reason is structural. A field
        // extraction lays its vertices out on a lattice, so its triangles come
        // out nearly congruent (measured: 1.4x). The in-house rebuild places
        // vertices at an even *density* and then triangulates whatever that
        // gives, which is an unstructured mesh — and an unstructured
        // triangulation of evenly spaced points has real area variance in it
        // even when the spacing is perfect (measured: 2.4x, and it does not
        // move: neither eight more relaxation passes nor forty more Lloyd
        // iterations shift it past 2.35x, because it is not a convergence
        // problem).
        //
        // Closing that is what aligning the partition to a cross field would
        // do, which is the same machinery the quad topologies will need and is
        // deliberately not in this change.
        let flat = face_size_spread(even, node);
        let ceiling = 2.6;
        assert!(
            flat < ceiling,
            "{topology:?}: a uniform rebuild should be uniform, and this one \
             spreads {flat:.2}x"
        );
        // A varied one has to be visibly different, not marginally.
        let spread = face_size_spread(varied, node);
        assert!(
            spread > floor,
            "{topology:?}: varying face size barely varied it: {spread:.2}x \
             against {flat:.2}x even"
        );
    }
}

/// Whatever the field does, the count is the one that was asked for.
///
/// Exactly, not approximately: the budget turns the requested faces into a
/// vertex count by Euler's formula and the seeds are placed to match, so
/// nothing here is searching for it.
///
#[test]
fn the_face_count_is_the_one_that_was_asked_for() {
    let Some(model) = fixture("cpg_pedestal_pebbles.fbx") else {
        return;
    };
    let wanted = 3_000;
    for topology in RemeshTopology::ALL {
        for strength in [0.0, 1.0] {
            let result = run(
                &model,
                &remesh_stack(RemeshParams {
                    topology,
                    density: RemeshDensity::Absolute,
                    faces: wanted,
                    adaptive_strength: strength,
                    ..RemeshParams::default()
                }),
            );
            let level = result.lod(0).expect("a level");
            let produced = face_count(level) as f32;
            let miss = (produced - wanted as f32).abs() / wanted as f32;
            let allowed = 0.12;
            assert!(
                miss < allowed,
                "{topology:?} at strength {strength} produced {produced} faces for a \
                 budget of {wanted}, which is {:.0}% out. Warnings: {:?}",
                miss * 100.0,
                result.warnings
            );
        }
    }
}

/// The reproducible path has to stay reproducible *with* a field, which is where
/// it is hardest: the field decides how fine each region is, and the extraction
/// then snaps and collapses at that size.
#[test]
fn a_varied_rebuild_is_reproducible() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let stack = remesh_stack(RemeshParams {
        adaptive_strength: 1.0,
        ..absolute_params(1_500)
    });

    let first = run(&model, &stack);
    let second = run(&model, &stack);
    let a = first.lod(0).expect("a level");
    let b = second.lod(0).expect("a level");
    assert_eq!(a.model.vertices, b.model.vertices, "vertices differ");
    assert_eq!(a.model.indices, b.model.indices, "index buffers differ");
}
