//! Row painting: drawing one flattened list of [`TreeRow`]s as full-width strips
//! (band, guides, disclosure arrow, kind glyph, visibility eye, truncated name),
//! and the indent-guide geometry that ties each row back to its parent.

use review_model::{ModelData, NodeKind};
use review_render::Selection;

use super::{RowsOutput, TreeRow, display_name};
use crate::assets;
use crate::assets::AppIcon;
use crate::state::UiState;
use crate::theme::{color, size};

/// Draw one flattened list as full-width rows. Reads state only; everything it
/// wants changed comes back in the [`RowsOutput`].
pub(super) fn draw_rows(
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
            let openness = if state.outliner.collapsed.contains(&row.node) {
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
                // The modifiers travel with the click: the primary modifier turns
                // a plain show/hide into an isolate, applied by
                // `apply_rows_output`.
                output.toggled_eye = Some((row.node, ui.input(|input| input.modifiers)));
            }
            if hovered {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            eye.on_hover_text(if hidden {
                concat!("Show mesh\n", primary_key!(), "+click: show only this mesh")
            } else {
                concat!("Hide mesh\n", primary_key!(), "+click: show only this mesh")
            });

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
        if state.outliner.scroll_to_selection && state.selection == Selection::Node(row.node) {
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
fn selected_path(rows: &[TreeRow], selected: Option<usize>) -> Vec<usize> {
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

/// The per-kind row glyph.
pub(super) fn kind_icon(kind: NodeKind) -> &'static AppIcon {
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
pub(super) fn kind_color(kind: NodeKind) -> egui::Color32 {
    match kind {
        NodeKind::Mesh => color::NODE_MESH,
        NodeKind::Bone => color::NODE_BONE,
        NodeKind::Light => color::NODE_LIGHT,
        NodeKind::Camera => color::NODE_CAMERA,
        NodeKind::Empty => color::NODE_EMPTY,
        NodeKind::Other => color::NODE_OTHER,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::outliner::test_fixture::{rows, scene};
    use review_model::NodeKind;

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
}
