//! Buttons and tab strips.

use crate::assets::{self, AppIcon};
use crate::theme::{self, color, font, size};
use crate::widgets::{Tip, tip};

/// A button that fills the width it is given, with its label centred.
///
/// egui left-aligns a button's label as soon as `min_size` widens the button past
/// the text ("if there are no growable atoms then everything will be left-aligned")
/// — growable spacers either side are what re-centre it.
pub(crate) fn wide_button<'a>(label: impl Into<egui::WidgetText>, width: f32) -> egui::Button<'a> {
    egui::Button::new((egui::Atom::grow(), label.into(), egui::Atom::grow()))
        .min_size(egui::vec2(width, 0.0))
}

/// A square icon toggle sized to the standard toolbar tile.
pub(crate) fn icon_toggle_button(
    ui: &mut egui::Ui,
    icon: &AppIcon,
    selected: bool,
    tooltip: Tip,
) -> egui::Response {
    icon_tile_button(
        ui,
        icon,
        selected,
        tooltip,
        egui::vec2(size::TOOLBAR_ICON_SIZE, size::TOOLBAR_ICON_SIZE),
        false,
    )
}

/// A standard icon toggle for a button that also exposes a right-click options
/// panel: identical to [`icon_toggle_button`] but while hovered it paints a thin
/// green line along the tile's top edge — end to end, fading from transparent at
/// both ends to opaque at the center — hinting that there are more options behind
/// it.
pub(crate) fn icon_toggle_button_with_options(
    ui: &mut egui::Ui,
    icon: &AppIcon,
    selected: bool,
    tooltip: Tip,
) -> egui::Response {
    icon_tile_button(
        ui,
        icon,
        selected,
        tooltip,
        egui::vec2(size::TOOLBAR_ICON_SIZE, size::TOOLBAR_ICON_SIZE),
        true,
    )
}

/// A custom-painted icon tile button: recessed when idle, accent-filled when
/// selected, lifted on hover. The icon is tinted brighter when selected. When
/// `has_options` is set, a hover paints the green options-hint underline.
pub(crate) fn icon_tile_button(
    ui: &mut egui::Ui,
    icon: &AppIcon,
    selected: bool,
    tooltip: Tip,
    tile_size: egui::Vec2,
    has_options: bool,
) -> egui::Response {
    let tint = if selected {
        color::TEXT_PRIMARY
    } else {
        color::ICON_IDLE
    };
    let (rect, response) = ui.allocate_exact_size(tile_size, egui::Sense::click());
    let fill = if selected {
        color::ACCENT
    } else if response.hovered() {
        color::HOVER_BG
    } else {
        color::GROUP_BG
    };

    ui.painter().rect(
        rect,
        size::TILE_CORNER_RADIUS,
        fill,
        egui::Stroke::new(size::HAIRLINE, egui::Color32::TRANSPARENT),
        egui::StrokeKind::Inside,
    );

    if let Some(texture) = assets::load_icon_texture(ui, icon) {
        let image_padding = rect.width().min(rect.height()) * size::TILE_ICON_INSET_RATIO;
        let image_rect = rect.shrink2(egui::vec2(image_padding, image_padding));
        egui::Image::from_texture(texture)
            .fit_to_exact_size(image_rect.size())
            .tint(tint)
            .paint_at(ui, image_rect);
    }

    // Hint that this button has a right-click options panel: a thin green line
    // along the top edge, end to end, fading from transparent at both ends to
    // opaque at the center. egui has no gradient-stroke primitive, so it's a
    // 1px-tall two-quad mesh with per-vertex colors, painted last so it sits over
    // the fill/icon.
    if has_options && response.hovered() {
        let edge = theme::with_opacity(color::OPTIONS_HINT, 0.0);
        let center = color::OPTIONS_HINT;
        let top = rect.top();
        let bottom = top + size::OPTIONS_HINT_THICKNESS;
        let mid_x = rect.center().x;
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(egui::pos2(rect.left(), top), edge); // 0
        mesh.colored_vertex(egui::pos2(rect.left(), bottom), edge); // 1
        mesh.colored_vertex(egui::pos2(mid_x, top), center); // 2
        mesh.colored_vertex(egui::pos2(mid_x, bottom), center); // 3
        mesh.colored_vertex(egui::pos2(rect.right(), top), edge); // 4
        mesh.colored_vertex(egui::pos2(rect.right(), bottom), edge); // 5
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(1, 3, 2);
        mesh.add_triangle(2, 3, 4);
        mesh.add_triangle(3, 5, 4);
        ui.painter().add(egui::Shape::mesh(mesh));
    }

    tip(response, tooltip)
}

