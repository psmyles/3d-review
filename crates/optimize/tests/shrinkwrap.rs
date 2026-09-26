//! End-to-end checks over the Shrinkwrap operation, on real fixtures.
//!
//! What the unit tests beside the implementation cannot show is the thing the
//! operation exists for: that a real game asset — a pile of overlapping parts
//! that no quad solver will touch — comes back as one closed shell, and that the
//! solver then runs on it.

#![cfg(has_meshopt)]

use review_model::ModelData;
use review_optimize::{OpKind, OptStack, RemeshDensity, RemeshParams, ShrinkwrapParams};

mod common;

use common::{fixture, run};

/// Every fixture this suite loads.
const FIXTURES: [&str; 2] = ["SM_Ammo_Crate_01a.fbx", "monkey.fbx"];

fn wrap_stack(params: ShrinkwrapParams) -> OptStack {
    let mut stack = OptStack::default();
    stack.push_op(OpKind::Shrinkwrap(params));
    stack
}

/// Whether a level's mesh is a closed, consistently oriented surface — the one
/// property the whole operation is for.
///
/// Checked over each *node* separately, since a level holds one shell per
/// wrapped object and the union of two shells is not itself connected. Positions
/// rather than indices, because the level's pieces are split by material and a
/// shell that spans two materials has its shared edge in both.
fn assert_closed_per_node(model: &ModelData, label: &str) {
    use std::collections::{HashMap, HashSet};

    let key = |index: u32| -> Point {
        let position = model.vertices[index as usize].position;
        [
            position.x.to_bits(),
            position.y.to_bits(),
            position.z.to_bits(),
        ]
    };
    let triangle_count = model.indices.len() / 3;
    /// A position, exactly — the identity two coincident level vertices share.
    type Point = [u32; 3];
    /// A directed edge between two positions.
    type Edge = (Point, Point);

    let mut by_node: HashMap<u32, HashSet<Edge>> = HashMap::new();
    for triangle in 0..triangle_count {
        let node = model
            .triangles
            .node
            .get(triangle)
            .copied()
            .unwrap_or_default();
        let corners = &model.indices[triangle * 3..triangle * 3 + 3];
        let edges = by_node.entry(node).or_default();
        for corner in 0..3 {
            let edge = (key(corners[corner]), key(corners[(corner + 1) % 3]));
            assert!(
                edges.insert(edge),
                "{label}: node {node} traverses one edge twice the same way"
            );
        }
    }
    for (node, edges) in &by_node {
        for (a, b) in edges {
            assert!(
                edges.contains(&(*b, *a)),
                "{label}: node {node} has an edge with no face on its other side"
            );
        }
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

#[test]
fn a_kitbashed_prop_comes_back_as_closed_shells() {
    let Some(model) = fixture("SM_Ammo_Crate_01a.fbx") else {
        return;
    };
    let result = run(
        &model,
        &wrap_stack(ShrinkwrapParams {
            resolution: 64,
            ..ShrinkwrapParams::default()
        }),
    );
    let level = result.lod(0).expect("the stack produced a level");

    assert!(
        level.model.stats.triangle_count > 0,
        "the wrap produced geometry"
    );
    assert_closed_per_node(&level.model, "wrapped ammo crate");
    assert!(
        level.model.faces.is_empty(),
        "a wrap emits triangles and publishes no face table"
    );

    // The shell hugs the object: its bounds are the object's, within a few
    // voxels of padding. Measured rather than read off `bounds`, which a source
    // model does not always carry.
    let extent = |mesh: &ModelData| {
        let mut bounds = review_model::Bounds::EMPTY;
        for vertex in &mesh.vertices {
            bounds.include_point(vertex.position);
        }
        bounds
    };
    let before = extent(&model);
    let after = extent(&level.model);
    let slack = before.size().max_element() / 64.0 * 4.0;
    assert!(
        before.min.abs_diff_eq(after.min, slack) && before.max.abs_diff_eq(after.max, slack),
        "the shell is not where the object is: {before:?} -> {after:?}"
    );
}

#[test]
fn a_wrap_below_a_shape_changing_operation_is_advised_against() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let mut stack = OptStack::default();
    stack.push_op(OpKind::Remesh(RemeshParams {
        density: RemeshDensity::Absolute,
        faces: 1_000,
        ..RemeshParams::default()
    }));
    stack.push_op(OpKind::Shrinkwrap(ShrinkwrapParams {
        resolution: 48,
        ..ShrinkwrapParams::default()
    }));

    let result = run(&model, &stack);

    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("Move Shrinkwrap to the top")),
        "{:?}",
        result.warnings
    );
}

#[test]
fn two_wraps_of_the_same_object_produce_the_same_mesh() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let stack = wrap_stack(ShrinkwrapParams {
        resolution: 48,
        ..ShrinkwrapParams::default()
    });

    let first = run(&model, &stack);
    let second = run(&model, &stack);

    let a = first.lod(0).expect("a level");
    let b = second.lod(0).expect("a level");
    assert_eq!(a.model.vertices, b.model.vertices, "vertices differ");
    assert_eq!(a.model.indices, b.model.indices, "index buffers differ");
}
