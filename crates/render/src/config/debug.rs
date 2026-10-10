//! The per-view debug toggles the renderer reads each frame.

use crate::selection::Selection;

use super::*;

/// Which geometry the bounding box (and its dimension labels) wraps: the whole
/// model, only the currently-selected mesh part / material, or only the
/// Outliner-visible meshes. Baked into the box line buffer so it rebuilds when
/// the scope — or the inputs the scope depends on — change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoundingBoxScope {
    /// Wrap every mesh, regardless of selection or Outliner visibility.
    #[default]
    AllMeshes,
    /// Wrap only the geometry the current Outliner selection covers (a node's
    /// subtree or a material slot); empty when nothing is selected.
    OnlySelection,
    /// Wrap only the currently-visible meshes (Outliner-hidden meshes excluded).
    VisibleOnly,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneDebugOptions {
    pub shading_mode: ShadingMode,
    /// Draw the wireframe edges on top of the filled surface. Independent of
    /// `shading_mode` (it is an overlay), so it combines with the unlit and
    /// shaded modes; it is also implied when `shading_mode` is
    /// [`ShadingMode::Wireframe`] (which draws the edges as the only geometry).
    pub wireframe_overlay: bool,
    /// Which material the filled faces show (source / UV checker / vertex colors /
    /// buffer view). Mutually exclusive; applies in every filled-face mode.
    pub active_material: ActiveMaterial,
    /// Which single buffer the filled faces show when `active_material` is
    /// [`ActiveMaterial::Buffers`] (ignored otherwise).
    pub buffer_view: BufferView,
    /// How the source material is shaded (imported / uniform standard / unique
    /// per-part hue). Behind the Source Material button's options panel; replaces
    /// the effective material table renderer-side, leaving the imported materials
    /// untouched.
    pub material_mode: MaterialMode,
    pub uv_checker_texture: CheckerTexture,
    /// Which vertex-color channels the view shows when `active_material` is
    /// [`ActiveMaterial::VertexColors`].
    pub vertex_color_mode: VertexColorMode,
    /// Checker repeats across the 0..1 UV range; clamped to 1..=16 by the UI.
    pub uv_checker_tiling: u32,
    /// Which model UV set the checker view samples (0-based). Only meaningful
    /// when the model carries more than one UV set.
    pub uv_channel: u32,
    pub show_grid: bool,
    /// Whether the model's axis-aligned bounding box is drawn as a wireframe box.
    pub show_bounding_box: bool,
    /// Whether the object pivot marker — a 3-axis cross at the model's origin (the
    /// root node's world-space pivot) — is drawn. A plain on/off overlay (no
    /// options), independent of the bounding box.
    pub show_pivot: bool,
    /// Whether the skeleton overlay is drawn: one octahedral bone per parent ->
    /// child joint pair, plus a marker at each leaf / root joint. Always drawn
    /// on top of the mesh (X-ray) — a skeleton lives *inside* its character, so
    /// depth-testing it would hide the whole thing.
    pub show_skeleton: bool,
    /// Multiplier on the skeleton overlay's computed bone thickness / marker size,
    /// so a dense rig can be thinned out and a sparse one fattened up.
    pub skeleton_joint_scale: f32,
    /// Skeleton bone color (gamma space, alpha applies to the solid octahedron
    /// fill; the outlines draw opaque).
    pub skeleton_color: [f32; 4],
    /// Color the hovered object is tinted while the Select tool is active — the
    /// preview of what a click would pick. A translucent fill in its own colour,
    /// distinct from the selection's, so "would be selected" never reads as
    /// "is selected". Alpha is the fill opacity, as the selection highlight's is.
    pub hover_color: [f32; 4],
    /// The same, for a hovered *bone*: with the skeleton overlay up the pick
    /// targets bones, and the tint is baked into the overlay's vertices rather
    /// than applied from a uniform, so it needs its own entry here.
    pub skeleton_hover_color: [f32; 4],
    /// Color for the bones in [`crate::SceneFrame::selected_bones`], so an Outliner
    /// selection reads in the viewport. Like the mesh selection highlight, this is
    /// persistent — it lasts as long as the selection does.
    pub skeleton_selected_color: [f32; 4],
    pub face_normals: bool,
    pub vertex_normals: bool,
    pub face_normal_length: f32,
    pub vertex_normal_length: f32,
    pub face_normal_color: [f32; 4],
    pub vertex_normal_color: [f32; 4],
    /// Whether the UV-seam overlay is drawn: the mesh edges across which
    /// [`uv_seam_channel`] is discontinuous, highlighted in [`uv_seam_color`] —
    /// the read Maya gives with "texture border edges". An ordinary depth-tested
    /// line view, so it composes with any shading mode.
    ///
    /// [`uv_seam_channel`]: SceneDebugOptions::uv_seam_channel
    /// [`uv_seam_color`]: SceneDebugOptions::uv_seam_color
    pub uv_seams: bool,
    /// Color of the UV-seam edges, baked into that view's line buffer and
    /// rebuilt when it changes. A seam is drawn over the wireframe on the same
    /// edge, so this is most of what separates the two.
    pub uv_seam_color: [f32; 4],
    /// Which model UV set the seam test reads (0-based). Deliberately its own
    /// channel rather than [`uv_channel`]: the seams a lightmap set carries are
    /// a different question from which set the checker is showing, and the two
    /// are useful side by side.
    ///
    /// [`uv_channel`]: SceneDebugOptions::uv_channel
    pub uv_seam_channel: u32,
    /// Color of the model wireframe overlay. Applied as a uniform at draw time, so
    /// changing it rebuilds nothing.
    pub wireframe_color: [f32; 4],
    /// Width of the model wireframe's lines in **points**, not pixels: the renderer
    /// multiplies it by [`crate::SceneFrame::pixels_per_point`], so a line is the
    /// same physical width on a Retina screen as on a 100% one. The wireframe is
    /// drawn as screen-space quads rather than hardware lines precisely so that this
    /// can hold — a hardware line is one *device* pixel wide whatever the display.
    pub wireframe_width: f32,
    /// Color of the bounding-box edges, baked into its line buffer and rebuilt
    /// when it changes.
    pub bounding_box_color: [f32; 4],
    /// Which geometry the bounding box wraps (whole model / only the selection /
    /// only the visible meshes). Baked into the box line buffer alongside the
    /// inputs the chosen scope depends on, so it rebuilds when they change.
    pub bounding_box_scope: BoundingBoxScope,
    /// The Outliner selection the box wraps in [`BoundingBoxScope::OnlySelection`]
    /// mode (ignored otherwise). Carried here so the box can be baked from the
    /// scene render without threading the selection through separately.
    pub bounding_box_selection: Selection,
    /// When `true` back-facing triangles are drawn (the mesh is double-sided);
    /// when `false` (the default) they are culled, so only camera-facing surfaces
    /// are rendered. The renderer keeps two mesh pipelines — culling vs.
    /// double-sided — and picks one per frame from this flag, so toggling it
    /// allocates nothing.
    pub render_backfaces: bool,
    /// A diagnostic colour map drawn in place of the material — the Aud
    /// workspace's density views. `None` everywhere else.
    pub heat_map: HeatMap,
    /// The heat map's three ramp stops (gamma rgb): below the target, on it,
    /// above it. Owned by the chrome's theme.
    pub heat_ramp: [[f32; 3]; 3],
}

