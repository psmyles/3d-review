//! The remesh, run with the worker pool forced to one thread.
//!
//! Its own binary because the pool is sized once per process from
//! `REVIEW_REMESH_THREADS`, so the only way to exercise a second thread count is
//! a second process. One test, so the variable is set before anything can have
//! read it.
//!
//! What it proves is thread-count independence: this run's mesh has to be
//! byte-identical to the one `tests/remesh.rs` produces from the same stack at
//! the machine's own core count. The two compare through a file under the
//! target directory, so whichever binary runs second does the comparison and
//! neither depends on the order.

#![cfg(all(has_meshopt, has_instant_meshes))]

use review_optimize::{OpKind, OptStack, RemeshDensity, RemeshParams, RemeshTopology};

mod common;

use common::{compare_digest_across_binaries, fixture, geometry_digest, run};

#[test]
fn a_single_threaded_run_produces_the_same_mesh_as_a_parallel_one() {
    // SAFETY: this is the first line of the only test in this binary, so no
    // other thread of this process is running and nothing has yet read the
    // environment. (The pool reads it on first use, which is inside `run`.)
    unsafe { std::env::set_var("REVIEW_REMESH_THREADS", "1") };

    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let mut stack = OptStack::default();
    stack.push_op(OpKind::Remesh(RemeshParams {
        topology: RemeshTopology::QuadDominant,
        density: RemeshDensity::Absolute,
        faces: 1_500,
        ..RemeshParams::default()
    }));

    let result = run(&model, &stack);
    let level = result.lod(0).expect("the stack produced a level");
    assert!(!level.model.faces.is_empty(), "the remesh produced faces");

    compare_digest_across_binaries("remesh_monkey_quads_1500", geometry_digest(level));
}
