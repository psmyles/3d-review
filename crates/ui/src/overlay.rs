//! Top-level overlay orchestration: assembles the viewport scene callback and
//! the egui chrome (toolbar, option panel, gizmo, stats, status bar) each frame,
//! and returns the [`UiOutput`] intents for `app` to apply.

use review_model::{ModelData, SceneBvh};
use review_render::{OrbitCamera, Selection};

use crate::dimensions::DimensionView;
use crate::opt_state::{ComparisonSide, GhostStyle, OptIntent, OptLayout};
use crate::state::{OptionPanel, ScopedStats, UiOutput, UiState, WorkspaceMode, sync_debug_state};
use crate::theme::{self, color, size};
use crate::{dimensions, gizmo, help, panels, stats, status_bar, texture_view, toolbar, transport};

/// The Opt workspace's second view, as the overlay needs to see it. `app` supplies
/// this whenever that workspace is active; every other workspace passes `None`.
///
/// It exists for the dimension labels. The split draws two meshes through two
/// cameras, so a readout describing what is actually on screen needs both —
/// measuring the source's box beside the processed mesh would report every LOD as
/// unchanged. Plain borrowed values, per invariant 2 — and `Copy`, like the camera
/// beside it, so `app` can hand it to egui's `FnMut` frame closure.
#[derive(Clone, Copy)]
pub struct OptOverlayView<'a> {
    /// The camera the second view is drawn through. Meaningful even before a run
    /// produces a level: the split draws the *source* through it into the right half
    /// until one lands, so the half is laid out the same either way.
    pub camera: OrbitCamera,
    /// The processed level, once a run has produced one. `None` leaves both halves
    /// of the split showing the source, which is exactly what the renderer draws.
    pub level: Option<OptOverlayLevel<'a>>,
}

/// One processed LOD level the overlay measures and occludes against.
#[derive(Clone, Copy)]
pub struct OptOverlayLevel<'a> {
    pub model: &'a ModelData,
    /// Occlusion structure over `model`; `None` until `app` has built it (lazily,
    /// the first frame the labels need it for this level).
    pub bvh: Option<&'a SceneBvh>,
    /// Identifies `model`, so the measured box can be cached across frames and
    /// re-measured when a reprocess replaces the mesh.
    pub revision: u64,
}

