//! Vertex Color tool options: which channels of the mesh's vertex-color
//! attribute to visualize. RGB shows the color channels, Alpha shows the alpha
//! as 0..1 greyscale, and RGB+A uses alpha as surface opacity.

use review_render::VertexColorMode;

use crate::docs::Page;
use crate::keys;
use crate::labels;
use crate::state::{UiState, VertexColorPanelState};
use crate::widgets::{Tip, grid_control, grid_label_tip, labeled_combo, panel_footer, panel_grid};

/// This panel's page in the manual, opened by its footer's `?`.
const PAGE: Page = Page::PanelsVertexColors;

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "vertex_colors", |ui| {
        color_mode_row(ui, &mut state.vertex_colors.mode);
    });
    if state.vertex_colors.mode == VertexColorMode::Indexed {
        // Its own grid, as the Wireframe panel's colliders are: rows that come
        // and go with the mode must not change the width the mode row measured.
        panel_grid(ui, "vertex_colors_index", |ui| {
            index_rows(ui, &mut state.vertex_colors);
        });
    }
    ui.separator();
    if panel_footer(ui, PAGE).clicked() {
        state.vertex_colors = VertexColorPanelState::default();
    }
}

/// "Color Mode" row: a dropdown choosing which vertex-color channels the view
/// shows.
fn color_mode_row(ui: &mut egui::Ui, mode: &mut VertexColorMode) {
    labeled_combo(
        ui,
        Tip::new(keys::ui_panels::COLOR_MODE)
            .describe(keys::ui_panels::COLOR_MODE_DESCRIPTION)
            .page(PAGE),
        "vertex_color_mode",
        labels::vertex_color_mode(*mode),
        |ui| {
            for choice in [
                VertexColorMode::Rgb,
                VertexColorMode::Alpha,
                VertexColorMode::RgbAlpha,
                VertexColorMode::Indexed,
            ] {
                ui.selectable_value(mode, choice, labels::vertex_color_mode(choice));
            }
        },
    );
}

/// The palette-index mode's rows: which channel holds the index, how it was
/// stored, and the palette it indexes.
fn index_rows(ui: &mut egui::Ui, panel: &mut VertexColorPanelState) {
    let channel_label = |channel: u8| match channel {
        0 => keys::ui_panels::INDEX_CHANNEL_RED,
        1 => keys::ui_panels::INDEX_CHANNEL_GREEN,
        2 => keys::ui_panels::INDEX_CHANNEL_BLUE,
        _ => keys::ui_panels::INDEX_CHANNEL_ALPHA,
    };
    labeled_combo(
        ui,
        Tip::new(keys::ui_panels::INDEX_CHANNEL)
            .describe(keys::ui_panels::INDEX_CHANNEL_DESCRIPTION)
            .page(PAGE),
        "vertex_color_index_channel",
        channel_label(panel.index_channel),
        |ui| {
            for channel in 0..4 {
                ui.selectable_value(&mut panel.index_channel, channel, channel_label(channel));
            }
        },
    );

    let srgb = Tip::new(keys::ui_panels::INDEX_SRGB)
        .describe(keys::ui_panels::INDEX_SRGB_DESCRIPTION)
        .page(PAGE);
    grid_label_tip(ui, srgb);
    grid_control(ui, |ui| ui.checkbox(&mut panel.index_srgb, ""));
    ui.end_row();

    // The palette: its file's name (or Built-in), then the buttons that change
    // it on a row of their own.
    grid_label_tip(
        ui,
        Tip::new(keys::ui_panels::PALETTE)
            .describe(keys::ui_panels::PALETTE_DESCRIPTION)
            .page(PAGE),
    );
    grid_control(ui, |ui| match panel.palette_file.as_ref() {
        Some(path) => {
            let name = path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned();
            let _ = ui
                .add(egui::Label::new(name).truncate())
                .on_hover_text(path.display().to_string());
        }
        None => {
            ui.weak(keys::ui_panels::PALETTE_BUILT_IN);
        }
    });
    ui.end_row();
    ui.label("");
    grid_control(ui, |ui| {
        if ui.button(keys::ui_panels::PALETTE_LOAD).clicked() {
            panel.palette_requested = true;
        }
        if panel.palette_file.is_some() && ui.button(keys::ui_panels::PALETTE_CLEAR).clicked() {
            panel.palette.clear();
            panel.palette_file = None;
        }
    });
    ui.end_row();
}
