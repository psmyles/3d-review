//! Sizes, spacings and radii.
//!
//! **Every token here is in egui points, and is used raw.** A point is already
//! the DPI-independent unit — egui multiplies by `pixels_per_point` when it
//! rasterizes — so a token authored against a 100%-scale display needs no
//! conversion to keep one apparent size on a HiDPI one. Anything needing the
//! height of the toolbar + status-bar band asks [`chrome_height`] rather than
//! adding [`TOOLBAR_HEIGHT`] and [`STATUS_BAR_HEIGHT`] up itself.
//!
//! These used to be "design pixels" divided by `pixels_per_point` at the use
//! site, which is a *second* correction on top of egui's and cancels it: a token
//! resolved to a fixed count of physical pixels, so the hand-painted chrome kept
//! one *pixel* size rather than one apparent size, shrinking against the native
//! `Window`/`Panel` chrome beside it as the display scale rose.
//!
//! **The chrome tokens were retuned by 2/3 when that conversion came out, and
//! that is not an arbitrary number.** They had been tuned by eye on a 150%
//! display *through* the divide-by-1.5, so each one held its apparent size × 1.5
//! — the toolbar's `73` was chosen to land 73 physical pixels on that screen, not
//! 73 points. Dropping the conversion without rescaling made the whole hand-painted
//! band render half again too large, while the option windows, side panels and
//! stock text — which were always real points — stayed put. The group widths
//! divide exactly (`4 + 28n + 2(n-1)`, where it was `6 + 42n + 3(n-1)`); the rest
//! are rounded to whole points. Don't re-inflate them to "match" the old numbers.

// ── Overlay layout ────────────────────────────────────────────────────
pub const TOOLBAR_HEIGHT: f32 = 48.0;
/// The About box's content width: wide enough that a long GPU name wraps once at
/// most rather than word by word.
pub const ABOUT_WIDTH: f32 = 320.0;
/// The gap between the About box's blocks (identity / renderer / links).
pub const ABOUT_SECTION_GAP: f32 = 6.0;
pub const STATUS_BAR_HEIGHT: f32 = 42.0;
pub const OVERLAY_MARGIN: f32 = 8.0;
/// Width of the dockable Outliner / Inspector side panels (their default /
/// also reused for [`SIDE_PANEL_DEFAULT_WIDTH`]). A touch narrower than the
/// option windows, since their content is lists / a compact property column.
pub const TOOL_WINDOW_WIDTH: f32 = 300.0;
/// Minimum height of an Outliner tab (Geometry / Materials), painted as text
/// with an active-underline rather than a button. The strip sizes off the
/// ambient button font (see [`OUTLINER_TAB_PAD_Y`]) so it tracks the stock rows
/// below it, and this is only the floor that padding is measured against.
pub const OUTLINER_TAB_HEIGHT: f32 = 34.0;
/// Breathing room above and below an Outliner tab's label, in points. The
/// strip's height is the label's row height plus twice this.
pub const OUTLINER_TAB_PAD_Y: f32 = 9.0;
/// Thickness of the active Outliner tab's underline accent.
pub const OUTLINER_TAB_UNDERLINE: f32 = 2.0;
/// Upper bound the user can drag the Outliner / Inspector side panels out to
/// (the `width_range` ceiling); they start at [`SIDE_PANEL_DEFAULT_WIDTH`].
pub const OUTLINER_MAX_WIDTH: f32 = 560.0;
/// Horizontal indent per scene-tree depth level.
pub const OUTLINER_INDENT: f32 = 14.0;
/// Edge length of a scene-tree row's node-kind glyph, and of the type-filter
/// row's toggle glyphs.
pub const OUTLINER_TYPE_ICON: f32 = 14.0;
/// Width reserved for a scene-tree row's expand/collapse arrow, so rows with
/// and without children align.
pub const OUTLINER_ARROW_WIDTH: f32 = 14.0;
/// Padding around an Outliner header toggle's glyph, giving it a hit target
/// and hover fill larger than the glyph itself.
pub const OUTLINER_ICON_PAD: f32 = 8.0;
/// Corner rounding on an Outliner header toggle's hover / active fill.
pub const OUTLINER_ICON_ROUNDING: f32 = 3.0;
/// Height of one Outliner row. Rows butt against each other (no item spacing)
/// so the alternating stripes read as continuous bands.
pub const OUTLINER_ROW_HEIGHT: f32 = 21.0;
/// Inset from the row's left edge to its first content column, and from its
/// right edge to the visibility eye.
pub const OUTLINER_ROW_PAD_X: f32 = 6.0;
/// Gap between a row's kind glyph and its name, and between the name's
/// truncation point and the eye.
pub const OUTLINER_ROW_GAP: f32 = 5.0;
/// Corner rounding on a row's hover / selection fill.
pub const OUTLINER_ROW_ROUNDING: f32 = 3.0;
/// Edge length of a row's visibility eye glyph.
pub const OUTLINER_EYE_ICON: f32 = 14.0;
/// Inset of the visibility eye from the row's right edge.
pub const OUTLINER_EYE_PAD: f32 = 6.0;

