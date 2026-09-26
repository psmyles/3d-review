//! Central semantic theme for the UI crate (invariant 8).
//!
//! Every color, size, spacing, font size and opacity used by the overlay
//! lives here under a **semantic** name (`panel_bg`, `selection`, `gizmo_ball`),
//! never a literal at the call site. The only values allowed to stay inline are
//! ones computed at runtime from state (e.g. a per-axis gizmo alpha derived from
//! its view-space depth).
//!
//! Tweak the app's look here: changing a token re-skins every place that reads
//! it. Tokens are grouped into [`color`], [`size`], [`font`] and [`motion`] (how
//! long a transient state lasts, how fast a gesture drives its value) so a change
//! of one kind doesn't have to scroll past the others.
//!
//! ## Layout
//!
//! The four token groups are one file each — [`color`], [`size`], [`font`],
//! [`motion`] — and the functions that install and convert them are here.

use egui::Color32;

pub mod color;
pub mod font;
pub mod motion;
pub mod size;

/// Install the app's fonts and visuals onto `ctx`. Called **once** at startup
/// (the visuals are derived only from these constant tokens, never per-frame
/// state, so there's nothing to re-sync each frame — and a per-frame `set_style`
/// would needlessly nudge the on-demand redraw loop, invariant 6).
pub fn init_style(ctx: &egui::Context) {
    crate::assets::install_fonts(ctx);
    apply_visuals(ctx);
}

/// Apply the app's egui style onto `ctx`.
///
/// Essentially **pure egui `Visuals::dark()`** — no surface/accent/spacing
/// overrides. The egui chrome (toolbar / status bar / option panels / side
/// panels / native widgets) uses egui's stock dark theme so every control,
/// visual and layout reads exactly like the egui demo. The app's *own* fonts are
/// still installed separately ([`init_style`] → [`crate::assets::install_fonts`]);
/// the only deliberate divergences from stock egui are the bundled fonts, a
/// slightly smaller `TextStyle::Heading` ([`font::PANEL_HEADING`]), and the
/// rectangular slider grab ([`size::SLIDER_HANDLE_ASPECT_RATIO`], matching the
/// egui.rs demo rather than the library default circle).
///
/// Note: the hand-painted overlays (axis gizmo, stats, dimension labels) and
/// the toolbar / status-bar chrome paint with explicit theme tokens in
/// their own draw code, *not* through this style — so they are unaffected by it
/// and invariant 8 still holds for them.
pub fn apply_visuals(ctx: &egui::Context) {
    let mut style = egui::Style {
        visuals: egui::Visuals::dark(),
        ..Default::default()
    };
    if let Some(heading) = style.text_styles.get_mut(&egui::TextStyle::Heading) {
        *heading = egui::FontId::proportional(font::PANEL_HEADING);
    }
    // Match the egui.rs demo's slim rectangular slider grab (the library default
    // is a `Circle`); the value box keeps our own fixed-width monospace styling.
    style.visuals.handle_shape = egui::style::HandleShape::Rect {
        aspect_ratio: size::SLIDER_HANDLE_ASPECT_RATIO,
    };
    // A tooltip here is where a control explains itself (`widgets::tooltip`), so
    // it is worth reaching sooner than egui's stock half-second.
    style.interaction.tooltip_delay = motion::TOOLTIP_DELAY_SECS;
    // `all_styles_mut` rather than a single-theme setter: the viewer is dark-only,
    // but installing the style under both themes keeps it correct if the OS theme
    // flips mid-session.
    ctx.all_styles_mut(|slot| *slot = style.clone());
}

/// Combined height of the toolbar and status-bar bands in egui points — the
/// chrome that overlays the full-window 3D scene, and so the band `app` keeps a
/// framed model clear of.
///
/// Scale-free, because every token is already in points: egui multiplies by
/// `pixels_per_point` when it rasterizes, so a point is the DPI-independent unit
/// and there is nothing left for us to convert. Ask this rather than adding
/// [`size::TOOLBAR_HEIGHT`] and [`size::STATUS_BAR_HEIGHT`] up at the call site.
pub fn chrome_height() -> f32 {
    size::TOOLBAR_HEIGHT + size::STATUS_BAR_HEIGHT
}

/// `[r, g, b, a]` in 0..1 from an egui `Color32` — a plain `/255` scale of the
/// sRGB-encoded bytes, with **no** sRGB→linear decode. Used to hand UI-chosen
/// colors to the renderer's debug options, whose overlay shader path consumes
/// them as-is.
pub fn color32_to_rgba(color: Color32) -> [f32; 4] {
    [
        color.r() as f32 / 255.0,
        color.g() as f32 / 255.0,
        color.b() as f32 / 255.0,
        color.a() as f32 / 255.0,
    ]
}

/// Return `color` with its alpha set to `opacity` (0..=1) of fully opaque,
/// regardless of the input alpha. Used to fade gizmo rings without dimming hue.
pub fn with_opacity(color: Color32, opacity: f32) -> Color32 {
    let alpha = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

/// The "remove this" mark, on the Opt stack's row buttons and the LOD list's.
///
/// A capital X, not a multiplication sign: it is a mark either way, so it reads
/// the same in every language and needs no catalog entry, but the bundled Inter
/// has no glyph for U+2715 and it came out as a tofu box. Written once here so
/// the two buttons cannot end up with different glyphs.
pub const REMOVE_GLYPH: &str = "X";
