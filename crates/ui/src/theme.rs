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
    /// Toolbar and status-bar background.
    pub const CHROME_BG: Color32 = Color32::from_rgb(40, 39, 38);
    /// Recessed background for toolbar button groups and idle icon tiles.
    pub const GROUP_BG: Color32 = Color32::from_rgb(18, 19, 22);
    /// Translucent fill behind the stats overlay.
    pub const STATS_OVERLAY_BG: Color32 = Color32::from_rgba_premultiplied(20, 22, 25, 130);
    /// Translucent fill behind the axis gizmo while hovered / dragged.
    pub const GIZMO_BG: Color32 = Color32::from_rgba_premultiplied(8, 12, 16, 90);
    /// Translucent pill behind a bounding-box dimension label. The pill keeps the
    /// readout legible over both the dark viewport and a light model surface; the
    /// text itself is tinted by the axis it measures (`DIMENSION_LABEL_{X,Y,Z}`)
    /// so the readout reads like the axis gizmo.
    pub const DIMENSION_LABEL_BG: Color32 = Color32::from_rgba_premultiplied(18, 20, 23, 210);
    pub const DIMENSION_LABEL_X: Color32 = Color32::from_rgb(246, 109, 120);
    pub const DIMENSION_LABEL_Y: Color32 = Color32::from_rgb(120, 226, 96);
    pub const DIMENSION_LABEL_Z: Color32 = Color32::from_rgb(96, 178, 244);

    // ── Accent / selection ──────────────────────────────────────────────────
    /// Selected / active control fill (toolbar toggles, mode segments).
    pub const ACCENT: Color32 = Color32::from_rgb(66, 103, 163);
    /// Outline on a selected swatch / selection stroke.
    pub const SELECTION_STROKE: Color32 = Color32::from_rgb(88, 135, 217);
    /// Viewport selection highlight: the outline drawn around the Outliner-selected
    /// node / material in the 3D scene. A punchy orange so it reads against an
    /// arbitrary model surface and the grey wireframe (Phase 2).
    pub const SELECTION_OUTLINE: Color32 = Color32::from_rgb(255, 140, 35);
    /// Hover fill for custom-painted tiles (icon tiles, mode segments).
    pub const HOVER_BG: Color32 = Color32::from_rgb(54, 56, 61);
    /// Gradient line drawn along the top of a hovered toolbar/status-bar icon tile
    /// that carries a right-click options panel — a quiet affordance hint that the
    /// button has more behind it. Opaque at the center, faded to transparent at
    /// both ends.
    pub const OPTIONS_HINT: Color32 = Color32::from_rgb(120, 226, 96);

    // ── Text ────────────────────────────────────────────────────────────────
    /// Brightest text (active labels, hovered glyphs).
    pub const TEXT_PRIMARY: Color32 = Color32::WHITE;
    /// Default body text.
    pub const TEXT_BODY: Color32 = Color32::from_rgb(216, 219, 224);
    /// Stats value column.
    pub const TEXT_VALUE: Color32 = Color32::from_gray(232);
    /// Unselected mode-segment label.
    pub const TEXT_SEGMENT_IDLE: Color32 = Color32::from_rgb(198, 207, 218);
    /// Idle icon-tile tint.
    pub const ICON_IDLE: Color32 = Color32::from_gray(230);
    /// Stats label column / muted row labels.
    pub const TEXT_MUTED: Color32 = Color32::from_gray(178);
    /// Selected option text in a compact combo (stands out by brightness).
    pub const TEXT_COMBO_SELECTED: Color32 = Color32::from_gray(238);
    /// Dimmed unselected option text in a compact combo.
    pub const TEXT_COMBO_DIM: Color32 = Color32::from_gray(150);

    // ── Strokes / dividers ──────────────────────────────────────────────────
    /// Toolbar bottom divider and window stroke.
    pub const DIVIDER: Color32 = Color32::from_gray(58);
    /// Status-bar top border.
    pub const STATUS_BORDER: Color32 = Color32::from_gray(72);
    /// Stats-overlay border.
    pub const STATS_BORDER: Color32 = Color32::from_gray(52);
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
    /// Default wireframe line color (60% grey).
    pub const WIREFRAME_DEFAULT: Color32 = Color32::from_gray(100);
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

    /// Swatch palette offered for the wireframe color row: 60% grey (default),
    /// black, red, cyan, green, white.
    pub const WIREFRAME_SWATCHES: [Color32; 6] = [
        WIREFRAME_DEFAULT,
        Color32::BLACK,
        Color32::from_rgb(255, 27, 27),
        Color32::from_rgb(32, 224, 232),
        Color32::from_rgb(29, 255, 27),
        Color32::WHITE,
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

    // ── Startup help overlay ────────────────────────────────────────────────
    /// Help-card surface (translucent, like the stats overlay — the viewport
    /// shows through faintly so the modal doesn't black out the scene) and border.
    pub const HELP_CARD_BG: Color32 = Color32::from_rgba_premultiplied(31, 32, 35, 214);
    pub const HELP_CARD_BORDER: Color32 = Color32::from_gray(70);
    /// Section dividers inside the help card.
    pub const HELP_DIVIDER: Color32 = Color32::from_gray(60);
    /// Key-cap tile fill and outline.
    pub const HELP_KEYCAP_BG: Color32 = Color32::from_rgb(24, 25, 28);
    pub const HELP_KEYCAP_BORDER: Color32 = Color32::from_gray(82);
}