// ── Toolbar groups ────────────────────────────────────────────────────
pub const TOOLBAR_GROUP_SPACING: f32 = 9.0;
pub const TOOLBAR_ICON_SIZE: f32 = 28.0;
pub const TOOLBAR_ICON_GAP: f32 = 2.0;
/// Inset of an icon inside its tile, as a fraction of the tile's edge — a
/// ratio rather than a length so a smaller tile (the animation transport's,
/// sized to a dropdown) keeps the toolbar's icon-to-tile proportion. Matches
/// the 8-of-42 proportion the toolbar tiles were originally drawn at.
pub const TILE_ICON_INSET_RATIO: f32 = 8.0 / 42.0;
pub const TOOLBAR_CENTER_WIDTH: f32 = 120.0;
/// Right toolbar cluster (3D and Opt): Help (32) + the view group + projection
/// (32) + the side-panels toggle (32), with three group spacings.
///
/// Sized for the **widest** case, which is a rigged model: the view group gains a
/// tile for the skeleton toggle there (152 rather than 122), so
/// 152 + 9 + 32 + 9 + 32 + 9 + 32 + 9 + 32 = 316, plus a little slack. Sizing it
/// for the unrigged case instead left the outermost group hanging off the bar the
/// moment a skinned mesh was opened - `tests/toolbar_layout.rs` is what now says
/// so. It grows leftward from the right edge, away from the centered mode
/// segments.
pub const TOOLBAR_RIGHT_WIDTH: f32 = 325.0;
/// Left toolbar cluster: the menu (32) + show-wireframe (32) + shading (4) +
/// material + geometry-debug (3) groups, with four group spacings between them.
///
/// Sized for the widest case like [`TOOLBAR_RIGHT_WIDTH`]: the material group
/// gains a tile for the skin-weight view on a skinned mesh (152 rather than 122),
/// so 32 + 9 + 32 + 9 + 122 + 9 + 152 + 9 + 92 = 466, plus a little slack.
pub const TOOLBAR_LEFT_WIDTH: f32 = 470.0;
/// Four-icon shading group: wireframe-only, unlit, shaded, backface-rendering
/// (4 padding + 4×28 icons + 3×2 gaps). Show Wireframe is a group of its own.
pub const TOOLBAR_SHADING_GROUP_WIDTH: f32 = 122.0;
/// Four-icon active-material group: source material, UV checker, vertex colors,
/// buffers (4 padding + 4×28 icons + 3×2 gaps).
pub const TOOLBAR_MATERIAL_GROUP_WIDTH: f32 = 122.0;
pub const TOOLBAR_SINGLE_ICON_GROUP_WIDTH: f32 = 32.0;
/// Width of a three-icon toolbar group (the geometry-debug face-normals /
/// vertex-normals / UV-seams group, and the UV-shading wire / shaded / islands
/// group): 4 padding + 3×28 icons + 2×2 gaps.
pub const TOOLBAR_TRIPLE_ICON_GROUP_WIDTH: f32 = 92.0;
/// Width of a four-icon toolbar group (the toolbar's bounding box / pivot /
/// gizmo / grid view group): 4 padding + 4×28 icons + 3×2 gaps.
pub const TOOLBAR_QUAD_ICON_GROUP_WIDTH: f32 = 122.0;
/// Width of a five-icon toolbar group (the status bar's Background / IBL / AO /
/// Tonemapper / Anti-aliasing rendering-quality cluster): 4 padding + 5×28 icons
/// + 4×2 gaps.
pub const TOOLBAR_QUINT_ICON_GROUP_WIDTH: f32 = 152.0;
/// Width of the centered workspace-mode group: 4 padding + 4×37 segments
/// (3D / UV / Tex / Opt) + 3×2 gaps.
pub const TOOLBAR_MODE_GROUP_WIDTH: f32 = 158.0;
/// Width of the UV-set dropdown shown on the right of the toolbar in UV mode.
pub const TOOLBAR_UV_DROPDOWN_WIDTH: f32 = 133.0;
/// Width of the LOD-level dropdown in the Opt workspace's status-bar group.
/// Narrower than the UV one: its entries are "LOD 0 (full)" at longest.
pub const TOOLBAR_OPT_LOD_DROPDOWN_WIDTH: f32 = 80.0;
pub const TOOLBAR_GROUP_HEIGHT: f32 = 32.0;
pub const TOOLBAR_GROUP_PADDING: f32 = 2.0;
/// Corner radius shared by toolbar groups, icon tiles and mode segments.
pub const TILE_CORNER_RADIUS: f32 = 4.0;
/// Mode-segment tile size.
pub const MODE_SEGMENT_WIDTH: f32 = 37.0;
pub const MODE_SEGMENT_HEIGHT: f32 = 28.0;

