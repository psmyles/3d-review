//! What the Tex viewport is asked to draw: the background fill and the placed image.
//!
//! Plain value types, host-agnostic — `app` resolves both from the live UI state
//! (invariant 2) and the GPU path in [`gpu`] consumes them. They live apart from that
//! path because the *background* is now the swapchain pass's clear colour rather than
//! a draw of its own (`docs/ARCHITECTURE.md`, Platform decisions: one swapchain
//! pass), so it is read a frame before any Tex GPU resource is touched — and on a
//! frame where none is.

use std::path::PathBuf;
use std::sync::Arc;

use crate::texture::DecodedImage;

mod gpu;

pub(crate) use gpu::TexGpu;

/// The grey solid fill (gamma space, written verbatim to the UNORM backbuffer) —
/// `Color32::from_gray(128)` in the UI theme.
const GREY_FILL: f32 = 128.0 / 255.0;

/// The Tex viewport's background fill (chosen in the status bar), drawn behind the
/// image so transparency reads against a known backdrop. A render-side mirror of the
/// UI's `TextureBackground`; `app` maps one to the other.
#[derive(Debug, Clone, Copy)]
pub enum TexBackground {
    Black,
    White,
    Grey,
    /// The 2-colour transparency checker; `cell_px` is one cell's size in physical
    /// pixels (the UI's cell size scaled by the points-per-pixel factor).
    Checker {
        cell_px: f32,
    },
}

impl TexBackground {
    /// The colour to clear the frame to for this background.
    ///
    /// A solid fill *is* the clear — there is no draw behind the image at all. The
    /// checker still needs one, and clearing to its darker cell means a frame that
    /// somehow skipped the checker draw reads as a dark backdrop rather than as
    /// whatever the last frame left.
    pub(crate) fn clear_color(self) -> [f32; 3] {
        match self {
            Self::Black => [0.0; 3],
            Self::White => [1.0; 3],
            Self::Grey => [GREY_FILL; 3],
            // `from_gray(110)` in the theme — the checker's dark cell.
            Self::Checker { .. } => [110.0 / 255.0; 3],
        }
    }
}

/// One image to draw in the Tex viewport: the decoded pixels (shared by `Arc` from
/// the app-owned pool, keyed by `path` for the GPU cache + a disk-reload identity
/// check), the channel to isolate (0 = RGB, 1..4 = R/G/B/A), and the image rectangle
/// in physical framebuffer pixels (`app` resolves it from the canvas + pan/zoom).
pub struct TexImage {
    pub path: PathBuf,
    pub image: Arc<DecodedImage>,
    pub channel: u32,
    pub min_px: [f32; 2],
    pub size_px: [f32; 2],
}