/// Draw the full egui overlay and return the intents emitted this frame. `model`
/// is the shared scene geometry and `bvh` an acceleration structure over it, both
/// read only for the bounding-box dimension labels' occlusion test (invariant 1:
/// borrowed, never copied). `bvh` is `None` until `app` has built it for the
/// current model (lazily, the first time the labels need it). `opt` carries the
/// same three things for the Opt workspace's processed level; see
/// [`OptOverlayView`].
pub fn draw_overlay(
    root: &mut egui::Ui,
    state: &mut UiState,
    camera: OrbitCamera,
    model: &ModelData,
    bvh: Option<&SceneBvh>,
    opt: Option<OptOverlayView<'_>>,
) -> UiOutput {
    let _z = crate::prof::zone!("Draw Overlay");
    // egui shows panels into a `Ui` rather than onto the `Context`, so the frame's
    // root `Ui` is what carves the chrome bands and the side panels out of the
    // window. Everything that floats — `Window`, `Area`, the layer painters — still
    // addresses the `Context` directly.
    let ctx = &root.ctx().clone();
    // Visuals + fonts are installed once at startup (`theme::init_style`); the
    // style is derived only from constant tokens, so there is nothing to re-apply
    // here each frame.
    sync_debug_state(state);
    let mut output = UiOutput::default();

    let toolbar_height = size::TOOLBAR_HEIGHT;
    let status_bar_height = size::STATUS_BAR_HEIGHT;

    // Native chrome panels first: the top toolbar and bottom status bar carve their
    // bands, then the dockable side panels fill the middle — declared in this order
    // so the side panels sit *between* the bars, not under them.
    toolbar::draw(root, state);
    status_bar::draw(root, state);

    // The side panels, option panels, axis gizmo and stats overlay are all 3D-scene
    // chrome; the UV / Texture workspaces keep a clean viewport (just the UV dropdown
    // in the toolbar), so they draw only in the scene workspaces (3D and Opt).
    if state.mode.is_scene() {
        // Outliner (left) + Inspector (right) dockable side panels. They paint over
        // the full-window background scene (exactly as the toolbar / status bar
        // already do); their live widths inset the floating viewport chrome below so
        // the gizmo / stats never land on top of a panel. The Inspector emits
        // material-edit intents for `app` to apply (invariant 2).
        let side = draw_side_panels(root, state, model);
        output.material_edit = side.inspector.material_edit;
        output.texture = side.inspector.texture;
        output.material_edit_active = side.inspector.material_edit_active;
        output.opt = side.opt.intent;
        output.opt_edit_active = side.opt.edit_active;

        // The free viewport: the screen minus the chrome bands (toolbar top,
        // status bar bottom) and the open side panels (left/right). Floating
        // chrome — the dimension labels and the option windows — is kept inside
        // this rect so it never overlaps the toolbar icons or a side panel.
        let screen = ctx.content_rect();
        let viewport = egui::Rect::from_min_max(
            egui::pos2(
                screen.left() + side.left_inset,
                screen.top() + toolbar_height,
            ),
            egui::pos2(
                screen.right() - side.right_inset,
                screen.bottom() - status_bar_height,
            ),
        );

        // `app` reads this back to lay the Opt split out inside the area the user
        // can actually see (the renderer's `SceneViewport`).
        state.scene_viewport = Some(viewport);
        draw_split_divider(ctx, state, viewport);

        // Bounding-box dimension labels sit on the viewport (under the chrome).
        // Each measured box is resolved here (cached per scope, and per level for
        // the processed one) so the overlay never redoes an O(triangle) bounds walk
        // per frame.
        let views = dimension_views(state, camera, model, bvh, opt, screen, viewport);
        dimensions::draw_dimension_labels(ctx, state, &views);

        draw_option_panels(ctx, state, viewport);

        if state.show_axis_gizmo {
            let gizmo_response = egui::Area::new(egui::Id::new("axis_gizmo"))
                .fade_in(false)
                .anchor(
                    egui::Align2::RIGHT_TOP,
                    egui::vec2(
                        -(size::GIZMO_INSET + side.right_inset),
                        toolbar_height + size::GIZMO_INSET,
                    ),
                )
                .show(ctx, |ui| {
                    gizmo::draw_axis_gizmo(ui, ctx, camera, state.projection_mode.into())
                });
            output.axis_gizmo_action = gizmo_response.inner;
        }

        // Re-scoped here rather than inside the card, which holds only `&UiState`:
        // the sums are cached against the selection + hidden set and rebuilt only
        // when one of them moves (invariant 6 — nothing O(mesh) per frame).
        let scoped = state.scoped_stats(model);
        draw_stats_overlay(
            ctx,
            state,
            scoped,
            status_bar_height,
            side.left_inset,
            side.right_inset,
        );
        draw_overlay_legend(
            ctx,
            state,
            status_bar_height,
            side.left_inset,
            side.right_inset,
        );
        transport::draw_transport(
            ctx,
            state,
            model,
            status_bar_height,
            side.left_inset,
            side.right_inset,
        );
    } else if state.mode == WorkspaceMode::Texture {
        // The Tex workspace paints a 2D image viewer (channel-isolated, pan/zoom)
        // over a chosen background fill, plus its own floating stats panel. The
        // image itself is drawn by the renderer into the same frame, behind the
        // chrome; this lays out the canvas + interaction + background fill only.
        texture_view::draw(root, state);
    }

    // The startup cheat-sheet sits on top of all the chrome (drawn last). It
    // consumes pointer input (so the chrome beneath stays inert while it's up);
    // `app` owns dismissing it — on any key, a click, a file drop, or a model
    // load — and double-clicking it opens the file picker.
    help::draw_help_overlay(ctx, state);

    output
}

/// The line between the Opt split's two views. Drawn on egui's background layer
/// (so the stats cards and option windows still float over it) in the same
/// hairline the chrome panels are edged with, so the divider reads as part of
/// the frame rather than as something in the scene.
fn draw_split_divider(ctx: &egui::Context, state: &UiState, viewport: egui::Rect) {
    if state.mode != WorkspaceMode::Opt || state.opt.layout != OptLayout::Split {
        return;
    }
    let (left, _) = split_halves(viewport);
    ctx.layer_painter(egui::LayerId::background()).line_segment(
        [
            egui::pos2(left.right(), viewport.top()),
            egui::pos2(left.right(), viewport.bottom()),
        ],
        egui::Stroke::new(size::HAIRLINE, color::DIVIDER),
    );
}

