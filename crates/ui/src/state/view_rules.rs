//! Which viewport controls the view on screen leaves without effect, and why.
//!
//! Several views replace what others draw. An overdraw count replaces the whole
//! frame; Wireframe Only shading draws no surface for a material to paint; the
//! flat views (Buffers, Skin Weights, the density maps) show their values with no
//! lighting, tone mapping or occlusion; Unlit shading has no lighting; the
//! environment drawn as the background covers the background colour, and is not
//! drawn at all with IBL off. A tile for
//! a setting the view on screen ignores used to stay lit, so the bars could show
//! two things on while the viewport showed one of them.
//!
//! This is the one table of those rules. The toolbar and status bar ask it before
//! drawing a tile and draw an overridden one dimmed and unlit, saying what
//! overrides it ([`Override::reason`]); its left click is withheld, its
//! right-click still opens its options. `app` asks it too, for the keyboard
//! shortcuts that flip the same settings, and for what to hand the renderer. Each
//! rule mirrors a decision the renderer makes, named beside it, so a change to
//! one is a change to the other.
//!
//! The material tiles and the diagnostic views are a different case: they are
//! peers, two halves of one choice of what paints the model, so picking one turns
//! the other off ([`UiState::shows_material`], [`UiState::diagnostic_view`]).

use review_audit::DiagnosticView;
use review_render::{ActiveMaterial, ShadingMode};

use super::{UiState, WorkspaceMode};
use crate::keys;

/// A viewport control the rules can override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewControl {
    /// Show Wireframe, the overlay toggle.
    WireframeOverlay,
    /// The three shading modes.
    WireframeShading,
    UnlitShading,
    ShadedShading,
    Backfaces,
    /// Anything that paints the surface: every material tile, Aud's Issues view
    /// and the two density maps.
    SurfacePaint,
    FaceNormals,
    VertexNormals,
    UvSeams,
    BoundingBox,
    Pivot,
    Grid,
    Skeleton,
    /// The viewport background, the status bar's tile.
    Background,
    /// Background in the Environment options: the environment drawn behind the
    /// model.
    EnvironmentBackground,
    Ibl,
    AmbientOcclusion,
    Tonemap,
    AntiAliasing,
}

/// A flat view: one that shows its values as they are, unlit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlatView {
    Buffers,
    SkinWeights,
    /// One of the two density maps.
    Density(DiagnosticView),
}

/// What is overriding a control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Override {
    /// An overdraw view, which draws its count in place of the scene.
    Replaced(DiagnosticView),
    /// Wireframe Only shading, which draws no surface.
    NoSurface,
    /// Wireframe Only shading already draws every edge.
    EdgesDrawn,
    /// A flat view.
    Flat(FlatView),
    /// Unlit shading.
    Unlit,
    /// The environment, drawn as the background.
    Skybox,
    /// IBL is off, so nothing is drawn from the environment.
    IblOff,
}

impl Override {
    /// What a tooltip says about it.
    pub fn reason(self) -> String {
        use review_localization::tr;
        match self {
            Override::Replaced(view) => keys::ui_toolbar::overridden_replaced(
                tr(crate::audit_labels::view(view).0).into_owned(),
            ),
            Override::NoSurface => tr(keys::ui_toolbar::OVERRIDDEN_NO_SURFACE).into_owned(),
            Override::EdgesDrawn => tr(keys::ui_toolbar::OVERRIDDEN_EDGES_DRAWN).into_owned(),
            Override::Flat(flat) => {
                let name = match flat {
                    FlatView::Buffers => keys::ui_toolbar::BUFFERS,
                    FlatView::SkinWeights => keys::ui_toolbar::SKIN_WEIGHTS,
                    FlatView::Density(view) => crate::audit_labels::view(view).0,
                };
                keys::ui_toolbar::overridden_flat(tr(name).into_owned())
            }
            Override::Unlit => tr(keys::ui_toolbar::OVERRIDDEN_UNLIT).into_owned(),
            Override::Skybox => tr(keys::ui_toolbar::OVERRIDDEN_SKYBOX).into_owned(),
            Override::IblOff => tr(keys::ui_toolbar::OVERRIDDEN_IBL_OFF).into_owned(),
        }
    }
}

