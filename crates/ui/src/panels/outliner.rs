//! The Outliner: a two-tab list (the scene's geometry + the deduplicated
//! materials) that drives the [`Selection`] used by the viewport highlight and
//! the Inspector.
//!
//! The Geometry tab has two presentations, switched by the header's tree button
//! ([`UiState::outliner_view`]):
//!
//! * **Flat** — the original list of mesh-bearing nodes, each a stock visibility
//!   checkbox + a selectable mesh name.
//! * **Scene tree** — the full node hierarchy (bones, lights, cameras, groups and
//!   meshes alike), indented and collapsible, with a per-kind glyph on every row
//!   and a type-filter toggle row above it.
//!
//! Clicking a row's name sets [`UiState::selection`]; on a *bone* row the
//! modifiers additionally build [`UiState::selected_bones`] (Ctrl toggles, Shift
//! takes a range), which feeds the skeleton overlay's highlight and the
//! skin-weight heat map. Reads the scene nodes from the borrowed [`ModelData`]
//! (invariant 2: borrowed, not owned) and the material list from the app→UI
//! snapshot.

use std::collections::HashSet;

use review_model::{ModelData, NodeKind, SceneNode};
use review_render::Selection;

use crate::assets::{self, AppIcon};
use crate::state::{OutlinerTab, OutlinerViewMode, UiState};
use crate::theme::{color, size};
use crate::widgets;

/// The Outliner's tabs, in strip order. The index into this array is what
/// [`widgets::tab_bar`] hands back on a click.
const TABS: [OutlinerTab; 2] = [OutlinerTab::Geometry, OutlinerTab::Materials];

/// One row of the flattened scene tree, as [`visible_tree_rows`] resolves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TreeRow {
    /// Index into [`ModelData::nodes`].
    pub(crate) node: usize,
    /// Indent level; roots are 0.
    pub(crate) depth: usize,
    /// Whether this node has children that survive the current filter — i.e.
    /// whether it gets a disclosure arrow.
    pub(crate) has_children: bool,
    /// `false` when the type filter hides this node's kind and the row is only
    /// present to keep a visible descendant connected. Such rows paint dim and
    /// don't accept clicks.
    pub(crate) selectable: bool,
}

pub(crate) fn body(ui: &mut egui::Ui, state: &mut UiState, model: &ModelData) {
    // ── Tabs: Geometry / Materials, as a full-width underlined tab strip ─────
    let materials_label = format!("Materials ({})", state.materials_snapshot.len());
    let labels = ["Geometry", materials_label.as_str()];
    let active = TABS
        .iter()
        .position(|tab| *tab == state.outliner_tab)
        .unwrap_or(0);
    if let Some(index) = widgets::tab_bar(ui, &labels, active) {
        state.outliner_tab = TABS[index];
    }
    ui.add_space(size::PANEL_ROW_GAP);

    if state.outliner_tab == OutlinerTab::Geometry {
        header_controls(ui, state, model);
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match state.outliner_tab {
            OutlinerTab::Geometry => match state.outliner_view {
                OutlinerViewMode::FlatGeometry => geometry_tab(ui, state, model),
                OutlinerViewMode::SceneTree => scene_tree_tab(ui, state, model),
            },
            OutlinerTab::Materials => materials_tab(ui, state),
        });
}