/// The two rects the Opt split lays its views out in. The renderer halves the same
/// rect the same way (`SceneViewport`), so these are where its two composites
/// actually land — which is what lets the divider sit on the seam and the dimension
/// labels project into the half they belong to.
fn split_halves(viewport: egui::Rect) -> (egui::Rect, egui::Rect) {
    let x = viewport.center().x;
    (
        egui::Rect::from_min_max(viewport.min, egui::pos2(x, viewport.max.y)),
        egui::Rect::from_min_max(egui::pos2(x, viewport.min.y), viewport.max),
    )
}

/// The labelled views this frame: normally one over the whole viewport, but two in
/// the Opt split — each with its own mesh, camera and half of the screen.
///
/// `screen` is the whole window, which is what a single view's clip space maps onto
/// (the renderer draws the scene over the full backbuffer and the chrome paints on
/// top); `viewport` is the chrome-free area, which labels are kept inside and which
/// the split halves.
fn dimension_views<'a>(
    state: &mut UiState,
    camera: OrbitCamera,
    model: &'a ModelData,
    bvh: Option<&'a SceneBvh>,
    opt: Option<OptOverlayView<'a>>,
    screen: egui::Rect,
    viewport: egui::Rect,
) -> Vec<DimensionView<'a>> {
    if !state.debug.show_bounding_box {
        return Vec::new();
    }
    let source_bounds = state.measured_bounds(model);
    // Measured before the branch so the cache is refreshed in whichever layout is up,
    // including the overlay, where only one of the two meshes is labelled.
    let level = opt.and_then(|opt| opt.level);
    let level_bounds = level.map(|level| state.processed_bounds(level.model, level.revision));

    // The split lays out two halves whether or not a run has landed — until one does,
    // the renderer draws the source into both — so the right half falls back to the
    // source rather than losing its labels. Every other case is a single view over
    // the whole screen, including an overlay with nothing processed, which the
    // renderer draws as the plain 3D scene.
    let split = state.mode == WorkspaceMode::Opt && state.opt.layout == OptLayout::Split;
    if split && let Some(opt) = opt {
        let (left, right) = split_halves(viewport);
        // Once a level exists its own measurement stands even when it is `None` — the
        // scope selecting no geometry there means the renderer draws no box on that
        // half either, so labelling it with the source's would describe nothing.
        let (right_bounds, right_model, right_bvh) = match level {
            Some(level) => (level_bounds.flatten(), level.model, level.bvh),
            None => (source_bounds, model, bvh),
        };
        return [
            (source_bounds, model, bvh, camera, left),
            (right_bounds, right_model, right_bvh, opt.camera, right),
        ]
        .into_iter()
        .filter_map(|(bounds, model, bvh, camera, half)| {
            Some(DimensionView {
                // Each view is half as wide as the area it draws into, and the
                // renderer corrects its camera's aspect to match; a label projected
                // through the uncorrected one drifts sideways from its own box.
                camera: half_camera(camera, half),
                model,
                bvh,
                bounds: bounds?,
                image: half,
                clamp: half,
            })
        })
        .collect();
    }

    // One view. In the overlay layout both meshes share the space, so the labels
    // describe whichever reads as solid — otherwise a swapped overlay measures the
    // mesh underneath the one you can see. Its two cameras are synced, so either
    // serves to project with.
    let (bounds, model, bvh) = match level {
        Some(level) if state.opt.side == ComparisonSide::Processed => {
            (level_bounds.flatten(), level.model, level.bvh)
        }
        _ => (source_bounds, model, bvh),
    };
    bounds
        .map(|bounds| DimensionView {
            camera,
            model,
            bvh,
            bounds,
            image: screen,
            clamp: viewport,
        })
        .into_iter()
        .collect()
}

/// `camera` re-aspected for one half of the Opt split. Each view is half as wide as
/// the area it draws into, and the renderer corrects for that on its side; a label
/// projected through the uncorrected camera would drift horizontally from the box it
/// belongs to.
fn half_camera(mut camera: OrbitCamera, half: egui::Rect) -> OrbitCamera {
    if half.height() > 0.0 {
        camera.aspect_ratio = half.width() / half.height();
    }
    camera
}