impl WorkspaceMode {
    /// Whether this workspace shows the diagnostic views on its toolbar.
    pub fn shows_diagnostic_views(self) -> bool {
        matches!(self, WorkspaceMode::Aud | WorkspaceMode::Opt)
    }
}

impl UiState {
    /// The diagnostic view chosen for this workspace: Aud's, or Opt's — which has
    /// no Issues view, findings being Aud's — and none anywhere else. `None` is
    /// the material tiles' turn to paint the model.
    pub fn diagnostic_view(&self) -> Option<DiagnosticView> {
        match self.mode {
            WorkspaceMode::Aud => self.aud.view,
            WorkspaceMode::Opt => self.aud.view.filter(|view| *view != DiagnosticView::Issues),
            _ => None,
        }
    }

    /// The diagnostic view actually on screen: [`Self::diagnostic_view`], less an
    /// overdraw view the adapter cannot draw (invariant 4) and a density map while
    /// Wireframe Only shading leaves it no surface to colour. What `app` hands the
    /// renderer and what the legend describes.
    pub fn shown_diagnostic(&self) -> Option<DiagnosticView> {
        let view = self.diagnostic_view()?;
        if !self.aud.view_supported(view, &self.capabilities) {
            return None;
        }
        let paints_surface = matches!(
            view,
            DiagnosticView::TexelDensity | DiagnosticView::TriangleDensity
        );
        if paints_surface && self.shading_mode == ShadingMode::Wireframe {
            return None;
        }
        Some(view)
    }

    /// Whether `material`'s tile is the one painting the model: it is the
    /// active material and no diagnostic view has taken the surface over.
    pub fn shows_material(&self, material: ActiveMaterial) -> bool {
        self.debug.active_material == material && self.shown_diagnostic().is_none()
    }

    /// Whether an overdraw count is drawn in place of the scene — the mesh, the
    /// overlays and, in Opt's overlay, the ghost.
    pub fn replaces_scene(&self) -> bool {
        matches!(
            self.shown_diagnostic(),
            Some(DiagnosticView::Overdraw | DiagnosticView::QuadOverdraw)
        )
    }

    /// The flat view on screen, if any (`review_render`'s `flat_display`). None in
    /// Wireframe Only: there is no surface for one to show.
    pub fn flat_view(&self) -> Option<FlatView> {
        if self.shading_mode == ShadingMode::Wireframe {
            return None;
        }
        match self.shown_diagnostic() {
            Some(view @ (DiagnosticView::TexelDensity | DiagnosticView::TriangleDensity)) => {
                Some(FlatView::Density(view))
            }
            Some(_) => None,
            None => match self.debug.active_material {
                ActiveMaterial::Buffers => Some(FlatView::Buffers),
                ActiveMaterial::SkinWeights => Some(FlatView::SkinWeights),
                _ => None,
            },
        }
    }

