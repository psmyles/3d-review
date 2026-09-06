//! Keyboard navigation and click semantics for the Outliner's rows: routing the
//! arrow keys into the drawn row list, resolving a press against it, and turning
//! a row click (with its modifiers) into the new selection state.

use std::collections::HashSet;

use review_model::{ModelData, NodeKind};
use review_render::Selection;

use super::{TreeRow, toggle};
use crate::state::UiState;

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
            let order: Vec<usize> = rows.iter().map(|row| row.node).collect();
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
/// * Plain click — select this node alone. On a bone that also becomes the whole
///   bone set and the range anchor; re-clicking the selected row clears both,
///   preserving the flat list's long-standing toggle behavior.
/// * Primary-click on a bone — toggle its membership. The primary [`Selection`]
///   follows the set's last member, or clears when the set empties.
/// * Shift-click on a bone — take every row between the anchor and this one, in
///   the order the rows are currently drawn, so the range matches what the user
///   sees rather than the model's internal node ordering.
///
/// Clicking any non-bone row clears the bone set: the skeleton highlight and the
/// weight heat map should never outlive the bone selection that produced them.
///
/// Pure (given `state`), so the modifier matrix is unit-testable.
pub(super) fn apply_row_click(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::outliner::fixture::{rows, scene};
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

        // Primary-clicking a member again removes it; the primary falls back.
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
