//! The Outliner: a two-tab list (the scene's nodes + the deduplicated materials)
//! that drives the [`Selection`] used by the viewport highlight and the Inspector.
//!
//! Both tabs sit under one always-present header — the flat/tree view toggle, the
//! node-kind filter glyphs, and a search box — so the controls never move or
//! vanish as the view changes. The Scene tab has three presentations:
//!
//! * **Scene tree** — the full node hierarchy, indented and collapsible, with
//!   guide lines tying each row back to its parent.
//! * **Flat** — every node in model order, no hierarchy.
//! * **Search results** — while the search box is non-empty it overrides the view
//!   toggle on both tabs: matches list flat, so a hit is never buried inside a
//!   folded branch.
//!
//! Every row is one full-width strip: an alternating background band, a per-kind
//! colored glyph, a truncating name, and — on mesh-bearing nodes — a visibility
//! eye pinned to the right edge. Clicking anywhere on the strip sets
//! [`UiState::selection`]; on a *bone* row the modifiers additionally build
//! [`UiState::selected_bones`] (the primary modifier toggles, Shift takes a range), which feeds the
//! skeleton overlay's highlight and the skin-weight heat map. The arrow keys walk
//! the same rows (see [`nav::resolve_nav`]) whenever no widget — the search box
//! above all — holds egui's keyboard focus.
//!
//! Reads the scene nodes from the borrowed [`ModelData`] (invariant 2: borrowed,
//! not owned) and the material list from the app→UI snapshot.

mod animations;
mod materials;
mod nav;
mod rows;
#[cfg(test)]
mod test_fixture;
mod tree;

use review_model::{ModelData, NodeKind, SceneNode};
use review_render::Selection;

use crate::assets::{self, AppIcon};
use crate::docs::Page;
use crate::keys;
use crate::labels;
use crate::state::{OutlinerTab, OutlinerViewMode, UiState, WorkspaceMode};
use crate::theme::{color, size};
use crate::widgets;
use crate::widgets::{Tip, tip};
use nav::isolate_mesh;
use rows::kind_icon;

use animations::animations_tab;
use materials::materials_tab;
use nav::{apply_row_click, handle_nav};
use rows::draw_rows;
use tree::{TreeRow, flat_rows, search_rows, visible_tree_rows};

/// The Outliner's tabs, in strip order. The index into this array is what
/// [`widgets::tab_bar`] hands back on a click. The Animations tab is appended
/// only while the model carries clips and the workspace can play them.
const TABS: [OutlinerTab; 2] = [OutlinerTab::Scene, OutlinerTab::Materials];

/// What one drawn frame of rows produced. Every mutation is deferred to after the
/// draw: [`apply_row_click`] needs the full visible row order for Shift-range
/// selection, and mutating selection state mid-draw would let rows within one
/// frame disagree about what is selected.
#[derive(Debug, Default)]
struct RowsOutput {
    clicked: Option<(usize, egui::Modifiers)>,
    toggled_collapse: Option<usize>,
    /// The mesh row whose visibility eye was clicked, and the modifiers held —
    /// the primary modifier isolates that mesh instead of toggling it.
    toggled_eye: Option<(usize, egui::Modifiers)>,
    /// Whether a pending [`OutlinerState::scroll_to_selection`] was honored.
    ///
    /// [`OutlinerState::scroll_to_selection`]: crate::state::OutlinerState::scroll_to_selection
    scrolled: bool,
}