/// Pixel sizes, spacings and radii. Raw logical pixels; most are DPI-scaled at
/// use via [`px`]. The option-panel block is intentionally **not** DPI-scaled,
/// matching the original panel chrome.
pub mod size {
    // ── Overlay layout ────────────────────────────────────────────────────
    pub const TOOLBAR_HEIGHT: f32 = 73.0;
    pub const STATUS_BAR_HEIGHT: f32 = 64.0;
    pub const OVERLAY_MARGIN: f32 = 12.0;
    /// Width of the dockable Outliner / Inspector side panels (their default /
    /// also reused for [`SIDE_PANEL_DEFAULT_WIDTH`]). A touch narrower than the
    /// option windows, since their content is lists / a compact property column.
    pub const TOOL_WINDOW_WIDTH: f32 = 300.0;
    /// Minimum height of an Outliner tab (Geometry / Materials), painted as text
    /// with an active-underline rather than a button. In egui **points**, not
    /// logical px: the strip sizes off the ambient button font (see
    /// [`OUTLINER_TAB_PAD_Y`]) so it tracks the stock rows below it, and this is
    /// only the floor that padding is measured against.
    pub const OUTLINER_TAB_HEIGHT: f32 = 34.0;
    /// Breathing room above and below an Outliner tab's label, in points. The
    /// strip's height is the label's row height plus twice this.
    pub const OUTLINER_TAB_PAD_Y: f32 = 9.0;
    /// Thickness of the active Outliner tab's underline accent.
    pub const OUTLINER_TAB_UNDERLINE: f32 = 2.0;
    /// Upper bound the user can drag the Outliner / Inspector side panels out to
    /// (the `width_range` ceiling); they start at [`SIDE_PANEL_DEFAULT_WIDTH`].
    pub const OUTLINER_MAX_WIDTH: f32 = 560.0;

