//! The Outliner's Comments tab: the file's review comments, filtered by status
//! and narrowed by the header's search box. Clicking a thread goes to it — the
//! object it is on becomes the selection, the Inspector shows the thread, and
//! `app` flies the camera to the view it was written from and jumps to its frame.

use review_annotate::thread::{FrameRange, Status};

use super::matches_search;
use crate::docs::Page;
use crate::keys;
use crate::state::{CommentFilter, CommentIntent, DraftAnchor, UiState};
use crate::theme::{color, font, size};
use crate::widgets;
use crate::widgets::{Tip, tip};

/// A thread's frame or frame range, as a row or the Inspector names it.
pub(crate) fn frames_label(frames: &FrameRange) -> String {
    if frames.start == frames.end {
        keys::ui_comments::frame(frames.clip.clone(), f64::from(frames.start))
    } else {
        keys::ui_comments::frames(
            frames.clip.clone(),
            f64::from(frames.start),
            f64::from(frames.end),
        )
    }
}

/// The filter's label.
fn filter_label(filter: CommentFilter) -> review_localization::Key {
    match filter {
        CommentFilter::Open => keys::ui_comments::FILTER_OPEN,
        CommentFilter::Resolved => keys::ui_comments::FILTER_RESOLVED,
        CommentFilter::All => keys::ui_comments::FILTER_ALL,
    }
}

pub(super) fn comments_tab(
    ui: &mut egui::Ui,
    state: &mut UiState,
    model: &review_model::ModelData,
) -> Option<CommentIntent> {
    // ── The filter and the pins toggle: stock widgets, one row.
    ui.horizontal(|ui| {
        for filter in CommentFilter::ALL {
            ui.selectable_value(&mut state.comments.filter, filter, filter_label(filter));
        }
        ui.separator();
        let pins = ui.checkbox(&mut state.comments.show_pins, keys::ui_comments::SHOW_PINS);
        tip(
            pins,
            Tip::new(keys::ui_comments::SHOW_PINS)
                .describe(keys::ui_comments::SHOW_PINS_DESCRIPTION)
                .page(Page::Comments),
        );
    });
    if state.comments.writable() && ui.button(keys::ui_comments::NEW_FILE_NOTE).clicked() {
        let center = state.scene_viewport.map_or_else(
            || ui.ctx().content_rect().center(),
            |viewport| viewport.center(),
        );
        state.start_comment(model, DraftAnchor::File, center);
    }
    ui.add_space(size::PANEL_ROW_GAP);

    if state.comments.threads.is_empty() {
        ui.weak(keys::ui_comments::EMPTY);
        return None;
    }
    if state.comments.threads.iter().any(|entry| entry.read_only) {
        ui.weak(keys::ui_comments::READ_ONLY);
        ui.add_space(size::PANEL_ROW_GAP);
    }

    let query = state.outliner.search.trim().to_lowercase();
    let selected = state
        .comment_inspected()
        .then_some(state.comments.selected)
        .flatten();
    let mut clicked = None;
    let mut listed = 0;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, entry) in state.comments.threads.iter().enumerate() {
                if !state.comments.listed(index) {
                    continue;
                }
                let thread = &entry.thread;
                let searchable = format!(
                    "{} {} {}",
                    thread.title(),
                    thread.author(),
                    entry.object_name
                );
                if !matches_search(&searchable, &query) {
                    continue;
                }
                listed += 1;
                let is_selected = selected == Some(index);
                let (response, content, _) = widgets::list_row(
                    ui,
                    ui.id().with(("comment", index)),
                    is_selected,
                    0.0,
                    size::COMMENT_ROW_HEIGHT,
                );
                paint_row(ui, content, index, entry, is_selected);
                if response.clicked() {
                    clicked = Some(index);
                }
                tip(
                    response,
                    Tip::new(keys::ui_comments::row_tooltip(entry.object_name.clone()))
                        .describe(keys::ui_comments::ROW_TOOLTIP_DESCRIPTION)
                        .page(Page::Comments),
                );
            }
            if listed == 0 {
                ui.weak(if !query.is_empty() {
                    keys::ui_outliner::NO_MATCHES
                } else if state.comments.filter == CommentFilter::Resolved {
                    keys::ui_comments::NONE_RESOLVED
                } else {
                    keys::ui_comments::NONE_OPEN
                });
            }
        });

    let index = clicked?;
    state.select_comment(index, true);
    Some(CommentIntent::Show(index))
}

/// One thread row: its number, the opening text, and a muted line naming who
/// wrote it, on which object, about which frames.
fn paint_row(
    ui: &egui::Ui,
    content: egui::Rect,
    index: usize,
    entry: &crate::state::CommentEntry,
    selected: bool,
) {
    let painter = ui.painter();
    let thread = &entry.thread;
    let resolved = thread.status == Status::Resolved;
    let text_color = if selected {
        color::TEXT_PRIMARY
    } else if resolved {
        color::TEXT_MUTED
    } else {
        color::TEXT_BODY
    };
    let (top, bottom) = content.split_top_bottom_at_fraction(0.5);

    // The number, in the pin's colour, so a row and its pin read as one thing.
    painter.text(
        egui::pos2(top.left(), top.bottom()),
        egui::Align2::LEFT_BOTTOM,
        keys::ui_comments::number(index as f64 + 1.0),
        egui::FontId::monospace(font::COMMENT_META),
        if resolved {
            color::COMMENT_PIN_RESOLVED
        } else {
            color::COMMENT_PIN_OPEN
        },
    );

    let text_left = content.left() + size::COMMENT_NUMBER_WIDTH;
    let line =
        |text: String, rect: egui::Rect, font_id: egui::FontId, tint: egui::Color32, align| {
            let mut job = egui::text::LayoutJob::simple_singleline(text, font_id, tint);
            job.wrap = egui::text::TextWrapping {
                max_width: (rect.right() - text_left).max(0.0),
                max_rows: 1,
                break_anywhere: true,
                overflow_character: Some('…'),
            };
            let galley = painter.layout_job(job);
            let y = match align {
                egui::Align::Max => rect.bottom() - galley.size().y,
                _ => rect.top(),
            };
            painter.galley(egui::pos2(text_left, y), galley, tint);
        };

    let title = thread.title().lines().next().unwrap_or_default().to_owned();
    let body = egui::TextStyle::Body.resolve(ui.style());
    line(title, top, body, text_color, egui::Align::Max);

    let mut meta = vec![thread.author().to_owned(), entry.object_name.clone()];
    if let Some(frames) = &thread.frames {
        meta.push(frames_label(frames));
    }
    let meta: Vec<String> = meta.into_iter().filter(|part| !part.is_empty()).collect();
    line(
        meta.join(" · "),
        bottom,
        egui::FontId::proportional(font::COMMENT_META),
        color::TEXT_MUTED,
        egui::Align::Min,
    );
}
