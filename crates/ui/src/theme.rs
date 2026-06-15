//! Central semantic theme for the UI crate (invariant 8).
//!
//! Every color, pixel size, spacing, font size and opacity used by the overlay
//! lives here under a **semantic** name (`panel_bg`, `selection`, `gizmo_ball`),
//! never a literal at the call site. The only values allowed to stay inline are
//! ones computed at runtime from state (e.g. a per-axis gizmo alpha derived from
//! its view-space depth).
//!
//! Tweak the app's look here: changing a token re-skins every place that reads
//! it. Tokens are grouped into [`color`], [`size`], and [`font`] so a change of
//! one kind doesn't have to scroll past the others.

use egui::Color32;

/// Semantic color tokens. Names describe the *role* a color plays, not its hue,
/// so a re-theme only touches this block.
pub mod color {
    use egui::Color32;

    // ── Surfaces ────────────────────────────────────────────────────────────
    /// Window / top-level panel background.
    pub const WINDOW_BG: Color32 = Color32::from_rgb(33, 33, 33);
    /// Toolbar and status-bar background.
    pub const CHROME_BG: Color32 = Color32::from_rgb(40, 39, 38);
    /// Recessed background for toolbar button groups and idle icon tiles.
    pub const GROUP_BG: Color32 = Color32::from_rgb(18, 19, 22);
    /// egui "extreme" background (text-edit / slider rails).
    pub const EXTREME_BG: Color32 = Color32::from_rgb(16, 17, 19);
    /// Non-interactive widget background.
    pub const WIDGET_NONINTERACTIVE_BG: Color32 = Color32::from_rgb(36, 36, 36);
    /// Open-combo widget background.
    pub const WIDGET_OPEN_BG: Color32 = Color32::from_rgb(41, 43, 48);
    /// Option-panel card background (header + body).
    pub const PANEL_CARD_BG: Color32 = Color32::from_rgb(39, 39, 39);
    /// Translucent fill behind the stats overlay.
    pub const STATS_OVERLAY_BG: Color32 = Color32::from_rgba_premultiplied(20, 22, 25, 130);
    /// Translucent fill behind the axis gizmo while hovered / dragged.
    pub const GIZMO_BG: Color32 = Color32::from_rgba_premultiplied(8, 12, 16, 90);

    // ── Accent / selection ──────────────────────────────────────────────────
    /// Selected / active control fill (toolbar toggles, mode segments).
    pub const ACCENT: Color32 = Color32::from_rgb(66, 103, 163);
    /// egui selection fill.
    pub const SELECTION: Color32 = Color32::from_rgb(68, 107, 177);
    /// egui pressed/active widget fill.
    pub const ACTIVE: Color32 = Color32::from_rgb(64, 105, 171);
    /// egui hovered widget fill (used by egui-native widgets like sliders).
    pub const WIDGET_HOVERED_BG: Color32 = Color32::from_rgb(47, 79, 131);
    /// Outline on a selected swatch / selection stroke.
    pub const SELECTION_STROKE: Color32 = Color32::from_rgb(88, 135, 217);
    /// Hover fill for custom-painted tiles (icon tiles, mode segments).
    pub const HOVER_BG: Color32 = Color32::from_rgb(54, 56, 61);
    /// Reset-button fills (idle / hovered).
    pub const BUTTON_BG: Color32 = Color32::from_rgb(20, 23, 26);
    pub const BUTTON_HOVER_BG: Color32 = Color32::from_rgb(28, 32, 37);

    // ── Text ────────────────────────────────────────────────────────────────
    /// Brightest text (active labels, hovered glyphs).
    pub const TEXT_PRIMARY: Color32 = Color32::WHITE;
    /// Default body text.
    pub const TEXT_BODY: Color32 = Color32::from_rgb(216, 219, 224);
    /// Panel title text.
    pub const TEXT_TITLE: Color32 = Color32::from_gray(224);
    /// Reset-button label.
    pub const TEXT_BUTTON: Color32 = Color32::from_gray(214);
    /// Stats value column.
    pub const TEXT_VALUE: Color32 = Color32::from_gray(232);
    /// Unselected mode-segment label.
    pub const TEXT_SEGMENT_IDLE: Color32 = Color32::from_rgb(198, 207, 218);
    /// Idle icon-tile tint.
    pub const ICON_IDLE: Color32 = Color32::from_gray(230);
    /// Stats label column / muted row labels.
    pub const TEXT_MUTED: Color32 = Color32::from_gray(178);
    /// Control-row label text in option panels.
    pub const TEXT_LABEL: Color32 = Color32::from_gray(180);
    /// Selected option text in a compact combo (stands out by brightness).
    pub const TEXT_COMBO_SELECTED: Color32 = Color32::from_gray(238);
    /// Dimmed unselected option text in a compact combo.
    pub const TEXT_COMBO_DIM: Color32 = Color32::from_gray(150);
    /// Close-glyph tint (idle / hovered).
    pub const ICON_CLOSE_IDLE: Color32 = Color32::from_gray(176);
    pub const ICON_CLOSE_HOVERED: Color32 = Color32::from_gray(235);

