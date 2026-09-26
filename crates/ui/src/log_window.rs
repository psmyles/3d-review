//! The Log window (Debug > View Log): this session's lines, newest at the
//! bottom, filtered by level.
//!
//! The lines are a plain value `app` hands over (invariant 2): each frame the
//! window is open, `app` appends whatever `review_log` has recorded since the
//! last handover to [`LogWindowState::entries`]. The window only reads them,
//! and **Clear** empties this copy — never the session buffer or the file, so
//! clearing the view loses nothing a bug report would need.
//!
//! Every line is in the monospace face, so the columns line up and a path reads
//! the way it is written: a fixed prefix (time, level, tag) and then the
//! message, which wraps to the window's width and continues under its own first
//! line rather than under the time. Only the rows in view are laid out - see
//! [`draw_lines`] - since a session can run to thousands of lines. The filter
//! bar and the footer keep the proportional face like every other control in
//! the chrome.

use std::path::PathBuf;

use egui::text::{LayoutJob, TextFormat};
use review_log::{Entry, Level};

use crate::keys;
use crate::labels;
use crate::theme::{font, size};

/// Whether the Log window is up, which levels it shows, and the lines `app` has
/// handed over for it.
#[derive(Debug, Clone)]
pub struct LogWindowState {
    /// Whether the window is up. Also what egui's title-bar ✕ clears.
    pub open: bool,
    /// Which levels are shown, one switch each, indexed like [`Level::ALL`].
    /// Debug starts off: it is developer detail, and the other three are what a
    /// reader opening the window is looking for.
    shown: [bool; Level::ALL.len()],
    /// This session's lines, oldest first, as `app` last handed them over —
    /// minus any **Clear** has since dropped.
    pub entries: Vec<Entry>,
    /// The first line `app` has not handed over yet (`review_log::read_since`
    /// advances it).
    pub next_seq: u64,
    /// Where the day's log is being written, once `app` has opened it.
    pub file: Option<PathBuf>,
    /// Each entry's drawn height at [`Self::heights_width`], parallel to
    /// `entries`; NaN until the entry has been drawn. Shorter than `entries`
    /// while new lines have not reached the screen yet.
    heights: Vec<f32>,
    /// The width `heights` was measured at. A new width moves every wrap.
    heights_width: f32,
}

impl Default for LogWindowState {
    fn default() -> Self {
        Self {
            open: false,
            shown: Level::ALL.map(|level| level != Level::Debug),
            entries: Vec::new(),
            next_seq: 0,
            file: None,
            heights: Vec::new(),
            heights_width: 0.0,
        }
    }
}

impl LogWindowState {
    /// Whether lines at `level` are shown.
    pub fn shows(&self, level: Level) -> bool {
        self.shown[level_index(level)]
    }

    /// Drop the oldest lines past what the session buffer itself keeps, so the
    /// window's copy is never the larger of the two.
    pub fn trim(&mut self) {
        let excess = self.entries.len().saturating_sub(review_log::CAPACITY);
        self.entries.drain(..excess);
        let measured = excess.min(self.heights.len());
        self.heights.drain(..measured);
    }
}

fn level_index(level: Level) -> usize {
    Level::ALL
        .iter()
        .position(|each| *each == level)
        .unwrap_or_default()
}

/// Draw the Log window while it is open, inside `viewport` like the Help window.
pub(crate) fn draw(ctx: &egui::Context, state: &mut LogWindowState, viewport: egui::Rect) {
    if !state.open {
        return;
    }
    let mut open = state.open;
    egui::Window::new(keys::ui_log::TITLE)
        .id(egui::Id::new("log_window"))
        .open(&mut open)
        .resizable(true)
        .default_size(size::LOG_WINDOW_DEFAULT)
        .min_size(size::LOG_WINDOW_MIN)
        .constrain_to(viewport)
        // Persistent chrome, not a transient popup: skip egui's fade so an open
        // window never spins the on-demand redraw loop (invariant 6).
        .fade_in(false)
        .fade_out(false)
        .show(ctx, |ui| {
            egui::Panel::top("log_filter").show(ui, |ui| draw_filter_bar(ui, state));
            if let Some(file) = &state.file {
                egui::Panel::bottom("log_footer").show(ui, |ui| {
                    ui.add(
                        egui::Label::new(keys::ui_log::file(file.display().to_string())).truncate(),
                    );
                });
            }
            egui::CentralPanel::default().show(ui, |ui| draw_lines(ui, state));
        });
    state.open = open;
}

