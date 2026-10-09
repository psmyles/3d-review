//! The Outliner's Textures tab: the scene texture pool, narrowed by the header's
//! search box. Clicking a row makes that image the current texture — the one the
//! Tex workspace views — and shows it in the Inspector; elsewhere, clicking the
//! inspected row again hands the Inspector back to the scene selection, the same
//! toggle every other Outliner row uses.

use super::matches_search;
use crate::docs::Page;
use crate::keys;
use crate::state::{UiState, WorkspaceMode};
use crate::theme::{color, font, size};
use crate::widgets;
use crate::widgets::{Tip, tip};

pub(super) fn textures_tab(ui: &mut egui::Ui, state: &mut UiState) {
    if state.texture_pool.is_empty() {
        ui.weak(keys::ui_outliner::NO_TEXTURES);
        return;
    }

    let query = state.outliner.search.trim().to_lowercase();
    // Highlighted only while the Inspector is showing it: outside Tex the current
    // texture is just a remembered index, and a lit row beside an Inspector
    // showing a node would claim a selection that isn't there.
    let inspected = state
        .texture_inspected()
        .then_some(state.texture_view.selected);
    // Deferred like every Outliner mutation: the rows are drawn against one
    // consistent state, then the click is applied.
    let mut clicked: Option<usize> = None;
    let mut matched = false;
    ui.spacing_mut().item_spacing.y = 0.0;

    for (index, entry) in state.texture_pool.iter().enumerate() {
        let name = entry.name();
        if !matches_search(&name, &query) {
            continue;
        }
        matched = true;
        let is_selected = inspected == Some(index);

        let (response, content, trailing) = widgets::list_row(
            ui,
            ui.id().with(("texture", index)),
            is_selected,
            size::TEXTURE_ROW_TRAILING_WIDTH,
            size::TEXTURE_ROW_HEIGHT,
        );

        // Thumbnail, then the name in what is left of the content rect.
        let thumb_rect = egui::Rect::from_center_size(
            egui::pos2(
                content.left() + size::TEXTURE_ROW_THUMB * 0.5,
                content.center().y,
            ),
            egui::Vec2::splat(size::TEXTURE_ROW_THUMB),
        );
        if let Some(texture) = widgets::texture_thumbnail(ui, entry, size::TEXTURE_ROW_THUMB) {
            // Fit inside the square, keeping the image's own aspect.
            let image_size = texture.size;
            let scale = size::TEXTURE_ROW_THUMB / image_size.x.max(image_size.y).max(1.0);
            let fitted = egui::Rect::from_center_size(thumb_rect.center(), image_size * scale);
            egui::Image::from_texture(texture)
                .fit_to_exact_size(fitted.size())
                .paint_at(ui, fitted);
        }
        let mut name_rect = content;
        name_rect.set_left(thumb_rect.right() + size::TEXTURE_ROW_THUMB_GAP);
        let name_color = if is_selected {
            color::TEXT_PRIMARY
        } else {
            color::TEXT_BODY
        };
        widgets::list_row_label(ui, name_rect, egui::RichText::new(&name).color(name_color));

        // The measured pixel size, right-aligned: a mono readout so the column
        // of a texture list lines up.
        ui.painter().text(
            trailing.right_center(),
            egui::Align2::RIGHT_CENTER,
            keys::ui_stats::tex_dimension_value(
                f64::from(entry.image.width),
                f64::from(entry.image.height),
            ),
            egui::FontId::monospace(font::STATS),
            color::TEXT_MUTED,
        );

        if response.clicked() {
            clicked = Some(index);
        }
        tip(
            response,
            Tip::new(keys::ui_outliner::texture_tooltip(
                entry.path.display().to_string(),
            ))
            .describe(keys::ui_outliner::TEXTURE_TOOLTIP_DESCRIPTION)
            .page(Page::OutlinerInspector),
        );
    }

    if !matched {
        ui.weak(keys::ui_outliner::NO_MATCHES);
    }

    if let Some(index) = clicked {
        // In UV the click also puts the texture behind the layout, where it stays
        // whatever is selected afterwards; a second click on it only hands the
        // Inspector back.
        if state.mode == WorkspaceMode::Uv
            && let Some(entry) = state.texture_pool.get(index)
        {
            state.uv_texture = Some(entry.path.clone());
        }
        // In Tex the current texture *is* the view, so there is nothing to
        // toggle back to; elsewhere a second click returns the Inspector to the
        // scene selection.
        if inspected == Some(index) && state.mode != WorkspaceMode::Texture {
            state.texture_view.inspected = false;
        } else {
            state.select_texture(index);
        }
    }
}