    // ── Strokes / dividers ──────────────────────────────────────────────────
    /// Toolbar bottom divider and window stroke.
    pub const DIVIDER: Color32 = Color32::from_gray(58);
    /// Option-panel border.
    pub const PANEL_BORDER: Color32 = Color32::from_gray(66);
    /// Status-bar top border.
    pub const STATUS_BORDER: Color32 = Color32::from_gray(72);
    /// Stats-overlay border.
    pub const STATS_BORDER: Color32 = Color32::from_gray(52);
    /// Reset-button border.
    pub const BUTTON_BORDER: Color32 = Color32::from_gray(62);
    /// Hover backplate behind the panel close glyph.
    pub const CLOSE_HOVER_BG: Color32 = Color32::from_gray(58);
    /// Idle (unselected) swatch outline.
    pub const SWATCH_BORDER: Color32 = Color32::from_gray(28);

    // ── Axis gizmo ──────────────────────────────────────────────────────────
    /// Gizmo axis ball colors.
    pub const GIZMO_AXIS_X: Color32 = Color32::from_rgb(242, 81, 93);
    pub const GIZMO_AXIS_Y: Color32 = Color32::from_rgb(78, 238, 57);
    pub const GIZMO_AXIS_Z: Color32 = Color32::from_rgb(63, 166, 239);
    /// Label glyph painted on a solid gizmo ball.
    pub const GIZMO_LABEL: Color32 = Color32::from_rgb(16, 21, 27);
    /// Reset-view button tint: dim at rest, brightened on hover.
    pub const GIZMO_RESET_IDLE: Color32 = Color32::from_gray(120);
    pub const GIZMO_RESET_HOVERED: Color32 = Color32::from_gray(225);

    // ── Debug-view defaults ─────────────────────────────────────────────────
    // Each default is also a member of its tool's swatch palette below, so a
    // reset lands on a swatch that stays highlighted (the swatch row marks the
    // entry exactly equal to the selected color).
    /// Default face-normal line color.
    pub const FACE_NORMAL_DEFAULT: Color32 = Color32::from_rgb(255, 27, 27);
    /// Default vertex-normal line color.
    pub const VERTEX_NORMAL_DEFAULT: Color32 = Color32::from_rgb(32, 224, 232);
    /// Default wireframe line color.
    pub const WIREFRAME_DEFAULT: Color32 = Color32::WHITE;
    /// Default bounding-box edge color (a distinct amber that reads against the
    /// shaded model and the grid).
    pub const BOUNDING_BOX_DEFAULT: Color32 = Color32::from_rgb(255, 205, 64);

    /// Swatch palette offered for the normal-line color rows. Includes both
    /// normal-view defaults so either can be restored by reset.
    pub const NORMAL_SWATCHES: [Color32; 5] = [
        Color32::WHITE,
        Color32::BLACK,
        VERTEX_NORMAL_DEFAULT,
        Color32::from_rgb(29, 255, 27),
        FACE_NORMAL_DEFAULT,
    ];

    /// Swatch palette offered for the wireframe color row: white (default),
    /// black, red, cyan, green, 60% grey.
    pub const WIREFRAME_SWATCHES: [Color32; 6] = [
        WIREFRAME_DEFAULT,
        Color32::BLACK,
        Color32::from_rgb(255, 27, 27),
        Color32::from_rgb(32, 224, 232),
        Color32::from_rgb(29, 255, 27),
        Color32::from_gray(153),
    ];

    /// Swatch palette offered for the bounding-box color row: amber (default),
    /// white, black, red, cyan, green. The default leads so reset stays
    /// highlighted on its swatch.
    pub const BOUNDING_BOX_SWATCHES: [Color32; 6] = [
        BOUNDING_BOX_DEFAULT,
        Color32::WHITE,
        Color32::BLACK,
        Color32::from_rgb(255, 27, 27),
        Color32::from_rgb(32, 224, 232),
        Color32::from_rgb(29, 255, 27),
    ];
}