// ── Notifications ─────────────────────────────────────────────────────
/// The width a notice card's text column wants. Fixed on purpose: a progress
/// card reports a job by rewriting its own text every few frames, and a card
/// sized to its content would shuffle its edges on every one of them.
///
/// Much wider than an option window's body, because a notice is a sentence
/// rather than a two-column row and the point of the card is that the sentence
/// can be read. It is a *want*, not a promise: a window too narrow for it
/// clamps to what the free viewport has (see `NOTIFICATION_MIN_WIDTH`).
pub const NOTIFICATION_WIDTH: f32 = 520.0;
/// The narrowest a card's text column is allowed to get on a small window.
/// Below this the wrapping breaks every sentence into stubs, at which point a
/// card that overhangs its viewport is the lesser problem.
pub const NOTIFICATION_MIN_WIDTH: f32 = 200.0;
/// Vertical gap between stacked notice cards.
pub const NOTIFICATION_CARD_GAP: f32 = 8.0;
/// Bottom inset of the notice column: the status bar's own height plus a
/// margin, so the cards sit clear of it.
pub const NOTIFICATION_MARGIN_Y: f32 = STATUS_BAR_HEIGHT + OVERLAY_MARGIN;
/// How many body lines a notice card shows before it summarises the rest as
/// `+ n more`. The column grows *upward* from the status bar, so an uncapped
/// card runs its own title off the top of the window — and an export report
/// carries a note per mesh per LOD level, which reaches double figures on an
/// ordinary game asset.
pub const NOTIFICATION_MAX_LINES: usize = 6;
/// How many notice cards the column shows at once, oldest dropped first. Same
/// reason: sticky cards (warnings, errors, reports) wait for a click, so a user
/// working through a run of failures would otherwise stack the column past the
/// top of the window. The newest are the ones being read.
pub const NOTIFICATION_MAX_VISIBLE: usize = 4;
/// Thickness of a progress card's bar, and the radius its track is rounded to.
pub const NOTIFICATION_PROGRESS_HEIGHT: f32 = 3.0;

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
pub const TEXTURE_CHANNEL_SEGMENT_WIDTH: f32 = 29.0;
/// Background-fill radio group (B / W / G / C), bottom-right of the Tex status
/// bar: one segment width and the group width (4 segments + gaps + padding).
pub const TEXTURE_BG_SEGMENT_WIDTH: f32 = 27.0;
pub const TEXTURE_BG_GROUP_WIDTH: f32 = 118.0;
/// Width of the texture-picker dropdown on the right of the Tex toolbar.
pub const TOOLBAR_TEXTURE_DROPDOWN_WIDTH: f32 = 147.0;
/// Width of the Tex viewport's stats panel (wider than the model-stats panel so
/// "Dimension  1024 × 1024" fits on one row).
pub const TEXTURE_STATS_PANEL_WIDTH: f32 = 188.0;
/// Width of the clickable zoom-percentage readout next to the texture-info
/// button in the Tex status bar. Sized to hold the widest readout
/// (`6400%` at the max zoom) without reflowing.
pub const TEXTURE_ZOOM_LABEL_WIDTH: f32 = 40.0;
/// Screen-point side of one checkerboard-background square.
pub const TEXTURE_CHECKER_CELL: f32 = 12.0;
/// Fraction of the viewport the image fills when first fit (a small margin so a
/// fitted image isn't flush to the edges).
pub const TEXTURE_FIT_MARGIN: f32 = 0.96;
/// Zoom clamp for the Tex viewport; the sensitivities that drive it live in
/// [`super::motion`].
pub const TEXTURE_ZOOM_MIN: f32 = 0.02;
pub const TEXTURE_ZOOM_MAX: f32 = 64.0;