/// A per-face diagnostic colour map: each face coloured by how far a measured
/// figure sits from its target, on a three-stop ramp (low, on target, high).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum HeatMap {
    #[default]
    None,
    /// Texture pixels per meter for a `texture_size`-pixel texture, against
    /// `target`; the ramp's ends are `target` divided and multiplied by
    /// `tolerance`.
    TexelDensity {
        texture_size: f32,
        target: f32,
        tolerance: f32,
    },
    /// The pixels a face's triangles cover with their object filling a
    /// `screen_height`-pixel screen, against `min_pixel_area`: dense faces read
    /// as the ramp's high end.
    TriangleDensity {
        min_pixel_area: f32,
        screen_height: f32,
    },
}

impl HeatMap {
    pub fn is_active(self) -> bool {
        self != HeatMap::None
    }
}

impl Default for SceneDebugOptions {
    fn default() -> Self {
        Self {
            shading_mode: ShadingMode::Shaded,
            wireframe_overlay: false,
            active_material: ActiveMaterial::Source,
            buffer_view: BufferView::default(),
            material_mode: MaterialMode::Source,
            uv_checker_texture: CheckerTexture::Greyscale,
            vertex_color_mode: VertexColorMode::Rgb,
            uv_checker_tiling: 4,
            uv_channel: 0,
            show_grid: true,
            show_bounding_box: false,
            show_pivot: false,
            show_skeleton: false,
            skeleton_joint_scale: 1.0,
            skeleton_color: [0.35, 0.72, 1.0, 1.0],
            skeleton_selected_color: [1.0, 0.55, 0.14, 1.0],
            // Overwritten from the theme every frame; these keep a renderer
            // built without a UI (the tests) drawing something sane.
            hover_color: [1.0, 0.77, 0.47, 0.18],
            skeleton_hover_color: [1.0, 0.77, 0.47, 1.0],
            face_normals: false,
            vertex_normals: false,
            face_normal_length: 0.03,
            vertex_normal_length: 0.03,
            face_normal_color: [1.0, 0.1, 0.1, 0.95],
            vertex_normal_color: [0.14, 0.92, 0.96, 0.95],
            uv_seams: false,
            uv_seam_color: [0.11, 1.0, 0.11, 1.0],
            uv_seam_channel: 0,
            wireframe_color: [0.6, 0.6, 0.6, 1.0],
            wireframe_width: 1.0,
            bounding_box_color: [1.0, 0.803_921_6, 0.250_980_4, 1.0],
            bounding_box_scope: BoundingBoxScope::default(),
            bounding_box_selection: Selection::None,
            render_backfaces: false,
            heat_map: HeatMap::None,
            heat_ramp: [[0.25, 0.5, 1.0], [0.3, 0.85, 0.35], [1.0, 0.3, 0.25]],
        }
    }
}

