//! Wall-clock for a rebuild, on shapes the size of real assets.
//!
//! Ignored by default: it is a measurement, not a check, and a timing assertion
//! on a shared machine is a flake waiting to happen. Run it with
//! `cargo test --release -p review-optimize --test remesh_timing -- --ignored
//! --nocapture` when the question is "how long does this take now".
//!
//! The assets this used to time (a palm plant, scanned statues of a million
//! triangles and more, a crystal cluster, a tree branch) left the repository and
//! are not coming back, and the fixtures that remain are a thousand triangles at
//! most - nothing to time. So it runs on the synthetic shapes in `shapes/`, at
//! densities that put them in the same range, and on the two real assets that
//! have since come in: a 614k-triangle scan. (The character fixture is skinned,
//! which Remesh leaves alone by design, so there is nothing of it to time.)
#![cfg(has_meshopt)]

mod common;
mod shapes;

use std::time::Instant;

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

    // The leaves are six objects of one size; the pedestal and stones are five
    // of very different sizes (the pedestal is as big as the four stones
    // together), which is where the threads an object is given have to follow
    // the ones still running.
    let mut subjects = vec![
        ("leaves, 0.4M tris", shapes::leaves(128, 256)),
        ("leaves, 1.6M tris", shapes::leaves(256, 512)),
        (
            "pedestal and stones, 0.5M tris",
            shapes::pedestal_and_stones(128, 256),
        ),
    ];
    // A real scan: one dense object, as photogrammetry delivers it.
    if let Some(model) = common::fixture("Johannes_Benk_C.fbx") {
        subjects.push(("scan (Johannes_Benk_C), 0.6M tris", model));
    }
    for (name, model) in subjects {
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