// ── Stats overlay ─────────────────────────────────────────────────────
pub const STATS_ROW_SPACING: f32 = 3.0;
pub const STATS_PANEL_WIDTH: f32 = 260.0;
/// Width of one scope column on the model-stats card (All / Sel / Vis).
/// Fixed, so the three numeric columns line up down the card however wide
/// the numbers in them are; wide enough for an eight-digit count in the
/// monospace [`font::STATS`] face. Divides up [`STATS_PANEL_WIDTH`].
pub const STATS_VALUE_COLUMN: f32 = 56.0;
pub const STATS_PANEL_PAD_X: i8 = 9;
pub const STATS_PANEL_PAD_Y: i8 = 9;
/// Inset of the stats overlay from the left and bottom viewport edges.
pub const STATS_OVERLAY_MARGIN: f32 = 12.0;
pub const STATS_CORNER_RADIUS: f32 = 6.0;

// ── Bounding-box dimension labels ─────────────────────────────────────
/// Inner padding between a dimension label's pill edge and its text, and the
/// pill's corner radius.
pub const DIMENSION_LABEL_PAD_X: f32 = 5.0;
pub const DIMENSION_LABEL_PAD_Y: f32 = 2.0;
pub const DIMENSION_LABEL_CORNER_RADIUS: f32 = 3.0;

// ── Axis gizmo ────────────────────────────────────────────────────────
pub const GIZMO_SIZE: f32 = 104.0;
pub const GIZMO_INSET: f32 = 17.0;
pub const GIZMO_REACH: f32 = 35.0;
pub const GIZMO_BALL_RADIUS: f32 = 9.0;
pub const GIZMO_CORNER_RADIUS: f32 = 8.0;
/// View-space depth above which an axis is treated as pointing at the viewer.
pub const GIZMO_AXIS_ALIGNED_DEPTH: f32 = 0.99;
pub const GIZMO_NEG_OPACITY: f32 = 0.05;
/// Depth-fade ramp for the gizmo axis lines: the floor opacity of a
/// pointing-away axis and the range up to fully-front (floor + range = 1).
pub const GIZMO_DEPTH_FADE_FLOOR: f32 = 0.4;
pub const GIZMO_DEPTH_FADE_RANGE: f32 = 0.6;
/// Reset-view button: icon size and inset of its center from the gizmo's
/// bottom-left corner.
pub const GIZMO_RESET_ICON_SIZE: f32 = 13.0;
pub const GIZMO_RESET_INSET: f32 = 11.0;

