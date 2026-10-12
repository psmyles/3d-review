//! Camera tool options: how fast each camera gesture moves the 3D cameras, and
//! their field of view. Opened by right-clicking the projection button.
//!
//! Writes straight into [`UiState::camera`], which `app` reads every frame — the
//! field of view goes to the renderer, the speeds scale `app`'s own gestures — so
//! there is no commit step.
//!
//! The field of view sits apart, under a note, because it is the one setting
//! here the orthographic camera ignores: a change dollies the camera to keep the
//! framing (`OrbitCamera::set_fov_keeping_extent`), which by construction leaves
//! an orthographic view exactly as it was. So it is greyed out while that camera
//! is up, and the note says why rather than leaving a dead control to puzzle
//! over.

use crate::docs::Page;
use crate::keys;
use crate::state::{
    CameraPanelState, UiState, ViewProjectionMode,
    range::{
        CAMERA_FLY_SPEED_MAX, CAMERA_FLY_SPEED_MIN, CAMERA_FOV_MAX, CAMERA_FOV_MIN,
        CAMERA_ORBIT_SPEED_MAX, CAMERA_ORBIT_SPEED_MIN, CAMERA_ZOOM_SPEED_MAX,
        CAMERA_ZOOM_SPEED_MIN,
    },
};
use crate::widgets::{
    Tip, labeled_log_slider_with_value, labeled_slider_with_value, panel_footer, panel_grid,
};

/// This panel's page in the manual, opened by its footer's `?`.
const PAGE: Page = Page::PanelsCamera;

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "camera_speeds", |ui| {
        labeled_log_slider_with_value(
            ui,
            Tip::new(keys::ui_panels::CAMERA_ORBIT_SPEED)
                .describe(keys::ui_panels::CAMERA_ORBIT_SPEED_DESCRIPTION)
                .page(PAGE),
            &mut state.camera.orbit_speed,
            CAMERA_ORBIT_SPEED_MIN..=CAMERA_ORBIT_SPEED_MAX,
            2,
        );
        labeled_log_slider_with_value(
            ui,
            Tip::new(keys::ui_panels::CAMERA_ZOOM_SPEED)
                .describe(keys::ui_panels::CAMERA_ZOOM_SPEED_DESCRIPTION)
                .page(PAGE),
            &mut state.camera.zoom_speed,
            CAMERA_ZOOM_SPEED_MIN..=CAMERA_ZOOM_SPEED_MAX,
            2,
        );
        labeled_log_slider_with_value(
            ui,
            Tip::new(keys::ui_panels::CAMERA_FLY_SPEED)
                .describe(keys::ui_panels::CAMERA_FLY_SPEED_DESCRIPTION)
                .page(PAGE),
            &mut state.camera.fly_speed,
            CAMERA_FLY_SPEED_MIN..=CAMERA_FLY_SPEED_MAX,
            2,
        );
    });

    ui.separator();
    perspective_section(ui, state);

    ui.separator();
    if panel_footer(ui, PAGE).clicked() {
        state.camera = CameraPanelState::default();
    }
}

/// The settings only the perspective camera reads: a note saying so, then the
/// rows, disabled while the orthographic camera is up.
///
/// A grid of its own rather than more rows in the one above: disabling has to
/// wrap whole rows, and a child `Ui` inside a grid would take the grid's place
/// in the layout rather than one row of it.
fn perspective_section(ui: &mut egui::Ui, state: &mut UiState) {
    // egui's own weak text, as the Outliner's and Inspector's notes are: it
    // reads as a remark about the rows rather than another one of them.
    ui.add(
        egui::Label::new(egui::RichText::from(keys::ui_panels::CAMERA_PERSPECTIVE_NOTE).weak())
            .wrap(),
    );
    let perspective = state.projection_mode == ViewProjectionMode::Perspective;
    ui.add_enabled_ui(perspective, |ui| {
        panel_grid(ui, "camera_perspective", |ui| {
            labeled_slider_with_value(
                ui,
                Tip::new(keys::ui_panels::CAMERA_FIELD_OF_VIEW)
                    .describe(keys::ui_panels::CAMERA_FIELD_OF_VIEW_DESCRIPTION)
                    .page(PAGE),
                &mut state.camera.fov_degrees,
                CAMERA_FOV_MIN..=CAMERA_FOV_MAX,
                0,
            );
        });
    });
}
