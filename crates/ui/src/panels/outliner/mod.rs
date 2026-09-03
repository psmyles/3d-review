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
//! [`UiState::selected_bones`] (Ctrl toggles, Shift takes a range), which feeds the
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
mod tree;

use review_model::{ModelData, NodeKind, SceneNode};
use review_render::Selection;

use crate::assets::{self, AppIcon};
use crate::state::{OutlinerTab, OutlinerViewMode, UiState, WorkspaceMode};
use crate::theme::{color, size};
use crate::widgets;

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
    /// Ctrl isolates that mesh instead of toggling it.
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
    let labels: Vec<&str> = tabs
        .iter()
        .map(|tab| match tab {
            OutlinerTab::Scene => "Scene",
            OutlinerTab::Materials => "Materials",
            OutlinerTab::Animations => "Animations",
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
            let tooltip = if tree_mode {
                "Showing the scene hierarchy - click for a flat list"
            } else {
                "Showing a flat node list - click for the scene hierarchy"
            };
            if icon_toggle(ui, &assets::ICON_TREE_VIEW, tree_mode, true, tooltip).clicked() {
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
                let tooltip = format!("{} nodes", kind.label());
                if icon_toggle(ui, kind_icon(kind), shown, false, &tooltip).clicked() {
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

    ui.add(
        egui::TextEdit::singleline(&mut state.outliner.search)
            .hint_text("Search")
            .desired_width(f32::INFINITY),
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
    tooltip: &str,
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
    response.on_hover_text(tooltip)
}

/// The per-kind row glyph.
pub(crate) fn kind_icon(kind: NodeKind) -> &'static AppIcon {
    match kind {
        NodeKind::Mesh => &assets::ICON_NODE_MESH,
        NodeKind::Bone => &assets::ICON_NODE_BONE,
        NodeKind::Light => &assets::ICON_NODE_LIGHT,
        NodeKind::Camera => &assets::ICON_NODE_CAMERA,
        NodeKind::Empty => &assets::ICON_NODE_EMPTY,
        NodeKind::Other => &assets::ICON_NODE_OTHER,
    }
}

/// The per-kind glyph tint, so a row's type reads by color before its name is.
fn kind_color(kind: NodeKind) -> egui::Color32 {
    match kind {
        NodeKind::Mesh => color::NODE_MESH,
        NodeKind::Bone => color::NODE_BONE,
        NodeKind::Light => color::NODE_LIGHT,
        NodeKind::Camera => color::NODE_CAMERA,
        NodeKind::Empty => color::NODE_EMPTY,
        NodeKind::Other => color::NODE_OTHER,
    }
}

/// The Scene tab: resolve the rows the current view/search calls for, draw them,
/// then apply everything the draw and the keyboard asked for.
fn scene_tab(ui: &mut egui::Ui, state: &mut UiState, model: &ModelData) {
    if model.nodes.is_empty() {
        ui.weak("No scene nodes.");
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
                    "No matches."
                } else {
                    "Every node type is filtered out."
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

/// Ctrl+click on a mesh row's eye: hide every *other* mesh node, so only this one
/// is left in the viewport.
///
/// Clicking it again on the mesh that is already alone shows everything back —
/// the same toggle a DCC's isolate gives, so the gesture is its own way out and
/// the user needn't hunt for the eye of each mesh they hid.
///
/// Note it works on the whole node, not its subtree: per-mesh visibility is a set
/// of mesh nodes ([`UiState::hidden_meshes`], which the renderer's hidden filter
/// matches per triangle), so isolating a group node would mean nothing.
fn isolate_mesh(state: &mut UiState, model: &ModelData, node: usize) {
    let meshes: Vec<usize> = model
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, mesh)| mesh.mesh_part.is_some())
        .map(|(index, _)| index)
        .collect();
    let already_isolated = meshes
        .iter()
        .all(|&mesh| state.hidden_meshes.contains(&mesh) != (mesh == node));

    state.hidden_meshes.clear();
    if !already_isolated {
        state
            .hidden_meshes
            .extend(meshes.into_iter().filter(|&mesh| mesh != node));
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
        format!("Node {index}")
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

#[cfg(test)]
mod isolate_tests {
    use super::*;
    use review_model::SceneNode;

    /// Three sibling meshes under one group node — the shape an isolate has to
    /// work over (the group itself carries no mesh, so it is never hidden).
    fn three_meshes() -> ModelData {
        let node = |name: &str, mesh_part: Option<usize>| SceneNode {
            name: name.to_owned(),
            parent: (name != "group").then_some(0),
            mesh_part,
            source_vertex_count: 0,
            transform: glam::Mat4::IDENTITY,
            rest_local: Default::default(),
            kind: if mesh_part.is_some() {
                NodeKind::Mesh
            } else {
                NodeKind::Empty
            },
            bone: None,
        };
        ModelData {
            nodes: vec![
                node("group", None),
                node("a", Some(0)),
                node("b", Some(1)),
                node("c", Some(2)),
            ],
            ..ModelData::default()
        }
    }

    fn hidden(state: &UiState) -> Vec<usize> {
        let mut hidden: Vec<usize> = state.hidden_meshes.iter().copied().collect();
        hidden.sort_unstable();
        hidden
    }

    #[test]
    fn isolating_a_mesh_hides_every_other_mesh() {
        let model = three_meshes();
        let mut state = UiState::default();
        isolate_mesh(&mut state, &model, 2);
        // The two sibling meshes, and not the group node, which has none.
        assert_eq!(hidden(&state), vec![1, 3]);
    }

    #[test]
    fn isolating_the_already_isolated_mesh_shows_everything_again() {
        let model = three_meshes();
        let mut state = UiState::default();
        isolate_mesh(&mut state, &model, 2);
        isolate_mesh(&mut state, &model, 2);
        assert!(hidden(&state).is_empty(), "the gesture is its own way out");
    }

    #[test]
    fn isolating_from_a_partly_hidden_scene_still_leaves_one_mesh() {
        let model = three_meshes();
        let mut state = UiState::default();
        // A different mesh already hidden by hand, and the target hidden too:
        // isolating must show the target and hide the rest, not toggle.
        state.hidden_meshes.insert(1);
        state.hidden_meshes.insert(2);
        isolate_mesh(&mut state, &model, 2);
        assert_eq!(hidden(&state), vec![1, 3]);
    }
}

/// The scene fixture the submodules' tests share. It lives on the parent module
/// so the tree, row-path and navigation tests all walk the exact same hierarchy.
#[cfg(test)]
mod fixture {
    use super::*;
    use review_model::{ModelStats, SceneNode};

    /// A tiny scene:
    ///
    /// ```text
    /// 0 root (Empty)
    /// ├── 1 mesh   (Mesh)
    /// └── 2 hips   (Bone)
    ///     ├── 3 spine (Bone)
    ///     └── 4 lamp  (Light)
    /// ```
    pub(super) fn scene() -> ModelData {
        let node = |name: &str, parent: Option<usize>, kind: NodeKind| SceneNode {
            name: name.to_owned(),
            parent,
            mesh_part: (kind == NodeKind::Mesh).then_some(0),
            source_vertex_count: 0,
            transform: glam::Mat4::IDENTITY,
            rest_local: Default::default(),
            kind,
            bone: (kind == NodeKind::Bone).then(Default::default),
        };
        ModelData {
            nodes: vec![
                node("root", None, NodeKind::Empty),
                node("mesh", Some(0), NodeKind::Mesh),
                node("hips", Some(0), NodeKind::Bone),
                node("spine", Some(2), NodeKind::Bone),
                node("lamp", Some(2), NodeKind::Light),
            ],
            stats: ModelStats {
                gpu_vertex_count: 0,
                bone_count: 2,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Build the adjacency the way `OutlinerState::ensure_tree` does, without
    /// needing a whole `UiState`.
    fn adjacency(model: &ModelData) -> (Vec<Vec<usize>>, Vec<usize>) {
        let mut children = vec![Vec::new(); model.nodes.len()];
        let mut roots = Vec::new();
        for (index, node) in model.nodes.iter().enumerate() {
            match node.parent {
                Some(parent) if parent < model.nodes.len() && parent != index => {
                    children[parent].push(index)
                }
                _ => roots.push(index),
            }
        }
        (children, roots)
    }

    pub(super) fn rows(
        model: &ModelData,
        collapsed: &[usize],
        hidden: &[NodeKind],
    ) -> Vec<TreeRow> {
        let (children, roots) = adjacency(model);
        visible_tree_rows(
            model,
            &children,
            &roots,
            &collapsed.iter().copied().collect(),
            &hidden.iter().copied().collect(),
        )
    }
}
