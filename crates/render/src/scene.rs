//! The scene render.
//!
//! The GPU-facing `#[repr(C)]` types kept in lockstep with `review.glsl` live in
//! [`gpu_types`] and are pinned to shdc's generated structs by size assertions
//! (invariant 11).
//!
//! The renderer itself — `SceneGpu`, its pipelines, its per-model resource cache and
//! the Opt workspace's comparison rendering — has not been moved onto sokol_gfx yet.
//! It is in `src/port_pending/` and returns in `mac-port-plan.md` Phase 1 step 4,
//! stage by stage; see that directory's README.

// The uniform encoders here have no caller until the scene stages land; the structs
// themselves are what the invariant-11 assertions lock against `review.glsl`, which is
// why the module stays compiled.
#[allow(dead_code)]
mod gpu_types;

pub(crate) use gpu_types::{InfluenceEntry, MorphEntry, SceneVertex};