/// The level switches, then **Clear** at the far end.
fn draw_filter_bar(ui: &mut egui::Ui, state: &mut LogWindowState) {
    ui.horizontal(|ui| {
        ui.weak(keys::ui_log::FILTER);
        let visuals = ui.visuals().clone();
        for level in Level::ALL {
            let name = review_localization::tr(labels::log_level(level)).into_owned();
            let text = match level_tint(level, &visuals) {
                Some(tint) => egui::RichText::new(name).color(tint),
                None => egui::RichText::new(name),
            };
            ui.checkbox(&mut state.shown[level_index(level)], text);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(keys::ui_log::CLEAR).clicked() {
                state.entries.clear();
                state.heights.clear();
            }
        });
    });
}

/// The colour a level's switch and lines are drawn in, where it has one of its
/// own: egui's own warning and error tints, as the notices use.
fn level_tint(level: Level, visuals: &egui::Visuals) -> Option<egui::Color32> {
    match level {
        Level::Debug | Level::Info => None,
        Level::Warning => Some(visuals.warn_fg_color),
        Level::Error => Some(visuals.error_fg_color),
    }
}

/// The shown lines, scrolled to the newest.
///
/// Rows differ in height once a message wraps, so this cannot be
/// [`egui::ScrollArea::show_rows`]. Instead each entry's height is remembered
/// the first time it is drawn (a single line is assumed until then) and only
/// the rows in view are laid out, which keeps a session of thousands of lines
/// as cheap as one of ten. A new width re-measures everything, since it moves
/// every wrap.
fn draw_lines(ui: &mut egui::Ui, state: &mut LogWindowState) {
    let shown: Vec<usize> = state
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| state.shows(entry.level))
        .map(|(index, _)| index)
        .collect();
    if shown.is_empty() {
        ui.weak(keys::ui_log::EMPTY);
        return;
    }

    let columns = Columns::new(ui, shown.iter().map(|&index| &state.entries[index]));
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show_viewport(ui, |ui, viewport| {
            let width = ui.available_width();
            if width != state.heights_width {
                state.heights.clear();
                state.heights_width = width;
            }
            state.heights.resize(state.entries.len(), f32::NAN);
            let spacing = ui.spacing().item_spacing.y;
            let origin = ui.max_rect().min;
            let mut top = 0.0;
            let mut remeasured = false;
            for &index in &shown {
                let known = state.heights[index];
                let mut height = if known.is_nan() {
                    columns.row_height
                } else {
                    known
                };
                if top + height >= viewport.min.y && top <= viewport.max.y {
                    let rect = egui::Rect::from_min_size(
                        origin + egui::vec2(0.0, top),
                        egui::vec2(width, height),
                    );
                    let drawn = ui
                        .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                            columns.draw(ui, &state.entries[index]);
                        })
                        .response
                        .rect
                        .height();
                    remeasured |= known.is_nan() || (drawn - known).abs() > 0.5;
                    state.heights[index] = drawn;
                    height = drawn;
                }
                top += height + spacing;
            }
            // The whole column, drawn or not, so the scroll bar is to scale. Not
            // `set_min_height`, which counts from the cursor - already past the
            // rows just drawn - and so doubles the height.
            ui.expand_to_include_rect(egui::Rect::from_min_size(origin, egui::vec2(width, top)));
            // A row drawn for the first time can turn out taller than the line it
            // was assumed to be; one more frame settles the scroll height on it.
            if remeasured {
                ui.ctx().request_repaint();
            }
        });
}