    // ── Toolbar groups ────────────────────────────────────────────────────
    pub const TOOLBAR_GROUP_SPACING: f32 = 14.0;
    pub const TOOLBAR_ICON_SIZE: f32 = 42.0;
    pub const TOOLBAR_ICON_GAP: f32 = 3.0;
    pub const TOOLBAR_ICON_PADDING: f32 = 8.0;
    pub const TOOLBAR_CENTER_WIDTH: f32 = 180.0;
    /// Right toolbar cluster (3D mode): view (183, four icons) + projection (48) +
    /// the Outliner/Inspector window-toggle group (93), with two group spacings
    /// (183 + 14 + 48 + 14 + 93 = 352), plus a little slack. It grows leftward from
    /// the right edge, away from the centered mode segments.
    pub const TOOLBAR_RIGHT_WIDTH: f32 = 365.0;
    /// Two-icon window-toggle group (Outliner / Inspector) in the toolbar right
    /// cluster.
    pub const TOOLBAR_TOOLS_GROUP_WIDTH: f32 = 93.0;
    /// Left toolbar cluster: shading (5) + material (4) + normals (2) groups,
    /// with two group spacings between them (228 + 14 + 183 + 14 + 93).
    pub const TOOLBAR_LEFT_WIDTH: f32 = 532.0;
    /// Five-icon shading group: show-wireframe, wireframe-only, unlit, shaded,
    /// backface-rendering.
    pub const TOOLBAR_SHADING_GROUP_WIDTH: f32 = 228.0;
    /// Four-icon active-material group: source material, UV checker, vertex colors,
    /// buffers (6 padding + 4×42 icons + 3×3 gaps).
    pub const TOOLBAR_MATERIAL_GROUP_WIDTH: f32 = 183.0;
    /// Two-icon normal-debug group: face normals, vertex normals.
    pub const TOOLBAR_NORMALS_GROUP_WIDTH: f32 = 93.0;
    pub const TOOLBAR_SINGLE_ICON_GROUP_WIDTH: f32 = 48.0;
    /// Width of a three-icon toolbar group (e.g. the UV-shading wire / shaded /
    /// islands group).
    pub const TOOLBAR_TRIPLE_ICON_GROUP_WIDTH: f32 = 138.0;
    /// Width of a four-icon toolbar group (the toolbar's bounding box / pivot /
    /// gizmo / grid view group): 6 padding + 4×42 icons + 3×3 gaps.
    pub const TOOLBAR_QUAD_ICON_GROUP_WIDTH: f32 = 183.0;
    /// Width of a five-icon toolbar group (the status bar's Background / IBL / AO /
    /// Tonemapper / Anti-aliasing rendering-quality cluster): 6 padding + 5×42 icons
    /// + 4×3 gaps.
    pub const TOOLBAR_QUINT_ICON_GROUP_WIDTH: f32 = 228.0;
    pub const TOOLBAR_MODE_GROUP_WIDTH: f32 = 180.0;
    /// Width of the UV-set dropdown shown on the right of the toolbar in UV mode.
    pub const TOOLBAR_UV_DROPDOWN_WIDTH: f32 = 200.0;
    pub const TOOLBAR_GROUP_HEIGHT: f32 = 48.0;
    pub const TOOLBAR_GROUP_PADDING: f32 = 3.0;
    /// Corner radius shared by toolbar groups, icon tiles and mode segments.
    pub const TILE_CORNER_RADIUS: f32 = 6.0;
    /// Mode-segment tile size.
    pub const MODE_SEGMENT_WIDTH: f32 = 56.0;
    pub const MODE_SEGMENT_HEIGHT: f32 = 42.0;

    // ── Notifications (egui-notify toasts) ────────────────────────────────
    /// Horizontal inset of the toast stacks from the right screen edge.
    pub const NOTIFICATION_MARGIN_X: f32 = OVERLAY_MARGIN;
    /// Vertical gap between stacked toasts (and the basis for one toast "row").
    pub const NOTIFICATION_SPACING: f32 = 8.0;
    /// Approximate height of one toast row (egui-notify's toast height plus the
    /// inter-toast spacing), used to stack the activity toast above the events.
    pub const NOTIFICATION_ROW: f32 = 34.0 + NOTIFICATION_SPACING;
    /// Bottom inset of the transient *event* toast stack. Clears the status bar at
    /// every display scale: the bar is at most `STATUS_BAR_HEIGHT` points tall, and
    /// this inset is that plus a margin, so the toasts always sit above it.
    pub const NOTIFICATION_EVENT_MARGIN_Y: f32 = STATUS_BAR_HEIGHT + OVERLAY_MARGIN;
    /// Bottom inset of the persistent *activity* toast: one row above the event
    /// stack so the work indicator and result toasts don't overlap.
    pub const NOTIFICATION_ACTIVITY_MARGIN_Y: f32 = NOTIFICATION_EVENT_MARGIN_Y + NOTIFICATION_ROW;

