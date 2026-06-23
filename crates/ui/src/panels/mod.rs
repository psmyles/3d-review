//! Tool option-panels. Each tool's controls live in its own submodule; this
//! module owns the dispatch ([`draw_panel_body`]) that routes an [`OptionPanel`]
//! to the right tool. The window chrome around each panel is a native
//! `egui::Window` owned by the overlay; the Outliner / Inspector are native
//! `egui::SidePanel`s. So this module only fills in controls.

mod anti_aliasing;
mod bloom;
mod bounding_box;
mod environment;
pub(crate) mod inspector;
mod normals;
pub(crate) mod outliner;
mod ssao;
mod tonemap;
mod uv_checker;
mod vertex_colors;
mod wireframe;

use crate::state::{OptionPanel, UiState};

/// Draw a tool panel's controls into `ui` (the body of its native window). The
/// window chrome — title bar, collapse triangle, X-close, drag, resize, shadow —
/// is owned by the `egui::Window` the overlay wraps this in, so this only fills
/// in the controls. Each panel lays its rows out in a stock striped
/// [`crate::widgets::panel_grid`] and uses egui's default spacing throughout.
pub(crate) fn draw_panel_body(ui: &mut egui::Ui, state: &mut UiState, panel: OptionPanel) {
    match panel {
        OptionPanel::Wireframe => wireframe::body(ui, state),
        OptionPanel::BoundingBox => bounding_box::body(ui, state),
        OptionPanel::UvChecker => uv_checker::body(ui, state),
        OptionPanel::FaceNormals => normals::face_body(ui, state),
        OptionPanel::VertexNormals => normals::vertex_body(ui, state),
        OptionPanel::VertexColors => vertex_colors::body(ui, state),
        OptionPanel::AntiAliasing => anti_aliasing::body(ui, state),
        OptionPanel::Environment => environment::body(ui, state),
        OptionPanel::Bloom => bloom::body(ui, state),
        OptionPanel::Ssao => ssao::body(ui, state),
        OptionPanel::Tonemap => tonemap::body(ui, state),
    }
}
