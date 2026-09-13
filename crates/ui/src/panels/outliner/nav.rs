//! Keyboard navigation and click semantics for the Outliner's rows: routing the
//! arrow keys into the drawn row list, resolving a press against it, and turning
//! a row click (with its modifiers) into the new selection state.

use std::collections::HashSet;

use review_model::{ModelData, NodeKind};
use review_render::Selection;

use super::TreeRow;
use crate::state::{SelectMode, SelectionKind, UiState};

/// One drawn row as a Shift-range sees it: which node it is, and whether it is a
/// bone. The kind is here because a range must not cross the two selection sets
/// — dragging Shift down a hand's bones should not sweep up the mesh row sitting
/// between two of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RowRef {
    pub(super) node: usize,
    pub(super) bone: bool,
}

/// An arrow key, as [`resolve_nav`] understands it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NavKey {
    Up,
    Down,
    Left,
    Right,
}

/// What an arrow press resolves to against the rows currently on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NavAction {
    /// Move the selection to this node.
    Select(usize),
    /// Fold this node's subtree away.
    Collapse(usize),
    /// Unfold this node's subtree.
    Expand(usize),
    /// The press has nowhere to go (an end of the list, a leaf, a root).
    None,
}

/// Route the arrow keys into the row list.
///
/// Any focused widget — the search box above all — owns the arrows first, so
/// typing a query still edits text. Otherwise the Outliner takes them while it
/// holds nav focus (earned by clicking a row or by a previous arrow press) or
/// while the pointer is over the panel, and *consumes* them, so they can never
/// reach `app`'s shortcut table.
pub(super) fn handle_nav(ui: &egui::Ui, state: &mut UiState, model: &ModelData, rows: &[TreeRow]) {
    let ctx = ui.ctx();
    if ctx.memory(|memory| memory.focused()).is_some() {
        state.outliner.nav_focus = false;
        return;
    }

    let panel = ui.max_rect();
    let pointer_inside = ctx
        .pointer_latest_pos()
        .is_some_and(|pos| panel.contains(pos));
    // A press anywhere else hands the arrows back.
    if ctx.input(|input| input.pointer.any_pressed()) && !pointer_inside {
        state.outliner.nav_focus = false;
    }
    if !state.outliner.nav_focus && !pointer_inside {
        return;
    }

    let pressed = [
        (egui::Key::ArrowDown, NavKey::Down),
        (egui::Key::ArrowUp, NavKey::Up),
        (egui::Key::ArrowLeft, NavKey::Left),
        (egui::Key::ArrowRight, NavKey::Right),
    ]
    .into_iter()
    .find(|(key, _)| ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, *key)))
    .map(|(_, nav)| nav);
    let Some(nav) = pressed else {
        return;
    };
    state.outliner.nav_focus = true;

    let current = match state.selection {
        Selection::Node(node) => rows.iter().position(|row| row.node == node),
        _ => None,
    };
    let changed = match resolve_nav(rows, &state.outliner.collapsed, current, nav) {
        // Routed through the click path so a bone still gets its set + anchor;
        // the equality guard keeps the plain-click "re-click clears" out of it.
        NavAction::Select(node) if state.selection != Selection::Node(node) => {
            let kind = model.nodes[node].kind;
            let order = row_refs(model, rows);
            apply_row_click(state, node, kind, egui::Modifiers::NONE, &order);
            true
        }
        NavAction::Collapse(node) => state.outliner.collapsed.insert(node),
        NavAction::Expand(node) => state.outliner.collapsed.remove(&node),
        NavAction::Select(_) | NavAction::None => false,
    };
    if changed {
        state.outliner.scroll_to_selection = true;
        // The list changes after this frame's draw, so ask for one more.
        ctx.request_repaint();
    }
}

