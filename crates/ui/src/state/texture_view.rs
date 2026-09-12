//! The Tex viewport's state: the scene texture pool, what is displayed, and the
//! intents the viewport emits back to `app`.
//!
//! The interaction lives in `texture_view.rs` at the crate root; this is the
//! plain data it edits. `ui` owns no GPU state — the image itself is handed to
//! the renderer.

use std::path::PathBuf;
use std::sync::Arc;

use review_render::DecodedImage;

/// A reference to one material's texture slot (the slot is a
/// [`review_render::TextureSlot`] index, `0..7`), used by the Inspector's
/// assign / clear intents. `app` owns the filesystem + decoded-texture pool; the
/// UI only points at the slot (invariant 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureSlotRef {
    pub material: usize,
    pub slot: usize,
}

/// One imported texture in the scene-wide pool, surfaced to the Inspector so it
/// can list the files (with a real thumbnail built from `image`) and offer them
/// in each material property's texture dropdown — and to the Tex viewport for the
/// full-image view + its stats panel. A plain app→UI snapshot value (invariant 2):
/// `app` owns the decode + the pool, the UI only reads this. The `Arc` makes
/// carrying it a refcount bump, not a pixel copy.
#[derive(Debug, Clone)]
pub struct TexturePoolEntry {
    pub path: PathBuf,
    pub image: Arc<DecodedImage>,
    /// Size of the source file on disk in bytes, measured by `app` when it builds
    /// the pool (0 if the file could not be stat'd). Shown in the Tex viewport's
    /// stats panel.
    pub file_size: u64,
}

impl TexturePoolEntry {
    /// Display name of this texture (its file name).
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("<texture>")
            .to_owned()
    }

    /// Uppercased source-file format label from the extension (e.g. `PNG`, `TGA`),
    /// or `—` when the path carries none. Shown in the Tex viewport's stats panel.
    pub fn format_label(&self) -> String {
        self.path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_uppercase())
            .unwrap_or_else(|| "-".to_owned())
    }
}

/// Which channel(s) of the viewed texture the Tex viewport displays. `Rgb` shows
/// the full color (transparency composites over the background fill); a single
/// channel shows that channel replicated as opaque greyscale.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextureChannelView {
    #[default]
    Rgb,
    R,
    G,
    B,
    A,
}

impl TextureChannelView {
    /// In display order (the toolbar radio group), RGB first.
    pub const ALL: [TextureChannelView; 5] = [
        TextureChannelView::Rgb,
        TextureChannelView::R,
        TextureChannelView::G,
        TextureChannelView::B,
        TextureChannelView::A,
    ];

    /// Toolbar segment label.
    pub fn label(self) -> &'static str {
        match self {
            TextureChannelView::Rgb => "RGB",
            TextureChannelView::R => "R",
            TextureChannelView::G => "G",
            TextureChannelView::B => "B",
            TextureChannelView::A => "A",
        }
    }

    /// Byte offset of a single channel within an RGBA8 pixel, or `None` for the
    /// full-RGB view.
    pub fn channel_offset(self) -> Option<usize> {
        match self {
            TextureChannelView::Rgb => None,
            TextureChannelView::R => Some(0),
            TextureChannelView::G => Some(1),
            TextureChannelView::B => Some(2),
            TextureChannelView::A => Some(3),
        }
    }

    /// The channel index the Tex viewport shader reads (`0` RGB, `1..4` R/G/B/A).
    /// Must match `tex.hlsl`'s `channel` switch.
    pub fn shader_index(self) -> u32 {
        match self {
            TextureChannelView::Rgb => 0,
            TextureChannelView::R => 1,
            TextureChannelView::G => 2,
            TextureChannelView::B => 3,
            TextureChannelView::A => 4,
        }
    }
}

/// The background fill drawn behind the viewed texture in the Tex viewport, so an
/// image's transparency reads against a known backdrop.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextureBackground {
    #[default]
    Black,
    White,
    Grey,
    Checker,
}

impl TextureBackground {
    /// In display order (the status-bar radio group).
    pub const ALL: [TextureBackground; 4] = [
        TextureBackground::Black,
        TextureBackground::White,
        TextureBackground::Grey,
        TextureBackground::Checker,
    ];

