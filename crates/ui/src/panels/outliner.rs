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
//! the same rows (see [`resolve_nav`]) whenever no widget — the search box above
//! all — holds egui's keyboard focus.
//!
//! Reads the scene nodes from the borrowed [`ModelData`] (invariant 2: borrowed,
//! not owned) and the material list from the app→UI snapshot.

use std::collections::HashSet;

use review_model::{ModelData, NodeKind, SceneNode};
use review_render::Selection;

use crate::assets::{self, AppIcon};
use crate::state::{OutlinerTab, OutlinerViewMode, UiState};
use crate::theme::{color, size};
use crate::widgets;

/// The Outliner's tabs, in strip order. The index into this array is what
/// [`widgets::tab_bar`] hands back on a click.
const TABS: [OutlinerTab; 2] = [OutlinerTab::Scene, OutlinerTab::Materials];

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
    /// Bit `L` set means an ancestor of this row sits at depth `L + 1` and still
    /// has siblings below, so that ancestor's parent draws a vertical guide
    /// straight through this row at indent level `L`. Depths past 64 simply lose
    /// their guides rather than panicking.
    pub(crate) ancestor_lines: u64,
    /// Whether this is the last *emitted* child of its parent, which decides
    /// whether its own guide elbow is a tee (`├`) or a corner (`└`).
    pub(crate) last_child: bool,
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

/// An arrow key, as [`resolve_nav`] understands it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavKey {
    Up,
    Down,
    Left,
    Right,
}

/// What an arrow press resolves to against the rows currently on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavAction {
    /// Move the selection to this node.
    Select(usize),
    /// Fold this node's subtree away.
    Collapse(usize),
    /// Unfold this node's subtree.
    Expand(usize),
    /// The press has nowhere to go (an end of the list, a leaf, a root).
    None,
}

/// What one drawn frame of rows produced. Every mutation is deferred to after the
/// draw: [`apply_row_click`] needs the full visible row order for Shift-range
/// selection, and mutating selection state mid-draw would let rows within one
/// frame disagree about what is selected.
#[derive(Debug, Default)]
struct RowsOutput {
    clicked: Option<(usize, egui::Modifiers)>,
    toggled_collapse: Option<usize>,
    toggled_eye: Option<usize>,
    /// Whether a pending [`UiState::outliner_scroll_to_selection`] was honored.
    scrolled: bool,
}

pub(crate) fn body(ui: &mut egui::Ui, state: &mut UiState, model: &ModelData) {
    // ── Tabs: Scene / Materials, as a full-width underlined tab strip ────────
    let materials_label = format!("Materials ({})", state.materials_snapshot.len());
    let labels = ["Scene", materials_label.as_str()];
    let active = TABS
        .iter()
        .position(|tab| *tab == state.outliner_tab)
        .unwrap_or(0);
    if let Some(index) = widgets::tab_bar(ui, &labels, active) {
        state.outliner_tab = TABS[index];
    }
    ui.add_space(size::PANEL_ROW_GAP);

    header_controls(ui, state, model);

    if state.outliner_tab == OutlinerTab::Materials {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| materials_tab(ui, state));
        return;
    }

    scene_tab(ui, state, model);
}

