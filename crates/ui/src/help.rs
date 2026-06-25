//! Startup help overlay: a centered modal cheat-sheet of the keyboard shortcuts
//! plus the drop / double-click hint. Shown once on launch (`show_help_overlay`
//! starts `true`). Like the rest of `ui` it owns no behavior: it just paints and
//! swallows pointer input so the chrome beneath stays inert while it's up. `app`
//! decides when to dismiss it (any key / click / file drop / model load) and
//! turns a double-click on it into a file-picker open.

use crate::state::UiState;
use crate::theme::{color, font, size};

/// Left-column shortcut rows: (key-cap glyph, description). Mirrors the bare-key
/// shortcuts handled in `app`'s `handle_keyboard_shortcut`.
const LEFT_SHORTCUTS: [(&str, &str); 6] = [
    ("`", "Toggle wireframe overlay"),
    ("1", "Wireframe only"),
    ("2", "Unlit shading mode"),
    ("3", "Lit shading mode"),
    ("I", "Toggle stats display"),
    ("G", "Toggle grid display"),
];

/// Right-column shortcut rows (camera framing + WASD orbit steps).
const RIGHT_SHORTCUTS: [(&str, &str); 6] = [
    ("F", "Frame model / selection"),
    ("R", "Reset camera"),
    ("W", "Orbit camera up"),
    ("A", "Orbit camera left"),
    ("S", "Orbit camera down"),
    ("D", "Orbit camera right"),
];

/// Draw the startup help overlay when active. A full-screen invisible catcher
/// plus the card both sit on the foreground order so they paint above the chrome
/// and swallow pointer input before it can reach the camera or toolbar beneath —
/// `app` reads the raw click/key/drop events to dismiss it (and to turn a
/// double-click into a file-picker open). The catcher has no fill: the viewport
/// is *not* dimmed; only the card itself reads as a translucent surface.
pub(crate) fn draw_help_overlay(ctx: &egui::Context, state: &UiState) {
    if !state.show_help_overlay {
        return;
    }

    let screen = ctx.screen_rect();

    egui::Area::new(egui::Id::new("help_backdrop"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            ui.allocate_rect(screen, egui::Sense::click());
        });

    egui::Area::new(egui::Id::new("help_card"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| draw_card(ui, state));
}

/// Paint the help card and return a click response covering its whole rect, so a
/// click on the card (not just the scrim) also dismisses the overlay.
fn draw_card(ui: &mut egui::Ui, state: &UiState) -> egui::Response {
    let frame = egui::Frame::NONE
        .fill(color::HELP_CARD_BG)
        .stroke(egui::Stroke::new(size::HAIRLINE, color::HELP_CARD_BORDER))
        .corner_radius(size::HELP_CARD_CORNER_RADIUS)
        .inner_margin(egui::Margin::symmetric(
            size::HELP_CARD_PAD_X,
            size::HELP_CARD_PAD_Y,
        ))
        .show(ui, |ui| draw_card_contents(ui, state));

    ui.interact(
        frame.response.rect,
        ui.id().with("help_card_hit"),
        egui::Sense::click(),
    )
}

fn draw_card_contents(ui: &mut egui::Ui, state: &UiState) {
    let content_width = size::HELP_COLUMN_WIDTH * 2.0 + size::HELP_COLUMN_GAP;
    ui.set_width(content_width);

    let version = if state.app_version.is_empty() {
        "?"
    } else {
        state.app_version.as_str()
    };
    let backend = if state.gpu_backend.is_empty() {
        "?"
    } else {
        state.gpu_backend.as_str()
    };

    centered_text(ui, "3D Review", font::HELP_TITLE, color::TEXT_PRIMARY);
    centered_text(
        ui,
        &format!("version: {version}, renderer: {backend}"),
        font::HELP_META,
        color::TEXT_MUTED,
    );
    section_divider(ui);
    centered_text(
        ui,
        "Drop a 3D file or double click on the empty viewport to open a file",
        font::HELP_SUBTITLE,
        color::TEXT_BODY,
    );
    section_divider(ui);

    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        shortcut_column(ui, &LEFT_SHORTCUTS);
        ui.add_space(size::HELP_COLUMN_GAP);
        shortcut_column(ui, &RIGHT_SHORTCUTS);
    });

    section_divider(ui);
    shortcut_row(ui, &["Ctrl", "N"], "Reset 3D Review to start state");
    ui.add_space(size::HELP_ROW_GAP);
    shortcut_row(ui, &["Ctrl", "O"], "Open model from file dialog");
    ui.add_space(size::HELP_ROW_GAP);
    shortcut_row(ui, &["Ctrl", "Z"], "Undo");
    ui.add_space(size::HELP_ROW_GAP);
    shortcut_row(ui, &["Ctrl", "Y"], "Redo");
    ui.add_space(size::HELP_ROW_GAP);
    shortcut_row(ui, &["Esc"], "Clear selection");

    section_divider(ui);
    centered_text(
        ui,
        "Click anywhere or press any key to dismiss this",
        font::HELP_FOOTER,
        color::TEXT_MUTED,
    );
}

