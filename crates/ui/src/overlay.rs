//! Top-level overlay orchestration: assembles the viewport scene callback and
//! the egui chrome (toolbar, option panel, gizmo, stats, status bar) each frame,
//! and returns the [`UiOutput`] intents for `app` to apply.

use review_model::{ModelData, SceneBvh};
use review_render::{OrbitCamera, Selection};

use crate::dimensions::DimensionView;
use crate::opt_state::{ComparisonSide, OptIntent, OptLayout};
use crate::state::{
    ChromeInsets, OptionPanel, ScopedStats, UiOutput, UiState, WorkspaceMode, sync_debug_state,
};
use crate::theme::{self, color, size};
use crate::{dimensions, gizmo, panels, stats, status_bar, texture_view, toolbar};

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
    // Zero unless a workspace docks panels below; the notice column reads this
    // back through `app`, and a stale value from a previous workspace would
    // shove it off-centre in the ones that dock nothing.
    state.chrome_insets = ChromeInsets::default();

    let toolbar_height = size::TOOLBAR_HEIGHT;
    let status_bar_height = size::STATUS_BAR_HEIGHT;

    // Native chrome panels first: the top toolbar and bottom status bar carve their
    // bands, then the dockable side panels fill the middle — declared in this order
    // so the side panels sit *between* the bars, not under them.
    toolbar::draw(root, state, &mut output);
    status_bar::draw(root, state, model);

    // Re-scoped before the panels rather than after: the Inspector's multi-part
    // summary states these same sums, and measuring them after it had drawn
    // would leave it one frame behind the selection it describes. The call is
    // cached against the selection + hidden set, so the second one below is a
    // lookup (invariant 6 — nothing O(mesh) per frame).
    let scoped = state.scoped_stats(model);

    // Outliner (left) + Inspector (right) dockable side panels, in every
    // workspace — each shows its own tabs (`OutlinerTab::available`). They paint
    // over the full-window background scene (exactly as the toolbar / status bar
    // already do); their live widths inset the floating viewport chrome below so
    // the gizmo / stats never land on top of a panel, and the Tex canvas is laid
    // out in what they leave. The Inspector emits material-edit intents for `app`
    // to apply (invariant 2).
    let side = draw_side_panels(root, state, model);
    state.chrome_insets = ChromeInsets {
        left: side.left_inset,
        right: side.right_inset,
    };
    output.material_edit = side.inspector.material_edit;
    output.texture = side.inspector.texture;
    output.material_edit_active = side.inspector.material_edit_active;
    output.opt = side.opt.intent;
    output.opt_edit_active = side.opt.edit_active;
    output.audit = side.aud.intent;
    output.aud_edit_active = side.aud.edit_active;

    // The free viewport: the screen minus the chrome bands (toolbar top, status
    // bar bottom) and the open side panels (left/right). Floating chrome — the
    // dimension labels, the option windows, the manual and the log — is kept
    // inside this rect so it never overlaps the toolbar icons or a side panel.
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

    // The option panels, axis gizmo and stats overlay are all 3D-scene chrome;
    // the UV / Texture workspaces keep a clean viewport, so they draw only in the
    // scene workspaces (3D and Opt).
    if state.mode.is_scene() {
        // `app` reads this back to lay the Opt split out inside the area the user
        // can actually see (the renderer's `SceneViewport`).
        state.scene_viewport = Some(viewport);

        // A pointing hand over anything the Select tool could pick, so the
        // viewport says it is clickable the way every other control does. Only
        // over the free viewport: egui owns the cursor wherever its own chrome
        // is under the pointer, and overriding that would fight a panel's
        // resize handle for it.
        if state.hover.is_some() && !ctx.is_pointer_over_egui() {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        draw_split_divider(ctx, state, viewport);

        // Bounding-box dimension labels sit on the viewport (under the chrome).
        // Each measured box is resolved here (cached per scope, and per level for
        // the processed one) so the overlay never redoes an O(triangle) bounds walk
        // per frame.
        let views = dimension_views(state, camera, model, bvh, opt, screen, viewport);
        dimensions::draw_dimension_labels(ctx, state, &views);

        draw_option_panels(ctx, state, viewport);

        // A `?`, a tooltip's "Learn more" link, or F1 parks its request in egui's
        // own store rather than reaching into `HelpState` from deep inside a
        // widget; this is where it is collected and applied, one frame later.
        if let Some(page) = crate::help::take_requested(ctx) {
            state.help.open_page(page);
        }
        crate::help::draw(ctx, &mut state.help, viewport);
        crate::log_window::draw(ctx, &mut state.log, viewport);

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
        draw_heat_legend(
            ctx,
            state,
            status_bar_height,
            side.left_inset,
            side.right_inset,
        );
    } else {
        if state.mode == WorkspaceMode::Texture {
            // The Tex workspace paints a 2D image viewer (channel-isolated,
            // pan/zoom) over a chosen background fill, in the central area the
            // side panels leave. The image itself is drawn by the renderer into
            // the same frame, behind the chrome; this lays out the canvas +
            // interaction + background fill only.
            texture_view::draw(root, state);
        }

        // The manual and the log can be opened from the menu in any workspace, so
        // outside the scene ones — which draw them above — they get the same free
        // viewport.
        if let Some(page) = crate::help::take_requested(ctx) {
            state.help.open_page(page);
        }
        crate::help::draw(ctx, &mut state.help, viewport);
        crate::log_window::draw(ctx, &mut state.log, viewport);
    }

    // Last, so the modal's backdrop covers every other piece of chrome.
    crate::about::draw(ctx, &mut state.about);

    output
}

