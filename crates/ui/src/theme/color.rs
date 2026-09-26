//! Semantic color tokens. Names describe the *role* a color plays, not its hue,
//! so a re-theme only touches this block.

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
/// arbitrary model surface and the grey wireframe.
pub const SELECTION_OUTLINE: Color32 = Color32::from_rgb(255, 140, 35);
/// How opaque the viewport's selection fill is. The highlight persists for as
/// long as something is selected, so this is the level it holds throughout.
///
/// Pitched to be unmistakable rather than tasteful: the fill has to answer
/// "which part is selected" from across a crowded scene, on a model whose own
/// colours are arbitrary and often already warm. A gentler wash looked right on
/// a grey test mesh and all but vanished on a textured one. The surface's
/// shading and wireframe still read through it, which is what keeps this a
/// highlight rather than a repaint.
pub const SELECTION_FILL_OPACITY: f32 = 0.45;
/// Viewport hover highlight: the tint over whatever a click would select while
/// the Select tool is active.
///
/// A pale red, deliberately across the colour wheel from
/// [`SELECTION_OUTLINE`]'s orange rather than a lighter shade of it: the two
/// fills sit side by side the moment the pointer rests next to something already
/// selected, and at these opacities a difference in *lightness* alone is not
/// enough to tell "would be selected" from "is selected". Equal green and blue
/// keep it a clean red with no orange cast to drift back toward the selection.
///
/// Its alpha is the fill opacity, stated the same way [`SELECTION_FILL_OPACITY`]
/// is — the renderer reads straight RGBA, so this is written premultiplied-free
/// as an RGB constant plus its own opacity below.
pub const HOVER_HIGHLIGHT: Color32 = Color32::from_rgb(255, 135, 135);
/// How opaque the hover tint is: firm enough to notice without looking for it,
/// and clearly under [`SELECTION_FILL_OPACITY`].
///
/// The gap is deliberate and does its own work. Hue already separates the two
/// (pale red against orange), but a preview that is also visibly *lighter* than
/// a selection says which is which even where the two fills meet, or where a
/// model's own colour pulls one of the hues toward the other.
pub const HOVER_FILL_OPACITY: f32 = 0.28;
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
/// The Opt stats card's change column. Every figure it annotates is one where
/// lower is better — fewer triangles and vertices, fewer cache misses, less
/// overdraw, fewer bytes fetched — so a fall reads green and a rise red.
/// Desaturated enough to sit beside the value column without shouting.
pub const STATS_DELTA_BETTER: Color32 = Color32::from_rgb(126, 202, 122);
pub const STATS_DELTA_WORSE: Color32 = Color32::from_rgb(226, 122, 118);
/// The one notice-card header tint that isn't already in egui's palette.
///
/// Every other kind takes a native colour — `error_fg_color`, `warn_fg_color`,
/// `hyperlink_color`, `strong_text_color` — so it follows any restyle of the
/// chrome. egui has no green, and "this worked" is the one thing a viewer says
/// often enough to be worth a colour of its own. Pitched to match the weight of
/// egui's own warn/error tints beside it.
pub const NOTICE_SUCCESS: Color32 = Color32::from_rgb(126, 202, 122);
/// A scene-tree row the type filter is hiding, kept visible only because a
/// shown node lives beneath it. Dim enough to read as structure, not content.
pub const OUTLINER_FILTERED: Color32 = Color32::from_gray(104);
/// Selected option text in a compact combo (stands out by brightness).
pub const TEXT_COMBO_SELECTED: Color32 = Color32::from_gray(238);
/// Dimmed unselected option text in a compact combo.
pub const TEXT_COMBO_DIM: Color32 = Color32::from_gray(150);