    // ── Material inspector: texture mapping + files ───────────────────────
    /// Fixed label-column width for a Texture-mapping row (property name), so the
    /// texture + channel dropdowns line up down the column.
    pub const TEXTURE_MAP_LABEL_W: f32 = 84.0;
    /// Width of the per-property channel dropdown (R/G/B/A, or RGB + single channels
    /// for color slots) in a Texture-mapping row; the texture dropdown fills the rest.
    pub const TEXTURE_CHANNEL_COMBO_W: f32 = 62.0;
    /// On-screen size of a pooled-texture thumbnail in the Texture files list,
    /// and the max edge its CPU downscale targets (doubled for crispness on HiDPI).
    pub const TEXTURE_THUMB_SIZE: f32 = 40.0;
    /// Width of the remove (✕) button column in a Texture files row.
    pub const TEXTURE_REMOVE_BTN_W: f32 = 24.0;
    /// Minimum width of the texture dropdown in a Texture-mapping row when the
    /// Inspector is dragged narrow (the row's elastic control).
    pub const TEXTURE_COMBO_MIN_W: f32 = 60.0;

    // ── Texture viewport ──────────────────────────────────────────────────
    /// One segment width of the channel radio group (RGB / R / G / B / A),
    /// top-left of the Tex toolbar. The group width is derived per-frame from the
    /// number of visible segments (the `A` segment is hidden for opaque images).
    pub const TEXTURE_CHANNEL_SEGMENT_WIDTH: f32 = 44.0;
    /// Background-fill radio group (B / W / G / C), bottom-right of the Tex status
    /// bar: one segment width and the group width (4 segments + gaps + padding).
    pub const TEXTURE_BG_SEGMENT_WIDTH: f32 = 40.0;
    pub const TEXTURE_BG_GROUP_WIDTH: f32 = 175.0;
    /// Width of the texture-picker dropdown on the right of the Tex toolbar.
    pub const TOOLBAR_TEXTURE_DROPDOWN_WIDTH: f32 = 220.0;
    /// Width of the Tex viewport's stats panel (wider than the model-stats panel so
    /// "Dimension  1024 × 1024" fits on one row).
    pub const TEXTURE_STATS_PANEL_WIDTH: f32 = 188.0;
    /// Width of the clickable zoom-percentage readout next to the texture-info
    /// button in the Tex status bar. Sized to hold the widest readout
    /// (`6400%` at the max zoom) without reflowing.
    pub const TEXTURE_ZOOM_LABEL_WIDTH: f32 = 60.0;
    /// Screen-point side of one checkerboard-background square.
    pub const TEXTURE_CHECKER_CELL: f32 = 12.0;
    /// Fraction of the viewport the image fills when first fit (a small margin so a
    /// fitted image isn't flush to the edges).
    pub const TEXTURE_FIT_MARGIN: f32 = 0.96;
    /// Zoom clamp + sensitivities for the Tex viewport pan/zoom. `ZOOM_SPEED`
    /// scales a scroll-delta, and `DRAG_ZOOM_SPEED` a right-drag vertical delta,
    /// into the exponent of the multiplicative zoom step.
    pub const TEXTURE_ZOOM_MIN: f32 = 0.02;
    pub const TEXTURE_ZOOM_MAX: f32 = 64.0;
    pub const TEXTURE_ZOOM_SPEED: f32 = 0.0015;
    pub const TEXTURE_DRAG_ZOOM_SPEED: f32 = 0.01;
    /// Duration (seconds) of the eased pan/zoom transition run when the view
    /// snaps to a target — the zoom-readout toggle (100% ↔ fit) and the `F` / `R`
    /// frame reset. Continuous wheel/drag zoom is not eased.
    pub const TEXTURE_ZOOM_ANIM_SECS: f32 = 0.1;