/// The line between the Opt split's two views. Drawn on egui's background layer
/// (so the stats cards and option windows still float over it) in the same
/// hairline the chrome panels are edged with, so the divider reads as part of
/// the frame rather than as something in the scene.
fn draw_split_divider(ctx: &egui::Context, state: &UiState, viewport: egui::Rect) {
    let opt_split = state.mode == WorkspaceMode::Opt && state.opt.layout == OptLayout::Split;
    let aud_split = state.mode == WorkspaceMode::Aud && state.aud.split();
    if !opt_split && !aud_split {
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
/// actually land — which is what lets the divider sit on the seam, the dimension
/// labels project into the half they belong to, and `app` route the pointer and
/// the pick to the view under it. Every one of those asks here.
pub fn split_halves(viewport: egui::Rect) -> (egui::Rect, egui::Rect) {
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
fn option_window<'open>(panel: OptionPanel, style: &egui::Style) -> egui::Window<'open> {
    egui::Window::new(panel.title())
        .id(egui::Id::new(panel.window_id()))
        // The option panels have compact, fixed content (a two-column table), so
        // they aren't resizable — which also drops egui's bottom-right resize
        // grip. The body pins a consistent width (see `draw_panel_body`).
        .resizable(false)
        // …and the *window* has to be told to take that width. `resizable(false)`
        // alone leaves egui's own default window size in force — 340pt wide, wider
        // than the body — so the frame sat 70pt wider than its content, as a band
        // of dead space down the right of every panel. The width is the body plus
        // the window frame's own margins and stroke, which is what egui subtracts
        // back off before laying the title bar and body out. Height stays
        // content-driven: a non-resizable window takes its content's height.
        //
        // Not `auto_sized`, which also hugs the body: a collapsed auto-sized
        // window lays its title bar out at zero width, so egui paints the window
        // frame round nothing — a sliver of fill and stroke down the left edge of
        // the title bar.
        .default_width(panels::body_width() + egui::Frame::window(style).total_margin().sum().x)
        .collapsible(true)
        // Persistent chrome, not a transient popup: skip egui's fade so an
        // always-present window never spins the on-demand redraw loop
        // (invariant 6).
        .fade_in(false)
        .fade_out(false)
}

fn draw_option_panels(ctx: &egui::Context, state: &mut UiState, viewport: egui::Rect) {
    let style = ctx.global_style();
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
        option_window(panel, &style)
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
    /// The Aud workspace's emissions: the profile / report intents and its
    /// drag-coalescing hint.
    aud: AudEmission,
    left_inset: f32,
    right_inset: f32,
}

/// What the Aud panels raised this frame.
#[derive(Debug, Clone, Default)]
struct AudEmission {
    intent: Option<crate::aud_state::AuditIntent>,
    edit_active: bool,
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
    // Keep the current texture in range before the Textures tab and the
    // Inspector read it: a removed texture may have shrunk the pool.
    if state.texture_view.selected >= state.texture_pool.len() {
        state.texture_view.selected = 0;
    }

    let opt_mode = state.mode == WorkspaceMode::Opt;
    let aud_mode = state.mode == WorkspaceMode::Aud;
    let mut opt = OptEmission::default();
    let mut aud = AudEmission::default();

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
                //
                // Both nested panels are already inside the side panel's frame,
                // whose fill and margin they would otherwise repeat: a default
                // `CentralPanel` adds 8pt on every side, which pushed the
                // Outliner's tabs down and in, in this workspace alone. So the
                // tree's panel draws no frame of its own, and the stack keeps only
                // its vertical margin — every workspace's Outliner, and both
                // halves of this one, share the side panel's single inset.
                if opt_mode {
                    let mut stack_frame = egui::Frame::side_top_panel(ui.style());
                    stack_frame.inner_margin.left = 0;
                    stack_frame.inner_margin.right = 0;
                    let stack = egui::Panel::bottom("opt_stack_pane")
                        .frame(stack_frame)
                        .resizable(true)
                        .default_size(size::OPT_STACK_DEFAULT_HEIGHT)
                        .size_range(size::OPT_STACK_MIN_HEIGHT..=size::OPT_STACK_MAX_HEIGHT)
                        .show(ui, |ui| panels::opt_stack::body(ui, state));
                    if stack.inner.is_some() {
                        opt.intent = stack.inner;
                    }
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ui, |ui| panels::outliner::body(ui, state, model));
                } else {
                    aud.intent = panels::outliner::body(ui, state, model);
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
                } else if aud_mode && panels::aud_inspector::wants_inspector(state) {
                    // Aud retargets the Inspector at the focused finding or the
                    // profile; a part selected in the Scene tab or the viewport
                    // still gets the ordinary part view.
                    let out = panels::aud_inspector::body(ui, state, model);
                    aud.intent = aud.intent.take().or(out.intent);
                    aud.edit_active |= out.edit_active;
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
        aud,
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
    // The legend names the ghost's style with the same words the status-bar
    // control uses, so the two read as the same setting.
    let ghost_style = review_localization::tr(crate::labels::ghost_style(state.opt.ghost_style));

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
                &crate::keys::ui_opt::legend_solid(
                    review_localization::tr(solid.label()).into_owned(),
                ),
            );
            legend_row(
                ui,
                color::GHOST_XRAY,
                &crate::keys::ui_opt::legend_ghost(
                    review_localization::tr(ghost.label()).into_owned(),
                    ghost_style.into_owned(),
                ),
            );
        },
    );
}