// ── Outliner rows ───────────────────────────────────────────────────────
/// Per-[`review_model::NodeKind`] row-glyph tints, so a node's type reads by
/// color before its name is parsed. Mesh blue, bone green, light yellow,
/// camera purple; the two type-less kinds stay neutral.
pub const NODE_MESH: Color32 = Color32::from_rgb(110, 178, 240);
pub const NODE_BONE: Color32 = Color32::from_rgb(140, 214, 130);
pub const NODE_LIGHT: Color32 = Color32::from_rgb(235, 205, 100);
pub const NODE_CAMERA: Color32 = Color32::from_rgb(188, 140, 235);
pub const NODE_EMPTY: Color32 = Color32::from_gray(225);
pub const NODE_OTHER: Color32 = Color32::from_gray(150);
/// Wash on every other Outliner row, so a long list reads as scannable bands.
/// Translucent: it tints whatever surface the panel sits on rather than
/// pinning the stripe to one background color. (White at alpha 6.)
pub const OUTLINER_ROW_ALT_BG: Color32 = Color32::from_rgba_premultiplied(6, 6, 6, 6);
/// Full-row selection fill: [`ACCENT`] at alpha 150, so the stripe and the
/// tree guide lines still read through a selected row.
pub const OUTLINER_ROW_SELECTED_BG: Color32 = Color32::from_rgba_premultiplied(39, 61, 96, 150);
/// Scene-tree indent guides — the verticals and elbows tying a child row back
/// to its parent. Dim enough to structure the list without competing with it.
pub const OUTLINER_GUIDE: Color32 = Color32::from_gray(72);
/// The stretch of guide running from the selected row up through its
/// ancestors, lit so the selection's place in a deep hierarchy is legible
/// without scrolling to find it.
pub const OUTLINER_GUIDE_SELECTED: Color32 = Color32::from_gray(160);
/// Closed-eye tint on a hidden mesh row: dimmer than the open eye, so a hidden
/// row reads as switched off at a glance.
pub const OUTLINER_EYE_HIDDEN: Color32 = Color32::from_gray(112);

// ── Strokes / dividers ──────────────────────────────────────────────────
/// Toolbar bottom divider and window stroke.
pub const DIVIDER: Color32 = Color32::from_gray(58);
/// Status-bar top border.
pub const STATUS_BORDER: Color32 = Color32::from_gray(72);
/// Stats-overlay border.
pub const STATS_BORDER: Color32 = Color32::from_gray(52);
/// Idle (unselected) swatch outline.
pub const SWATCH_BORDER: Color32 = Color32::from_gray(28);

/// The Opt overlay's ghost mesh — a cool translucent blue, deliberately
/// unlike the selection flash's warm orange so the two never read as the same
/// thing. Owned here rather than in the renderer (invariant 8) and handed to
/// it with the frame, so the viewport's ghost and the legend's swatch cannot
/// drift apart. The renderer supplies the alpha, which differs by ghost style.
pub const GHOST_XRAY: Color32 = Color32::from_rgb(89, 158, 255);

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
/// Default UV-seam edge color: the bright green a DCC tool marks texture
/// borders in. The seam lines are the same 1px the wireframe is, so the color
/// is all that separates them — hence one that no other overlay default uses.
pub const UV_SEAM_DEFAULT: Color32 = Color32::from_rgb(29, 255, 27);

/// Default skeleton-overlay bone color: a bright cyan-blue that separates
/// from both skin tones and the orange selection highlight.
pub const SKELETON_DEFAULT: Color32 = Color32::from_rgb(89, 184, 255);

/// Swatch palette offered for the skeleton bone color row: the default,
/// white, black, amber, magenta, green.
pub const SKELETON_SWATCHES: [Color32; 6] = [
    SKELETON_DEFAULT,
    Color32::WHITE,
    Color32::BLACK,
    Color32::from_rgb(255, 205, 64),
    Color32::from_rgb(255, 96, 216),
    Color32::from_rgb(29, 255, 27),
];

/// Swatch palette offered for the normal-line color rows. Includes both
/// normal-view defaults so either can be restored by reset.
pub const NORMAL_SWATCHES: [Color32; 5] = [
    Color32::WHITE,
    Color32::BLACK,
    VERTEX_NORMAL_DEFAULT,
    Color32::from_rgb(29, 255, 27),
    FACE_NORMAL_DEFAULT,
];

/// Swatch palette offered for the UV-seam color row: green (default), white,
/// black, magenta, amber, cyan. The default leads so reset stays highlighted
/// on its swatch.
pub const UV_SEAM_SWATCHES: [Color32; 6] = [
    UV_SEAM_DEFAULT,
    Color32::WHITE,
    Color32::BLACK,
    Color32::from_rgb(255, 96, 216),
    Color32::from_rgb(255, 205, 64),
    Color32::from_rgb(32, 224, 232),
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