/// What every row shares: the face, and the fixed columns' text.
struct Columns {
    font: egui::FontId,
    /// Each level's column text, padded to the longest so the tag column lines up.
    levels: [String; Level::ALL.len()],
    /// How many characters the widest `[tag]` takes, so every message starts in
    /// one column and wraps back to it.
    tag_width: usize,
    /// One line's height: what a row not yet drawn is assumed to take.
    row_height: f32,
}

impl Columns {
    fn new<'a>(ui: &egui::Ui, shown: impl Iterator<Item = &'a Entry>) -> Self {
        let font = egui::FontId::monospace(font::LOG_TEXT);
        let names = Level::ALL.map(|level| review_localization::tr(labels::log_row_level(level)));
        let width = names
            .iter()
            .map(|name| name.chars().count())
            .max()
            .unwrap_or_default();
        let levels = names.map(|name| format!("{name:<width$}"));
        let tag_width = shown
            .map(|entry| entry.tag.chars().count() + 2)
            .max()
            .unwrap_or_default();
        let row_height = ui.ctx().fonts_mut(|fonts| fonts.row_height(&font));
        Self {
            font,
            levels,
            tag_width,
            row_height,
        }
    }

    /// One entry: `time  LEVEL  [tag]` as a fixed prefix, then the message,
    /// wrapped to whatever width is left and indented under its own first line.
    fn draw(&self, ui: &mut egui::Ui, entry: &Entry) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = size::LOG_COLUMN_GAP;
            let visuals = ui.visuals().clone();
            ui.add(egui::Label::new(self.prefix(&visuals, entry)).extend());
            ui.add(egui::Label::new(self.message(&visuals, entry)).wrap());
        });
    }

    fn format(&self, color: egui::Color32) -> TextFormat {
        TextFormat::simple(self.font.clone(), color)
    }

    fn prefix(&self, visuals: &egui::Visuals, entry: &Entry) -> LayoutJob {
        let tint = level_tint(entry.level, visuals);
        let level = &self.levels[level_index(entry.level)];
        let tag = format!("[{}]", entry.tag);
        let tag = format!("{tag:<width$}", width = self.tag_width);
        let gap = size::LOG_COLUMN_GAP;
        let mut job = LayoutJob::default();
        job.append(&entry.time, 0.0, self.format(visuals.weak_text_color()));
        job.append(
            level,
            gap,
            self.format(tint.unwrap_or_else(|| visuals.weak_text_color())),
        );
        job.append(&tag, gap, self.format(visuals.hyperlink_color));
        job
    }

    fn message(&self, visuals: &egui::Visuals, entry: &Entry) -> LayoutJob {
        let color = match entry.level {
            Level::Debug => visuals.weak_text_color(),
            level => level_tint(level, visuals).unwrap_or_else(|| visuals.text_color()),
        };
        LayoutJob::single_section(entry.message.clone(), self.format(color))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(seq: u64, level: Level) -> Entry {
        Entry {
            seq,
            time: "08:10:02.840".to_owned(),
            level,
            tag: "app".to_owned(),
            message: "line".to_owned(),
        }
    }

    #[test]
    fn debug_starts_hidden_and_the_rest_shown() {
        let state = LogWindowState::default();
        assert!(!state.shows(Level::Debug));
        assert!(state.shows(Level::Info));
        assert!(state.shows(Level::Warning));
        assert!(state.shows(Level::Error));
    }

    #[test]
    fn trimming_keeps_the_newest() {
        let mut state = LogWindowState {
            entries: (0..review_log::CAPACITY as u64 + 5)
                .map(|seq| entry(seq, Level::Info))
                .collect(),
            ..LogWindowState::default()
        };
        state.trim();
        assert_eq!(state.entries.len(), review_log::CAPACITY);
        assert_eq!(state.entries[0].seq, 5);
    }
}