/// The Aud density views' key: the colour ramp and what its ends and middle
/// mean, in the profile's own numbers. Centred like the overlay legend.
fn draw_heat_legend(
    ctx: &egui::Context,
    state: &UiState,
    status_bar_height: f32,
    left_inset: f32,
    right_inset: f32,
) {
    use review_audit::{DiagnosticView, Measured, RuleId};
    if state.mode != WorkspaceMode::Aud {
        return;
    }
    let profile = &state.aud.profile;
    let (title, low, mid, high) = match state.aud.view {
        DiagnosticView::TexelDensity => {
            let rule = profile.rule(RuleId::TexelDensity);
            let size = rule.number("texture_size").unwrap_or(2048.0);
            let target = rule.number("target").unwrap_or(512.0);
            let tolerance = rule.number("tolerance").unwrap_or(2.0).max(1.0);
            let label = |value| crate::audit_labels::measured(&Measured::PxPerMeter(value));
            (
                crate::keys::ui_audit::legend_texel(format!("{size:.0}")),
                label(target / tolerance),
                label(target),
                label(target * tolerance),
            )
        }
        DiagnosticView::TriangleDensity => {
            let rule = profile.rule(RuleId::TriangleLod);
            let area = rule.number("min_pixel_area").unwrap_or(10.0);
            (
                review_localization::tr(crate::keys::ui_audit::LEGEND_TRIANGLE).into_owned(),
                review_localization::tr(crate::keys::ui_audit::LEGEND_COARSE).into_owned(),
                crate::audit_labels::measured(&Measured::SquarePixels(area)),
                review_localization::tr(crate::keys::ui_audit::LEGEND_DENSE).into_owned(),
            )
        }
        _ => return,
    };
    let offset = (left_inset - right_inset) * 0.5;
    crate::widgets::stats_overlay_card_at(
        ctx,
        "aud_heat_legend",
        crate::widgets::StatsCardSide::Center,
        offset,
        status_bar_height,
        size::AUD_LEGEND_WIDTH,
        |ui| {
            ui.spacing_mut().item_spacing.y = size::STATS_ROW_SPACING;
            ui.label(crate::widgets::mono_label(
                &title,
                theme::font::STATS,
                color::TEXT_BODY,
            ));
            let (bar, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), size::AUD_LEGEND_BAR_HEIGHT),
                egui::Sense::hover(),
            );
            paint_ramp(ui.painter(), bar);
            ui.horizontal(|ui| {
                let third = ui.available_width() / 3.0;
                for (text, align) in [
                    (low, egui::Align::Min),
                    (mid, egui::Align::Center),
                    (high, egui::Align::Max),
                ] {
                    ui.allocate_ui_with_layout(
                        egui::vec2(third, ui.spacing().interact_size.y),
                        egui::Layout::top_down(align),
                        |ui| {
                            ui.label(crate::widgets::mono_label(
                                &text,
                                theme::font::STATS,
                                color::TEXT_MUTED,
                            ));
                        },
                    );
                }
            });
        },
    );
}

