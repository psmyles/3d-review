//! Bounding-box dimension labels: when the bounding-box view is active, the
//! length of each axis is drawn at the centre of every box edge, projected from
//! the live camera into the viewport (mirrors the reference viewer's readout).
//! A label whose edge midpoint is hidden behind the model is culled, so the
//! readout reads as part of the scene rather than floating over it.
//!
//! Per invariant 2 this reads only shared, read-only data — the plain values in
//! [`UiState`], the shared [`ModelData`] geometry and a [`SceneBvh`] over it (both for
//! the occlusion test) — and paints into the egui overlay; it never mutates
//! renderer/model internals. The occlusion test runs through the [`SceneBvh`] so its
//! cost is sub-linear in the triangle count (no per-frame brute-force ray-cast).

use glam::{Mat4, Vec3};
use review_model::{Bounds, ModelData, SceneBvh};
use review_render::OrbitCamera;

use crate::state::{UiState, ViewProjectionMode};
use crate::theme::{color, font, size};
use crate::units::match_known_unit;

/// One labelled view: a mesh, the camera looking at it, and the two rects that
/// place its labels.
///
/// There is normally one of these — the whole viewport — but the Opt workspace's
/// split is two, each with its own mesh, camera and half of the screen. Bundling
/// them is what keeps the projection honest there: a label's position depends on
/// *which* rect the camera's image covers and on that rect's aspect, and both
/// differ per half.
pub(crate) struct DimensionView<'a> {
    /// The camera as it draws into [`DimensionView::image`] — the caller is
    /// responsible for its `aspect_ratio` matching that rect, since the renderer
    /// overrides the aspect per half in the split.
    pub(crate) camera: OrbitCamera,
    pub(crate) model: &'a ModelData,
    /// Occlusion structure over `model`; `None` draws every on-screen label.
    pub(crate) bvh: Option<&'a SceneBvh>,
    /// The box to measure — resolved by the caller (through
    /// [`UiState::measured_bounds`] / [`UiState::processed_bounds`]) so the
    /// O(triangle) scoped scans stay cached rather than per-frame.
    pub(crate) bounds: Bounds,
    /// The rect this view's image covers, i.e. what its clip space maps onto. The
    /// whole window for a single view (the renderer draws the scene over the full
    /// backbuffer and the chrome paints on top), one half of the scene viewport
    /// for a split half.
    pub(crate) image: egui::Rect,
    /// The rect labels are kept inside, so one near an edge never spills over the
    /// toolbar, a side panel, or — in the split — the other half.
    pub(crate) clamp: egui::Rect,
}

/// Draw the axis-length label at the centre of each of the 12 bounding-box edges,
/// for every view. No-op unless the bounding-box view is active.
///
/// An edge whose midpoint is hidden behind the *visible* model geometry (the
/// camera→midpoint segment is blocked by a triangle of a non-hidden mesh part,
/// tested through the view's `bvh`) is skipped; meshes hidden in the Outliner don't
/// occlude, since they aren't drawn. When a view's `bvh` is `None` (not built yet
/// for that mesh) every one of its edge labels is drawn.
pub(crate) fn draw_dimension_labels(
    ctx: &egui::Context,
    state: &UiState,
    views: &[DimensionView<'_>],
) {
    if !state.debug.show_bounding_box {
        return;
    }
    // The Outliner-hidden meshes are excluded from the occlusion test: they aren't
    // drawn, so they can't hide a label. (The measured box, above, already accounts
    // for them in "visible only" mode.) Node indices are shared across the source and
    // its processed levels, so one list serves every view.
    let hidden: Vec<u32> = state.hidden_meshes.iter().map(|&i| i as u32).collect();

    // Painted on the background order, after the viewport scene callback (added
    // earlier in the frame) but beneath the egui chrome areas, so the labels sit
    // over the model and the box but under the toolbar / panels / gizmo.
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new("dimension_labels"),
    ));

    for view in views {
        draw_view_labels(state, &painter, view, &hidden);
    }
}

/// Every label for one view. Split out from [`draw_dimension_labels`] so the two
/// halves of the Opt split share one implementation rather than the caller
/// duplicating the projection.
fn draw_view_labels(
    state: &UiState,
    painter: &egui::Painter,
    view: &DimensionView<'_>,
    hidden: &[u32],
) {
    let (bounds, camera, model) = (view.bounds, view.camera, view.model);
    let size_world = bounds.size();
    // Per-axis label text (the axis length in the file's authored unit), shared
    // by the four box edges parallel to that axis.
    let labels = [
        format_dimension(size_world.x, state.stats.source_unit_meters),
        format_dimension(size_world.y, state.stats.source_unit_meters),
        format_dimension(size_world.z, state.stats.source_unit_meters),
    ];

    let view_projection = camera.view_projection(state.projection_mode.into());

    let axis_colors = [
        color::DIMENSION_LABEL_X,
        color::DIMENSION_LABEL_Y,
        color::DIMENSION_LABEL_Z,
    ];

    // The occlusion ray is cast from the camera toward each midpoint. Under
    // orthographic projection every view ray is parallel to the camera forward,
    // so the ray starts a fixed distance *behind* the midpoint along that
    // direction (far enough to clear the whole model); under perspective it
    // starts at the eye.
    let orthographic = matches!(state.projection_mode, ViewProjectionMode::Orthographic);
    let ortho_back = camera.forward_dir() * bounds.size().length().max(1.0) * 2.0;
    let eye = camera.eye_position();

    for (midpoint, axis) in edge_midpoints(bounds.min, bounds.max) {
        let ray_origin = if orthographic {
            midpoint - ortho_back
        } else {
            eye
        };
        if view
            .bvh
            .is_some_and(|bvh| bvh.segment_occluded(model, ray_origin, midpoint, hidden))
        {
            continue;
        }
        if let Some(pos) = project(view_projection, midpoint, view.image) {
            draw_label(painter, pos, &labels[axis], axis_colors[axis], view.clamp);
        }
    }
}