/// The viewport background fill drawn behind the 3D / UV scene. A preset that the
/// composite pass paints (in display space, after tone mapping) wherever no
/// geometry covers the pixel — so the chosen value is the exact displayed color,
/// undistorted by the tone curve. The skybox (Environment → "show background")
/// covers the whole viewport when on, so the IBL environment overrides this.
///
/// Flat presets paint one solid color; [`Gradient`] paints a vertical interpolation
/// (top → bottom). The values are sRGB display levels (0..1), matching their
/// labels (e.g. 50% grey reads as a mid-grey on screen).
///
/// [`Gradient`]: ViewportBackground::Gradient
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ViewportBackground {
    /// Solid black — the default neutral backdrop.
    #[default]
    Black,
    /// Solid 25% grey.
    Grey25,
    /// Solid 50% grey.
    Grey50,
    /// Solid 75% grey.
    Grey75,
    /// Solid white.
    White,
    /// Vertical gradient: 80% grey at the top fading to black at the bottom.
    Gradient,
}

impl ViewportBackground {
    /// Every preset in display / cycle order — Black (the default) first, then the
    /// greys ascending to White, then the gradient. Drives both the status-bar
    /// cycle button and the options-panel swatch row (left to right).
    pub const ALL: [ViewportBackground; 6] = [
        ViewportBackground::Black,
        ViewportBackground::Grey25,
        ViewportBackground::Grey50,
        ViewportBackground::Grey75,
        ViewportBackground::White,
        ViewportBackground::Gradient,
    ];

    /// Human-readable label (tooltips / notifications).
    pub fn label(self) -> &'static str {
        match self {
            ViewportBackground::Black => "Black",
            ViewportBackground::Grey25 => "25% Grey",
            ViewportBackground::Grey50 => "50% Grey",
            ViewportBackground::Grey75 => "75% Grey",
            ViewportBackground::White => "White",
            ViewportBackground::Gradient => "Gradient",
        }
    }

    /// The next preset in [`ViewportBackground::ALL`] order, wrapping after the
    /// last — for cycling by left-clicking the status-bar background button.
    pub fn next(self) -> ViewportBackground {
        let all = ViewportBackground::ALL;
        let index = all.iter().position(|&v| v == self).unwrap_or(0);
        all[(index + 1) % all.len()]
    }

    /// The preset's top and bottom fill colors in **display (sRGB) space**, 0..1.
    /// The composite interpolates vertically between them (top at the viewport's
    /// top edge); a flat preset returns the same color for both. Handed to the post
    /// pass directly so the painted background matches the label exactly, untouched
    /// by tone mapping.
    pub fn gradient_srgb(self) -> ([f32; 3], [f32; 3]) {
        let grey = |v: f32| [v, v, v];
        match self {
            ViewportBackground::Black => (grey(0.0), grey(0.0)),
            ViewportBackground::Grey25 => (grey(0.25), grey(0.25)),
            ViewportBackground::Grey50 => (grey(0.5), grey(0.5)),
            ViewportBackground::Grey75 => (grey(0.75), grey(0.75)),
            ViewportBackground::White => (grey(1.0), grey(1.0)),
            ViewportBackground::Gradient => (grey(0.8), grey(0.0)),
        }
    }
}