/// Pixel sizes, spacings and radii. Raw logical pixels; most are DPI-scaled at
/// use via [`px`]. The option-panel block is intentionally **not** DPI-scaled,
/// matching the original panel chrome.
pub mod size {
    // ── Overlay layout ────────────────────────────────────────────────────
    pub const TOOLBAR_HEIGHT: f32 = 73.0;
    pub const STATUS_BAR_HEIGHT: f32 = 64.0;
    pub const OVERLAY_MARGIN: f32 = 12.0;
    pub const LEFT_PANEL_WIDTH: f32 = 432.0;

    // ── Toolbar groups ────────────────────────────────────────────────────
    pub const TOOLBAR_GROUP_SPACING: f32 = 14.0;
    pub const TOOLBAR_ICON_SIZE: f32 = 42.0;
    pub const TOOLBAR_ICON_GAP: f32 = 3.0;
    pub const TOOLBAR_ICON_PADDING: f32 = 8.0;
    pub const TOOLBAR_CENTER_WIDTH: f32 = 180.0;
    pub const TOOLBAR_RIGHT_WIDTH: f32 = 200.0;
    pub const TOOLBAR_LEFT_WIDTH: f32 = 333.0;
    pub const TOOLBAR_SHADING_GROUP_WIDTH: f32 = 183.0;
    pub const TOOLBAR_DEBUG_GROUP_WIDTH: f32 = 138.0;
    pub const TOOLBAR_SINGLE_ICON_GROUP_WIDTH: f32 = 48.0;
    pub const TOOLBAR_DOUBLE_ICON_GROUP_WIDTH: f32 = 93.0;
    /// Width of a three-icon toolbar group (e.g. bounding box / gizmo / grid).
    pub const TOOLBAR_TRIPLE_ICON_GROUP_WIDTH: f32 = 138.0;
    pub const TOOLBAR_MODE_GROUP_WIDTH: f32 = 180.0;
    pub const TOOLBAR_GROUP_HEIGHT: f32 = 48.0;
    pub const TOOLBAR_GROUP_PADDING: f32 = 3.0;
    /// Corner radius shared by toolbar groups, icon tiles and mode segments.
    pub const TILE_CORNER_RADIUS: f32 = 6.0;
    /// Mode-segment tile size.
    pub const MODE_SEGMENT_WIDTH: f32 = 56.0;
    pub const MODE_SEGMENT_HEIGHT: f32 = 42.0;

    // ── Stats overlay ─────────────────────────────────────────────────────
    pub const STATS_ROW_SPACING: f32 = 3.0;
    pub const STATS_PANEL_WIDTH: f32 = 132.0;
    pub const STATS_PANEL_PAD_X: i8 = 9;
    pub const STATS_PANEL_PAD_Y: i8 = 9;
    /// Inset of the stats overlay from the left and bottom viewport edges.
    pub const STATS_OVERLAY_MARGIN: f32 = 18.0;
    pub const STATS_CORNER_RADIUS: f32 = 6.0;

    // ── Axis gizmo ────────────────────────────────────────────────────────
    pub const GIZMO_SIZE: f32 = 156.0;
    pub const GIZMO_INSET: f32 = 26.0;
    pub const GIZMO_REACH: f32 = 52.0;
    pub const GIZMO_BALL_RADIUS: f32 = 13.0;
    pub const GIZMO_CORNER_RADIUS: f32 = 12.0;
    /// View-space depth above which an axis is treated as pointing at the viewer.
    pub const GIZMO_AXIS_ALIGNED_DEPTH: f32 = 0.99;
    pub const GIZMO_NEG_OPACITY: f32 = 0.05;
    /// Reset-view button: icon size and inset of its center from the gizmo's
    /// bottom-left corner.
    pub const GIZMO_RESET_ICON_SIZE: f32 = 20.0;
    pub const GIZMO_RESET_INSET: f32 = 17.0;