/// The Geometry tab's header strip: the flat/tree view toggle, and — in tree mode
/// — the node-kind filter row. Only the kinds the model actually contains get a
/// toggle, so an unrigged mesh never shows a dead bone filter.
fn header_controls(ui: &mut egui::Ui, state: &mut UiState, model: &ModelData) {
    let tree_mode = state.outliner_view == OutlinerViewMode::SceneTree;
    ui.horizontal(|ui| {
        if icon_toggle(
            ui,
            &assets::ICON_TREE_VIEW,
            tree_mode,
            true,
            "Show the full scene hierarchy",
        )
        .clicked()
        {
            state.outliner_view = if tree_mode {
                OutlinerViewMode::FlatGeometry
            } else {
                OutlinerViewMode::SceneTree
            };
        }

        if !tree_mode {
            return;
        }

        ui.separator();
        for kind in NodeKind::ALL {
            if !model.nodes.iter().any(|node| node.kind == kind) {
                continue;
            }
            let shown = !state.hidden_kinds.contains(&kind);
            let tooltip = format!("{} nodes", kind.label());
            if icon_toggle(ui, kind_icon(kind), shown, false, &tooltip).clicked() {
                if shown {
                    state.hidden_kinds.insert(kind);
                } else {
                    state.hidden_kinds.remove(&kind);
                }
            }
        }
    });
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

/// The Geometry tab's flat presentation: the model's mesh-bearing nodes, each a
/// stock visibility checkbox + a selectable mesh name. Group / bone / other node
/// types live in the scene-tree presentation instead.
fn geometry_tab(ui: &mut egui::Ui, state: &mut UiState, model: &ModelData) {
    let has_mesh = model.nodes.iter().any(|node| node.mesh_part.is_some());
    if !has_mesh {
        ui.weak("No meshes.");
        return;
    }

    let selection = state.selection;
    let mut clicked: Option<Selection> = None;

    for (index, node) in model.nodes.iter().enumerate() {
        if node.mesh_part.is_none() {
            continue;
        }
        let name = display_name(node, index);
        let selected = selection == Selection::Node(index);
        ui.horizontal(|ui| {
            visibility_checkbox(ui, state, index);
            // Name toggles the selection.
            if ui.selectable_label(selected, &name).clicked() {
                clicked = Some(toggle(selected, Selection::Node(index)));
            }
        });
    }

    if let Some(new_selection) = clicked {
        state.selection = new_selection;
        // The flat list has no bone rows, so any bone multi-selection is stale.
        state.selected_bones.clear();
        state.bone_anchor = None;
    }
}

/// The Geometry tab's scene-tree presentation: the whole node hierarchy, filtered
/// by kind, indented by depth, collapsible, with modifier-aware bone selection.
fn scene_tree_tab(ui: &mut egui::Ui, state: &mut UiState, model: &ModelData) {
    if model.nodes.is_empty() {
        ui.weak("No scene nodes.");
        return;
    }

    state.ensure_outliner_tree(model);
    let rows = visible_tree_rows(
        model,
        &state.outliner_children,
        &state.outliner_roots,
        &state.outliner_collapsed,
        &state.hidden_kinds,
    );
    if rows.is_empty() {
        ui.weak("Every node type is filtered out.");
        return;
    }

    // The click is resolved after the draw: `apply_row_click` needs the full
    // visible row order for Shift-range selection, and mutating selection state
    // mid-draw would let rows within one frame disagree about what is selected.
    let mut clicked: Option<(usize, egui::Modifiers)> = None;
    let mut toggled_collapse: Option<usize> = None;

    for row in &rows {
        let node = &model.nodes[row.node];
        ui.horizontal(|ui| {
            ui.add_space(row.depth as f32 * size::OUTLINER_INDENT);

            // Disclosure arrow — a fixed-width slot either way, so names line up.
            let (_, arrow_response) = ui.allocate_exact_size(
                egui::vec2(size::OUTLINER_ARROW_WIDTH, size::OUTLINER_TYPE_ICON),
                if row.has_children {
                    egui::Sense::click()
                } else {
                    egui::Sense::hover()
                },
            );
            if row.has_children {
                let openness = if state.outliner_collapsed.contains(&row.node) {
                    0.0
                } else {
                    1.0
                };
                egui::collapsing_header::paint_default_icon(ui, openness, &arrow_response);
                if arrow_response.clicked() {
                    toggled_collapse = Some(row.node);
                }
                if arrow_response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
            }

            // Kind glyph.
            let (icon_rect, _) = ui.allocate_exact_size(
                egui::vec2(size::OUTLINER_TYPE_ICON, size::OUTLINER_TYPE_ICON),
                egui::Sense::hover(),
            );
            if let Some(texture) = assets::load_icon_texture(ui, kind_icon(node.kind)) {
                let tint = if row.selectable {
                    color::TEXT_MUTED
                } else {
                    color::OUTLINER_FILTERED
                };
                egui::Image::from_texture(texture)
                    .fit_to_exact_size(icon_rect.size())
                    .tint(tint)
                    .paint_at(ui, icon_rect);
            }

            if node.mesh_part.is_some() && row.selectable {
                visibility_checkbox(ui, state, row.node);
            }

            let name = display_name(node, row.node);
            if !row.selectable {
                // Present only to keep a visible descendant attached to its
                // ancestry — readable as structure, but not a target.
                ui.label(egui::RichText::new(name).color(color::OUTLINER_FILTERED));
                return;
            }

            let selected = state.selection == Selection::Node(row.node)
                || state.selected_bones.contains(&row.node);
            if ui.selectable_label(selected, &name).clicked() {
                clicked = Some((row.node, ui.input(|input| input.modifiers)));
            }
        });
    }

    if let Some(node) = toggled_collapse
        && !state.outliner_collapsed.remove(&node)
    {
        state.outliner_collapsed.insert(node);
    }
    if let Some((node, modifiers)) = clicked {
        let kind = model.nodes[node].kind;
        let order: Vec<usize> = rows.iter().map(|row| row.node).collect();
        apply_row_click(state, node, kind, modifiers, &order);
    }
}

/// The per-row visibility checkbox, toggling membership in
/// [`UiState::hidden_meshes`].
fn visibility_checkbox(ui: &mut egui::Ui, state: &mut UiState, index: usize) {
    let mut visible = !state.hidden_meshes.contains(&index);
    if ui.checkbox(&mut visible, "").changed() {
        if visible {
            state.hidden_meshes.remove(&index);
        } else {
            state.hidden_meshes.insert(index);
        }
    }
}

/// Whether any descendant of `node` survives the kind filter.
fn has_shown_descendant(
    node: usize,
    model: &ModelData,
    children: &[Vec<usize>],
    hidden_kinds: &HashSet<NodeKind>,
) -> bool {
    children.get(node).into_iter().flatten().any(|&child| {
        !hidden_kinds.contains(&model.nodes[child].kind)
            || has_shown_descendant(child, model, children, hidden_kinds)
    })
}

fn walk_tree(
    node: usize,
    depth: usize,
    model: &ModelData,
    children: &[Vec<usize>],
    collapsed: &HashSet<usize>,
    hidden_kinds: &HashSet<NodeKind>,
    out: &mut Vec<TreeRow>,
) {
    let shown = !hidden_kinds.contains(&model.nodes[node].kind);
    let descendant_shown = has_shown_descendant(node, model, children, hidden_kinds);
    if !shown && !descendant_shown {
        return;
    }
    out.push(TreeRow {
        node,
        depth,
        has_children: descendant_shown,
        selectable: shown,
    });
    if collapsed.contains(&node) {
        return;
    }
    for &child in children.get(node).into_iter().flatten() {
        walk_tree(
            child,
            depth + 1,
            model,
            children,
            collapsed,
            hidden_kinds,
            out,
        );
    }
}

/// Flatten the hierarchy into the rows to draw, honoring the collapsed set and
/// the kind filter.
///
/// A node is *shown* when its kind isn't filtered out. A filtered node is still
/// emitted — as a non-selectable row — if any descendant is shown, so the tree
/// never splits into orphaned fragments; a filtered subtree with nothing shown in
/// it is dropped entirely. Collapsing hides a node's descendants but never the
/// node itself.
///
/// Pure, so the filter/collapse matrix is unit-testable without an egui context.
pub(crate) fn visible_tree_rows(
    model: &ModelData,
    children: &[Vec<usize>],
    roots: &[usize],
    collapsed: &HashSet<usize>,
    hidden_kinds: &HashSet<NodeKind>,
) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    for &root in roots {
        walk_tree(root, 0, model, children, collapsed, hidden_kinds, &mut rows);
    }
    rows
}