    /// What overrides `control` right now, or `None` while it has its effect.
    /// The first rule that applies wins, in the order the renderer settles them.
    pub fn overridden(&self, control: ViewControl) -> Option<Override> {
        use ViewControl::*;

        // An overdraw view counts into its own target and paints the count over
        // the whole view (`SceneGpu::record_overdraw` returns before the scene
        // pass). It culls by the backface setting, and the surface paints are its
        // peers — picking one is how it is left — so those two stay live.
        if self.replaces_scene() {
            let view = self.shown_diagnostic()?;
            return match control {
                Backfaces | SurfacePaint => None,
                _ => Some(Override::Replaced(view)),
            };
        }

        // The rules below that leave nothing lit override IBL only while the
        // environment is not asked for as the background: IBL is also what lets
        // that be drawn (`draws_skybox`), so the tile still has an effect.
        let lighting = |control: ViewControl| {
            control == AmbientOcclusion || (control == Ibl && !self.environment.show_background)
        };

        // Wireframe Only draws no surface (`mesh_draw_list`), so nothing paints
        // it, nothing lights it and nothing occludes it, and the edges are drawn
        // whatever the overlay toggle says (`wireframe_on`). The shading tiles
        // are how it is left, and no rule below touches them here — there is no
        // flat view without a surface ([`Self::flat_view`]).
        if self.shading_mode == ShadingMode::Wireframe {
            match control {
                SurfacePaint | Backfaces => return Some(Override::NoSurface),
                _ if lighting(control) => return Some(Override::NoSurface),
                WireframeOverlay => return Some(Override::EdgesDrawn),
                _ => {}
            }
        }

        // The flat views emit display pixels from the scene shader and the
        // composite passes them straight through (`flat_display`): no lighting,
        // no tone map, no occlusion — and so no difference between Unlit and
        // Shaded.
        if let Some(flat) = self.flat_view()
            && (matches!(control, UnlitShading | ShadedShading | Tonemap) || lighting(control))
        {
            return Some(Override::Flat(flat));
        }

        // Unlit returns the base colour before any light is applied, and writes
        // no ambient for the occlusion to darken (`fs_main`).
        if self.shading_mode == ShadingMode::Unlit && lighting(control) {
            return Some(Override::Unlit);
        }

        // The skybox is drawn only with IBL on (`draws_skybox`), and then writes
        // full coverage, so the background colour shows nowhere
        // (`record_scene_pass`, the composite's coverage blend).
        match control {
            EnvironmentBackground if !self.environment.ibl_enabled => Some(Override::IblOff),
            Background if self.environment.draws_skybox() => Some(Override::Skybox),
            _ => None,
        }
    }