// ── Option panels ─────────────────────────────────────────────────────
pub const PANEL_HEADER_HEIGHT: f32 = 34.0;
pub const PANEL_HEADER_PAD_X: f32 = 16.0;
pub const PANEL_CORNER_RADIUS: u8 = 6;
pub const PANEL_CONTENT_MARGIN: i8 = 14;
pub const PANEL_BODY_TOP_MARGIN: i8 = 10;
/// Compact control-row height used by panel combos (the dropdown button is
/// shrunk to this so combo rows stay dense — combos keep their own styling).
/// Handed straight to egui's `interact_size.y`, and the transport's tile
/// buttons match it so a row of buttons and a dropdown are the same height.
pub const PANEL_ROW_H: f32 = 22.0;
pub const PANEL_ROW_GAP: f32 = 6.0;
/// Gap between the label and control columns of the striped panel grid (the
/// demo widget-gallery layout).
pub const PANEL_GRID_COL_GAP: f32 = 12.0;
/// Option-panel grid column widths. The label column is pinned
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
pub const GIZMO_LINE_WIDTH: f32 = 1.7;
pub const GIZMO_RING_WIDTH: f32 = 1.3;
pub const GIZMO_BOLD_OFFSET: f32 = 0.4;
pub const GIZMO_BALL_HOVER_SCALE: f32 = 1.18;
pub const GIZMO_BALL_HIT_EXPAND: f32 = 2.7;

// ── Native window / panel chrome ──────────────────────────────────────
// Values handed to native `egui::Window` / `egui::SidePanel` builders.
/// Default / minimum width of the dockable Outliner / Inspector side panels;
/// the upper bound reuses [`OUTLINER_MAX_WIDTH`].
pub const SIDE_PANEL_DEFAULT_WIDTH: f32 = TOOL_WINDOW_WIDTH;
pub const SIDE_PANEL_MIN_WIDTH: f32 = 180.0;
/// Down-right offset between successive option windows' first-open positions
/// so several opened at once don't stack exactly atop each other.
pub const PANEL_CASCADE_STEP: f32 = 26.0;

// ── Opt workspace ─────────────────────────────────────────────────────
/// Starting height of the operation-stack pane docked under the Outliner
/// tree. Deep enough for the preset row, the add button and a handful of
/// operations before it needs scrolling. A native panel size.
pub const OPT_STACK_DEFAULT_HEIGHT: f32 = 240.0;
/// How far the stack pane can be dragged, so neither it nor the tree above
/// can be collapsed to nothing.
pub const OPT_STACK_MIN_HEIGHT: f32 = 96.0;
pub const OPT_STACK_MAX_HEIGHT: f32 = 620.0;
/// Height of one operation row in the stack pane.
pub const OPT_STACK_ROW_HEIGHT: f32 = 24.0;
/// Width of the square reorder / remove buttons on an operation row.
pub const OPT_STACK_ROW_BUTTON: f32 = 20.0;
/// Width of the processed-mesh stats card. Wider than the source card: its
/// rows carry a measured value *and* the delta against the source.
pub const OPT_STATS_PANEL_WIDTH: f32 = 186.0;
/// Width of the overlay-mode legend card, sized to "Processed — wireframe".
pub const OPT_LEGEND_WIDTH: f32 = 152.0;
/// Edge length of a legend row's colour swatch, and the gap after it.
pub const OPT_LEGEND_SWATCH: f32 = 9.0;
pub const OPT_LEGEND_SWATCH_GAP: f32 = 7.0;

