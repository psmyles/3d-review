//! Top-level overlay orchestration: assembles the viewport scene callback and
//! the egui chrome (toolbar, option panel, gizmo, stats, status bar) each frame,
//! and returns the [`UiOutput`] intents for `app` to apply.

use review_model::{ModelData, SceneBvh};
use review_render::{OrbitCamera, Selection};

use crate::opt_state::{GhostStyle, OptIntent, OptLayout};
use crate::state::{OptionPanel, ScopedStats, UiOutput, UiState, WorkspaceMode, sync_debug_state};
use crate::theme::{self, color, size};
use crate::{dimensions, gizmo, help, panels, stats, status_bar, texture_view, toolbar};

/// Draw the full egui overlay and return the intents emitted this frame. `model`
/// is the shared scene geometry and `bvh` an acceleration structure over it, both
/// read only for the bounding-box dimension labels' occlusion test (invariant 1:
/// borrowed, never copied). `bvh` is `None` until `app` has built it for the
/// current model (lazily, the first time the labels need it).
pub fn draw_overlay(
    ctx: &egui::Context,
    state: &mut UiState,
    camera: OrbitCamera,
    model: &ModelData,
    bvh: Option<&SceneBvh>,
) -> UiOutput {
    let _z = crate::prof::zone!("Draw Overlay");
    // Visuals + fonts are installed once at startup (`theme::init_style`); the
    // style is derived only from constant tokens, so there is nothing to re-apply
    // here each frame.
    sync_debug_state(state);
    let mut output = UiOutput::default();

    let toolbar_height = theme::px(ctx, size::TOOLBAR_HEIGHT);
    let status_bar_height = theme::px(ctx, size::STATUS_BAR_HEIGHT);

    // Native chrome panels first: the top toolbar and bottom status bar carve their
    // bands, then the dockable side panels fill the middle — declared in this order
    // so the side panels sit *between* the bars, not under them.
    toolbar::draw(ctx, state);
    status_bar::draw(ctx, state);

    // The side panels, option panels, axis gizmo and stats overlay are all 3D-scene
    // chrome; the UV / Texture workspaces keep a clean viewport (just the UV dropdown
    // in the toolbar), so they draw only in the scene workspaces (3D and Opt).
    if state.mode.is_scene() {
        // Outliner (left) + Inspector (right) dockable side panels. They paint over
        // the full-window background scene (exactly as the toolbar / status bar
        // already do); their live widths inset the floating viewport chrome below so
        // the gizmo / stats never land on top of a panel. The Inspector emits
        // material-edit intents for `app` to apply (invariant 2).
        let side = draw_side_panels(ctx, state, model);
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
        // The measured box is resolved here (cached for the "visible only" scan)
        // so the overlay never redoes the O(triangle) bounds walk per frame.
        let bounds = state.measured_bounds(model);
        dimensions::draw_dimension_labels(ctx, state, camera, model, bvh, bounds, viewport);

        draw_option_panels(ctx, state, viewport);

        if state.show_axis_gizmo {
            let gizmo_response = egui::Area::new(egui::Id::new("axis_gizmo"))
                .fade_in(false)
                .anchor(
                    egui::Align2::RIGHT_TOP,
                    egui::vec2(
                        -(theme::px(ctx, size::GIZMO_INSET) + side.right_inset),
                        toolbar_height + theme::px(ctx, size::GIZMO_INSET),
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
    } else if state.mode == WorkspaceMode::Texture {
        // The Tex workspace paints a 2D image viewer (channel-isolated, pan/zoom)
        // over a chosen background fill, plus its own floating stats panel. The
        // image itself is drawn by `app` through the D3D11 RHI (migration Phase 4);
        // this lays out the canvas + interaction + background fill only.
        texture_view::draw(ctx, state);
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
    // The renderer splits the same rect the same way (`SceneViewport`), so the
    // line lands exactly on the seam between the two composites.
    let x = viewport.center().x;
    ctx.layer_painter(egui::LayerId::background()).line_segment(
        [
            egui::pos2(x, viewport.top()),
            egui::pos2(x, viewport.bottom()),
        ],
        egui::Stroke::new(size::HAIRLINE, color::DIVIDER),
    );
}

/// Draw every open tool option panel as its own native `egui::Window`
/// (resizable, collapsible, closable, drop-shadowed — egui owns each window's
/// position/size/collapsed state in memory, constrained to `viewport` so they
/// stay inside the free scene area, and several can be open at once). A window's
/// title-bar X clears it from [`UiState::panels_open`].
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
        let margin = theme::px(ctx, size::OVERLAY_MARGIN);
        let default_pos = egui::pos2(
            viewport.left() + margin + step,
            viewport.top() + margin + step,
        );
        // egui's `.open(&mut bool)` paints the title-bar X and flips this false
        // when it's clicked; mirror that back into the open-set after the window.
        let mut open = true;
        egui::Window::new(panel.title())
            .id(egui::Id::new(panel.window_id()))
            .open(&mut open)
            // The option panels have compact, fixed content (a two-column table),
            // so they aren't resizable — which also drops egui's bottom-right
            // resize grip. The body pins a consistent width (see `draw_panel_body`).
            .resizable(false)
            .collapsible(true)
            .default_pos(default_pos)
            // Keep the window inside the free viewport so it can never be dragged
            // over the toolbar, the status bar or a side panel.
            .constrain_to(viewport)
            // Persistent chrome, not a transient popup: skip egui's fade so an
            // always-present window never spins the on-demand redraw loop
            // (invariant 6).
            .fade_in(false)
            .fade_out(false)
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
/// `egui::SidePanel`s — resizable by dragging their inner edge, with egui owning
/// the width across frames — and both are gated on the one
/// [`UiState::side_panels_open`] flag, so the pair opens and closes together. The
/// Outliner mutates [`UiState::selection`] directly; the Inspector returns an
/// intent for `app` to apply (invariant 2).
fn draw_side_panels(
    ctx: &egui::Context,
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
        let response = egui::SidePanel::left("outliner_panel")
            .resizable(true)
            .default_width(size::SIDE_PANEL_DEFAULT_WIDTH)
            .width_range(size::SIDE_PANEL_MIN_WIDTH..=size::OUTLINER_MAX_WIDTH)
            .show(ctx, |ui| {
                // In Opt the left panel is split horizontally: the operation
                // stack takes a resizable band at the bottom and the scene tree
                // keeps the rest. `show_inside` is egui's own nested-panel
                // primitive, so egui owns the divider drag and the split height
                // across frames exactly as it owns the side panel's width.
                if opt_mode {
                    let stack = egui::TopBottomPanel::bottom("opt_stack_pane")
                        .resizable(true)
                        .default_height(size::OPT_STACK_DEFAULT_HEIGHT)
                        .height_range(size::OPT_STACK_MIN_HEIGHT..=size::OPT_STACK_MAX_HEIGHT)
                        .show_inside(ui, |ui| panels::opt_stack::body(ui, state));
                    if stack.inner.is_some() {
                        opt.intent = stack.inner;
                    }
                    egui::CentralPanel::default()
                        .show_inside(ui, |ui| panels::outliner::body(ui, state, model));
                } else {
                    panels::outliner::body(ui, state, model);
                }
            });
        left_inset = response.response.rect.width();
    }

    let mut right_inset = 0.0;
    let mut inspector = panels::inspector::InspectorOutput::default();
    if state.side_panels_open {
        let response = egui::SidePanel::right("inspector_panel")
            .resizable(true)
            .default_width(size::SIDE_PANEL_DEFAULT_WIDTH)
            .width_range(size::SIDE_PANEL_MIN_WIDTH..=size::OUTLINER_MAX_WIDTH)
            .show(ctx, |ui| {
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