    /// [`Self::overridden`]'s reason, for a tile's tooltip.
    pub(crate) fn override_reason(&self, control: ViewControl) -> Option<String> {
        self.overridden(control).map(Override::reason)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHADING: [ShadingMode; 3] = [
        ShadingMode::Wireframe,
        ShadingMode::Unlit,
        ShadingMode::Shaded,
    ];
    const MATERIALS: [ActiveMaterial; 5] = [
        ActiveMaterial::Source,
        ActiveMaterial::UvChecker,
        ActiveMaterial::VertexColors,
        ActiveMaterial::Buffers,
        ActiveMaterial::SkinWeights,
    ];
    const VIEWS: [Option<DiagnosticView>; 6] = [
        None,
        Some(DiagnosticView::Issues),
        Some(DiagnosticView::TexelDensity),
        Some(DiagnosticView::TriangleDensity),
        Some(DiagnosticView::Overdraw),
        Some(DiagnosticView::QuadOverdraw),
    ];
    const WORKSPACES: [WorkspaceMode; 3] = [
        WorkspaceMode::ThreeD,
        WorkspaceMode::Opt,
        WorkspaceMode::Aud,
    ];

    /// Every combination of the settings the rules read.
    fn every_state() -> Vec<UiState> {
        let mut states = Vec::new();
        for mode in WORKSPACES {
            for shading in SHADING {
                for material in MATERIALS {
                    for view in VIEWS {
                        for (ibl, skybox) in
                            [(false, false), (false, true), (true, false), (true, true)]
                        {
                            let mut state = UiState {
                                mode,
                                shading_mode: shading,
                                ..UiState::default()
                            };
                            state.capabilities.overdraw = true;
                            state.debug.active_material = material;
                            state.aud.view = view;
                            state.environment.ibl_enabled = ibl;
                            state.environment.show_background = skybox;
                            states.push(state);
                        }
                    }
                }
            }
        }
        states
    }

    fn describe(state: &UiState) -> String {
        format!(
            "{:?} / {:?} / {:?} / {:?} / ibl {} / skybox {}",
            state.mode,
            state.shading_mode,
            state.debug.active_material,
            state.aud.view,
            state.environment.ibl_enabled,
            state.environment.show_background
        )
    }

    /// At most one of the material tiles and the diagnostic views is ever lit,
    /// and in the workspaces that show both, one always is.
    #[test]
    fn one_tile_paints_the_model() {
        for state in every_state() {
            let materials = MATERIALS
                .iter()
                .filter(|&&material| state.shows_material(material))
                .count();
            let views = usize::from(state.shown_diagnostic().is_some());
            assert_eq!(
                materials + views,
                1,
                "{}: {materials} material tiles and {views} views lit",
                describe(&state)
            );
        }
    }

    /// An override never locks the user in: the controls that leave each
    /// overriding mode are never overridden themselves.
    #[test]
    fn every_override_can_be_left() {
        for state in every_state() {
            if state.replaces_scene() {
                // Left by picking a surface paint.
                assert_eq!(state.overridden(ViewControl::SurfacePaint), None);
            }
            if state.shading_mode == ShadingMode::Wireframe && !state.replaces_scene() {
                // Left by picking another shading mode.
                for control in [ViewControl::UnlitShading, ViewControl::ShadedShading] {
                    assert_eq!(state.overridden(control), None, "{}", describe(&state));
                }
            }
            if state.flat_view().is_some() && !state.replaces_scene() {
                // Left by another material, or by Wireframe Only.
                assert_eq!(state.overridden(ViewControl::SurfacePaint), None);
                assert_eq!(state.overridden(ViewControl::WireframeShading), None);
            }
            // Wireframe Only is the one shading tile nothing short of an
            // overdraw view can grey out.
            if !state.replaces_scene() {
                assert_eq!(state.overridden(ViewControl::WireframeShading), None);
            }
        }
    }

    #[test]
    fn an_overdraw_view_overrides_everything_it_draws_over() {
        let mut state = UiState {
            mode: WorkspaceMode::Opt,
            ..UiState::default()
        };
        state.capabilities.overdraw = true;
        state.aud.view = Some(DiagnosticView::Overdraw);
        for control in [
            ViewControl::WireframeOverlay,
            ViewControl::ShadedShading,
            ViewControl::FaceNormals,
            ViewControl::BoundingBox,
            ViewControl::Grid,
            ViewControl::Background,
            ViewControl::Ibl,
            ViewControl::Tonemap,
            ViewControl::AntiAliasing,
        ] {
            assert_eq!(
                state.overridden(control),
                Some(Override::Replaced(DiagnosticView::Overdraw)),
                "{control:?}"
            );
        }
        assert_eq!(state.overridden(ViewControl::Backfaces), None);

        // Without the capability the view is not drawn, so it overrides nothing.
        state.capabilities.overdraw = false;
        assert!(!state.replaces_scene());
        assert_eq!(state.overridden(ViewControl::Grid), None);
        assert!(state.shows_material(state.debug.active_material));
    }

    #[test]
    fn the_flat_views_override_lighting_and_tone_mapping() {
        let mut state = UiState::default();
        state.debug.active_material = ActiveMaterial::Buffers;
        for control in [
            ViewControl::UnlitShading,
            ViewControl::ShadedShading,
            ViewControl::Ibl,
            ViewControl::AmbientOcclusion,
            ViewControl::Tonemap,
        ] {
            assert_eq!(
                state.overridden(control),
                Some(Override::Flat(FlatView::Buffers)),
                "{control:?}"
            );
        }
        assert_eq!(state.overridden(ViewControl::Background), None);
        assert_eq!(state.overridden(ViewControl::AntiAliasing), None);

        // A density map is one too, and only where it is shown.
        state.debug.active_material = ActiveMaterial::Source;
        state.aud.view = Some(DiagnosticView::TexelDensity);
        assert_eq!(state.overridden(ViewControl::Tonemap), None, "not in 3D");
        state.mode = WorkspaceMode::Aud;
        assert_eq!(
            state.overridden(ViewControl::Tonemap),
            Some(Override::Flat(FlatView::Density(
                DiagnosticView::TexelDensity
            )))
        );
    }

    #[test]
    fn wireframe_only_leaves_nothing_to_paint_or_light() {
        let state = UiState {
            shading_mode: ShadingMode::Wireframe,
            ..UiState::default()
        };
        for control in [
            ViewControl::SurfacePaint,
            ViewControl::Backfaces,
            ViewControl::Ibl,
            ViewControl::AmbientOcclusion,
        ] {
            assert_eq!(
                state.overridden(control),
                Some(Override::NoSurface),
                "{control:?}"
            );
        }
        assert_eq!(
            state.overridden(ViewControl::WireframeOverlay),
            Some(Override::EdgesDrawn)
        );
        assert_eq!(state.overridden(ViewControl::Tonemap), None);
    }

    #[test]
    fn unlit_overrides_only_the_lighting() {
        let state = UiState {
            shading_mode: ShadingMode::Unlit,
            ..UiState::default()
        };
        assert_eq!(state.overridden(ViewControl::Ibl), Some(Override::Unlit));
        assert_eq!(
            state.overridden(ViewControl::AmbientOcclusion),
            Some(Override::Unlit)
        );
        assert_eq!(state.overridden(ViewControl::Tonemap), None);
        assert_eq!(state.overridden(ViewControl::SurfacePaint), None);
    }

    /// The environment is drawn behind the model only with IBL on, and only then
    /// hides the background colour.
    #[test]
    fn the_environment_background_needs_ibl() {
        let mut state = UiState::default();
        state.environment.show_background = true;
        state.environment.ibl_enabled = false;
        assert_eq!(
            state.overridden(ViewControl::EnvironmentBackground),
            Some(Override::IblOff)
        );
        assert_eq!(state.overridden(ViewControl::Background), None);
        assert_eq!(state.overridden(ViewControl::Ibl), None);

        state.environment.ibl_enabled = true;
        assert_eq!(state.overridden(ViewControl::EnvironmentBackground), None);
        assert_eq!(
            state.overridden(ViewControl::Background),
            Some(Override::Skybox)
        );

        // IBL still decides whether the environment is drawn behind the model,
        // so the surface being unlit does not grey its tile out.
        state.shading_mode = ShadingMode::Unlit;
        assert_eq!(state.overridden(ViewControl::Ibl), None);
        assert_eq!(
            state.overridden(ViewControl::AmbientOcclusion),
            Some(Override::Unlit)
        );
        state.environment.show_background = false;
        assert_eq!(state.overridden(ViewControl::Ibl), Some(Override::Unlit));
    }

    #[test]
    fn opt_has_no_issues_view() {
        let mut state = UiState {
            mode: WorkspaceMode::Opt,
            ..UiState::default()
        };
        state.aud.view = Some(DiagnosticView::Issues);
        assert_eq!(state.diagnostic_view(), None);
        assert!(state.shows_material(ActiveMaterial::Source));
        state.mode = WorkspaceMode::Aud;
        assert_eq!(state.diagnostic_view(), Some(DiagnosticView::Issues));
        assert!(!state.shows_material(ActiveMaterial::Source));
    }

    /// Every reason formats: a missing catalog variable shows as an error marker
    /// rather than failing, so this is what catches one.
    #[test]
    fn every_reason_reads() {
        for reason in [
            Override::Replaced(DiagnosticView::QuadOverdraw),
            Override::NoSurface,
            Override::EdgesDrawn,
            Override::Flat(FlatView::Buffers),
            Override::Flat(FlatView::SkinWeights),
            Override::Flat(FlatView::Density(DiagnosticView::TriangleDensity)),
            Override::Unlit,
            Override::Skybox,
            Override::IblOff,
        ] {
            let text = reason.reason();
            assert!(
                !text.is_empty() && !text.contains('{'),
                "{reason:?}: {text}"
            );
        }
    }
}
