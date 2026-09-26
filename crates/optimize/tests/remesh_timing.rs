//! Wall-clock for a rebuild, on the assets it is pointed at.
//!
//! Ignored by default: it is a measurement, not a check, and a timing assertion
//! on a shared machine is a flake waiting to happen. Run it with
//! `cargo test --release -p review-optimize --test remesh_timing -- --ignored
//! --nocapture` when the question is "how long does this take now".
#![cfg(has_meshopt)]

mod common;

use std::time::Instant;

use common::fixture;
use review_optimize::{
    OpKind, OptStack, ProcessInput, RemeshDensity, RemeshParams, RemeshTopology, process,
};

/// A stand-in for the renderer's `SceneVertex` size.
const VERTEX_SIZE: usize = 80;

#[test]
#[ignore = "a measurement, not a check"]
fn how_long_a_rebuild_takes() {
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    println!("cores: {cores}");

    for name in [
        "stylized_palm_plant_04.fbx",
        "cpg_pedestal_pebbles.fbx",
        "lucy.fbx",
        "xyzrgb_dragon.fbx",
    ] {
        let Some(model) = fixture(name) else {
            continue;
        };
        let triangles = model.indices.len() / 3;

        // Both topologies, because quads are not free: they are the only thing
        // that builds the cross field, which is a smoothing sweep over the whole
        // *input* mesh and so scales with the model rather than the budget.
        for topology in RemeshTopology::ALL {
            let mut stack = OptStack::default();
            stack.push_op(OpKind::Remesh(RemeshParams {
                topology,
                density: RemeshDensity::Ratio,
                ratio: 0.25,
                ..RemeshParams::default()
            }));

            let started = Instant::now();
            let result = process(ProcessInput {
                model: &model,
                stack: &stack,
                render_vertex_size: VERTEX_SIZE,
                hidden_nodes: &[],
                extras: None,
            });
            let elapsed = started.elapsed();

            match result {
                Ok(result) => {
                    let drawn = result
                        .lod(0)
                        .map_or(0, |level| level.model.indices.len() / 3);
                    println!(
                        "{name}: {triangles} tris -> {drawn} drawn as {} in {:.2}s",
                        topology.label(),
                        elapsed.as_secs_f64()
                    );
                    for warning in result.warnings.iter().take(2) {
                        println!("    {warning}");
                    }
                }
                Err(error) => println!("{name}: {triangles} tris -> {error}"),
            }
        }
    }
}