// ── Animation ─────────────────────────────────────────────────────────
/// Height of one clip row in the Outliner's Animations tab, matching the Opt
/// stack rows it shares its primitive with.
pub const ANIM_ROW_HEIGHT: f32 = 24.0;
/// Width reserved at a clip row's right edge for its frame-count / duration
/// readout.
pub const ANIM_ROW_TRAILING_WIDTH: f32 = 104.0;
/// The transport's preferred width in the status bar's centre span. A
/// *maximum*, not a fixed size: a span wider than this leaves the row centred at
/// this width rather than stretching the scrubber across the whole bar, and a
/// narrower one shrinks it (see `transport::transport_row`).
pub const ANIM_TRANSPORT_WIDTH: f32 = 610.0;
/// The span below which the transport doesn't draw at all. Its fixed controls —
/// five tiles, the speed picker and their gaps — plus a usable rail come to
/// about this much, and a row squeezed under it would be buttons with no
/// scrubber between them.
pub const ANIM_TRANSPORT_MIN_WIDTH: f32 = 210.0;
/// Gap between transport controls.
pub const ANIM_TRANSPORT_GAP: f32 = 4.0;
/// Extra breathing room reserved after the transport's frame readout, so the
/// loop / speed group is separated from the counter rather than butted
/// against it. Part of the readout's fixed block, which is why the
/// group can't drift when the counter gains a digit.
pub const ANIM_TRANSPORT_GROUP_GAP: f32 = 9.0;
/// Floor on the transport scrubber's rail width. The rail is normally whatever
/// the row has left over after the fixed groups; reaching this floor is what
/// drops the frame readout and hands the rail its block.
pub const ANIM_SCRUB_MIN_WIDTH: f32 = 53.0;
/// Width of the playback-speed dropdown.
pub const ANIM_SPEED_COMBO_WIDTH: f32 = 43.0;

// ── Help and tooltips ─────────────────────────────────────────────────
/// Width a rich tooltip wraps its description at. Without a bound egui lays a
/// paragraph out on one line and runs it off the side of the window.
pub const TOOLTIP_MAX_WIDTH: f32 = 280.0;
/// Slack left between the Help window's reading pane and the width its text is
/// wrapped at, in points.
///
/// Text is wrapped at a width, but the galley that comes back is *measured*, and
/// egui rounds a measurement up to the display's pixel grid. So a paragraph
/// wrapped at exactly the viewport's width reports back a shade wider than it,
/// and egui raises a horizontal scrollbar on any overflow at all - a scrollbar
/// with a fraction of a point to scroll, which is worse than useless. The
/// rounding is per display scale, and at 125% a pixel is 0.8 points, so a couple
/// of points covers the worst of it with room to spare.
///
/// Only reachable by leaving room: the wrap width and the measured width are
/// rounded by egui at different moments, so they cannot be made to agree.
pub const HELP_WRAP_SLACK: f32 = 3.0;
/// Gap between the blocks under a tooltip's title - its description, its
/// operating notes and its manual link. The title itself is separated from them
/// by a rule rather than by this gap.
pub const TOOLTIP_TITLE_GAP: f32 = 4.0;
/// The Help window's default size. Wide enough for the contents list beside a
/// comfortable measure of prose, and short enough to leave the model visible
/// beside it on a laptop screen.
pub const HELP_WINDOW_DEFAULT: [f32; 2] = [720.0, 520.0];
/// Minimum the Help window may be dragged down to before its two panes stop
/// being readable.
pub const HELP_WINDOW_MIN: [f32; 2] = [420.0, 260.0];
/// Width of the Help window's contents list.
pub const HELP_TOC_WIDTH: f32 = 196.0;
/// Indent per nesting level in the contents list.
pub const HELP_TOC_INDENT: f32 = 12.0;
/// Widest a manual page's image is drawn at. Screenshots are captured at window
/// size, which is wider than the reading pane.
pub const HELP_IMAGE_MAX_WIDTH: f32 = 640.0;
/// Padding around the Help window's reading pane.
pub const HELP_PAGE_MARGIN: f32 = 8.0;
