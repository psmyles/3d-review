//! The scene render.
//!
//! The GPU-facing `#[repr(C)]` types kept in lockstep with `review.glsl` live in
//! [`gpu_types`] and are pinned to shdc's generated structs by size assertions
//! (invariant 11). [`pipelines`] builds the programs the scene pass draws with,
//! [`resources`] caches everything derived per model (invariant 3), and [`gpu`] is
//! the renderer itself: the offscreen 2-MRT pass and the composite job that reads it.
//!
//! The Opt workspace's comparison rendering has not been moved onto sokol_gfx yet; it
//! is `src/port_pending/scene_opt.rs` and returns in `mac-port-plan.md` Phase 1
//! step 4's last stages.

mod gpu;
mod gpu_types;
mod pipelines;
mod resources;

pub(crate) use gpu::SceneGpu;
pub(crate) use gpu_types::{InfluenceEntry, MorphEntry, SceneVertex};