pub(crate) fn body(ui: &mut egui::Ui, state: &mut UiState, model: &ModelData) {
    // ── Tabs: Scene / Materials [/ Animations], as a full-width underlined
    // tab strip. The Animations tab exists only while there is something to
    // list and the workspace can play it: the Opt workspace compares static
    // geometry in the bind pose, so it never offers clips.
    let show_animations = state.animation.has_clips && state.mode != WorkspaceMode::Opt;
    let mut tabs: Vec<OutlinerTab> = TABS.to_vec();
    if show_animations {
        tabs.push(OutlinerTab::Animations);
    }
    if !tabs.contains(&state.outliner.tab) {
        state.outliner.tab = OutlinerTab::Scene;
    }
    let labels: Vec<egui::WidgetText> = tabs
        .iter()
        .map(|tab| match tab {
            OutlinerTab::Scene => keys::ui_outliner::TAB_SCENE.into(),
            OutlinerTab::Materials => keys::ui_outliner::TAB_MATERIALS.into(),
            OutlinerTab::Animations => keys::ui_outliner::TAB_ANIMATIONS.into(),
        })
        .collect();
    let active = tabs
        .iter()
        .position(|tab| *tab == state.outliner.tab)
        .unwrap_or(0);
    if let Some(index) = widgets::tab_bar(ui, &labels, active) {
        state.outliner.tab = tabs[index];
    }
    ui.add_space(size::PANEL_ROW_GAP);

    header_controls(ui, state, model);

    match state.outliner.tab {
        OutlinerTab::Materials => {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| materials_tab(ui, state));
        }
        OutlinerTab::Animations => {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| animations_tab(ui, state, model));
        }
        OutlinerTab::Scene => scene_tab(ui, state, model),
    }
}

/// The header strip, drawn on every tab so the controls never shift underfoot:
/// the flat/tree view toggle and the node-kind filter row (Scene tab only — they
/// say nothing about a material), then the search box, which filters both tabs.
/// Only the kinds the model actually contains get a toggle, so an unrigged mesh
/// never shows a dead bone filter.
fn header_controls(ui: &mut egui::Ui, state: &mut UiState, model: &ModelData) {
    if state.outliner.tab == OutlinerTab::Scene {
        let tree_mode = state.outliner.view == OutlinerViewMode::SceneTree;
        ui.horizontal(|ui| {
            let view_tip = if tree_mode {
                Tip::new(keys::ui_outliner::VIEW_TREE)
                    .describe(keys::ui_outliner::VIEW_TREE_DESCRIPTION)
            } else {
                Tip::new(keys::ui_outliner::VIEW_FLAT)
                    .describe(keys::ui_outliner::VIEW_FLAT_DESCRIPTION)
            };
            if icon_toggle(
                ui,
                &assets::ICON_TREE_VIEW,
                tree_mode,
                true,
                view_tip.page(Page::OutlinerInspector),
            )
            .clicked()
            {
                state.outliner.view = if tree_mode {
                    OutlinerViewMode::Flat
                } else {
                    OutlinerViewMode::SceneTree
                };
            }

            ui.separator();
            for kind in NodeKind::ALL {
                if !model.nodes.iter().any(|node| node.kind == kind) {
                    continue;
                }
                let shown = !state.outliner.hidden_kinds.contains(&kind);
                let kind_tip = Tip::new(keys::ui_outliner::filter_kind(
                    review_localization::tr(labels::node_kind(kind)).into_owned(),
                ))
                .describe(keys::ui_outliner::FILTER_KIND_DESCRIPTION)
                .page(Page::OutlinerInspector);
                if icon_toggle(ui, kind_icon(kind), shown, false, kind_tip).clicked() {
                    if shown {
                        state.outliner.hidden_kinds.insert(kind);
                    } else {
                        state.outliner.hidden_kinds.remove(&kind);
                    }
                }
            }
        });
        ui.add_space(size::PANEL_ROW_GAP);
    }

    let search = ui.add(
        egui::TextEdit::singleline(&mut state.outliner.search)
            .hint_text(keys::ui_outliner::SEARCH_HINT)
            .desired_width(f32::INFINITY),
    );
    tip(
        search,
        Tip::new(keys::ui_outliner::SEARCH_HINT)
            .describe(keys::ui_outliner::SEARCH_HINT_DESCRIPTION)
            .page(Page::OutlinerInspector),
    );
    ui.add_space(size::PANEL_ROW_GAP);
}