/// Draw every open tool option panel as its own native `egui::Window`
/// (resizable, collapsible, closable, drop-shadowed — egui owns each window's
/// position/size/collapsed state in memory, constrained to `viewport` so they
/// stay inside the free scene area, and several can be open at once). A window's
/// title-bar X clears it from [`UiState::panels_open`].
/// The frame every option window shares. Extracted from [`draw_option_panels`] so
/// the size test below measures the window the overlay actually builds — a test
/// that rebuilt this by hand would keep passing after this lost a call.
///
/// Position and the open-flag stay with the caller: they are per-window, and
/// `open` borrows.
fn option_window<'open>(panel: OptionPanel) -> egui::Window<'open> {
    egui::Window::new(panel.title())
        .id(egui::Id::new(panel.window_id()))
        // The option panels have compact, fixed content (a two-column table), so
        // they aren't resizable — which also drops egui's bottom-right resize
        // grip. The body pins a consistent width (see `draw_panel_body`).
        .resizable(false)
        // …and the *window* has to be told to take that width. `resizable(false)`
        // alone leaves egui's own default window size in force — 340pt wide, wider
        // than the 256pt body — so the frame sat 70pt wider than its content, as a
        // band of dead space down the right of every panel. `auto_sized` shrinks
        // the frame onto the body instead. It also turns scrolling off, which
        // these already had off.
        .auto_sized()
        .collapsible(true)
        // Persistent chrome, not a transient popup: skip egui's fade so an
        // always-present window never spins the on-demand redraw loop
        // (invariant 6).
        .fade_in(false)
        .fade_out(false)
}

fn draw_option_panels(ctx: &egui::Context, state: &mut UiState, viewport: egui::Rect) {
    for (slot, panel) in OptionPanel::ALL.into_iter().enumerate() {
        if !state.panels_open.is_open(panel) {
            continue;
        }
        // Cascade fresh windows down-right from the viewport's top-left so several
        // opened at once don't land exactly atop each other. egui only honors this
        // the first time a given window id appears; afterwards the user's dragged
        // position (kept in egui memory) wins.
        let step = size::PANEL_CASCADE_STEP * slot as f32;
        let margin = size::OVERLAY_MARGIN;
        let default_pos = egui::pos2(
            viewport.left() + margin + step,
            viewport.top() + margin + step,
        );
        // egui's `.open(&mut bool)` paints the title-bar X and flips this false
        // when it's clicked; mirror that back into the open-set after the window.
        let mut open = true;
        option_window(panel)
            .open(&mut open)
            .default_pos(default_pos)
            // Keep the window inside the free viewport so it can never be dragged
            // over the toolbar, the status bar or a side panel.
            .constrain_to(viewport)
            .show(ctx, |ui| panels::draw_panel_body(ui, state, panel));
        if !open {
            state.panels_open.set(panel, false);
        }
    }
}

/// The result of laying out the dockable side panels: the Inspector's emitted
/// intents (material edit / texture import / assign / clear / remove) plus the
/// live widths of the open panels, used to inset the floating viewport chrome
/// (gizmo / stats) so it doesn't land over a panel.
struct SidePanelLayout {
    inspector: panels::inspector::InspectorOutput,
    /// The Opt workspace's own emissions: the preset / export intents raised by
    /// the stack pane or the Opt inspector, and its drag-coalescing hint.
    opt: OptEmission,
    left_inset: f32,
    right_inset: f32,
}

/// What the Opt panels raised this frame.
#[derive(Debug, Clone, Default)]
struct OptEmission {
    intent: Option<OptIntent>,
    edit_active: bool,
}

