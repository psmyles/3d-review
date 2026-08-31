//! Environment / image-based-lighting options: which HDR environment lights the
//! scene, whether it is drawn as the background, an intensity multiplier, and the
//! environment's yaw rotation.
//!
//! All write straight into [`UiState::environment`], which the viewport callback
//! reads each frame; the renderer rebuilds the precomputed IBL maps only when the
//! chosen environment changes (rotation/intensity are live, no rebuild). The IBL
//! on/off master toggle is the IBL button in the status bar (see `status_bar.rs`),
//! not a row here.

use review_render::{EnvironmentMap, EnvironmentSettings};

use crate::assets::{self, AppIcon};
use crate::state::{
    UiState,
    range::{ENV_INTENSITY_MAX, ENV_INTENSITY_MIN, ENV_ROTATION_MAX, ENV_ROTATION_MIN},
};
use crate::theme::size;
use crate::widgets::{
    labeled_checkbox, labeled_combo, labeled_slider_with_value, panel_grid, reset_button,
};

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "environment", |ui| {
        environment_row(ui, state);
        labeled_checkbox(ui, "Background", &mut state.environment.show_background);
        labeled_slider_with_value(
            ui,
            "Intensity",
            &mut state.environment.intensity,
            ENV_INTENSITY_MIN..=ENV_INTENSITY_MAX,
            2,
        );
        labeled_slider_with_value(
            ui,
            "Rotation",
            &mut state.environment.rotation_degrees,
            ENV_ROTATION_MIN..=ENV_ROTATION_MAX,
            0,
        );
    });

    ui.separator();
    if reset_button(ui).clicked() {
        // The IBL on/off state is owned by the status-bar toggle, so reset only
        // the panel's own options and leave `ibl_enabled` untouched.
        state.environment = EnvironmentSettings {
            ibl_enabled: state.environment.ibl_enabled,
            ..EnvironmentSettings::default()
        };
    }
}

/// "Environment" row: a dropdown over the built-in HDR maps, each option showing
/// the map's preview thumbnail beside its label.
fn environment_row(ui: &mut egui::Ui, state: &mut UiState) {
    labeled_combo(
        ui,
        "Environment",
        "environment_map",
        state.environment.map.label(),
        |ui| {
            for map in EnvironmentMap::ALL {
                environment_option(ui, &mut state.environment.map, map);
            }
        },
    );
}

/// One Environment dropdown option: the map's preview thumbnail + its label, as a
/// single frameless selectable button. A click writes `map` into `current`; the
/// combo's `CloseOnClick` default then dismisses the popup.
fn environment_option(ui: &mut egui::Ui, current: &mut EnvironmentMap, map: EnvironmentMap) {
    let selected = *current == map;
    let label = map.label();
    let button = match assets::load_icon_texture(ui, thumbnail_icon(map)) {
        Some(texture) => {
            let image = egui::Image::from_texture(texture)
                .fit_to_exact_size(egui::vec2(size::ENV_THUMB_WIDTH, size::ENV_THUMB_HEIGHT));
            egui::Button::image_and_text(image, label)
        }
        // Thumbnail decode failed (a packaging bug) — fall back to a text option.
        None => egui::Button::new(label),
    };
    if ui.add(button.frame(false).selected(selected)).clicked() {
        *current = map;
    }
}

/// The embedded preview thumbnail for each environment map (see [`assets`]).
fn thumbnail_icon(map: EnvironmentMap) -> &'static AppIcon {
    match map {
        EnvironmentMap::Hdr01 => &assets::THUMB_HDR_01,
        EnvironmentMap::Hdr02 => &assets::THUMB_HDR_02,
        EnvironmentMap::Hdr03 => &assets::THUMB_HDR_03,
        EnvironmentMap::Hdr04 => &assets::THUMB_HDR_04,
        EnvironmentMap::Hdr05 => &assets::THUMB_HDR_05,
        EnvironmentMap::Hdr06 => &assets::THUMB_HDR_06,
    }
}