/// A custom-painted **text** segment tile (a labelled radio cell), sized to
/// `width`: accent-filled when selected, lifted on hover, idle transparent. The
/// shared primitive behind the mode tabs (3D/UV/Tex), the Tex channel group
/// (RGB/R/G/B/A) and the Tex background group (B/W/G/C). Returns the response so
/// the caller drives selection on `clicked()`.
pub(crate) fn segment_button(
    ui: &mut egui::Ui,
    label: &str,
    selected: bool,
    width: f32,
) -> egui::Response {
    let desired = egui::vec2(width, size::MODE_SEGMENT_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());
    let fill = if selected {
        color::ACCENT
    } else if response.hovered() {
        color::HOVER_BG
    } else {
        egui::Color32::TRANSPARENT
    };
    let text_color = if selected {
        color::TEXT_PRIMARY
    } else {
        color::TEXT_SEGMENT_IDLE
    };

    ui.painter().rect(
        rect,
        size::TILE_CORNER_RADIUS,
        fill,
        egui::Stroke::NONE,
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(font::MODE_SEGMENT),
        text_color,
    );
    response
}

/// Where each tab of a strip goes: the tabs drawn in it, left to right, with
/// their widths, and the ones moved into its overflow menu.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TabLayout {
    pub(crate) visible: Vec<(usize, f32)>,
    pub(crate) overflow: Vec<usize>,
}

/// Lay `natural` tab widths (each label plus its padding) out across a strip
/// `width` wide.
///
/// * When every tab fits at the widest one's width, they share the strip
///   equally — the strip's usual look.
/// * When they fit only at their own widths, each gets its own width plus an
///   equal share of what is left.
/// * When they don't fit at all, an overflow button takes `overflow_width` at the
///   right and the tabs that don't fit go into its menu — the trailing ones, but
///   never the `selected` one, which takes the place of whatever it has to.
pub(crate) fn tab_layout(
    width: f32,
    natural: &[f32],
    selected: usize,
    overflow_width: f32,
) -> TabLayout {
    let count = natural.len();
    if count == 0 {
        return TabLayout {
            visible: Vec::new(),
            overflow: Vec::new(),
        };
    }
    let widest = natural.iter().copied().fold(0.0, f32::max);
    if widest * count as f32 <= width {
        let share = width / count as f32;
        return TabLayout {
            visible: (0..count).map(|index| (index, share)).collect(),
            overflow: Vec::new(),
        };
    }
    let total: f32 = natural.iter().sum();
    if total <= width {
        let extra = (width - total) / count as f32;
        return TabLayout {
            visible: natural
                .iter()
                .enumerate()
                .map(|(index, &natural)| (index, natural + extra))
                .collect(),
            overflow: Vec::new(),
        };
    }

    let room = (width - overflow_width).max(0.0);
    let mut shown = Vec::new();
    let mut used = 0.0;
    for (index, &natural) in natural.iter().enumerate() {
        if used + natural > room {
            break;
        }
        shown.push(index);
        used += natural;
    }
    let selected = selected.min(count - 1);
    if !shown.contains(&selected) {
        while used + natural[selected] > room {
            let Some(last) = shown.pop() else { break };
            used -= natural[last];
        }
        shown.push(selected);
        used += natural[selected];
        shown.sort_unstable();
    }
    let visible = if used > room {
        // Narrower than even the selected tab: it takes what there is.
        shown
            .iter()
            .map(|&index| (index, room / shown.len() as f32))
            .collect()
    } else {
        let extra = (room - used) / shown.len() as f32;
        shown
            .iter()
            .map(|&index| (index, natural[index] + extra))
            .collect()
    };
    TabLayout {
        visible,
        overflow: (0..count).filter(|index| !shown.contains(index)).collect(),
    }
}

