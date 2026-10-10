//! The scene render.
//!
//! The GPU-facing `#[repr(C)]` types kept in lockstep with `review.glsl` live in
//! [`gpu_types`] and are pinned to shdc's generated structs by size assertions
//! (invariant 11). [`pipelines`] builds the programs the scene pass draws with,
//! [`resources`] caches everything derived per model (invariant 3), [`gpu`] is the
//! renderer itself (the offscreen 2-MRT pass, the GTAO passes and the composite job
//! that reads them), and [`opt`] is the Opt workspace's comparison layout over the
//! same passes.

mod ao_accum;
mod aud;
mod deform_gpu;
mod draw;
mod draw_lists;
mod gpu;
mod gpu_types;
mod gtao;
mod line_views;
mod opt;
mod overdraw;
mod pipelines;
mod resources;
mod slot;
mod targets;
mod uniforms;
mod uv;

pub(crate) use gpu::SceneGpu;
pub(crate) use gpu_types::{InfluenceEntry, MorphEntry, SceneVertex};
