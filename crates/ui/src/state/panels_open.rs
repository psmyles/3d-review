//! Which option tools are open.
//!
//! They are native `egui::Window`s and several can be open at once, so this is a
//! set rather than a single selection.

use std::collections::HashSet;

use review_l10n::Key;

use crate::docs::Page;
use crate::keys;

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

    /// Title shown in the panel's native window title bar, and beside the tool's
    /// button in the toolbar and status bar.
    pub(crate) fn title(self) -> Key {
        match self {
            OptionPanel::Wireframe => keys::ui_panel_titles::WIREFRAME,
            OptionPanel::MaterialMode => keys::ui_panel_titles::MATERIAL_MODE,
            OptionPanel::BufferView => keys::ui_panel_titles::BUFFER_VIEW,
            OptionPanel::BoundingBox => keys::ui_panel_titles::BOUNDING_BOX,
            OptionPanel::UvChecker => keys::ui_panel_titles::UV_CHECKER,
            OptionPanel::FaceNormals => keys::ui_panel_titles::FACE_NORMALS,
            OptionPanel::VertexNormals => keys::ui_panel_titles::VERTEX_NORMALS,
            OptionPanel::UvSeams => keys::ui_panel_titles::UV_SEAMS,
            OptionPanel::Skeleton => keys::ui_panel_titles::SKELETON,
            OptionPanel::VertexColors => keys::ui_panel_titles::VERTEX_COLORS,
            OptionPanel::AntiAliasing => keys::ui_panel_titles::ANTI_ALIASING,
            OptionPanel::Background => keys::ui_panel_titles::BACKGROUND,
            OptionPanel::Environment => keys::ui_panel_titles::ENVIRONMENT,
            OptionPanel::Gtao => keys::ui_panel_titles::GTAO,
            OptionPanel::Tonemap => keys::ui_panel_titles::TONEMAP,
        }
    }

    /// The tool's page in the manual: what its footer's `?` opens, what `F1` over
    /// its window opens, and where its toolbar button's tooltip links.
    ///
    /// Exhaustive on purpose — a panel whose page is renamed or removed is a
    /// compile error here, not a dead link at run time (invariant 12).
    pub(crate) fn help_page(self) -> Page {
        match self {
            OptionPanel::Wireframe => Page::PanelsWireframe,
            OptionPanel::MaterialMode => Page::PanelsMaterialMode,
            OptionPanel::BufferView => Page::PanelsBuffers,
            OptionPanel::BoundingBox => Page::PanelsBoundingBox,
            OptionPanel::UvChecker => Page::PanelsUvChecker,
            OptionPanel::FaceNormals => Page::PanelsFaceNormals,
            OptionPanel::VertexNormals => Page::PanelsVertexNormals,
            OptionPanel::UvSeams => Page::PanelsUvSeams,
            OptionPanel::Skeleton => Page::PanelsSkeleton,
            OptionPanel::VertexColors => Page::PanelsVertexColors,
            OptionPanel::AntiAliasing => Page::PanelsAntiAliasing,
            OptionPanel::Background => Page::PanelsBackground,
            OptionPanel::Environment => Page::PanelsEnvironment,
            OptionPanel::Gtao => Page::PanelsAmbientOcclusion,
            OptionPanel::Tonemap => Page::PanelsTonemapper,
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