    // ── Stats overlay ─────────────────────────────────────────────────────
    pub const STATS_ROW_SPACING: f32 = 3.0;
    pub const STATS_PANEL_WIDTH: f32 = 132.0;
    pub const STATS_PANEL_PAD_X: i8 = 9;
    pub const STATS_PANEL_PAD_Y: i8 = 9;
    /// Inset of the stats overlay from the left and bottom viewport edges.
    pub const STATS_OVERLAY_MARGIN: f32 = 18.0;
    pub const STATS_CORNER_RADIUS: f32 = 6.0;

    // ── Bounding-box dimension labels ─────────────────────────────────────
    /// Inner padding between a dimension label's pill edge and its text, and the
    /// pill's corner radius.
    pub const DIMENSION_LABEL_PAD_X: f32 = 7.0;
    pub const DIMENSION_LABEL_PAD_Y: f32 = 3.0;
    pub const DIMENSION_LABEL_CORNER_RADIUS: f32 = 5.0;

    // ── Axis gizmo ────────────────────────────────────────────────────────
    pub const GIZMO_SIZE: f32 = 156.0;
    pub const GIZMO_INSET: f32 = 26.0;
    pub const GIZMO_REACH: f32 = 52.0;
    pub const GIZMO_BALL_RADIUS: f32 = 13.0;
    pub const GIZMO_CORNER_RADIUS: f32 = 12.0;
    /// View-space depth above which an axis is treated as pointing at the viewer.
    pub const GIZMO_AXIS_ALIGNED_DEPTH: f32 = 0.99;
    pub const GIZMO_NEG_OPACITY: f32 = 0.05;
    /// Depth-fade ramp for the gizmo axis lines: the floor opacity of a
    /// pointing-away axis and the range up to fully-front (floor + range = 1).
    pub const GIZMO_DEPTH_FADE_FLOOR: f32 = 0.4;
    pub const GIZMO_DEPTH_FADE_RANGE: f32 = 0.6;
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
    /// Compact control-row height used by panel combos (the dropdown button is
    /// shrunk to this so combo rows stay dense — combos keep their own styling).
    pub const PANEL_ROW_H: f32 = 22.0;
    pub const PANEL_ROW_GAP: f32 = 6.0;
    /// Gap between the label and control columns of the striped panel grid (the
    /// demo widget-gallery layout).
    pub const PANEL_GRID_COL_GAP: f32 = 12.0;
    /// Option-panel grid column widths (raw points). The label column is pinned
    /// (long labels truncate rather than widen it); the control column is
    /// *elastic* — controls fill all remaining row width so their right edge
    /// always sits flush with the panel's right edge (no trailing empty space),
    /// however wide egui sizes the window. [`PANEL_CONTROL_COL_WIDTH`] is only
    /// the control column's **minimum**, used to pin the panel's minimum width
    /// (`PANEL_LABEL_COL_WIDTH + PANEL_GRID_COL_GAP + PANEL_CONTROL_COL_WIDTH`).
    pub const PANEL_LABEL_COL_WIDTH: f32 = 116.0;
    pub const PANEL_CONTROL_COL_WIDTH: f32 = 128.0;
    /// Fixed width of a slider row's value box (a separate `DragValue`, not the
    /// slider's auto-sized inline readout — fixed so the box never reflows as the
    /// number's digit count changes). The slider rail fills the rest of the row.
    pub const PANEL_SLIDER_VALUE_W: f32 = 48.0;
    /// Slider grab handle aspect ratio (width / height). egui's library default
    /// is a `Circle` handle; we use the egui.rs demo's slim rounded-`Rect` grab
    /// instead (`HandleShape::Rect { aspect_ratio }`), set in `apply_visuals`.
    pub const SLIDER_HANDLE_ASPECT_RATIO: f32 = 0.5;
    pub const PANEL_BUTTON_HEIGHT: f32 = 32.0;
    pub const PANEL_SWATCH_SIZE: f32 = 24.0;
    pub const PANEL_SWATCH_GAP: f32 = 6.0;
    pub const PANEL_COMBO_BUTTON_PAD_Y: f32 = 2.0;
    pub const PANEL_COMBO_POPUP_MAX_H: f32 = 240.0;
    pub const PANEL_COMBO_OPTION_H: f32 = 20.0;
    pub const PANEL_COMBO_OPTION_GAP: f32 = 2.0;
    pub const PANEL_COMBO_OPTION_PAD_Y: f32 = 2.0;
    /// Max popup height for the default-styled `labeled_combo` dropdowns. Raised
    /// above egui's stock 200pt cap so the longest panel list (the Buffers panel's
    /// 11 options) shows every entry at once without a scrollbar. It's only a *max*
    /// — egui shrinks the popup to its content, so short combos (Workflow,
    /// Transparency) are unaffected.
    pub const LABELED_COMBO_POPUP_MAX_H: f32 = 360.0;
    /// Hairline alignment nudge: painted 1px strokes land crisp when centered on
    /// a half-pixel boundary (the toolbar's bottom divider).
    pub const HAIRLINE_NUDGE: f32 = 0.5;
    /// On-screen size of an HDR environment preview thumbnail shown beside each
    /// option in the Environment dropdown. The source PNGs are 2:1
    /// equirectangular previews, so the height is half the width.
    pub const ENV_THUMB_WIDTH: f32 = 56.0;
    pub const ENV_THUMB_HEIGHT: f32 = 28.0;
    /// Corner radius of a color swatch.
    pub const SWATCH_CORNER_RADIUS: f32 = 5.0;

