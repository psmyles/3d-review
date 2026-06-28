//! Tool option-panels. Each tool's controls live in its own submodule; this
//! module owns the dispatch ([`draw_panel_body`]) that routes an [`OptionPanel`]
//! to the right tool. The window chrome around each panel is a native
//! `egui::Window` owned by the overlay; the Outliner / Inspector are native
//! `egui::SidePanel`s. So this module only fills in controls.

mod anti_aliasing;
mod background;
mod bounding_box;
mod buffer_view;
mod environment;
mod gtao;
pub(crate) mod inspector;
mod material_mode;
mod normals;
pub(crate) mod outliner;
mod tonemap;
mod uv_checker;
mod vertex_colors;
mod wireframe;

use crate::state::{OptionPanel, UiState};
use crate::theme::{font, size};

/// Draw a tool panel's controls into `ui` (the body of its native window). The
/// window chrome — title bar, collapse triangle, X-close, drag, resize, shadow —
/// is owned by the `egui::Window` the overlay wraps this in, so this only fills
/// in the controls. Each panel lays its rows out in a stock striped
/// [`crate::widgets::panel_grid`] and uses egui's default spacing throughout. All
/// panels are pinned to one width so the (non-resizable) windows look consistent.
pub(crate) fn draw_panel_body(ui: &mut egui::Ui, state: &mut UiState, panel: OptionPanel) {
    // Pin every panel to an *exact* width (the grid's two columns plus their
    // gap): all panels are the same snug width, and — crucially — bounding the
    // width makes `ui.available_width()` deterministic, so the elastic control
    // column (see `widgets::grid_control`) fills to the right edge instead of
    // feeding back into the window's auto-size and expanding without bound.
    ui.set_width(
        size::PANEL_LABEL_COL_WIDTH + size::PANEL_GRID_COL_GAP + size::PANEL_CONTROL_COL_WIDTH,
    );
    // Only the numeric value boxes (egui `DragValue`) render in the monospace
    // face, one step smaller than egui's default, so numeric readouts share a
    // fixed-width glyph grid. Everything else — labels, dropdowns, the reset
    // button — stays on the proportional Inter face. We retarget the
    // `DragValue`'s own text style (`drag_value_text_style`) to a downsized
    // `TextStyle::Monospace` rather than overriding `TextStyle::Button` (which
    // combos and buttons also use). Scoped to this body.
    let style = ui.style_mut();
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        egui::FontId::monospace(font::PANEL_BODY),
    );
    style.drag_value_text_style = egui::TextStyle::Monospace;
    match panel {
        OptionPanel::Wireframe => wireframe::body(ui, state),
        OptionPanel::MaterialMode => material_mode::body(ui, state),
        OptionPanel::BufferView => buffer_view::body(ui, state),
        OptionPanel::BoundingBox => bounding_box::body(ui, state),
        OptionPanel::UvChecker => uv_checker::body(ui, state),
        OptionPanel::FaceNormals => normals::face_body(ui, state),
        OptionPanel::VertexNormals => normals::vertex_body(ui, state),
        OptionPanel::VertexColors => vertex_colors::body(ui, state),
        OptionPanel::AntiAliasing => anti_aliasing::body(ui, state),
        OptionPanel::Background => background::body(ui, state),
        OptionPanel::Environment => environment::body(ui, state),
        OptionPanel::Gtao => gtao::body(ui, state),
        OptionPanel::Tonemap => tonemap::body(ui, state),
    }
}
