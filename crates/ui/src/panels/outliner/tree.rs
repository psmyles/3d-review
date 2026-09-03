//! The Outliner's tree model: the pure functions that turn the scene's nodes,
//! the collapsed set and the kind filter into the flat list of [`TreeRow`]s the
//! panel draws, in each of its three presentations (scene tree / flat / search).
//!
//! Nothing here touches egui, so the whole flattening matrix is unit-testable.

use std::collections::HashSet;

use review_model::{ModelData, NodeKind};

use super::{display_name, matches_search};

/// One row of the flattened scene tree, as [`visible_tree_rows`] resolves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TreeRow {
    /// Index into [`ModelData::nodes`].
    pub(super) node: usize,
    /// Indent level; roots are 0.
    pub(super) depth: usize,
    /// Whether this node has children that survive the current filter — i.e.
    /// whether it gets a disclosure arrow.
    pub(super) has_children: bool,
    /// `false` when the type filter hides this node's kind and the row is only
    /// present to keep a visible descendant connected. Such rows paint dim and
    /// don't accept clicks.
    pub(super) selectable: bool,
    /// Bit `L` set means an ancestor of this row sits at depth `L + 1` and still
    /// has siblings below, so that ancestor's parent draws a vertical guide
    /// straight through this row at indent level `L`. Depths past 64 simply lose
    /// their guides rather than panicking.
    pub(super) ancestor_lines: u64,
    /// Whether this is the last *emitted* child of its parent, which decides
    /// whether its own guide elbow is a tee (`├`) or a corner (`└`).
    pub(super) last_child: bool,
}

impl TreeRow {
    /// A row with no hierarchy decoration, for the flat and search presentations.
    fn flat(node: usize) -> Self {
        Self {
            node,
            depth: 0,
            has_children: false,
            selectable: true,
            ancestor_lines: 0,
            last_child: true,
        }
    }
}

/// The read-only inputs a tree walk needs, bundled so the recursion stays a short
/// signature instead of threading eight parameters through every step.
struct TreeCtx<'a> {
    model: &'a ModelData,
    children: &'a [Vec<usize>],
    collapsed: &'a HashSet<usize>,
    hidden_kinds: &'a HashSet<NodeKind>,
}

/// Whether any descendant of `node` survives the kind filter.
fn has_shown_descendant(ctx: &TreeCtx, node: usize) -> bool {
    ctx.children.get(node).into_iter().flatten().any(|&child| {
        !ctx.hidden_kinds.contains(&ctx.model.nodes[child].kind) || has_shown_descendant(ctx, child)
    })
}

/// Whether `node` gets a row at all — either its own kind survives the filter, or
/// it stays as a connector so a shown descendant keeps its ancestry.
fn emits_row(ctx: &TreeCtx, node: usize) -> bool {
    !ctx.hidden_kinds.contains(&ctx.model.nodes[node].kind) || has_shown_descendant(ctx, node)
}

