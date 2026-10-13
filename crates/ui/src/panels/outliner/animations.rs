//! The Outliner's Animations tab: every clip on offer, narrowed by the header's
//! search box. Clicking a row selects that clip — paused on its first frame —
//! and brings up the transport; clicking the selected row again clears it and
//! returns the model to its rest pose, the same toggle node rows use.
//!
//! The clips come from the model's own file and from every animation file
//! attached to it. Once anything is attached each file gets a collapsible group
//! of its own, headed by its name and the rate its clips are timed at; a model on
//! its own lists its clips flat, as there is nothing to tell apart.

use review_model::{AnimationClip, ClipId, Clips, ModelData};

use super::matches_search;
use crate::docs::Page;
use crate::keys;
use crate::state::UiState;
use crate::theme::{color, font, size};
use crate::widgets;
use crate::widgets::{Tip, tip};

pub(super) fn animations_tab(ui: &mut egui::Ui, state: &mut UiState, model: &ModelData) {
    // A handful of `Arc`s: cloned so the rows can be drawn while the selection
    // below is still free to change.
    let attached = state.attached_animations.clone();
    let clips = Clips::new(model, &attached);
    if clips.is_empty() {
        ui.weak(keys::ui_outliner::NO_CLIPS);
        return;
    }

    let query = state.outliner.search.trim().to_lowercase();
    let selected = state.animation.selected_clip;
    // Deferred like every Outliner mutation: the rows are drawn against one
    // consistent selection, then the click is applied.
    let mut clicked: Option<Option<ClipId>> = None;
    let mut matched = false;
    ui.spacing_mut().item_spacing.y = 0.0;

    let grouped = !attached.is_empty();
    for file in 0..=attached.len() {
        let rows: Vec<(usize, String)> = clips
            .of_file(file)
            .iter()
            .enumerate()
            .map(|(index, clip)| (index, clip_name(clip, index)))
            .filter(|(_, name)| matches_search(name, &query))
            .collect();
        if rows.is_empty() {
            continue;
        }
        matched = true;
        let mut draw_rows = |ui: &mut egui::Ui| {
            for (index, name) in &rows {
                let id = ClipId { file, clip: *index };
                let Some(clip) = clips.get(id) else {
                    continue;
                };
                if clip_row(ui, id, clip, name, selected == Some(id)) {
                    clicked = Some(if selected == Some(id) { None } else { Some(id) });
                }
            }
        };
        if grouped {
            let name = match file {
                0 => model.name.as_str(),
                n => attached[n - 1].name.as_str(),
            };
            let fps = clips
                .of_file(file)
                .first()
                .map_or(0.0, AnimationClip::frame_rate_or_default);
            egui::CollapsingHeader::new(keys::ui_outliner::clip_group(name, format!("{fps:.0}")))
                .id_salt(("clip-file", file))
                .default_open(true)
                .show(ui, draw_rows);
        } else {
            draw_rows(ui);
        }
    }

    if !matched {
        ui.weak(keys::ui_outliner::NO_MATCHES);
    }

    if let Some(clip) = clicked {
        state.select_clip(model, clip);
    }
}

/// What a clip row is called: the clip's own name, or a numbered stand-in for a
/// clip the file left unnamed.
fn clip_name(clip: &AnimationClip, index: usize) -> String {
    if clip.name.is_empty() {
        keys::ui_outliner::unnamed_clip(index as f64)
    } else {
        clip.name.clone()
    }
}

/// One clip's row: its name, and its length as a trailing readout. True when it
/// was clicked.
fn clip_row(
    ui: &mut egui::Ui,
    id: ClipId,
    clip: &AnimationClip,
    name: &str,
    is_selected: bool,
) -> bool {
    // Frames are numbered from the clip's first frame, so a clip spanning `n`
    // frame intervals reads as `n` frames long — what the transport's "frame /
    // total" readout counts to.
    let frames = clip.frame_count().saturating_sub(1);
    let duration = clip.duration();
    let fps = clip.frame_rate_or_default();

    let (response, content, trailing) = widgets::list_row(
        ui,
        ui.id().with(("clip", id.file, id.clip)),
        is_selected,
        size::ANIM_ROW_TRAILING_WIDTH,
        size::ANIM_ROW_HEIGHT,
    );
    let name_color = if is_selected {
        color::TEXT_PRIMARY
    } else {
        color::TEXT_BODY
    };
    widgets::list_row_label(ui, content, egui::RichText::new(name).color(name_color));
    // The measured length, right-aligned in the trailing rect: a mono readout so
    // the columns of a clip list line up.
    ui.painter().text(
        trailing.right_center(),
        egui::Align2::RIGHT_CENTER,
        keys::ui_outliner::clip_summary(frames as f64, format!("{duration:.2}")),
        egui::FontId::monospace(font::STATS),
        color::TEXT_MUTED,
    );
    let clicked = response.clicked();
    tip(
        response,
        Tip::new(name.to_owned())
            .describe(format!(
                "{}\n{}",
                keys::ui_outliner::clip_tooltip(
                    frames as f64,
                    format!("{fps:.0}"),
                    format!("{duration:.2}"),
                ),
                review_localization::tr(keys::ui_outliner::CLIP_TOOLTIP_DESCRIPTION),
            ))
            .page(Page::Animation),
    );
    clicked
}