/// Resolve an arrow press against the rows on screen.
///
/// * **Down / Up** — the next / previous *selectable* row, so filtered connector
///   rows are stepped over. Clamps at either end; with nothing selected, Down
///   enters at the top and Up at the bottom.
/// * **Left** — fold an expanded node, otherwise climb to its parent row.
/// * **Right** — unfold a collapsed node, otherwise descend to its first child.
///
/// In the flat and search presentations every row is depth 0 with no children, so
/// Left/Right resolve to [`NavAction::None`] and only Down/Up do anything — no
/// special-casing needed.
///
/// Pure, so the navigation matrix is unit-testable without an egui context.
pub(super) fn resolve_nav(
    rows: &[TreeRow],
    collapsed: &HashSet<usize>,
    current: Option<usize>,
    key: NavKey,
) -> NavAction {
    let select = |row: Option<&TreeRow>| match row {
        Some(row) => NavAction::Select(row.node),
        None => NavAction::None,
    };
    let at = current.and_then(|at| rows.get(at).map(|row| (at, row)));

    match key {
        NavKey::Down => match at {
            Some((at, _)) => select(rows[at + 1..].iter().find(|row| row.selectable)),
            None => select(rows.iter().find(|row| row.selectable)),
        },
        NavKey::Up => match at {
            Some((at, _)) => select(rows[..at].iter().rev().find(|row| row.selectable)),
            None => select(rows.iter().rev().find(|row| row.selectable)),
        },
        NavKey::Left => {
            let Some((at, row)) = at else {
                return NavAction::None;
            };
            if row.has_children && !collapsed.contains(&row.node) {
                return NavAction::Collapse(row.node);
            }
            // In depth-first order the nearest row above at a shallower depth is
            // exactly this row's parent.
            match rows[..at]
                .iter()
                .rev()
                .find(|above| above.depth < row.depth)
            {
                Some(parent) if parent.selectable => NavAction::Select(parent.node),
                _ => NavAction::None,
            }
        }
        NavKey::Right => {
            let Some((at, row)) = at else {
                return NavAction::None;
            };
            if !row.has_children {
                return NavAction::None;
            }
            if collapsed.contains(&row.node) {
                return NavAction::Expand(row.node);
            }
            select(
                rows[at + 1..]
                    .iter()
                    .take_while(|below| below.depth > row.depth)
                    .find(|below| below.selectable),
            )
        }
    }
}

/// Resolve a scene-tree row click into the new selection state.
///
/// * Plain click — select this row alone, and make it the range anchor.
///   Re-clicking a row that is already the *only* selection clears it, which is
///   the flat list's long-standing toggle behaviour and the one way to empty the
///   selection without leaving the panel.
/// * Primary-click — toggle this row's membership. The primary [`Selection`]
///   follows the set's last member, or clears when the set empties.
/// * Shift-click — take every row between the anchor and this one, in the order
///   the rows are currently drawn, so the range matches what the user sees
///   rather than the model's internal node ordering.
///
/// Mesh and bone rows behave identically, over their own sets: a bone click
/// builds [`UiState::selected_bones`] (what the skeleton highlight and the
/// weight heat map read), anything else builds [`UiState::selected_nodes`]. A
/// click on either kind clears the other set, so the skeleton highlight can
/// never outlive the bone selection that produced it.
///
/// Pure (given `state`), so the modifier matrix is unit-testable.
pub(super) fn apply_row_click(
    state: &mut UiState,
    node: usize,
    kind: NodeKind,
    modifiers: egui::Modifiers,
    rows: &[RowRef],
) {
    let bone = kind == NodeKind::Bone;
    let set_kind = if bone {
        SelectionKind::Bone
    } else {
        SelectionKind::Node
    };
    let mode = SelectMode::from_modifiers(modifiers);

    // Shift takes a range here rather than adding one row (which is what it does
    // in the viewport, where there is no row order to sweep along).
    if mode.add
        && !mode.toggle
        && let Some(range) = row_range(rows, state.row_anchor, node, bone)
    {
        state.select_only(node, set_kind);
        *state.selection_set_for(set_kind) = range;
        // The clicked row is the primary regardless of which way the range ran.
        state.selection = Selection::Node(node);
        return;
    }

    if state.apply_select_mode(node, set_kind, mode) {
        return;
    }

    // A plain click on the row that is already the whole selection clears it.
    let alone =
        state.selection == Selection::Node(node) && state.selection_set_for(set_kind).len() <= 1;
    if alone {
        state.clear_selection();
    } else {
        state.select_only(node, set_kind);
    }
}