fn walk_tree(
    ctx: &TreeCtx,
    node: usize,
    depth: usize,
    ancestor_lines: u64,
    last_child: bool,
    out: &mut Vec<TreeRow>,
) {
    let shown = !ctx.hidden_kinds.contains(&ctx.model.nodes[node].kind);
    let descendant_shown = has_shown_descendant(ctx, node);
    if !shown && !descendant_shown {
        return;
    }
    out.push(TreeRow {
        node,
        depth,
        has_children: descendant_shown,
        selectable: shown,
        ancestor_lines,
        last_child,
    });
    if ctx.collapsed.contains(&node) {
        return;
    }

    // Only children that actually get a row may count when deciding which one is
    // last, or an elbow would corner off at a branch the filter dropped.
    let shown_children: Vec<usize> = ctx
        .children
        .get(node)
        .into_iter()
        .flatten()
        .copied()
        .filter(|&child| emits_row(ctx, child))
        .collect();
    // This node's own line continues past its descendants only while it still has
    // siblings of its own to come.
    let child_lines = match depth.checked_sub(1) {
        Some(level) if !last_child && level < u64::BITS as usize => ancestor_lines | (1 << level),
        _ => ancestor_lines,
    };
    for (index, &child) in shown_children.iter().enumerate() {
        walk_tree(
            ctx,
            child,
            depth + 1,
            child_lines,
            index + 1 == shown_children.len(),
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
pub(super) fn visible_tree_rows(
    model: &ModelData,
    children: &[Vec<usize>],
    roots: &[usize],
    collapsed: &HashSet<usize>,
    hidden_kinds: &HashSet<NodeKind>,
) -> Vec<TreeRow> {
    let ctx = TreeCtx {
        model,
        children,
        collapsed,
        hidden_kinds,
    };
    let shown: Vec<usize> = roots
        .iter()
        .copied()
        .filter(|&root| emits_row(&ctx, root))
        .collect();
    let mut rows = Vec::new();
    for (index, &root) in shown.iter().enumerate() {
        walk_tree(&ctx, root, 0, 0, index + 1 == shown.len(), &mut rows);
    }
    rows
}

/// Every node the type filter allows, in model order and without hierarchy. The
/// filter applies here exactly as it does in the tree, so the toggles read as
/// "what the panel is showing" in either presentation.
///
/// Pure, so it is unit-testable without an egui context.
pub(super) fn flat_rows(model: &ModelData, hidden_kinds: &HashSet<NodeKind>) -> Vec<TreeRow> {
    (0..model.nodes.len())
        .filter(|&node| !hidden_kinds.contains(&model.nodes[node].kind))
        .map(TreeRow::flat)
        .collect()
}

/// The flat list narrowed to the nodes whose display name matches `query` (which
/// the caller has already trimmed and lowercased). Search results ignore the
/// hierarchy and the collapsed set entirely — a hit is never buried inside a
/// folded branch — but still honor the type filter.
///
/// Pure, so it is unit-testable without an egui context.
pub(super) fn search_rows(
    model: &ModelData,
    hidden_kinds: &HashSet<NodeKind>,
    query: &str,
) -> Vec<TreeRow> {
    model
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| !hidden_kinds.contains(&node.kind))
        .filter(|(index, node)| matches_search(&display_name(node, *index), query))
        .map(|(index, _)| TreeRow::flat(index))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::outliner::fixture::{rows, scene};
    use review_model::SceneNode;

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

    // ── Guide geometry ───────────────────────────────────────────────────────

    #[test]
    fn guides_tee_until_the_last_child_and_carry_no_ancestor_lines() {
        let model = scene();
        let rows = rows(&model, &[], &[]);
        assert_eq!(
            rows.iter()
                .map(|row| (row.node, row.last_child))
                .collect::<Vec<_>>(),
            vec![(0, true), (1, false), (2, true), (3, false), (4, true)],
            "mesh and spine still have siblings below them"
        );
        assert!(
            rows.iter().all(|row| row.ancestor_lines == 0),
            "the only branch is under hips, which is root's last child, so no \
             vertical ever has to run past a row"
        );
    }

    #[test]
    fn a_branch_with_siblings_below_carries_a_vertical_through_its_descendants() {
        // Give root a second child *after* hips, so hips' subtree rows have to
        // draw root's line continuing past them.
        let mut model = scene();
        model.nodes.push(SceneNode {
            name: "tail".to_owned(),
            parent: Some(0),
            mesh_part: None,
            source_vertex_count: 0,
            transform: glam::Mat4::IDENTITY,
            rest_local: Default::default(),
            kind: NodeKind::Empty,
            bone: None,
        });
        let rows = rows(&model, &[], &[]);
        let line_of = |node: usize| {
            rows.iter()
                .find(|row| row.node == node)
                .expect("row is present")
                .ancestor_lines
        };
        assert_eq!(
            line_of(3),
            1,
            "spine sits under hips, which now has a sibling"
        );
        assert_eq!(line_of(4), 1, "and so does lamp");
        assert_eq!(line_of(2), 0, "hips itself only draws its own elbow");
        assert_eq!(line_of(5), 0, "tail is root's last child");
    }

    #[test]
    fn a_filtered_out_sibling_promotes_the_one_before_it_to_last_child() {
        let model = scene();
        // Hiding lights drops lamp, so spine becomes hips' last emitted child and
        // must corner off rather than tee into a row that is not there.
        let rows = rows(&model, &[], &[NodeKind::Light]);
        assert_eq!(
            rows.iter().map(|row| row.node).collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
        assert!(
            rows[3].last_child,
            "spine is now the last child hips actually emits"
        );
    }

    // ── Flat + search rows ───────────────────────────────────────────────────

    #[test]
    fn flat_rows_list_every_kind_not_just_meshes() {
        let model = scene();
        let rows = flat_rows(&model, &HashSet::new());
        assert_eq!(
            rows.iter().map(|row| row.node).collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4]
        );
        assert!(
            rows.iter()
                .all(|row| row.depth == 0 && !row.has_children && row.selectable),
            "a flat row carries no hierarchy decoration"
        );
    }

    #[test]
    fn flat_rows_honor_the_type_filter() {
        let model = scene();
        let rows = flat_rows(&model, &[NodeKind::Bone].into_iter().collect());
        assert_eq!(
            rows.iter().map(|row| row.node).collect::<Vec<_>>(),
            vec![0, 1, 4],
            "both bones drop out; nothing is kept as a connector in a flat list"
        );
    }

    /// Match what `scene_tab` hands `search_rows`, so the tests exercise the real
    /// trimming + lowercasing contract.
    fn search(model: &ModelData, hidden: &[NodeKind], query: &str) -> Vec<usize> {
        search_rows(
            model,
            &hidden.iter().copied().collect(),
            &query.trim().to_lowercase(),
        )
        .iter()
        .map(|row| row.node)
        .collect()
    }

    #[test]
    fn search_matches_a_case_insensitive_substring() {
        let model = scene();
        assert_eq!(search(&model, &[], "SPI"), vec![3]);
        assert_eq!(
            search(&model, &[], "  spi  "),
            vec![3],
            "the query is trimmed"
        );
        assert_eq!(
            search(&model, &[], "s"),
            vec![1, 2, 3],
            "a substring matches anywhere in the name, not just its start"
        );
    }

    #[test]
    fn an_empty_search_keeps_every_row() {
        let model = scene();
        assert_eq!(search(&model, &[], "   "), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn search_still_honors_the_type_filter() {
        let model = scene();
        assert!(
            search(&model, &[NodeKind::Bone], "spine").is_empty(),
            "a filtered-out kind cannot be searched back into view"
        );
    }

    #[test]
    fn search_with_no_match_yields_no_rows() {
        let model = scene();
        assert!(search(&model, &[], "nothing").is_empty());
    }
}