/// A full-width tab strip: one cell per label (see [`tab_layout`] for how they
/// share the width, and when the trailing ones move into a `»` overflow menu),
/// all sharing a single baseline hairline, with the active tab marked by an
/// accent underline and a brightened label. Hovering an inactive tab lifts it with
/// the standard hover fill and a pointing-hand cursor.
///
/// The label is drawn in the ambient `TextStyle::Button` font — the same one the
/// stock `selectable_label` rows beneath the strip resolve — so the tabs and the
/// list they head always read at one size.
///
/// This is the `egui_tabs` look, painted here rather than pulled in: that crate
/// pins `egui 0.29`/`0.30` and this workspace is on 0.36, so linking it would fork a
/// second `egui` into the graph.
/// It also parks the selection in `ctx` temp data, which would duplicate the
/// `UiState` field that already owns it (invariant 2).
///
/// Returns the index of the tab clicked this frame, if any — in the strip or in
/// its overflow menu; the caller owns the selection. `strip_id` names the strip:
/// each cell's id is `strip_id.with(("tab", index))` wherever the strip is laid
/// out, so it doesn't change with the panels nested around it.
pub(crate) fn tab_bar(
    ui: &mut egui::Ui,
    strip_id: egui::Id,
    labels: &[egui::WidgetText],
    selected: usize,
) -> Option<usize> {
    if labels.is_empty() {
        return None;
    }

    let font = egui::TextStyle::Button.resolve(ui.style());
    // Tall enough to clear the label with the standard tab padding above and
    // below it, and never shorter than the strip's floor.
    let height = (ui.text_style_height(&egui::TextStyle::Button) + 2.0 * size::OUTLINER_TAB_PAD_Y)
        .max(size::OUTLINER_TAB_HEIGHT);
    let underline = size::OUTLINER_TAB_UNDERLINE;
    let (strip, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let painter = ui.painter().clone();
    let galleys: Vec<_> = labels
        .iter()
        .map(|label| {
            painter.layout_no_wrap(label.text().to_owned(), font.clone(), color::TEXT_PRIMARY)
        })
        .collect();
    let natural: Vec<f32> = galleys
        .iter()
        .map(|galley| galley.size().x + 2.0 * size::OUTLINER_TAB_PAD_X)
        .collect();
    let layout = tab_layout(
        strip.width(),
        &natural,
        selected,
        size::OUTLINER_TAB_OVERFLOW_WIDTH,
    );
    // The underline is centred on the rail, so both share this baseline.
    let baseline = strip.bottom() - underline * 0.5;
    let mut clicked = None;

    // One continuous rail under the whole strip, so the active underline reads as
    // a lit segment of it rather than a floating bar.
    painter.hline(
        strip.x_range(),
        baseline,
        egui::Stroke::new(size::HAIRLINE, color::DIVIDER),
    );

    let mut left = strip.left();
    for &(index, width) in &layout.visible {
        let cell =
            egui::Rect::from_min_size(egui::pos2(left, strip.top()), egui::vec2(width, height));
        left += width;
        let response = ui.interact(cell, strip_id.with(("tab", index)), egui::Sense::click());
        let active = index == selected;

        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            if !active {
                painter.rect_filled(cell, size::TILE_CORNER_RADIUS, color::HOVER_BG);
            }
        }
        if response.clicked() {
            clicked = Some(index);
        }

        // Centre the label in the space above the rail, not in the whole cell —
        // clipped to the cell, for a strip narrower than its one remaining tab.
        let galley = galleys[index].clone();
        let anchor = egui::pos2(cell.center().x, (cell.top() + baseline) * 0.5);
        painter.with_clip_rect(cell).galley(
            anchor - galley.size() * 0.5,
            galley,
            if active {
                color::TEXT_PRIMARY
            } else {
                color::TEXT_SEGMENT_IDLE
            },
        );

        if active {
            painter.hline(
                cell.x_range(),
                baseline,
                egui::Stroke::new(underline, color::ACCENT),
            );
        }
    }

    if !layout.overflow.is_empty() {
        let cell = egui::Rect::from_min_max(
            egui::pos2(
                strip.right() - size::OUTLINER_TAB_OVERFLOW_WIDTH,
                strip.top(),
            ),
            egui::pos2(strip.right(), strip.top() + height),
        );
        let response = ui.interact(cell, strip_id.with("overflow"), egui::Sense::click());
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            painter.rect_filled(cell, size::TILE_CORNER_RADIUS, color::HOVER_BG);
        }
        painter.text(
            egui::pos2(cell.center().x, (cell.top() + baseline) * 0.5),
            egui::Align2::CENTER_CENTER,
            OVERFLOW_GLYPH,
            font.clone(),
            color::TEXT_SEGMENT_IDLE,
        );
        egui::Popup::menu(&response)
            .id(strip_id.with("overflow_menu"))
            .show(|ui| {
                for &index in &layout.overflow {
                    if ui.selectable_label(false, labels[index].clone()).clicked() {
                        clicked = Some(index);
                    }
                }
            });
        tip(response, Tip::new(crate::keys::ui_widgets::MORE_TABS));
    }

    clicked
}