/// Draw the dockable Outliner (left) and Inspector (right) side panels and return
/// the Inspector's material edit plus the panels' live widths. Both are native
/// `egui::Panel`s — resizable by dragging their inner edge, with egui owning
/// the width across frames — and both are gated on the one
/// [`UiState::side_panels_open`] flag, so the pair opens and closes together. The
/// Outliner mutates [`UiState::selection`] directly; the Inspector returns an
/// intent for `app` to apply (invariant 2).
fn draw_side_panels(
    root: &mut egui::Ui,
    state: &mut UiState,
    model: &ModelData,
) -> SidePanelLayout {
    // Measure the selection's influence once per frame, before either panel reads
    // it (the Inspector shows it; the scan is far too heavy to repeat per repaint).
    state.sync_bone_influence(model);

    let opt_mode = state.mode == WorkspaceMode::Opt;
    let mut opt = OptEmission::default();

    let mut left_inset = 0.0;
    if state.side_panels_open {
        let response = egui::Panel::left("outliner_panel")
            .resizable(true)
            .default_size(size::SIDE_PANEL_DEFAULT_WIDTH)
            .size_range(size::SIDE_PANEL_MIN_WIDTH..=size::OUTLINER_MAX_WIDTH)
            .show(root, |ui| {
                // In Opt the left panel is split horizontally: the operation
                // stack takes a resizable band at the bottom and the scene tree
                // keeps the rest. A nested `Panel` is egui's own primitive for
                // this, so egui owns the divider drag and the split height across
                // frames exactly as it owns the side panel's width.
                if opt_mode {
                    let stack = egui::Panel::bottom("opt_stack_pane")
                        .resizable(true)
                        .default_size(size::OPT_STACK_DEFAULT_HEIGHT)
                        .size_range(size::OPT_STACK_MIN_HEIGHT..=size::OPT_STACK_MAX_HEIGHT)
                        .show(ui, |ui| panels::opt_stack::body(ui, state));
                    if stack.inner.is_some() {
                        opt.intent = stack.inner;
                    }
                    egui::CentralPanel::default()
                        .show(ui, |ui| panels::outliner::body(ui, state, model));
                } else {
                    panels::outliner::body(ui, state, model);
                }
            });
        left_inset = response.response.rect.width();
    }

    let mut right_inset = 0.0;
    let mut inspector = panels::inspector::InspectorOutput::default();
    if state.side_panels_open {
        let response = egui::Panel::right("inspector_panel")
            .resizable(true)
            .default_size(size::SIDE_PANEL_DEFAULT_WIDTH)
            .size_range(size::SIDE_PANEL_MIN_WIDTH..=size::OUTLINER_MAX_WIDTH)
            .show(root, |ui| {
                // Opt retargets the Inspector at whatever the stack pane has
                // selected — an operation's parameters, the export settings, or
                // the selected object's overrides. A material selection still
                // reaches the material editor, since Opt keeps the full 3D
                // chrome and materials remain inspectable.
                if opt_mode && !matches!(state.selection, Selection::Material(_)) {
                    let out = panels::opt_inspector::body(ui, state, model);
                    opt.intent = opt.intent.take().or(out.intent);
                    opt.edit_active |= out.edit_active;
                    panels::inspector::InspectorOutput::default()
                } else {
                    panels::inspector::body(ui, state, model)
                }
            });
        right_inset = response.response.rect.width();
        inspector = response.inner;
    }

    SidePanelLayout {
        inspector,
        opt,
        left_inset,
        right_inset,
    }
}

fn draw_stats_overlay(
    ctx: &egui::Context,
    state: &UiState,
    scoped: ScopedStats,
    status_bar_height: f32,
    left_inset: f32,
    right_inset: f32,
) {
    if !state.show_stats {
        return;
    }
    crate::widgets::stats_overlay_card(
        ctx,
        "stats_overlay",
        left_inset,
        status_bar_height,
        size::STATS_PANEL_WIDTH,
        |ui| stats::stats_grid(ui, state, scoped),
    );

    // Opt shows a second card on the opposite edge, so the two sets of counts
    // read as a comparison. It is up as soon as the workspace has measured
    // anything — before an operation is added it carries the source's own cache
    // and overdraw figures, which is what a user reads to decide what to add.
    if state.mode == WorkspaceMode::Opt && state.opt.result.is_some() {
        crate::widgets::stats_overlay_card_at(
            ctx,
            "opt_processed_stats_overlay",
            crate::widgets::StatsCardSide::Right,
            right_inset,
            status_bar_height,
            size::OPT_STATS_PANEL_WIDTH,
            |ui| stats::processed_stats_grid(ui, state),
        );
    }
}

