//! The [`Renderer`](crate::Renderer) façade's API, split by what it is about.
//!
//! The type itself and the `render_*` entry points stay in the crate root; these
//! are additional `impl Renderer` blocks, which is what lets a 460-line impl be
//! read in two coherent halves without changing a single call site.

mod camera_api;
mod material_api;