    // ── Stroke widths ─────────────────────────────────────────────────────
    /// Default hairline stroke (dividers, tile outlines, swatch outlines).
    pub const HAIRLINE: f32 = 1.0;
    /// Selected-swatch outline width.
    pub const SELECTION_STROKE_WIDTH: f32 = 2.0;
    /// Height of the options-hint gradient line atop a hovered icon tile. Two
    /// pixels so it stays visible over the accent fill of an active button.
    pub const OPTIONS_HINT_THICKNESS: f32 = 2.0;

    // ── Gizmo strokes / interaction ───────────────────────────────────────
    pub const GIZMO_LINE_WIDTH: f32 = 2.5;
    pub const GIZMO_RING_WIDTH: f32 = 2.0;
    pub const GIZMO_BOLD_OFFSET: f32 = 0.6;
    pub const GIZMO_BALL_HOVER_SCALE: f32 = 1.18;
    pub const GIZMO_BALL_HIT_EXPAND: f32 = 4.0;

    // ── Native window / panel chrome ──────────────────────────────────────
    // Values handed to native `egui::Window` / `egui::SidePanel` builders. These
    // are egui *points* (the builder sizes are scaled by DPI inside egui), so —
    // unlike the hand-painted overlays — they are NOT routed through `px()`.
    /// Default / minimum width of the dockable Outliner / Inspector side panels;
    /// the upper bound reuses [`OUTLINER_MAX_WIDTH`]. Raw points.
    pub const SIDE_PANEL_DEFAULT_WIDTH: f32 = TOOL_WINDOW_WIDTH;
    pub const SIDE_PANEL_MIN_WIDTH: f32 = 180.0;
    /// Down-right offset between successive option windows' first-open positions
    /// so several opened at once don't stack exactly atop each other.
    pub const PANEL_CASCADE_STEP: f32 = 26.0;

