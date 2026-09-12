//! Which option tools are open.
//!
//! They are native `egui::Window`s and several can be open at once, so this is a
//! set rather than a single selection.

use std::collections::HashSet;

/// A tool's options panel. Each is shown as its own native `egui::Window`, so
/// several can be open at once (see [`PanelsOpen`]); a panel is toggled by
/// right-clicking its toolbar / status-bar button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OptionPanel {
    Wireframe,
    MaterialMode,
    BufferView,
    BoundingBox,
    UvChecker,
    FaceNormals,
    VertexNormals,
    UvSeams,
    Skeleton,
    VertexColors,
    AntiAliasing,
    Background,
    Environment,
    Gtao,
    Tonemap,
}

impl OptionPanel {
    /// Every panel, in toolbar order. Iterated each frame to draw the open ones
    /// (and to give each a stable cascade slot), so the order is deterministic.
    pub(crate) const ALL: [OptionPanel; 15] = [
        OptionPanel::Wireframe,
        OptionPanel::MaterialMode,
        OptionPanel::BufferView,
        OptionPanel::BoundingBox,
        OptionPanel::UvChecker,
        OptionPanel::FaceNormals,
        OptionPanel::VertexNormals,
        OptionPanel::UvSeams,
        OptionPanel::Skeleton,
        OptionPanel::VertexColors,
        OptionPanel::AntiAliasing,
        OptionPanel::Background,
        OptionPanel::Environment,
        OptionPanel::Gtao,
        OptionPanel::Tonemap,
    ];

    /// Title shown in the panel's native window title bar.
    pub(crate) fn title(self) -> &'static str {
        match self {
            OptionPanel::Wireframe => "Wireframe",
            OptionPanel::MaterialMode => "Material Mode",
            OptionPanel::BufferView => "Buffers",
            OptionPanel::BoundingBox => "Bounding Box",
            OptionPanel::UvChecker => "UV Checker",
            OptionPanel::FaceNormals => "Face Normals",
            OptionPanel::VertexNormals => "Vertex Normals",
            OptionPanel::UvSeams => "UV Seams",
            OptionPanel::Skeleton => "Skeleton",
            OptionPanel::VertexColors => "Vertex Colors",
            OptionPanel::AntiAliasing => "Anti Aliasing",
            OptionPanel::Background => "Background",
            OptionPanel::Environment => "Environment",
            OptionPanel::Gtao => "Ambient Occlusion",
            OptionPanel::Tonemap => "Tonemapper",
        }
    }

    /// Stable, unique id string for the panel's window, so egui keeps each
    /// window's position/size independent in its memory across frames.
    pub(crate) fn window_id(self) -> &'static str {
        match self {
            OptionPanel::Wireframe => "panel_wireframe",
            OptionPanel::MaterialMode => "panel_material_mode",
            OptionPanel::BufferView => "panel_buffer_view",
            OptionPanel::BoundingBox => "panel_bounding_box",
            OptionPanel::UvChecker => "panel_uv_checker",
            OptionPanel::FaceNormals => "panel_face_normals",
            OptionPanel::VertexNormals => "panel_vertex_normals",
            OptionPanel::UvSeams => "panel_uv_seams",
            OptionPanel::Skeleton => "panel_skeleton",
            OptionPanel::VertexColors => "panel_vertex_colors",
            OptionPanel::AntiAliasing => "panel_anti_aliasing",
            OptionPanel::Background => "panel_background",
            OptionPanel::Environment => "panel_environment",
            OptionPanel::Gtao => "panel_gtao",
            OptionPanel::Tonemap => "panel_tonemap",
        }
    }
}

/// The set of option panels currently open. Each open panel is its own native
/// `egui::Window` (egui owns its position/size/collapsed state in memory), so
/// any number can be open simultaneously — a button right-click toggles its
/// panel's membership here.
#[derive(Debug, Clone, Default)]
pub struct PanelsOpen {
    pub(crate) open: HashSet<OptionPanel>,
}

impl PanelsOpen {
    pub(crate) fn is_open(&self, panel: OptionPanel) -> bool {
        self.open.contains(&panel)
    }

    /// Toggle a panel open/closed (the right-click-button behavior).
    pub(crate) fn toggle(&mut self, panel: OptionPanel) {
        if !self.open.remove(&panel) {
            self.open.insert(panel);
        }
    }

    /// Force a panel's open state (used when egui's window X-button closes it).
    pub(crate) fn set(&mut self, panel: OptionPanel, open: bool) {
        if open {
            self.open.insert(panel);
        } else {
            self.open.remove(&panel);
        }
    }
}
