//! The scene render. The live Direct3D 11 renderer is [`SceneGpu`] in [`d3d`]; the
//! GPU-facing `#[repr(C)]` types it keeps in lockstep with the HLSL shaders live in
//! [`gpu_types`], and the optional `--tracy` GPU timestamp profiler in
//! [`gpu_profiler`].

mod d3d;
mod gpu_profiler;
mod gpu_types;

pub(crate) use d3d::SceneGpu;
pub use gpu_profiler::enable_tracy_gpu;
pub(crate) use gpu_types::SceneVertex;
