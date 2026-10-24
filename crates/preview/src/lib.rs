//! What a file browser shows of a model: a thumbnail, and an interactive
//! preview — drawn by the viewer's own renderer, with the textures the model
//! names, on a windowless GPU.
//!
//! This is the safe, host-agnostic half of the shell extensions. A shell loads a
//! [`PreviewScene`] (from a path, or from the bytes Explorer hands a thumbnail
//! handler), draws it through a [`Viewport`] of the process's one [`Host`], and
//! moves the camera with an [`Interaction`] fed from its own window's input.
//! Every frame comes back as an [`Image`] — straight-alpha RGBA over no
//! background — which the shell turns into whatever its OS wants: an `HBITMAP`
//! for a thumbnail, pixels blended over the pane's own colour for a preview.
//!
//! Drawing offscreen and reading back is deliberate. A preview pane is a child
//! of a window in another process, there can be two of them in one host, and the
//! host decides when either goes away; a swapchain bound to one of those windows
//! would have to follow all of that. A readback costs a millisecond or two a
//! frame, and a preview draws only while it is being moved.
//!
//! One [`Host`] per process: sokol_gfx is a process singleton, so every
//! viewport lives inside the one host and the shell's lock on it is what
//! serializes the GPU (see [`Host`]).

#![forbid(unsafe_code)]

mod interaction;
mod scene;
mod thumbnail;
mod viewport;

pub use interaction::{Button, Interaction, Modifiers};
pub use review_render::{GpuError, GpuResult};
pub use scene::{LoadError, PreviewScene, SceneSource};
pub use thumbnail::render_thumbnail;
pub use viewport::{Host, HostedViewport, Image, Viewport, ViewportId};