/// The heat ramp as a horizontal bar: low, on target, high.
fn paint_ramp(painter: &egui::Painter, rect: egui::Rect) {
    let stops = [color::HEAT_LOW, color::HEAT_TARGET, color::HEAT_HIGH];
    let mut mesh = egui::Mesh::default();
    for (index, stop) in stops.iter().enumerate() {
        let x = rect.left() + rect.width() * index as f32 / 2.0;
        mesh.colored_vertex(egui::pos2(x, rect.top()), *stop);
        mesh.colored_vertex(egui::pos2(x, rect.bottom()), *stop);
    }
    for segment in 0..2_u32 {
        let base = segment * 2;
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base + 1, base + 3, base + 2);
    }
    painter.add(egui::Shape::mesh(mesh));
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
    use crate::panels::body_width;

    /// Lay `panel` out in a headless context, expanded or collapsed, and return
    /// its window's outer width plus the width of its title text.
    fn measured_width(panel: OptionPanel, expanded: bool) -> (f32, f32) {
        let ctx = egui::Context::default();
        crate::theme::init_style(&ctx);
        let mut state = UiState::default();
        let mut width = 0.0;
        let mut title_width = 0.0;
        // Three passes: egui's `Grid` learns its column widths from the previous
        // frame, so the first pass is not yet settled.
        for _ in 0..3 {
            ctx.begin_pass(Default::default());
            let style = ctx.global_style();
            let mut open = true;
            if let Some(response) = option_window(panel, &style)
                .default_open(expanded)
                .open(&mut open)
                .show(&ctx, |ui| panels::draw_panel_body(ui, &mut state, panel))
            {
                width = response.response.rect.width();
            }
            let font = egui::TextStyle::Heading.resolve(&style);
            let title = review_localization::tr(panel.title()).to_string();
            title_width = ctx.fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap(title, font, egui::Color32::WHITE)
                    .size()
                    .x
            });
            let mut output = ctx.end_pass();
            output.textures_delta.clear();
        }
        (width, title_width)
    }

    /// Where the first Outliner tab lands with `mode` up, after the overlay has
    /// settled (egui sizes panels from the previous pass).
    fn first_tab_rect(mode: WorkspaceMode) -> egui::Rect {
        let ctx = egui::Context::default();
        crate::theme::init_style(&ctx);
        let model = review_model::demo_cube_model();
        let mut state = UiState {
            mode,
            ..UiState::default()
        };
        let input = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600.0, 900.0),
            )),
            ..Default::default()
        };
        for _ in 0..3 {
            let mut output = ctx.run_ui(input(), |ui| {
                draw_overlay(ui, &mut state, OrbitCamera::default(), &model, None, None);
            });
            output.textures_delta.clear();
        }
        let id = egui::Id::new(panels::outliner::TAB_STRIP_ID).with(("tab", 0_usize));
        ctx.read_response(id)
            .unwrap_or_else(|| panic!("{mode:?}: no Outliner tab was laid out"))
            .rect
    }

    /// The Outliner's tab strip sits at the same place in every workspace.
    ///
    /// Opt nests the tree in a `CentralPanel` under its operation stack, and a
    /// default one repeats the side panel's frame — 8pt more on every side, so the
    /// tabs alone in that workspace sat lower and narrower than everywhere else.
    #[test]
    fn the_outliner_tabs_sit_at_one_place_in_every_workspace() {
        let reference = first_tab_rect(WorkspaceMode::ThreeD);
        for mode in [
            WorkspaceMode::Uv,
            WorkspaceMode::Texture,
            WorkspaceMode::Opt,
        ] {
            let rect = first_tab_rect(mode);
            assert_eq!(
                rect.min, reference.min,
                "{mode:?}: the tab strip starts at {:?}, not where 3D's does ({:?})",
                rect.min, reference.min
            );
            assert_eq!(
                rect.height(),
                reference.height(),
                "{mode:?}: tab height differs"
            );
        }
    }

    /// A collapsed option window's frame must surround its title bar.
    ///
    /// An `auto_sized` window lays a collapsed title bar out at zero width, so
    /// egui painted the window frame round nothing: a sliver of fill and stroke
    /// down the left edge of the title bar, under the full-width header.
    #[test]
    fn collapsed_option_windows_frame_their_title() {
        for panel in OptionPanel::ALL {
            let (width, title) = measured_width(panel, false);
            assert!(
                width > title,
                "{}: collapsed window is {width}pt wide, narrower than its {title}pt \
                 title — the frame is painted round a zero-width title bar",
                review_localization::tr(panel.title())
            );
        }
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
            let (width, _) = measured_width(panel, true);
            assert!(
                width >= body,
                "{}: window {width} is narrower than its {body}pt body",
                review_localization::tr(panel.title())
            );
            assert!(
                width - body <= MAX_FRAME,
                "{}: window {width} is {:.0}pt wider than its {body}pt body — \
                 dead space on the right (is `default_width` still set?)",
                review_localization::tr(panel.title()),
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