/// A small icon toggle for the header strip. `accent` picks the selected look:
/// the view toggle fills as an active mode, the filter glyphs merely brighten.
fn icon_toggle(
    ui: &mut egui::Ui,
    icon: &AppIcon,
    active: bool,
    accent: bool,
    tooltip: Tip,
) -> egui::Response {
    let side = size::OUTLINER_TYPE_ICON + size::OUTLINER_ICON_PAD;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
    if active && accent {
        ui.painter()
            .rect_filled(rect, size::OUTLINER_ICON_ROUNDING, color::ACCENT);
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, size::OUTLINER_ICON_ROUNDING, color::HOVER_BG);
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if let Some(texture) = assets::load_icon_texture(ui, icon) {
        let tint = if active {
            color::TEXT_PRIMARY
        } else {
            color::OUTLINER_FILTERED
        };
        let image_rect = egui::Rect::from_center_size(
            rect.center(),
            egui::vec2(size::OUTLINER_TYPE_ICON, size::OUTLINER_TYPE_ICON),
        );
        egui::Image::from_texture(texture)
            .fit_to_exact_size(image_rect.size())
            .tint(tint)
            .paint_at(ui, image_rect);
    }
    tip(response, tooltip)
}

/// The Scene tab: resolve the rows the current view/search calls for, draw them,
/// then apply everything the draw and the keyboard asked for.
fn scene_tab(ui: &mut egui::Ui, state: &mut UiState, model: &ModelData) {
    if model.nodes.is_empty() {
        ui.weak(keys::ui_outliner::NO_NODES);
        return;
    }
    state.outliner.ensure_tree(model);

    let query = state.outliner.search.trim().to_lowercase();
    let searching = !query.is_empty();
    let rows = scene_rows(state, model, &query);
    // Guides describe a hierarchy; the flat and search presentations have none.
    let guides = !searching && state.outliner.view == OutlinerViewMode::SceneTree;

    let output = egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if rows.is_empty() {
                ui.weak(if searching {
                    keys::ui_outliner::NO_MATCHES
                } else {
                    keys::ui_outliner::ALL_FILTERED
                });
                return RowsOutput::default();
            }
            draw_rows(ui, state, model, &rows, guides)
        })
        .inner;

    apply_rows_output(state, model, &rows, output);
    handle_nav(ui, state, model, &rows);
}

/// Which flattening the Scene tab is showing: a search overrides the view toggle,
/// because a match inside a collapsed branch would otherwise be invisible.
fn scene_rows(state: &UiState, model: &ModelData, query: &str) -> Vec<TreeRow> {
    if !query.is_empty() {
        return search_rows(model, &state.outliner.hidden_kinds, query);
    }
    match state.outliner.view {
        OutlinerViewMode::Flat => flat_rows(model, &state.outliner.hidden_kinds),
        OutlinerViewMode::SceneTree => visible_tree_rows(
            model,
            &state.outliner.children,
            &state.outliner.roots,
            &state.outliner.collapsed,
            &state.outliner.hidden_kinds,
        ),
    }
}

/// Apply everything the drawn rows asked for, in the order that keeps one frame
/// self-consistent.
fn apply_rows_output(state: &mut UiState, model: &ModelData, rows: &[TreeRow], output: RowsOutput) {
    if output.scrolled {
        state.outliner.scroll_to_selection = false;
    }
    if let Some((node, modifiers)) = output.toggled_eye {
        if modifiers.command {
            isolate_mesh(state, model, node);
        } else if !state.hidden_meshes.remove(&node) {
            state.hidden_meshes.insert(node);
        }
    }
    if let Some(node) = output.toggled_collapse
        && !state.outliner.collapsed.remove(&node)
    {
        state.outliner.collapsed.insert(node);
    }
    if let Some((node, modifiers)) = output.clicked {
        // Clicking a row is also how the Outliner claims the arrow keys.
        state.outliner.nav_focus = true;
        let kind = model.nodes[node].kind;
        let order: Vec<usize> = rows.iter().map(|row| row.node).collect();
        apply_row_click(state, node, kind, modifiers, &order);
    }
}

/// Case-insensitive substring match. `query` is expected already trimmed and
/// lowercased by the caller; an empty one matches everything.
fn matches_search(name: &str, query: &str) -> bool {
    query.is_empty() || name.to_lowercase().contains(query)
}

/// A node's display label: its source name, or a generated `Node N` fallback.
fn display_name(node: &SceneNode, index: usize) -> String {
    if node.name.is_empty() {
        keys::ui_outliner::unnamed_node(index as f64)
    } else {
        node.name.clone()
    }
}

/// Clicking the already-selected row clears the selection; otherwise it selects
/// the clicked target.
fn toggle(already_selected: bool, target: Selection) -> Selection {
    if already_selected {
        Selection::None
    } else {
        target
    }
}