/// World-space midpoint of each of the box's 12 edges, paired with the axis it
/// runs along (0 = X, 1 = Y, 2 = Z). The four edges parallel to an axis sit at
/// the box's min/max on the other two axes, centred along the axis itself.
fn edge_midpoints(min: Vec3, max: Vec3) -> [(Vec3, usize); 12] {
    let c = (min + max) * 0.5;
    [
        // Edges along X (centred in x, at each y/z extreme).
        (Vec3::new(c.x, min.y, min.z), 0),
        (Vec3::new(c.x, min.y, max.z), 0),
        (Vec3::new(c.x, max.y, min.z), 0),
        (Vec3::new(c.x, max.y, max.z), 0),
        // Edges along Y.
        (Vec3::new(min.x, c.y, min.z), 1),
        (Vec3::new(min.x, c.y, max.z), 1),
        (Vec3::new(max.x, c.y, min.z), 1),
        (Vec3::new(max.x, c.y, max.z), 1),
        // Edges along Z.
        (Vec3::new(min.x, min.y, c.z), 2),
        (Vec3::new(min.x, max.y, c.z), 2),
        (Vec3::new(max.x, min.y, c.z), 2),
        (Vec3::new(max.x, max.y, c.z), 2),
    ]
}

/// Project a world point into `image` — the rect the camera's picture covers — or
/// `None` when it is behind the camera or outside that picture.
fn project(view_projection: Mat4, world: Vec3, image: egui::Rect) -> Option<egui::Pos2> {
    let clip = view_projection * world.extend(1.0);
    if clip.w <= 0.0 {
        return None; // behind the camera
    }
    let ndc = clip.truncate() / clip.w;
    if !(-1.0..=1.0).contains(&ndc.x) || !(-1.0..=1.0).contains(&ndc.y) {
        return None; // off-screen
    }
    Some(egui::pos2(
        image.left() + (ndc.x * 0.5 + 0.5) * image.width(),
        image.top() + (0.5 - ndc.y * 0.5) * image.height(),
    ))
}

/// Paint one dimension label: a rounded pill centred on `center` with the axis
/// length, the text tinted by the axis it measures (`text_color`). The pill is
/// nudged so it stays fully inside `viewport`, so a label whose edge midpoint
/// projects near the screen edge never spills over the toolbar / status bar /
/// side panels.
fn draw_label(
    painter: &egui::Painter,
    center: egui::Pos2,
    text: &str,
    text_color: egui::Color32,
    viewport: egui::Rect,
) {
    let font_id = egui::FontId::monospace(font::DIMENSION_LABEL);
    let galley = painter.layout_no_wrap(text.to_owned(), font_id, text_color);
    let pad = egui::vec2(size::DIMENSION_LABEL_PAD_X, size::DIMENSION_LABEL_PAD_Y);
    let rect = clamp_to(
        egui::Rect::from_center_size(center, galley.size() + pad * 2.0),
        viewport,
    );
    painter.rect_filled(
        rect,
        size::DIMENSION_LABEL_CORNER_RADIUS,
        color::DIMENSION_LABEL_BG,
    );
    painter.galley(rect.min + pad, galley, text_color);
}

/// Translate `rect` by the smallest offset that brings it fully inside `bounds`
/// (no scaling). When `rect` is larger than `bounds` on an axis it is pinned to
/// that side's min so the value stays readable rather than centre-clipped.
fn clamp_to(rect: egui::Rect, bounds: egui::Rect) -> egui::Rect {
    let mut offset = egui::Vec2::ZERO;
    if rect.left() < bounds.left() {
        offset.x = bounds.left() - rect.left();
    } else if rect.right() > bounds.right() {
        offset.x = (bounds.right() - rect.right()).min(0.0);
    }
    if rect.top() < bounds.top() {
        offset.y = bounds.top() - rect.top();
    } else if rect.bottom() > bounds.bottom() {
        offset.y = (bounds.bottom() - rect.bottom()).min(0.0);
    }
    rect.translate(offset)
}

/// Format an axis length (world meters) for display. Shows the file's authored
/// unit when it matches a known DCC unit (e.g. a centimeter file reads `122 cm`);
/// falls back to meters when the file declared no / an unrecognised unit.
fn format_dimension(length_meters: f32, source_unit_meters: f32) -> String {
    match match_known_unit(source_unit_meters) {
        Some(unit) => (unit.value)(round_dimension(length_meters / unit.meters)),
        None => crate::units::meters_value(round_dimension(length_meters)),
    }
}

/// Round a dimension value to a compact, readable precision: large values read
/// as whole numbers, small ones keep a couple of decimals.
fn round_dimension(value: f32) -> String {
    if value >= 100.0 {
        format!("{value:.0}")
    } else if value >= 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.2}")
    }
}