/// Resolve a scene-tree row click into the new selection state.
///
/// * Plain click — select this node alone. On a bone that also becomes the whole
///   bone set and the range anchor; re-clicking the selected row clears both,
///   preserving the flat list's long-standing toggle behavior.
/// * Ctrl-click on a bone — toggle its membership. The primary [`Selection`]
///   follows the set's last member, or clears when the set empties.
/// * Shift-click on a bone — take every row between the anchor and this one, in
///   the order the rows are currently drawn, so the range matches what the user
///   sees rather than the model's internal node ordering.
///
/// Clicking any non-bone row clears the bone set: the skeleton highlight and the
/// weight heat map should never outlive the bone selection that produced them.
///
/// Pure (given `state`), so the modifier matrix is unit-testable.
pub(crate) fn apply_row_click(
    state: &mut UiState,
    node: usize,
    kind: NodeKind,
    modifiers: egui::Modifiers,
    visible_order: &[usize],
) {
    if kind != NodeKind::Bone {
        let selected = state.selection == Selection::Node(node);
        state.selection = toggle(selected, Selection::Node(node));
        state.selected_bones.clear();
        state.bone_anchor = None;
        return;
    }

    if modifiers.command || modifiers.ctrl {
        if let Some(at) = state.selected_bones.iter().position(|&bone| bone == node) {
            state.selected_bones.remove(at);
        } else {
            state.selected_bones.push(node);
        }
        state.bone_anchor = Some(node);
        state.selection = match state.selected_bones.last() {
            Some(&last) => Selection::Node(last),
            None => Selection::None,
        };
        return;
    }

    // No usable anchor (or a stale one no longer on screen) falls through to a
    // plain selection rather than doing nothing.
    if modifiers.shift
        && let Some(anchor) = state.bone_anchor
        && let Some(from) = visible_order.iter().position(|&row| row == anchor)
        && let Some(to) = visible_order.iter().position(|&row| row == node)
    {
        let (from, to) = if from <= to { (from, to) } else { (to, from) };
        state.selected_bones = visible_order[from..=to].to_vec();
        // The clicked row is the primary regardless of range direction.
        state.selection = Selection::Node(node);
        return;
    }

    let already = state.selection == Selection::Node(node) && state.selected_bones.len() == 1;
    if already {
        state.selection = Selection::None;
        state.selected_bones.clear();
        state.bone_anchor = None;
    } else {
        state.selection = Selection::Node(node);
        state.selected_bones = vec![node];
        state.bone_anchor = Some(node);
    }
}

