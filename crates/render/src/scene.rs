//! The scene render. The live Direct3D 11 renderer is [`SceneGpu`] in [`d3d`]; the
//! GPU-facing `#[repr(C)]` types it keeps in lockstep with the HLSL shaders live in
//! [`gpu_types`]. (The `--tracy` GPU timestamp profiler is D3D11 plumbing and lives
//! in `rhi::gpu_profiler`, per invariant 9.)

mod d3d;
mod gpu_types;
mod opt;
mod pipelines;
mod resources;

pub(crate) use d3d::SceneGpu;
pub(crate) use gpu_types::SceneVertex;