    // ── Option panels (not DPI-scaled) ────────────────────────────────────
    pub const PANEL_HEADER_HEIGHT: f32 = 34.0;
    pub const PANEL_HEADER_PAD_X: f32 = 16.0;
    pub const PANEL_CORNER_RADIUS: u8 = 6;
    pub const PANEL_CONTENT_MARGIN: i8 = 14;
    pub const PANEL_BODY_TOP_MARGIN: i8 = 10;
    pub const PANEL_LABEL_COL_W: f32 = 116.0;
    pub const PANEL_COL_GAP: f32 = 12.0;
    pub const PANEL_ROW_H: f32 = 22.0;
    pub const PANEL_ROW_GAP: f32 = 6.0;
    pub const PANEL_ACTION_GAP: f32 = 10.0;
    pub const PANEL_BUTTON_HEIGHT: f32 = 32.0;
    pub const PANEL_SWATCH_SIZE: f32 = 24.0;
    pub const PANEL_SWATCH_GAP: f32 = 6.0;
    pub const PANEL_TILING_TEXT_W: f32 = 48.0;
    pub const PANEL_COMBO_BUTTON_PAD_Y: f32 = 2.0;
    pub const PANEL_COMBO_POPUP_MAX_H: f32 = 240.0;
    pub const PANEL_COMBO_OPTION_H: f32 = 20.0;
    pub const PANEL_COMBO_OPTION_GAP: f32 = 2.0;
    pub const PANEL_COMBO_OPTION_PAD_Y: f32 = 2.0;
    pub const PANEL_CLOSE_ICON_SIZE: f32 = 20.0;
    /// Corner radius of a color swatch and the close-glyph hover backplate.
    pub const SWATCH_CORNER_RADIUS: f32 = 5.0;
    pub const CLOSE_HOVER_RADIUS: f32 = 5.0;
    /// Corner radius of the full-width reset button.
    pub const BUTTON_CORNER_RADIUS: f32 = 6.0;
    /// Inset of the close-glyph hover backplate inside its hit area.
    pub const CLOSE_HOVER_INSET: f32 = 4.0;

    // ── Stroke widths ─────────────────────────────────────────────────────
    /// Default hairline stroke (dividers, tile outlines, swatch outlines).
    pub const HAIRLINE: f32 = 1.0;
    /// Selected-swatch outline width.
    pub const SELECTION_STROKE_WIDTH: f32 = 2.0;

    // ── Gizmo strokes / interaction ───────────────────────────────────────
    pub const GIZMO_LINE_WIDTH: f32 = 2.5;
    pub const GIZMO_RING_WIDTH: f32 = 2.0;
    pub const GIZMO_BOLD_OFFSET: f32 = 0.6;
    pub const GIZMO_BALL_HOVER_SCALE: f32 = 1.18;
    pub const GIZMO_BALL_HIT_EXPAND: f32 = 4.0;

    // ── egui base spacing ─────────────────────────────────────────────────
    pub const ITEM_SPACING: f32 = 8.0;
    pub const BUTTON_PADDING_X: f32 = 10.0;
    pub const BUTTON_PADDING_Y: f32 = 8.0;
}

/// Font sizes (logical px). The proportional face is Inter, monospace is
/// JetBrains Mono; see [`crate::assets`].
pub mod font {
    pub const STATS: f32 = 12.0;
    pub const GIZMO_LABEL: f32 = 16.0;
    pub const GIZMO_LABEL_NEG: f32 = 15.0;
    pub const MODE_SEGMENT: f32 = 18.0;
    pub const PANEL_TITLE: f32 = 14.5;
    pub const PANEL_LABEL: f32 = 14.0;
    pub const PANEL_BUTTON: f32 = 14.0;
}

/// Apply the app's egui visuals (dark theme) onto `ctx`. Called once per frame at
/// the top of the overlay so the style is always in sync with these tokens.
pub fn apply_visuals(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals.window_fill = color::WINDOW_BG;
    style.visuals.panel_fill = color::WINDOW_BG;
    style.visuals.extreme_bg_color = color::EXTREME_BG;
    style.visuals.widgets.noninteractive.bg_fill = color::WIDGET_NONINTERACTIVE_BG;
    style.visuals.widgets.inactive.bg_fill = color::GROUP_BG;
    style.visuals.widgets.hovered.bg_fill = color::WIDGET_HOVERED_BG;
    style.visuals.widgets.active.bg_fill = color::ACTIVE;
    style.visuals.widgets.open.bg_fill = color::WIDGET_OPEN_BG;
    style.visuals.widgets.inactive.fg_stroke.color = color::TEXT_BODY;
    style.visuals.widgets.hovered.fg_stroke.color = color::TEXT_PRIMARY;
    style.visuals.selection.bg_fill = color::SELECTION;
    style.visuals.selection.stroke = egui::Stroke::new(1.0, color::SELECTION_STROKE);
    style.visuals.window_stroke = egui::Stroke::new(1.0, color::DIVIDER);
    style.spacing.item_spacing = egui::vec2(size::ITEM_SPACING, size::ITEM_SPACING);
    style.spacing.button_padding = egui::vec2(size::BUTTON_PADDING_X, size::BUTTON_PADDING_Y);
    ctx.set_style(style);
}

/// Convert a logical-pixel design value into egui points for the current DPI, so
/// the overlay keeps the same physical size across display scales.
pub fn px(ctx: &egui::Context, value: f32) -> f32 {
    value / ctx.pixels_per_point()
}

/// Linear `[r, g, b, a]` in 0..1 from an egui `Color32` (sRGB bytes). Used to
/// hand UI-chosen colors to the renderer's debug options.
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