/// The header strip, drawn on every tab so the controls never shift underfoot:
/// the flat/tree view toggle and the node-kind filter row (Scene tab only — they
/// say nothing about a material), then the search box, which filters both tabs.
/// Only the kinds the model actually contains get a toggle, so an unrigged mesh
/// never shows a dead bone filter.
fn header_controls(ui: &mut egui::Ui, state: &mut UiState, model: &ModelData) {
    if state.outliner_tab == OutlinerTab::Scene {
        let tree_mode = state.outliner_view == OutlinerViewMode::SceneTree;
        ui.horizontal(|ui| {
            let tooltip = if tree_mode {
                "Showing the scene hierarchy — click for a flat list"
            } else {
                "Showing a flat node list — click for the scene hierarchy"
            };
            if icon_toggle(ui, &assets::ICON_TREE_VIEW, tree_mode, true, tooltip).clicked() {
                state.outliner_view = if tree_mode {
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

    ui.add(
        egui::TextEdit::singleline(&mut state.outliner_search)
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
    state.ensure_outliner_tree(model);

    let query = state.outliner_search.trim().to_lowercase();
    let searching = !query.is_empty();
    let rows = scene_rows(state, model, &query);
    // Guides describe a hierarchy; the flat and search presentations have none.
    let guides = !searching && state.outliner_view == OutlinerViewMode::SceneTree;

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
        return search_rows(model, &state.hidden_kinds, query);
    }
    match state.outliner_view {
        OutlinerViewMode::Flat => flat_rows(model, &state.hidden_kinds),
        OutlinerViewMode::SceneTree => visible_tree_rows(
            model,
            &state.outliner_children,
            &state.outliner_roots,
            &state.outliner_collapsed,
            &state.hidden_kinds,
        ),
    }
}

/// Draw one flattened list as full-width rows. Reads state only; everything it
/// wants changed comes back in the [`RowsOutput`].
fn draw_rows(
    ui: &mut egui::Ui,
    state: &UiState,
    model: &ModelData,
    rows: &[TreeRow],
    guides: bool,
) -> RowsOutput {
    let mut output = RowsOutput::default();
    // Rows butt against each other so the alternating bands read as continuous.
    ui.spacing_mut().item_spacing.y = 0.0;
    // Every row fills the panel's width, so the whole strip — not just its text —
    // is the target and the band spans edge to edge.
    let x_range = ui.max_rect().x_range();
    let pointer = ui.ctx().pointer_interact_pos();
    let font = egui::TextStyle::Body.resolve(ui.style());
    // Detached from `ui` so the row can interleave painting with the `&mut ui`
    // calls (the disclosure arrow, the icon textures) and keep paint order.
    let painter = ui.painter().clone();
    // Resolved once for the whole list: the guides along it light up.
    let path = selected_path(
        rows,
        match state.selection {
            Selection::Node(node) => Some(node),
            _ => None,
        },
    );

    for (index, row) in rows.iter().enumerate() {
        let node = &model.nodes[row.node];
        let (slot, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), size::OUTLINER_ROW_HEIGHT),
            egui::Sense::hover(),
        );
        let row_rect = egui::Rect::from_x_y_ranges(x_range, slot.y_range());
        let row_id = ui.id().with(("outliner_row", row.node));
        let response = ui.interact(
            row_rect,
            row_id,
            if row.selectable {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            },
        );

        // ── Backgrounds ─────────────────────────────────────────────────────
        let selected = state.selection == Selection::Node(row.node)
            || state.selected_bones.contains(&row.node);
        if index % 2 == 1 {
            painter.rect_filled(row_rect, 0.0, color::OUTLINER_ROW_ALT_BG);
        }
        if selected {
            painter.rect_filled(
                row_rect,
                size::OUTLINER_ROW_ROUNDING,
                color::OUTLINER_ROW_SELECTED_BG,
            );
        } else if response.hovered() {
            painter.rect_filled(row_rect, size::OUTLINER_ROW_ROUNDING, color::HOVER_BG);
        }

        let content_left = row_rect.left() + size::OUTLINER_ROW_PAD_X;
        if guides {
            paint_guides(&painter, row, row_rect, content_left, index, &path);
        }

        // ── Disclosure arrow — a fixed-width slot either way, so glyphs align ─
        let mut x = content_left + row.depth as f32 * size::OUTLINER_INDENT;
        let arrow_rect = egui::Rect::from_center_size(
            egui::pos2(x + size::OUTLINER_ARROW_WIDTH * 0.5, row_rect.center().y),
            egui::Vec2::splat(size::OUTLINER_ARROW_WIDTH),
        );
        x += size::OUTLINER_ARROW_WIDTH;
        if row.has_children {
            let arrow = ui.interact(arrow_rect, row_id.with("arrow"), egui::Sense::click());
            let openness = if state.outliner_collapsed.contains(&row.node) {
                0.0
            } else {
                1.0
            };
            egui::collapsing_header::paint_default_icon(ui, openness, &arrow);
            if arrow.clicked() {
                output.toggled_collapse = Some(row.node);
            }
            if arrow.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }

        // ── Kind glyph ──────────────────────────────────────────────────────
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(x + size::OUTLINER_TYPE_ICON * 0.5, row_rect.center().y),
            egui::Vec2::splat(size::OUTLINER_TYPE_ICON),
        );
        x += size::OUTLINER_TYPE_ICON + size::OUTLINER_ROW_GAP;
        if let Some(texture) = assets::load_icon_texture(ui, kind_icon(node.kind)) {
            let tint = if row.selectable {
                kind_color(node.kind)
            } else {
                color::OUTLINER_FILTERED
            };
            egui::Image::from_texture(texture)
                .fit_to_exact_size(icon_rect.size())
                .tint(tint)
                .paint_at(ui, icon_rect);
        }

        // ── Visibility eye, pinned to the right edge of mesh rows ───────────
        let eye_rect = (row.selectable && node.mesh_part.is_some()).then(|| {
            egui::Rect::from_center_size(
                egui::pos2(
                    row_rect.right() - size::OUTLINER_EYE_PAD - size::OUTLINER_EYE_ICON * 0.5,
                    row_rect.center().y,
                ),
                egui::Vec2::splat(size::OUTLINER_EYE_ICON),
            )
        });
        let mut name_right = row_rect.right() - size::OUTLINER_ROW_PAD_X;
        if let Some(eye_rect) = eye_rect {
            name_right = eye_rect.left() - size::OUTLINER_ROW_GAP;
            let hidden = state.hidden_meshes.contains(&row.node);
            let eye = ui.interact(eye_rect, row_id.with("eye"), egui::Sense::click());
            let hovered = eye.hovered();
            if eye.clicked() {
                output.toggled_eye = Some(row.node);
            }
            if hovered {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            eye.on_hover_text(if hidden { "Show mesh" } else { "Hide mesh" });

            let icon = if hidden {
                &assets::ICON_EYE_CLOSED
            } else {
                &assets::ICON_EYE_OPEN
            };
            if let Some(texture) = assets::load_icon_texture(ui, icon) {
                let tint = match (hovered, hidden) {
                    (true, _) => color::TEXT_PRIMARY,
                    (false, true) => color::OUTLINER_EYE_HIDDEN,
                    (false, false) => color::TEXT_MUTED,
                };
                egui::Image::from_texture(texture)
                    .fit_to_exact_size(eye_rect.size())
                    .tint(tint)
                    .paint_at(ui, eye_rect);
            }
        }

        // ── Name, truncated so it never runs under the eye ──────────────────
        let text_color = if !row.selectable {
            // Present only to keep a visible descendant attached to its ancestry —
            // readable as structure, but not a target.
            color::OUTLINER_FILTERED
        } else if selected {
            color::TEXT_PRIMARY
        } else {
            color::TEXT_BODY
        };
        let mut job = egui::text::LayoutJob::simple_singleline(
            display_name(node, row.node),
            font.clone(),
            text_color,
        );
        job.wrap = egui::text::TextWrapping {
            max_width: (name_right - x).max(0.0),
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        let galley = painter.layout_job(job);
        painter.galley(
            egui::pos2(x, row_rect.center().y - galley.size().y * 0.5),
            galley,
            text_color,
        );

        // ── Interaction ─────────────────────────────────────────────────────
        // The arrow and the eye sit inside the row, so a click on either must not
        // also move the selection.
        let on_eye = eye_rect
            .zip(pointer)
            .is_some_and(|(rect, pos)| rect.contains(pos));
        let on_arrow = row.has_children && pointer.is_some_and(|pos| arrow_rect.contains(pos));
        if response.clicked() && !on_eye && !on_arrow {
            output.clicked = Some((row.node, ui.input(|input| input.modifiers)));
        }
        if row.selectable && response.hovered() && !on_eye && !on_arrow {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if state.outliner_scroll_to_selection && state.selection == Selection::Node(row.node) {
            response.scroll_to_me(None);
            output.scrolled = true;
        }
    }

    output
}

/// The tree's indent guides for one row: full-height verticals for the ancestors
/// that still have rows below, then this row's own elbow into its parent's line.
///
/// `path` is [`selected_path`]'s root-first ancestry of the selection. The stretch
/// of guide that actually leads to the selected row is lit and the rest stays dim,
/// so a deep selection can be traced back to its root at a glance.
fn paint_guides(
    painter: &egui::Painter,
    row: &TreeRow,
    row_rect: egui::Rect,
    content_left: f32,
    index: usize,
    path: &[usize],
) {
    let dim = egui::Stroke::new(size::HAIRLINE, color::OUTLINER_GUIDE);
    let lit = egui::Stroke::new(size::HAIRLINE, color::OUTLINER_GUIDE_SELECTED);
    let stroke = |on_path: bool| if on_path { lit } else { dim };
    // A level's line runs down the middle of that depth's indent cell, so it
    // lands just left of the child rows it connects.
    let level_x = |level: usize| {
        content_left + level as f32 * size::OUTLINER_INDENT + size::OUTLINER_INDENT * 0.5
    };
    // The guide at `level` leads to the selection while it spans the gap between
    // the path's row at that depth and the path's row one step deeper.
    let leads_to_selection = |level: usize| {
        matches!(
            (path.get(level), path.get(level + 1)),
            (Some(&from), Some(&to)) if from < index && index <= to
        )
    };

    let carried = row.depth.saturating_sub(1).min(u64::BITS as usize);
    for level in 0..carried {
        if row.ancestor_lines & (1 << level) != 0 {
            let x = level_x(level);
            painter.line_segment(
                [
                    egui::pos2(x, row_rect.top()),
                    egui::pos2(x, row_rect.bottom()),
                ],
                stroke(leads_to_selection(level)),
            );
        }
    }

    let Some(level) = row.depth.checked_sub(1) else {
        return;
    };
    let x = level_x(level);
    let middle = row_rect.center().y;
    // Whether this row is itself the selection or one of its ancestors, which is
    // what its own tick into the name column reports — a row the path merely runs
    // past does not get one.
    let on_path = path.get(row.depth) == Some(&index);
    // The upper half links back up to the parent; the lower half carries on to the
    // next sibling, which has left the path once this row is the one on it. A last
    // child corners off (└) and has no lower half at all; the rest tee (├).
    painter.line_segment(
        [egui::pos2(x, row_rect.top()), egui::pos2(x, middle)],
        stroke(leads_to_selection(level)),
    );
    if !row.last_child {
        painter.line_segment(
            [egui::pos2(x, middle), egui::pos2(x, row_rect.bottom())],
            stroke(leads_to_selection(level) && !on_path),
        );
    }
    painter.line_segment(
        [
            egui::pos2(x, middle),
            egui::pos2(
                content_left + row.depth as f32 * size::OUTLINER_INDENT,
                middle,
            ),
        ],
        stroke(on_path),
    );
}

/// The rows making up the selected node's ancestry: entry `d` is the index (into
/// `rows`) of the path node drawn at depth `d`, so the first entry is its root and
/// the last is the selected row itself. Empty when nothing is selected, when the
/// selection isn't a node, or when its row isn't currently on screen.
///
/// Pure, so the path resolution is unit-testable without an egui context.
pub(crate) fn selected_path(rows: &[TreeRow], selected: Option<usize>) -> Vec<usize> {
    let Some(node) = selected else {
        return Vec::new();
    };
    let Some(mut at) = rows.iter().position(|row| row.node == node) else {
        return Vec::new();
    };
    let mut path = vec![at; rows[at].depth + 1];
    while rows[at].depth > 0 {
        let parent_depth = rows[at].depth - 1;
        // In depth-first order the nearest row above at the parent's depth is the
        // parent itself: everything drawn between them is deeper than both.
        let Some(parent) = rows[..at]
            .iter()
            .rposition(|above| above.depth == parent_depth)
        else {
            return Vec::new();
        };
        path[parent_depth] = parent;
        at = parent;
    }
    path
}

/// Apply everything the drawn rows asked for, in the order that keeps one frame
/// self-consistent.
fn apply_rows_output(state: &mut UiState, model: &ModelData, rows: &[TreeRow], output: RowsOutput) {
    if output.scrolled {
        state.outliner_scroll_to_selection = false;
    }
    if let Some(node) = output.toggled_eye
        && !state.hidden_meshes.remove(&node)
    {
        state.hidden_meshes.insert(node);
    }
    if let Some(node) = output.toggled_collapse
        && !state.outliner_collapsed.remove(&node)
    {
        state.outliner_collapsed.insert(node);
    }
    if let Some((node, modifiers)) = output.clicked {
        // Clicking a row is also how the Outliner claims the arrow keys.
        state.outliner_nav_focus = true;
        let kind = model.nodes[node].kind;
        let order: Vec<usize> = rows.iter().map(|row| row.node).collect();
        apply_row_click(state, node, kind, modifiers, &order);
    }
}

/// Route the arrow keys into the row list.
///
/// Any focused widget — the search box above all — owns the arrows first, so
/// typing a query still edits text. Otherwise the Outliner takes them while it
/// holds nav focus (earned by clicking a row or by a previous arrow press) or
/// while the pointer is over the panel, and *consumes* them, so they can never
/// reach `app`'s shortcut table.
fn handle_nav(ui: &egui::Ui, state: &mut UiState, model: &ModelData, rows: &[TreeRow]) {
    let ctx = ui.ctx();
    if ctx.memory(|memory| memory.focused()).is_some() {
        state.outliner_nav_focus = false;
        return;
    }

    let panel = ui.max_rect();
    let pointer_inside = ctx
        .pointer_latest_pos()
        .is_some_and(|pos| panel.contains(pos));
    // A press anywhere else hands the arrows back.
    if ctx.input(|input| input.pointer.any_pressed()) && !pointer_inside {
        state.outliner_nav_focus = false;
    }
    if !state.outliner_nav_focus && !pointer_inside {
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
    state.outliner_nav_focus = true;

    let current = match state.selection {
        Selection::Node(node) => rows.iter().position(|row| row.node == node),
        _ => None,
    };
    let changed = match resolve_nav(rows, &state.outliner_collapsed, current, nav) {
        // Routed through the click path so a bone still gets its set + anchor;
        // the equality guard keeps the plain-click "re-click clears" out of it.
        NavAction::Select(node) if state.selection != Selection::Node(node) => {
            let kind = model.nodes[node].kind;
            let order: Vec<usize> = rows.iter().map(|row| row.node).collect();
            apply_row_click(state, node, kind, egui::Modifiers::NONE, &order);
            true
        }
        NavAction::Collapse(node) => state.outliner_collapsed.insert(node),
        NavAction::Expand(node) => state.outliner_collapsed.remove(&node),
        NavAction::Select(_) | NavAction::None => false,
    };
    if changed {
        state.outliner_scroll_to_selection = true;
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
pub(crate) fn resolve_nav(
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
pub(crate) fn visible_tree_rows(
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
pub(crate) fn flat_rows(model: &ModelData, hidden_kinds: &HashSet<NodeKind>) -> Vec<TreeRow> {
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
pub(crate) fn search_rows(
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

/// Case-insensitive substring match. `query` is expected already trimmed and
/// lowercased by the caller; an empty one matches everything.
fn matches_search(name: &str, query: &str) -> bool {
    query.is_empty() || name.to_lowercase().contains(query)
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
/// each a stock selectable name row, narrowed by the header's search box. Indices
/// stay the snapshot's own, so filtering never re-points a [`Selection::Material`].
fn materials_tab(ui: &mut egui::Ui, state: &mut UiState) {
    if state.materials_snapshot.is_empty() {
        ui.weak("No materials.");
        return;
    }

    let query = state.outliner_search.trim().to_lowercase();
    let selection = state.selection;
    let mut clicked: Option<Selection> = None;
    let mut matched = false;
    for (index, material) in state.materials_snapshot.iter().enumerate() {
        if !matches_search(&material.name, &query) {
            continue;
        }
        matched = true;
        let selected = selection == Selection::Material(index);
        if ui.selectable_label(selected, &material.name).clicked() {
            clicked = Some(toggle(selected, Selection::Material(index)));
        }
    }
    if !matched {
        ui.weak("No matches.");
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
            transform: glam::Mat4::IDENTITY,
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

    // ── Selected-path guide highlighting ─────────────────────────────────────

    #[test]
    fn the_selected_path_runs_from_the_root_down_to_the_selection() {
        let model = scene();
        let rows = rows(&model, &[], &[]);
        // spine (node 3) is drawn at row 3, under hips (row 2), under root (row 0).
        assert_eq!(selected_path(&rows, Some(3)), vec![0, 2, 3]);
        assert_eq!(
            selected_path(&rows, Some(0)),
            vec![0],
            "a selected root is its own whole path"
        );
    }

    #[test]
    fn a_selection_that_is_not_on_screen_lights_nothing() {
        let model = scene();
        let open = rows(&model, &[], &[]);
        assert!(selected_path(&open, None).is_empty());
        assert!(
            selected_path(&open, Some(9)).is_empty(),
            "a node with no row (filtered out, or a stale index) has no path"
        );

        // Folding hips away takes spine off screen with it.
        let folded = rows(&model, &[2], &[]);
        assert!(selected_path(&folded, Some(3)).is_empty());
    }

    #[test]
    fn the_path_indexes_rows_by_depth_even_through_a_connector() {
        let model = scene();
        // Rows are [root, mesh, hips(connector), lamp]; lamp's path still runs
        // through the dimmed hips row, so the guide lights the whole way up.
        let rows = rows(&model, &[], &[NodeKind::Bone]);
        let path = selected_path(&rows, Some(4));
        assert_eq!(path, vec![0, 2, 3]);
        for (depth, &at) in path.iter().enumerate() {
            assert_eq!(
                rows[at].depth, depth,
                "entry {depth} is the row at that depth"
            );
        }
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
