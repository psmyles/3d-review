//! The Outliner's Animations tab: the loaded file's clips, narrowed by the
//! header's search box. Clicking a row selects that clip — paused on its first
//! frame — and brings up the transport; clicking the selected row again clears
//! it and returns the model to its rest pose, the same toggle node rows use.

use review_model::ModelData;

use super::matches_search;
use crate::state::UiState;
use crate::theme::{color, font, size};
use crate::widgets;

pub(super) fn animations_tab(ui: &mut egui::Ui, state: &mut UiState, model: &ModelData) {
    if model.animations.is_empty() {
        ui.weak("No animation clips.");
        return;
    }

    let query = state.outliner.search.trim().to_lowercase();
    let fps = model.frame_rate_or_default();
    let selected = state.animation.selected_clip;
    // Deferred like every Outliner mutation: the rows are drawn against one
    // consistent selection, then the click is applied.
    let mut clicked: Option<Option<usize>> = None;
    let mut matched = false;
    ui.spacing_mut().item_spacing.y = 0.0;

    for (index, clip) in model.animations.iter().enumerate() {
        let name = if clip.name.is_empty() {
            format!("Clip {index}")
        } else {
            clip.name.clone()
        };
        if !matches_search(&name, &query) {
            continue;
        }
        matched = true;
        let is_selected = selected == Some(index);
        // Frames are numbered from the clip's first frame, so a clip spanning
        // `n` frame intervals reads as `n` frames long — what the transport's
        // "frame / total" readout counts to.
        let frames = clip.frame_count(fps).saturating_sub(1);
        let duration = clip.duration();

        let (response, content, trailing) = widgets::list_row(
            ui,
            ui.id().with(("clip", index)),
            is_selected,
            size::ANIM_ROW_TRAILING_WIDTH,
            size::ANIM_ROW_HEIGHT,
        );
        let name_color = if is_selected {
            color::TEXT_PRIMARY
        } else {
            color::TEXT_BODY
        };
        widgets::list_row_label(ui, content, egui::RichText::new(&name).color(name_color));
        // The measured length, right-aligned in the trailing rect: a mono readout
        // so the columns of a clip list line up.
        ui.painter().text(
            trailing.right_center(),
            egui::Align2::RIGHT_CENTER,
            format!("{frames} f · {duration:.2} s"),
            egui::FontId::monospace(font::STATS),
            color::TEXT_MUTED,
        );
        if response.clicked() {
            clicked = Some(if is_selected { None } else { Some(index) });
        }
        response.on_hover_text(format!(
            "{name}\n{frames} frames at {fps:.0} fps · {duration:.2} s"
        ));
    }

    if !matched {
        ui.weak("No matches.");
    }

    if let Some(clip) = clicked {
        state.select_clip(model, clip);
    }
}