    /// Single-letter status-bar segment label (Black / White / Grey / Checker).
    pub fn label(self) -> &'static str {
        match self {
            TextureBackground::Black => "B",
            TextureBackground::White => "W",
            TextureBackground::Grey => "G",
            TextureBackground::Checker => "C",
        }
    }
}

/// State backing the Tex viewport: which pooled texture is shown, the channel
/// isolation + background fill, the floating stats toggle, and the pan/zoom view.
/// All plain UI values (invariant 2) — the pixels live in [`UiState::texture_pool`].
#[derive(Debug, Clone)]
pub struct TextureViewState {
    /// Index into [`UiState::texture_pool`] of the viewed texture. Clamped to the
    /// pool each frame; ignored when the pool is empty.
    pub selected: usize,
    pub channel: TextureChannelView,
    pub background: TextureBackground,
    /// Whether the texture stats panel (Format / Dimension / Channels / Bit depth /
    /// File size) is shown — the Tex viewport's analogue of the model-stats overlay.
    pub show_stats: bool,
    /// Screen-point offset of the image center from the viewport center (pan).
    pub pan: egui::Vec2,
    /// Image-pixels → screen-points scale (zoom). 1.0 = one texel per point.
    pub zoom: f32,
    /// Identity (decoded-image `Arc` pointer) the current pan/zoom was fit for; a
    /// mismatch re-fits the image to the viewport (on first show / texture switch /
    /// disk reload). `None` forces a fit on the next frame.
    pub fitted_key: Option<usize>,
    /// A requested animated view change (the zoom-readout toggle / `F` frame
    /// reset), resolved to a concrete pan/zoom target + eased on the next paint.
    /// `None` when nothing is pending.
    pub request: Option<TexViewRequest>,
    /// An in-flight ease of the pan/zoom toward a target. `None` when settled.
    pub transition: Option<TexViewTransition>,
}

impl Default for TextureViewState {
    fn default() -> Self {
        Self {
            selected: 0,
            channel: TextureChannelView::default(),
            background: TextureBackground::default(),
            show_stats: true,
            pan: egui::Vec2::ZERO,
            zoom: 1.0,
            fitted_key: None,
            request: None,
            transition: None,
        }
    }
}

/// A requested animated change to the Tex view, set by the zoom-readout toggle
/// and the `F` / `R` frame reset and consumed by `texture_view` on the next paint
/// (where the viewport rect — needed to compute a fit — is known). Resolving it
/// starts a [`TexViewTransition`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TexViewRequest {
    /// Ease to an absolute zoom, keeping the current pan (the 100% reset).
    Zoom(f32),
    /// Ease to the fitted view (computed from the current viewport).
    Fit,
}

/// An in-flight ease of the Tex view's pan/zoom toward a target, run over
/// [`crate::theme::motion::TEXTURE_ZOOM_ANIM_SECS`]. `start_time` is egui's input
/// time (monotonic seconds) at the ease's start.
#[derive(Debug, Clone, Copy)]
pub struct TexViewTransition {
    pub from_zoom: f32,
    pub to_zoom: f32,
    pub from_pan: egui::Vec2,
    pub to_pan: egui::Vec2,
    pub start_time: f64,
}

/// The Inspector asked to bind a pooled texture to a material slot: `app` looks
/// the decoded image up in its pool and assigns it (auto-detecting the channel
/// routing). The matching "unbind" is [`TextureIntent::Clear`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureAssign {
    pub slot: TextureSlotRef,
    pub path: PathBuf,
}

/// A texture-pool command the Inspector emits (at most one per frame). `app` is
/// the sole applier (invariant 2): all decode / pool / disk-watch work lives
/// there. New texture intents (e.g. the Phase 6 Tex-viewport picks) add a variant
/// here rather than another `Option` field on [`UiOutput`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextureIntent {
    /// "Add textures…" was clicked: open the image picker + import into the pool.
    Import,
    /// A pooled texture was chosen in a property's dropdown: bind it to the slot.
    Assign(TextureAssign),
    /// A property's dropdown was set back to "select texture": revert that slot to
    /// the shader's neutral fallback.
    Clear(TextureSlotRef),
    /// A pooled texture's remove (✕) was clicked: drop it from the pool and unbind
    /// every material slot that referenced it.
    Remove(PathBuf),
}