    // ── Startup help overlay (treated as egui points, like the option panels) ─
    /// Width of each of the two shortcut columns and the gap between them; the
    /// card content width is derived as `2 * COLUMN_WIDTH + COLUMN_GAP`.
    pub const HELP_COLUMN_WIDTH: f32 = 196.0;
    pub const HELP_COLUMN_GAP: f32 = 18.0;
    /// Card inner padding and rounding.
    pub const HELP_CARD_PAD_X: i8 = 18;
    pub const HELP_CARD_PAD_Y: i8 = 14;
    pub const HELP_CARD_CORNER_RADIUS: u8 = 7;
    /// Vertical breathing room on each side of a section divider.
    pub const HELP_SECTION_GAP: f32 = 8.0;
    /// Vertical gap between adjacent shortcut rows.
    pub const HELP_ROW_GAP: f32 = 5.0;
    /// Key-cap tile geometry: square single keys, wider caps for word labels
    /// ("Ctrl"), shared corner radius, and the gaps that flank them.
    pub const HELP_KEYCAP_SIZE: f32 = 22.0;
    pub const HELP_KEYCAP_WIDE_WIDTH: f32 = 38.0;
    pub const HELP_KEYCAP_CORNER_RADIUS: f32 = 4.0;
    /// Gap between two key-caps in one chord (Ctrl + N).
    pub const HELP_KEYCAP_GAP: f32 = 6.0;
    /// Gap between a row's key-cap(s) and its description text.
    pub const HELP_KEY_LABEL_GAP: f32 = 10.0;
}

/// Font sizes (logical px). The proportional face is Inter, monospace is
/// JetBrains Mono; see [`crate::assets`].
pub mod font {
    pub const STATS: f32 = 12.0;
    /// Clickable zoom-percentage readout in the Tex status bar (monospace).
    pub const STATUS_ZOOM: f32 = 16.0;
    /// Option-panel body text (labels, slider value boxes, combo text, the reset
    /// button). Rendered in the monospace face one step smaller than egui's
    /// default body size so numeric readouts line up on a fixed grid.
    pub const PANEL_BODY: f32 = 12.0;
    /// Bounding-box dimension-label text (monospace).
    pub const DIMENSION_LABEL: f32 = 18.0;
    pub const GIZMO_LABEL: f32 = 16.0;
    pub const GIZMO_LABEL_NEG: f32 = 15.0;
    pub const MODE_SEGMENT: f32 = 18.0;
    /// Section-heading font in the Inspector side panel.
    pub const PANEL_LABEL: f32 = 14.0;
    /// egui `TextStyle::Heading` size — used by native window title bars and the
    /// Inspector's `ui.heading` section headings. Trimmed from egui's stock 18 so
    /// the option-window title bars are a touch shorter.
    pub const PANEL_HEADING: f32 = 14.0;

    /// Centered "nothing loaded yet" hint shown in an empty viewport (e.g. the Tex
    /// view's "No textures loaded…" prompt). Larger than panel body text so it
    /// reads clearly across the otherwise-empty canvas.
    pub const VIEWPORT_EMPTY_HINT: f32 = 20.0;

    // ── Startup help overlay (sized to match the app's other overlay text) ──
    pub const HELP_TITLE: f32 = 14.0;
    pub const HELP_META: f32 = 11.5;
    pub const HELP_SUBTITLE: f32 = 12.0;
    pub const HELP_BODY: f32 = 12.0;
    pub const HELP_KEYCAP: f32 = 11.0;
    /// Backtick key-cap glyph: bumped up because `` ` `` renders tiny and
    /// top-aligned at the regular key-cap size.
    pub const HELP_KEYCAP_BACKTICK: f32 = 18.0;
    pub const HELP_FOOTER: f32 = 11.0;
}

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
/// Note: the hand-painted overlays (axis gizmo, stats, dimension labels, help
/// card) and the toolbar / status-bar chrome paint with explicit theme tokens in
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
    ctx.set_style(style);
}

/// Convert a logical-pixel design value into egui points for the current DPI, so
/// the overlay keeps the same physical size across display scales.
pub fn px(ctx: &egui::Context, value: f32) -> f32 {
    value / ctx.pixels_per_point()
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