/// The tab strip's overflow button: a glyph, not a word, like every other icon
/// in the chrome.
const OVERFLOW_GLYPH: &str = "\u{00bb}";

/// A panel's footer row: the stock "Reset all" button, and — pushed to the right
/// edge — a `?` that opens this panel's page in the manual.
///
/// The reset button's [`egui::Response`] comes back, because each panel resets
/// its own fields its own way (several deliberately keep their `enabled` flag).
/// The `?` needs nothing back: it parks the page request the way a tooltip's link
/// does, and [`crate::overlay`] applies it.
pub(crate) fn panel_footer(ui: &mut egui::Ui, page: crate::docs::Page) -> egui::Response {
    ui.horizontal(|ui| {
        let reset = ui.button(crate::keys::common::RESET_ALL);
        let reset = tip(
            reset,
            Tip::new(crate::keys::common::RESET_ALL)
                .describe(crate::keys::common::RESET_ALL_DESCRIPTION),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let help = ui.small_button("?");
            let help = tip(
                help,
                Tip::new(crate::keys::common::HELP).describe(crate::keys::common::HELP_OPEN),
            );
            if help.clicked() {
                crate::help::request_page(ui.ctx(), page);
            }
        });
        reset
    })
    .inner
}

#[cfg(test)]
mod tab_layout_tests {
    use super::tab_layout;

    fn widths(layout: &super::TabLayout) -> f32 {
        layout.visible.iter().map(|&(_, width)| width).sum()
    }

    #[test]
    fn tabs_that_fit_at_the_widest_share_the_strip_equally() {
        let layout = tab_layout(300.0, &[40.0, 60.0, 50.0], 0, 24.0);
        assert!(layout.overflow.is_empty());
        assert!(layout.visible.iter().all(|&(_, width)| width == 100.0));
    }

    #[test]
    fn tabs_that_fit_only_at_their_own_widths_keep_them() {
        let layout = tab_layout(200.0, &[40.0, 90.0, 50.0], 0, 24.0);
        assert!(layout.overflow.is_empty());
        let extra = (200.0 - 180.0) / 3.0;
        assert_eq!(layout.visible[1], (1, 90.0 + extra));
        assert!((widths(&layout) - 200.0).abs() < 1e-3);
    }

    #[test]
    fn tabs_that_do_not_fit_overflow_from_the_end() {
        let layout = tab_layout(150.0, &[40.0, 50.0, 60.0, 70.0], 0, 24.0);
        assert_eq!(layout.overflow, [2, 3]);
        assert_eq!(
            layout.visible.iter().map(|&(i, _)| i).collect::<Vec<_>>(),
            [0, 1]
        );
        assert!(
            (widths(&layout) - 126.0).abs() < 1e-3,
            "the strip minus the overflow button"
        );
    }

    #[test]
    fn the_selected_tab_is_never_in_the_overflow() {
        let layout = tab_layout(150.0, &[40.0, 50.0, 60.0, 70.0], 3, 24.0);
        assert!(layout.visible.iter().any(|&(i, _)| i == 3));
        assert!(!layout.overflow.contains(&3));
        let shown: Vec<usize> = layout.visible.iter().map(|&(i, _)| i).collect();
        assert!(
            shown.windows(2).all(|pair| pair[0] < pair[1]),
            "still in order"
        );
    }

    #[test]
    fn a_strip_narrower_than_one_tab_still_shows_the_selected_one() {
        let layout = tab_layout(30.0, &[40.0, 50.0], 1, 24.0);
        assert_eq!(layout.visible.len(), 1);
        assert_eq!(layout.visible[0].0, 1);
    }
}