/// One fixed-width column of single-key shortcut rows.
fn shortcut_column(ui: &mut egui::Ui, rows: &[(&str, &str)]) {
    ui.allocate_ui_with_layout(
        egui::vec2(size::HELP_COLUMN_WIDTH, 0.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_width(size::HELP_COLUMN_WIDTH);
            for (i, (key, desc)) in rows.iter().enumerate() {
                shortcut_row(ui, &[key], desc);
                if i + 1 < rows.len() {
                    ui.add_space(size::HELP_ROW_GAP);
                }
            }
        },
    );
}

/// A single row: one or more key-caps followed by a description.
fn shortcut_row(ui: &mut egui::Ui, keys: &[&str], desc: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (i, key) in keys.iter().enumerate() {
            if i > 0 {
                ui.add_space(size::HELP_KEYCAP_GAP);
            }
            key_cap(ui, key);
        }
        ui.add_space(size::HELP_KEY_LABEL_GAP);
        ui.label(
            egui::RichText::new(desc)
                .size(font::HELP_BODY)
                .color(color::TEXT_BODY),
        );
    });
}

/// Paint one key-cap tile with its glyph centered. Word labels ("Ctrl") get a
/// wider cap; single glyphs are square.
fn key_cap(ui: &mut egui::Ui, label: &str) {
    let width = if label.chars().count() > 1 {
        size::HELP_KEYCAP_WIDE_WIDTH
    } else {
        size::HELP_KEYCAP_SIZE
    };
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(width, size::HELP_KEYCAP_SIZE),
        egui::Sense::hover(),
    );
    ui.painter().rect(
        rect,
        size::HELP_KEYCAP_CORNER_RADIUS,
        color::HELP_KEYCAP_BG,
        egui::Stroke::new(size::HAIRLINE, color::HELP_KEYCAP_BORDER),
        egui::StrokeKind::Inside,
    );
    // Key glyphs read as monospace, like physical key-cap legends. The backtick
    // is the lone glyph that renders tiny at the shared size, so bump just it.
    let glyph_size = if label == "`" {
        font::HELP_KEYCAP_BACKTICK
    } else {
        font::HELP_KEYCAP
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::monospace(glyph_size),
        color::TEXT_PRIMARY,
    );
}

/// A centered single line of text spanning the card width.
fn centered_text(ui: &mut egui::Ui, text: &str, font_size: f32, color: egui::Color32) {
    ui.vertical_centered(|ui| {
        ui.label(egui::RichText::new(text).size(font_size).color(color));
    });
}

/// A full-width hairline divider with symmetric vertical breathing room.
fn section_divider(ui: &mut egui::Ui) {
    ui.add_space(size::HELP_SECTION_GAP);
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, size::HAIRLINE), egui::Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        egui::Stroke::new(size::HAIRLINE, color::HELP_DIVIDER),
    );
    ui.add_space(size::HELP_SECTION_GAP);
}