/// The materials tab: a flat, deduplicated list of the model's editable materials,
/// each a stock selectable name row.
fn materials_tab(ui: &mut egui::Ui, state: &mut UiState) {
    if state.materials_snapshot.is_empty() {
        ui.weak("No materials.");
        return;
    }

    let selection = state.selection;
    let mut clicked: Option<Selection> = None;
    for (index, material) in state.materials_snapshot.iter().enumerate() {
        let selected = selection == Selection::Material(index);
        if ui.selectable_label(selected, &material.name).clicked() {
            clicked = Some(toggle(selected, Selection::Material(index)));
        }
    }

    if let Some(new_selection) = clicked {
        state.selection = new_selection;
        state.selected_bones.clear();
        state.bone_anchor = None;
    }
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
mod tests {
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
    fn scene() -> ModelData {
        let node = |name: &str, parent: Option<usize>, kind: NodeKind| SceneNode {
            name: name.to_owned(),
            parent,
            mesh_part: (kind == NodeKind::Mesh).then_some(0),
            transform: glam::Mat4::IDENTITY,
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
                bone_count: 2,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Build the adjacency the way `UiState::ensure_outliner_tree` does, without
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

    fn rows(model: &ModelData, collapsed: &[usize], hidden: &[NodeKind]) -> Vec<TreeRow> {
        let (children, roots) = adjacency(model);
        visible_tree_rows(
            model,
            &children,
            &roots,
            &collapsed.iter().copied().collect(),
            &hidden.iter().copied().collect(),
        )
    }

    #[test]
    fn tree_lists_every_node_depth_first_with_indent() {
        let model = scene();
        let rows = rows(&model, &[], &[]);
        assert_eq!(
            rows.iter()
                .map(|row| (row.node, row.depth))
                .collect::<Vec<_>>(),
            vec![(0, 0), (1, 1), (2, 1), (3, 2), (4, 2)]
        );
        // Only the two nodes that actually have children get a disclosure arrow.
        assert!(rows[0].has_children, "root parents the mesh and the hips");
        assert!(!rows[1].has_children, "the mesh is a leaf");
        assert!(rows[2].has_children, "hips parents spine + lamp");
        assert!(rows.iter().all(|row| row.selectable));
    }

    #[test]
    fn collapsing_hides_descendants_but_not_the_node() {
        let model = scene();
        let rows = rows(&model, &[2], &[]);
        assert_eq!(
            rows.iter().map(|row| row.node).collect::<Vec<_>>(),
            vec![0, 1, 2],
            "hips stays, its subtree is folded away"
        );
        assert!(
            rows[2].has_children,
            "a collapsed node keeps its arrow so it can be reopened"
        );
    }

    #[test]
    fn filtering_a_kind_keeps_ancestors_that_lead_to_visible_nodes() {
        let model = scene();
        // Hide bones: hips is a bone, but it still has to appear so the light
        // beneath it stays attached to the hierarchy — as a dimmed, unselectable
        // connector row.
        let rows = rows(&model, &[], &[NodeKind::Bone]);
        assert_eq!(
            rows.iter()
                .map(|row| (row.node, row.selectable))
                .collect::<Vec<_>>(),
            vec![(0, true), (1, true), (2, false), (4, true)],
            "spine (a bone with no visible descendant) drops out entirely"
        );
    }

    #[test]
    fn a_subtree_with_nothing_visible_is_dropped() {
        let model = scene();
        // Hiding bones *and* lights leaves nothing under hips, so hips itself goes.
        let rows = rows(&model, &[], &[NodeKind::Bone, NodeKind::Light]);
        assert_eq!(
            rows.iter().map(|row| row.node).collect::<Vec<_>>(),
            vec![0, 1]
        );
    }

    #[test]
    fn filtering_every_kind_yields_no_rows() {
        let model = scene();
        assert!(rows(&model, &[], &NodeKind::ALL).is_empty());
    }

    // ── apply_row_click ──────────────────────────────────────────────────────

    fn mods(ctrl: bool, shift: bool) -> egui::Modifiers {
        egui::Modifiers {
            ctrl,
            shift,
            command: ctrl,
            ..Default::default()
        }
    }

    /// The visible row order the scene tree draws for `scene()` with nothing
    /// filtered or collapsed.
    const ORDER: [usize; 5] = [0, 1, 2, 3, 4];

    #[test]
    fn plain_click_on_a_bone_selects_just_it() {
        let mut state = UiState::default();
        apply_row_click(&mut state, 2, NodeKind::Bone, mods(false, false), &ORDER);
        assert_eq!(state.selection, Selection::Node(2));
        assert_eq!(state.selected_bones, vec![2]);
        assert_eq!(state.bone_anchor, Some(2));
    }

    #[test]
    fn re_clicking_the_only_selected_bone_clears_it() {
        let mut state = UiState::default();
        apply_row_click(&mut state, 2, NodeKind::Bone, mods(false, false), &ORDER);
        apply_row_click(&mut state, 2, NodeKind::Bone, mods(false, false), &ORDER);
        assert_eq!(state.selection, Selection::None);
        assert!(state.selected_bones.is_empty());
        assert_eq!(state.bone_anchor, None);
    }

    #[test]
    fn ctrl_click_toggles_membership_and_tracks_the_primary() {
        let mut state = UiState::default();
        apply_row_click(&mut state, 2, NodeKind::Bone, mods(false, false), &ORDER);
        apply_row_click(&mut state, 3, NodeKind::Bone, mods(true, false), &ORDER);
        assert_eq!(state.selected_bones, vec![2, 3]);
        assert_eq!(
            state.selection,
            Selection::Node(3),
            "primary follows the last member"
        );

        // Ctrl-clicking a member again removes it; the primary falls back.
        apply_row_click(&mut state, 3, NodeKind::Bone, mods(true, false), &ORDER);
        assert_eq!(state.selected_bones, vec![2]);
        assert_eq!(state.selection, Selection::Node(2));

        // Emptying the set clears the selection rather than leaving a stale node.
        apply_row_click(&mut state, 2, NodeKind::Bone, mods(true, false), &ORDER);
        assert!(state.selected_bones.is_empty());
        assert_eq!(state.selection, Selection::None);
    }

    #[test]
    fn shift_click_takes_the_range_in_visible_row_order() {
        let mut state = UiState::default();
        apply_row_click(&mut state, 1, NodeKind::Bone, mods(false, false), &ORDER);
        apply_row_click(&mut state, 4, NodeKind::Bone, mods(false, true), &ORDER);
        assert_eq!(state.selected_bones, vec![1, 2, 3, 4]);
        assert_eq!(state.selection, Selection::Node(4));
    }

    #[test]
    fn shift_click_works_upward_too() {
        let mut state = UiState::default();
        apply_row_click(&mut state, 4, NodeKind::Bone, mods(false, false), &ORDER);
        apply_row_click(&mut state, 2, NodeKind::Bone, mods(false, true), &ORDER);
        assert_eq!(state.selected_bones, vec![2, 3, 4]);
        // The clicked row is the primary regardless of which way the range ran.
        assert_eq!(state.selection, Selection::Node(2));
    }

    #[test]
    fn shift_click_without_an_anchor_falls_back_to_a_plain_click() {
        let mut state = UiState::default();
        apply_row_click(&mut state, 3, NodeKind::Bone, mods(false, true), &ORDER);
        assert_eq!(state.selected_bones, vec![3]);
        assert_eq!(state.selection, Selection::Node(3));
    }

    #[test]
    fn clicking_a_non_bone_row_clears_the_bone_selection() {
        let mut state = UiState::default();
        apply_row_click(&mut state, 2, NodeKind::Bone, mods(false, false), &ORDER);
        apply_row_click(&mut state, 3, NodeKind::Bone, mods(true, false), &ORDER);
        assert_eq!(state.selected_bones.len(), 2);

        // A mesh row is an ordinary single selection — the skeleton highlight and
        // the weight heat map must not outlive the bone selection.
        apply_row_click(&mut state, 1, NodeKind::Mesh, mods(false, false), &ORDER);
        assert_eq!(state.selection, Selection::Node(1));
        assert!(state.selected_bones.is_empty());
        assert_eq!(state.bone_anchor, None);
    }

    #[test]
    fn ctrl_click_on_a_non_bone_is_still_a_plain_selection() {
        let mut state = UiState::default();
        apply_row_click(&mut state, 1, NodeKind::Mesh, mods(true, false), &ORDER);
        assert_eq!(state.selection, Selection::Node(1));
        assert!(state.selected_bones.is_empty());
    }

    #[test]
    fn selected_bone_nodes_is_sorted_and_deduped() {
        let mut state = UiState::default();
        // Click order is 3 then 2, but the renderer wants a sorted set it can
        // binary-search.
        apply_row_click(&mut state, 3, NodeKind::Bone, mods(false, false), &ORDER);
        apply_row_click(&mut state, 2, NodeKind::Bone, mods(true, false), &ORDER);
        assert_eq!(state.selected_bones, vec![3, 2], "click order is preserved");
        assert_eq!(state.selected_bone_nodes(), vec![2, 3]);
    }
}