/// The drawn rows as a Shift-range sees them, reading each node's kind from the
/// model — the one place the two are joined, so a caller never has to.
pub(super) fn row_refs(model: &ModelData, rows: &[TreeRow]) -> Vec<RowRef> {
    rows.iter()
        .map(|row| RowRef {
            node: row.node,
            bone: model.nodes[row.node].kind == NodeKind::Bone,
        })
        .collect()
}

/// The nodes of the same kind between the anchor row and `node`, in drawn order
/// and inclusive of both ends. `None` when there is no anchor, or it has scrolled
/// out of the filtered rows — in which case the caller falls back to a plain
/// click rather than doing nothing.
fn row_range(
    rows: &[RowRef],
    anchor: Option<usize>,
    node: usize,
    bone: bool,
) -> Option<Vec<usize>> {
    let anchor = anchor?;
    let from = rows.iter().position(|row| row.node == anchor)?;
    let to = rows.iter().position(|row| row.node == node)?;
    let (from, to) = if from <= to { (from, to) } else { (to, from) };
    Some(
        rows[from..=to]
            .iter()
            .filter(|row| row.bone == bone)
            .map(|row| row.node)
            .collect(),
    )
}

/// Primary+click on a mesh row's eye (`Ctrl` here, `Cmd` on macOS - egui's own
/// `Modifiers::command`): hide every *other* mesh node, so only this one
/// is left in the viewport.
///
/// Clicking it again on the mesh that is already alone shows everything back —
/// the same toggle a DCC's isolate gives, so the gesture is its own way out and
/// the user needn't hunt for the eye of each mesh they hid.
///
/// Note it works on the whole node, not its subtree: per-mesh visibility is a set
/// of mesh nodes ([`UiState::hidden_meshes`], which the renderer's hidden filter
/// matches per triangle), so isolating a group node would mean nothing.
pub(super) fn isolate_mesh(state: &mut UiState, model: &ModelData, node: usize) {
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
#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::outliner::test_fixture::{rows, scene};
    use crate::panels::outliner::tree::flat_rows;

    // ── resolve_nav ──────────────────────────────────────────────────────────

    fn nav(
        rows: &[TreeRow],
        collapsed: &[usize],
        current: Option<usize>,
        key: NavKey,
    ) -> NavAction {
        resolve_nav(rows, &collapsed.iter().copied().collect(), current, key)
    }

    #[test]
    fn down_and_up_step_one_row() {
        let model = scene();
        let rows = rows(&model, &[], &[]);
        assert_eq!(nav(&rows, &[], Some(0), NavKey::Down), NavAction::Select(1));
        assert_eq!(nav(&rows, &[], Some(2), NavKey::Up), NavAction::Select(1));
    }

    #[test]
    fn down_and_up_clamp_at_the_ends() {
        let model = scene();
        let rows = rows(&model, &[], &[]);
        assert_eq!(nav(&rows, &[], Some(4), NavKey::Down), NavAction::None);
        assert_eq!(nav(&rows, &[], Some(0), NavKey::Up), NavAction::None);
    }

    #[test]
    fn with_nothing_selected_down_enters_at_the_top_and_up_at_the_bottom() {
        let model = scene();
        let rows = rows(&model, &[], &[]);
        assert_eq!(nav(&rows, &[], None, NavKey::Down), NavAction::Select(0));
        assert_eq!(nav(&rows, &[], None, NavKey::Up), NavAction::Select(4));
    }

    #[test]
    fn navigation_steps_over_unselectable_connector_rows() {
        let model = scene();
        // Rows are [root, mesh, hips(connector), lamp]; stepping down from mesh
        // must land on lamp, not on the dimmed hips row.
        let rows = rows(&model, &[], &[NodeKind::Bone]);
        assert_eq!(rows[2].node, 2);
        assert!(!rows[2].selectable);
        assert_eq!(nav(&rows, &[], Some(1), NavKey::Down), NavAction::Select(4));
        assert_eq!(nav(&rows, &[], Some(3), NavKey::Up), NavAction::Select(1));
    }

    #[test]
    fn left_collapses_an_open_parent_then_climbs_to_it() {
        let model = scene();
        let rows = rows(&model, &[], &[]);
        // hips (row 2) is open, so Left folds it.
        assert_eq!(
            nav(&rows, &[], Some(2), NavKey::Left),
            NavAction::Collapse(2)
        );
        // Already collapsed, so Left climbs to the parent instead.
        assert_eq!(
            nav(&rows, &[2], Some(2), NavKey::Left),
            NavAction::Select(0)
        );
        // A leaf climbs straight away.
        assert_eq!(nav(&rows, &[], Some(3), NavKey::Left), NavAction::Select(2));
    }

    #[test]
    fn left_on_a_root_has_nowhere_to_go() {
        let model = scene();
        let rows = rows(&model, &[0], &[]);
        assert_eq!(nav(&rows, &[0], Some(0), NavKey::Left), NavAction::None);
    }

    #[test]
    fn right_expands_a_collapsed_parent_then_descends_into_it() {
        let model = scene();
        let folded = rows(&model, &[2], &[]);
        assert_eq!(
            nav(&folded, &[2], Some(2), NavKey::Right),
            NavAction::Expand(2)
        );

        let open = rows(&model, &[], &[]);
        assert_eq!(
            nav(&open, &[], Some(2), NavKey::Right),
            NavAction::Select(3)
        );
        assert_eq!(
            nav(&open, &[], Some(3), NavKey::Right),
            NavAction::None,
            "a leaf has nothing to descend into"
        );
    }

    #[test]
    fn a_flat_list_navigates_vertically_only() {
        let model = scene();
        let rows = flat_rows(&model, &HashSet::new());
        assert_eq!(nav(&rows, &[], Some(1), NavKey::Down), NavAction::Select(2));
        assert_eq!(nav(&rows, &[], Some(1), NavKey::Left), NavAction::None);
        assert_eq!(nav(&rows, &[], Some(1), NavKey::Right), NavAction::None);
    }

    #[test]
    fn navigating_an_empty_list_does_nothing() {
        for key in [NavKey::Up, NavKey::Down, NavKey::Left, NavKey::Right] {
            assert_eq!(nav(&[], &[], None, key), NavAction::None);
            assert_eq!(nav(&[], &[], Some(0), key), NavAction::None);
        }
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
    /// filtered or collapsed, with every row a bone — the shape the bone-range
    /// cases below need. [`mesh_rows`] is its non-bone twin.
    fn rows_of(bones: [bool; 5]) -> Vec<RowRef> {
        (0..5)
            .map(|node| RowRef {
                node,
                bone: bones[node],
            })
            .collect()
    }

    /// Every row a bone.
    fn bone_rows() -> Vec<RowRef> {
        rows_of([true; 5])
    }

    /// Every row a mesh.
    fn mesh_rows() -> Vec<RowRef> {
        rows_of([false; 5])
    }

    #[test]
    fn plain_click_on_a_bone_selects_just_it() {
        let mut state = UiState::default();
        apply_row_click(
            &mut state,
            2,
            NodeKind::Bone,
            mods(false, false),
            &bone_rows(),
        );
        assert_eq!(state.selection, Selection::Node(2));
        assert_eq!(state.selected_bones, vec![2]);
        assert_eq!(state.row_anchor, Some(2));
    }

    #[test]
    fn re_clicking_the_only_selected_bone_clears_it() {
        let mut state = UiState::default();
        apply_row_click(
            &mut state,
            2,
            NodeKind::Bone,
            mods(false, false),
            &bone_rows(),
        );
        apply_row_click(
            &mut state,
            2,
            NodeKind::Bone,
            mods(false, false),
            &bone_rows(),
        );
        assert_eq!(state.selection, Selection::None);
        assert!(state.selected_bones.is_empty());
        assert_eq!(state.row_anchor, None);
    }

    #[test]
    fn ctrl_click_toggles_membership_and_tracks_the_primary() {
        let mut state = UiState::default();
        apply_row_click(
            &mut state,
            2,
            NodeKind::Bone,
            mods(false, false),
            &bone_rows(),
        );
        apply_row_click(
            &mut state,
            3,
            NodeKind::Bone,
            mods(true, false),
            &bone_rows(),
        );
        assert_eq!(state.selected_bones, vec![2, 3]);
        assert_eq!(
            state.selection,
            Selection::Node(3),
            "primary follows the last member"
        );

        // Primary-clicking a member again removes it; the primary falls back.
        apply_row_click(
            &mut state,
            3,
            NodeKind::Bone,
            mods(true, false),
            &bone_rows(),
        );
        assert_eq!(state.selected_bones, vec![2]);
        assert_eq!(state.selection, Selection::Node(2));

        // Emptying the set clears the selection rather than leaving a stale node.
        apply_row_click(
            &mut state,
            2,
            NodeKind::Bone,
            mods(true, false),
            &bone_rows(),
        );
        assert!(state.selected_bones.is_empty());
        assert_eq!(state.selection, Selection::None);
    }

    #[test]
    fn shift_click_takes_the_range_in_visible_row_order() {
        let mut state = UiState::default();
        apply_row_click(
            &mut state,
            1,
            NodeKind::Bone,
            mods(false, false),
            &bone_rows(),
        );
        apply_row_click(
            &mut state,
            4,
            NodeKind::Bone,
            mods(false, true),
            &bone_rows(),
        );
        assert_eq!(state.selected_bones, vec![1, 2, 3, 4]);
        assert_eq!(state.selection, Selection::Node(4));
    }

    #[test]
    fn shift_click_works_upward_too() {
        let mut state = UiState::default();
        apply_row_click(
            &mut state,
            4,
            NodeKind::Bone,
            mods(false, false),
            &bone_rows(),
        );
        apply_row_click(
            &mut state,
            2,
            NodeKind::Bone,
            mods(false, true),
            &bone_rows(),
        );
        assert_eq!(state.selected_bones, vec![2, 3, 4]);
        // The clicked row is the primary regardless of which way the range ran.
        assert_eq!(state.selection, Selection::Node(2));
    }

    #[test]
    fn shift_click_without_an_anchor_falls_back_to_a_plain_click() {
        let mut state = UiState::default();
        apply_row_click(
            &mut state,
            3,
            NodeKind::Bone,
            mods(false, true),
            &bone_rows(),
        );
        assert_eq!(state.selected_bones, vec![3]);
        assert_eq!(state.selection, Selection::Node(3));
    }

    #[test]
    fn clicking_a_non_bone_row_clears_the_bone_selection() {
        let mut state = UiState::default();
        apply_row_click(
            &mut state,
            2,
            NodeKind::Bone,
            mods(false, false),
            &bone_rows(),
        );
        apply_row_click(
            &mut state,
            3,
            NodeKind::Bone,
            mods(true, false),
            &bone_rows(),
        );
        assert_eq!(state.selected_bones.len(), 2);

        // The skeleton highlight and the weight heat map must not outlive the
        // bone selection; the mesh row becomes the new selection and the new
        // anchor, so a Shift-click can range from it.
        apply_row_click(
            &mut state,
            1,
            NodeKind::Mesh,
            mods(false, false),
            &bone_rows(),
        );
        assert_eq!(state.selection, Selection::Node(1));
        assert_eq!(state.selected_nodes, vec![1]);
        assert!(state.selected_bones.is_empty());
        assert_eq!(state.row_anchor, Some(1));
    }

    #[test]
    fn ctrl_click_on_a_mesh_toggles_membership_and_tracks_the_primary() {
        let mut state = UiState::default();
        apply_row_click(
            &mut state,
            1,
            NodeKind::Mesh,
            mods(false, false),
            &mesh_rows(),
        );
        apply_row_click(
            &mut state,
            3,
            NodeKind::Mesh,
            mods(true, false),
            &mesh_rows(),
        );
        assert_eq!(state.selected_nodes, vec![1, 3]);
        assert_eq!(
            state.selection,
            Selection::Node(3),
            "primary follows the last member"
        );

        // Clicking a member again removes it; the primary falls back.
        apply_row_click(
            &mut state,
            3,
            NodeKind::Mesh,
            mods(true, false),
            &mesh_rows(),
        );
        assert_eq!(state.selected_nodes, vec![1]);
        assert_eq!(state.selection, Selection::Node(1));

        // Emptying the set clears the selection rather than leaving a stale node.
        apply_row_click(
            &mut state,
            1,
            NodeKind::Mesh,
            mods(true, false),
            &mesh_rows(),
        );
        assert!(state.selected_nodes.is_empty());
        assert_eq!(state.selection, Selection::None);
    }

    #[test]
    fn shift_click_on_meshes_takes_the_visible_range() {
        let mut state = UiState::default();
        apply_row_click(
            &mut state,
            1,
            NodeKind::Mesh,
            mods(false, false),
            &mesh_rows(),
        );
        apply_row_click(
            &mut state,
            4,
            NodeKind::Mesh,
            mods(false, true),
            &mesh_rows(),
        );
        assert_eq!(state.selected_nodes, vec![1, 2, 3, 4]);
        assert_eq!(state.selection, Selection::Node(4));
        assert!(state.selected_bones.is_empty());
    }

    /// A range must not cross the two sets: sweeping down a run of bones skips
    /// the mesh row sitting between them, and vice versa.
    #[test]
    fn a_range_takes_only_rows_of_its_own_kind() {
        // Rows 0-4, with row 2 a mesh among bones.
        let mixed = rows_of([true, true, false, true, true]);
        let mut state = UiState::default();
        apply_row_click(&mut state, 1, NodeKind::Bone, mods(false, false), &mixed);
        apply_row_click(&mut state, 4, NodeKind::Bone, mods(false, true), &mixed);
        assert_eq!(
            state.selected_bones,
            vec![1, 3, 4],
            "the mesh row is skipped"
        );

        let mut state = UiState::default();
        apply_row_click(&mut state, 2, NodeKind::Mesh, mods(false, false), &mixed);
        apply_row_click(&mut state, 4, NodeKind::Mesh, mods(false, true), &mixed);
        assert_eq!(
            state.selected_nodes,
            vec![2],
            "no other mesh row in the range"
        );
    }

    #[test]
    fn re_clicking_the_only_selected_mesh_clears_it() {
        let mut state = UiState::default();
        apply_row_click(
            &mut state,
            1,
            NodeKind::Mesh,
            mods(false, false),
            &mesh_rows(),
        );
        apply_row_click(
            &mut state,
            1,
            NodeKind::Mesh,
            mods(false, false),
            &mesh_rows(),
        );
        assert_eq!(state.selection, Selection::None);
        assert!(state.selected_nodes.is_empty());
        assert_eq!(state.row_anchor, None);
    }

    /// Plain-clicking the primary of a *multi* selection collapses to it rather
    /// than clearing — the click still says something, so it must not read as
    /// "deselect everything".
    #[test]
    fn plain_clicking_the_primary_of_a_multi_selection_collapses_to_it() {
        let mut state = UiState::default();
        apply_row_click(
            &mut state,
            1,
            NodeKind::Mesh,
            mods(false, false),
            &mesh_rows(),
        );
        apply_row_click(
            &mut state,
            3,
            NodeKind::Mesh,
            mods(true, false),
            &mesh_rows(),
        );
        apply_row_click(
            &mut state,
            3,
            NodeKind::Mesh,
            mods(false, false),
            &mesh_rows(),
        );
        assert_eq!(state.selected_nodes, vec![3]);
        assert_eq!(state.selection, Selection::Node(3));
    }

    #[test]
    fn selected_node_set_is_sorted_and_deduped() {
        let mut state = UiState::default();
        apply_row_click(
            &mut state,
            3,
            NodeKind::Mesh,
            mods(false, false),
            &mesh_rows(),
        );
        apply_row_click(
            &mut state,
            1,
            NodeKind::Mesh,
            mods(true, false),
            &mesh_rows(),
        );
        assert_eq!(state.selected_nodes, vec![3, 1], "click order is preserved");
        assert_eq!(state.selected_node_set(), vec![1, 3]);
    }

    #[test]
    fn selected_bone_nodes_is_sorted_and_deduped() {
        let mut state = UiState::default();
        // Click order is 3 then 2, but the renderer wants a sorted set it can
        // binary-search.
        apply_row_click(
            &mut state,
            3,
            NodeKind::Bone,
            mods(false, false),
            &bone_rows(),
        );
        apply_row_click(
            &mut state,
            2,
            NodeKind::Bone,
            mods(true, false),
            &bone_rows(),
        );
        assert_eq!(state.selected_bones, vec![3, 2], "click order is preserved");
        assert_eq!(state.selected_bone_nodes(), vec![2, 3]);
    }
}
