//! Convex Hull on a real part: the object it selects becomes a closed convex
//! shape of at most the vertices it is allowed, and the rest pass through.

#![cfg(has_meshopt)]

use std::collections::HashSet;
use std::path::Path;

use review_model::ModelData;
use review_optimize::{HullParams, OpKind, OptStack};

mod common;

fn link_named(name: &str) -> ModelData {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../experiments/cad-kit/inputs/link.glb");
    let mut model = review_import::load_model(&path).expect("the kit's link reads");
    model.nodes[0].name = name.to_owned();
    model
}

fn hull(params: HullParams) -> OptStack {
    let mut stack = OptStack::default();
    stack.push_op(OpKind::ConvexHull(params));
    stack
}

fn positions(model: &ModelData) -> HashSet<[u32; 3]> {
    model
        .vertices
        .iter()
        .map(|vertex| vertex.position.to_array().map(f32::to_bits))
        .collect()
}

#[test]
fn a_selected_object_becomes_a_hull_within_its_vertex_limit() {
    let model = link_named("link_hull");
    let result = common::run(&model, &hull(HullParams::default()));
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    let level = &result.lods[0].model;
    let corners = positions(level);
    assert_eq!(corners.len(), 32, "the default limit");
    // A closed convex surface of V vertices has 2V - 4 triangles.
    assert_eq!(level.indices.len() / 3, 2 * 32 - 4);
    // Every hull vertex is a point of the part, and every point of the part is
    // inside the hull or close to it (the hull is the furthest points' own).
    let source = positions(&model);
    assert!(corners.iter().all(|corner| source.contains(corner)));
    let bounds = |model: &ModelData| model.bounds.expect("bounds");
    let (part, shell) = (bounds(&model), bounds(level));
    assert!((part.size() - shell.size()).abs().max_element() < 0.02 * part.size().max_element());
}

#[test]
fn a_higher_limit_keeps_more_and_other_objects_pass_through() {
    let model = link_named("link_hull");
    let result = common::run(
        &model,
        &hull(HullParams {
            max_vertices: 64,
            ..HullParams::default()
        }),
    );
    assert_eq!(positions(&result.lods[0].model).len(), 64);

    let plain = link_named("link");
    let result = common::run(&plain, &hull(HullParams::default()));
    assert_eq!(
        result.lods[0].model.indices.len(),
        plain.indices.len(),
        "a name the patterns do not match is left alone"
    );
}