/// The overlay layout's legend: which mesh is the shaded one and which is the
/// ghost drawn over it.
///
/// Without it the view is two meshes in one space with nothing saying which is
/// which — and the `X` swap silently exchanges them, so a reader who looked away
/// has no way back to the answer. Centred between the two stats cards, and drawn
/// only for the overlay, since the split labels its halves by their own cards.
fn draw_overlay_legend(
    ctx: &egui::Context,
    state: &UiState,
    status_bar_height: f32,
    left_inset: f32,
    right_inset: f32,
) {
    if state.mode != WorkspaceMode::Opt
        || state.opt.layout != OptLayout::Overlay
        || !state.opt.has_result()
    {
        return;
    }

    // The solid mesh is the one the A/B swap is *not* showing as the ghost.
    let solid = state.opt.side;
    let ghost = solid.swapped();
    let ghost_style = match state.opt.ghost_style {
        GhostStyle::Xray => "x-ray",
        GhostStyle::Wireframe => "wireframe",
    };

    // The card is centred in the free viewport, so the panels' insets shift it by
    // half their difference rather than by either one.
    let offset = (left_inset - right_inset) * 0.5;
    crate::widgets::stats_overlay_card_at(
        ctx,
        "opt_overlay_legend",
        crate::widgets::StatsCardSide::Center,
        offset,
        status_bar_height,
        size::OPT_LEGEND_WIDTH,
        |ui| {
            ui.spacing_mut().item_spacing.y = size::STATS_ROW_SPACING;
            legend_row(
                ui,
                color::TEXT_VALUE,
                &format!("{} · shaded", solid.label()),
            );
            legend_row(
                ui,
                color::GHOST_XRAY,
                &format!("{} · {ghost_style}", ghost.label()),
            );
        },
    );
}

/// One legend line: a colour swatch and what it labels.
fn legend_row(ui: &mut egui::Ui, swatch: egui::Color32, text: &str) {
    ui.horizontal(|ui| {
        let size = egui::Vec2::splat(size::OPT_LEGEND_SWATCH);
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, size::STATS_CORNER_RADIUS, swatch);
        ui.add_space(size::OPT_LEGEND_SWATCH_GAP - ui.spacing().item_spacing.x);
        ui.label(crate::widgets::mono_label(
            text,
            theme::font::STATS,
            color::TEXT_BODY,
        ));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::size;

    /// The width `draw_panel_body` pins every option panel's contents to.
    fn body_width() -> f32 {
        size::PANEL_LABEL_COL_WIDTH + size::PANEL_GRID_COL_GAP + size::PANEL_CONTROL_COL_WIDTH
    }

    /// Lay `panel` out in a headless context and return its window's outer width.
    fn measured_width(panel: OptionPanel) -> f32 {
        let ctx = egui::Context::default();
        crate::theme::init_style(&ctx);
        let mut state = UiState::default();
        let mut width = 0.0;
        // Three passes: egui's `Grid` learns its column widths from the previous
        // frame, so the first pass is not yet settled.
        for _ in 0..3 {
            ctx.begin_pass(Default::default());
            let mut open = true;
            if let Some(response) = option_window(panel)
                .open(&mut open)
                .show(&ctx, |ui| panels::draw_panel_body(ui, &mut state, panel))
            {
                width = response.response.rect.width();
            }
            let mut output = ctx.end_pass();
            output.textures_delta.clear();
        }
        width
    }

    /// Every option window must hug the width its body pins, and they must all
    /// come out the same width.
    ///
    /// `egui::Window` defaults to 340pt wide and keeps that unless it is told to
    /// size itself to its contents — `resizable(false)` alone does not do it. The
    /// panels went out for a while with 70pt of dead space down their right-hand
    /// side because of exactly that, so this pins the frame to the body.
    #[test]
    fn option_windows_hug_their_body() {
        // The frame's own margins and stroke, which the window is allowed to add
        // on top of the body. Generous: the point is to catch egui's 340pt default
        // (84pt of slack), not to pin the exact frame thickness.
        const MAX_FRAME: f32 = 32.0;

        let body = body_width();
        let mut widths = Vec::new();
        for panel in OptionPanel::ALL {
            let width = measured_width(panel);
            assert!(
                width >= body,
                "{}: window {width} is narrower than its {body}pt body",
                panel.title()
            );
            assert!(
                width - body <= MAX_FRAME,
                "{}: window {width} is {:.0}pt wider than its {body}pt body — \
                 dead space on the right (is `auto_sized` still set?)",
                panel.title(),
                width - body,
            );
            widths.push(width);
        }
        assert!(
            widths.windows(2).all(|w| (w[0] - w[1]).abs() < 0.01),
            "option windows must all be the same width, got {widths:?}"
        );
    }
}
