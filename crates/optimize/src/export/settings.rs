//! The scene's own settings: axes, unit, time mode, and the metadata beside them.

use review_model::SourceExtras;
use review_model::extras::CoordinateAxis;

use super::*;

/// The `ufbxw_time_mode` code for a frame rate, when it is one of the standard
/// modes; `None` means custom.
pub(crate) fn time_mode_for(fps: f64) -> Option<i32> {
    const MODES: [(f64, i32); 12] = [
        (120.0, 1),
        (100.0, 2),
        (60.0, 3),
        (50.0, 4),
        (48.0, 5),
        (30.0, 6),
        (25.0, 10),
        (24.0, 11),
        (1000.0, 12),
        (96.0, 15),
        (72.0, 16),
        (59.94, 17),
    ];
    MODES
        .into_iter()
        .find(|(rate, _)| (fps - rate).abs() < 1e-3)
        .map(|(_, mode)| mode)
}

pub(crate) fn axis_code(axis: CoordinateAxis) -> i32 {
    match axis {
        CoordinateAxis::Unknown | CoordinateAxis::Unnamed(_) => -1,
        other => other.code() as i32,
    }
}

/// Scene settings and metadata from the capture.
pub(crate) fn build_settings(scene: &mut SceneData, extras: &SourceExtras) {
    let source = &extras.scene;
    let axes = [
        axis_code(source.axes[0]),
        axis_code(source.axes[1]),
        axis_code(source.axes[2]),
    ];
    // Only a complete, known triple is declared; a partial one would describe
    // a frame no reader could resolve.
    scene.settings.axes = if axes.iter().all(|&axis| axis >= 0) {
        axes
    } else {
        [-1; 3]
    };
    // The source's own mode when it was a standard one; otherwise its rate as
    // a custom mode, so the clips play at the speed they were authored.
    let fps = source.frames_per_second;
    scene.settings.time_mode = match source.time_mode {
        review_model::extras::TimeMode::Custom | review_model::extras::TimeMode::Default => {
            time_mode_for(fps).unwrap_or(14)
        }
        other => other.code() as i32,
    };
    scene.settings.frame_rate = fps;
    scene.settings.settings_props = scene.push_props(&source.settings_props);
    scene.settings.scene_info_props = scene.push_props(&source.scene_props);
    scene.settings.original_application = [
        c_string_or_empty(&source.original_application.vendor),
        c_string_or_empty(&source.original_application.name),
        c_string_or_empty(&source.original_application.version),
    ];
    scene.settings.original_filename = c_string_or_empty(&source.original_file_path);
}
