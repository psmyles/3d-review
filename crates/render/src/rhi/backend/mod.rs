//! The device + swapchain leaf, one module per OS — the only platform GPU code left
//! in the workspace (`docs/ARCHITECTURE.md`, Platform decisions: platform leaves).
//!
//! sokol_gfx does not own a window. It is handed a device at `sg_setup` (through
//! `sg_environment`) and a render-target view per frame (through `sg_swapchain`),
//! and the shell owns everything around them. That is all these modules are.
//!
//! They are **twins aliased as [`backend`](self), not a trait**: the set of targets
//! is closed and known, so the alias costs nothing at run time and keeps `cfg` out
//! of every shared file above it. Anything added to one must be added to the other.
//! The contract is exactly:
//!
//! * `SWAPCHAIN_FORMAT` — what the backbuffer is created as, and what a swapchain
//!   pass is told to expect. Plain UNORM on both, so the shaders sRGB-encode their
//!   own output (D20).
//! * `Device` — `create()`, `fill_environment(&mut sg::Environment)`,
//!   `supported_sample_counts(color, depth)`; `Send`, so it can be built on the
//!   bring-up thread (D8).
//! * `Swapchain` — `new(&Device, &Window, w, h)`, `size()`, `resize(w, h)`,
//!   `set_scale_factor(f64)`, `acquire(&mut sg::Swapchain) -> bool`,
//!   `present(vsync) -> PresentStatus`.

#[cfg(windows)]
mod d3d11;
#[cfg(target_os = "macos")]
mod metal;

#[cfg(windows)]
pub(crate) use d3d11::*;
#[cfg(target_os = "macos")]
pub(crate) use metal::*;
