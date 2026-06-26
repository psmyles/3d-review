//! Renderer configuration + per-view option types: shading / projection /
//! anti-aliasing / environment / GTAO / tone-map / UV / debug-overlay
//! settings, the adapter-capability probe, and `RendererConfig`. These form the
//! UI→render and model→render *option contract* (a clean seam for the future
//! renderer swap); `Renderer` and the cameras live in the crate root and read
//! these as plain values.

use crate::scene::SCENE_DEPTH_FORMAT;
use crate::selection::Selection;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ShadingMode {
    /// Wireframe only: the filled surface is not drawn, just its edges.
    Wireframe,
    /// Filled faces showing the active material as flat emissive color (no light).
    Unlit,
    /// Filled faces lit and shaded with the active material color.
    #[default]
    Shaded,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CameraProjection {
    #[default]
    Perspective,
    Orthographic,
}

/// Multisample level for the offscreen scene render (the geometry MSAA). Distinct
/// from egui's own framebuffer MSAA ([`EGUI_MSAA_SAMPLE_COUNT`], fixed): this is
/// the per-edge antialiasing of the 3D scene, chosen at runtime. `Off` renders
/// single-sample (no resolve); the rest render multisampled and resolve to a
/// single-sample texture the composite pass samples.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MsaaSamples {
    Off,
    X2,
    #[default]
    X4,
    X8,
    X16,
}

impl MsaaSamples {
    /// Every variant in ascending order, for building UI menus.
    pub const ALL: [MsaaSamples; 5] = [
        MsaaSamples::Off,
        MsaaSamples::X2,
        MsaaSamples::X4,
        MsaaSamples::X8,
        MsaaSamples::X16,
    ];

    /// The wgpu sample count this level maps to (`Off` = 1).
    pub fn sample_count(self) -> u32 {
        match self {
            MsaaSamples::Off => 1,
            MsaaSamples::X2 => 2,
            MsaaSamples::X4 => 4,
            MsaaSamples::X8 => 8,
            MsaaSamples::X16 => 16,
        }
    }
}

/// The viewer's antialiasing configuration: a master on/off plus the MSAA level.
/// Read by [`SceneCallback`] to size the offscreen targets / scene pipelines.
///
/// `enabled` is the toolbar toggle (left-click): when off, the scene renders with
/// no antialiasing at all regardless of `msaa`, but the level is retained so
/// toggling back on restores it. The default is enabled at 4× MSAA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AntiAliasing {
    pub enabled: bool,
    pub msaa: MsaaSamples,
}

impl Default for AntiAliasing {
    fn default() -> Self {
        Self {
            enabled: true,
            msaa: MsaaSamples::X4,
        }
    }
}

impl AntiAliasing {
    /// The MSAA sample count to actually render at: the chosen level when AA is
    /// enabled, otherwise 1 (single-sample, no resolve).
    pub fn effective_sample_count(self) -> u32 {
        if self.enabled {
            self.msaa.sample_count()
        } else {
            1
        }
    }
}

/// The MSAA levels the active adapter can actually render the scene at, in
/// ascending order. Queried against both the HDR color and depth target formats
/// so a level is only offered when both support it (invariant 4: capability-gate,
/// never crash). `Off` (single-sample) is always included. The UI uses this to
/// drop unsupported entries from the antialiasing menu.
///
/// `Adapter::get_texture_format_features` reports the adapter's full
/// (adapter-specific) sample-count support regardless of which device features
/// are enabled. We only get those extra counts on the *device* when
/// `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES` is enabled (the app enables it iff
/// the adapter offers it). So unless that feature is present we clamp to the
/// WebGPU-guaranteed counts (1 and 4) for these render formats — otherwise we'd
/// offer a level whose pipeline build the device would reject.
pub fn supported_msaa_levels(adapter: &wgpu::Adapter) -> Vec<MsaaSamples> {
    let adapter_specific = adapter
        .features()
        .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES);
    let color = adapter
        .get_texture_format_features(crate::targets::SCENE_HDR_FORMAT)
        .flags;
    let depth = adapter
        .get_texture_format_features(SCENE_DEPTH_FORMAT)
        .flags;
    MsaaSamples::ALL
        .into_iter()
        .filter(|level| {
            let count = level.sample_count();
            // Single-sample and the WebGPU-guaranteed 4× are always safe; any
            // other count requires the adapter to both report it and have the
            // adapter-specific feature enabled on the device.
            count == 1
                || count == 4
                || (adapter_specific
                    && color.sample_count_supported(count)
                    && depth.sample_count_supported(count))
        })
        .collect()
}

/// Which built-in HDR environment lights the scene (image-based lighting) and,
/// optionally, is shown as the background. The maps are baked into the binary
/// from `assets/textures` (invariant: assets via `include_bytes!`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EnvironmentMap {
    #[default]
    Hdr01,
    Hdr02,
    Hdr03,
    Hdr04,
    Hdr05,
    Hdr06,
}

impl EnvironmentMap {
    /// Every variant in display order, for building UI menus.
    pub const ALL: [EnvironmentMap; 6] = [
        EnvironmentMap::Hdr01,
        EnvironmentMap::Hdr02,
        EnvironmentMap::Hdr03,
        EnvironmentMap::Hdr04,
        EnvironmentMap::Hdr05,
        EnvironmentMap::Hdr06,
    ];

    /// Short menu label.
    pub fn label(self) -> &'static str {
        match self {
            EnvironmentMap::Hdr01 => "HDR 01",
            EnvironmentMap::Hdr02 => "HDR 02",
            EnvironmentMap::Hdr03 => "HDR 03",
            EnvironmentMap::Hdr04 => "HDR 04",
            EnvironmentMap::Hdr05 => "HDR 05",
            EnvironmentMap::Hdr06 => "HDR 06",
        }
    }
}

/// Image-based lighting / environment configuration for the shaded view. Read by
/// [`SceneCallback`] to choose + precompute the IBL maps and drive the PBR shaded
/// path and optional skybox.
///
/// `ibl_enabled` is the default lighting for Shaded mode: on (the env lights the
/// surface via diffuse irradiance + specular reflection). When off, the shaded
/// path falls back to the analytic neutral-hemisphere lighting. `show_background`
/// draws the chosen map as a skybox behind the model (off by default — the
/// neutral background is kept so the model stands out, while the surface still
/// reflects the environment). `intensity` scales the IBL contribution.
/// `rotation_degrees` spins the environment about the world Y axis (0..360, 0 =
/// as-authored); applied live at sample time in the scene shader, so it never
/// rebuilds the precomputed IBL maps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnvironmentSettings {
    pub ibl_enabled: bool,
    pub show_background: bool,
    pub map: EnvironmentMap,
    pub intensity: f32,
    /// Yaw applied to the environment about the world Y axis, in degrees (0..360).
    pub rotation_degrees: f32,
}

impl Default for EnvironmentSettings {
    fn default() -> Self {
        Self {
            ibl_enabled: true,
            show_background: false,
            map: EnvironmentMap::default(),
            intensity: 1.0,
            rotation_degrees: 0.0,
        }
    }
}

/// Ground-Truth Ambient Occlusion configuration for the shaded view (CLAUDE.md
/// render roadmap, Phase 5). Read by [`SceneCallback`] to drive the GTAO + blur
/// passes and the composite multiply. (User-facing UI calls this "Ambient
/// Occlusion"; the internal implementation is GTAO.)
///
/// GTAO samples a single-sample view-space normal + depth G-buffer, marches the
/// screen-space horizon per slice to estimate the cosine-weighted visible arc,
/// edge-aware blurs the result, and applies it only to the scene's ambient
/// radiance in the composite — darkening contact creases and cavities without
/// muting direct/specular light. `radius` is expressed as a **fraction of the
/// framed model's bounding-sphere radius**, so the look is scale-invariant across
/// models (the renderer multiplies it by the live scene radius). `enabled` is the
/// toolbar toggle; the default is on but subtle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GtaoSettings {
    pub enabled: bool,
    /// Sample radius, as a fraction of the scene bounding-sphere radius.
    pub radius: f32,
    /// Strength of the darkening: a power on the GTAO visibility (1 = ground
    /// truth, >1 darkens, 0 disables).
    pub intensity: f32,
    /// Thickness heuristic (0..1): how much an occluder past the near horizon is
    /// "seen through", keeping thin geometry from over-occluding.
    pub thickness: f32,
    /// Sampling quality — the slice / step counts of the horizon search.
    pub quality: GtaoQuality,
}

impl Default for GtaoSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            radius: 0.35,
            intensity: 1.0,
            thickness: 0.25,
            quality: GtaoQuality::Medium,
        }
    }
}

/// GTAO sampling quality: the number of slice directions and horizon steps per
/// slice. More of each means smoother, more accurate occlusion at higher cost.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GtaoQuality {
    /// 2 slices × 3 steps — cheapest, noisier (the blur cleans most of it up).
    Low,
    /// 3 slices × 5 steps — the balanced default.
    #[default]
    Medium,
    /// 4 slices × 8 steps — smoothest, most expensive.
    High,
}

impl GtaoQuality {
    /// Every variant in display order, for building UI menus.
    pub const ALL: [GtaoQuality; 3] = [GtaoQuality::Low, GtaoQuality::Medium, GtaoQuality::High];

    /// Short menu label.
    pub fn label(self) -> &'static str {
        match self {
            GtaoQuality::Low => "Low",
            GtaoQuality::Medium => "Medium",
            GtaoQuality::High => "High",
        }
    }

    /// `(slices, steps_per_slice)` fed into the GTAO horizon search.
    pub fn slices_steps(self) -> (u32, u32) {
        match self {
            GtaoQuality::Low => (2, 3),
            GtaoQuality::Medium => (3, 5),
            GtaoQuality::High => (4, 8),
        }
    }
}

/// The tone-mapping operator applied in the composite pass when tone mapping is
/// enabled, converting the scene's linear HDR radiance to a display range before
/// sRGB encoding. `Linear` is a pass-through (the master toggle off is equivalent
/// to `Linear`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TonemapOperator {
    /// Khronos PBR Neutral — hue-preserving highlight roll-off (the default look).
    #[default]
    PbrNeutral,
    /// No tone curve: linear radiance straight to sRGB (highlights hard-clip).
    Linear,
    /// Reinhard `c / (1 + c)` — simple, classic, low-contrast roll-off.
    Reinhard,
    /// ACES filmic (Narkowicz/Hill fit) — punchy, contrasty film response.
    Aces,
    /// AgX — modern neutral filmic with graceful, low-chroma highlight handling.
    Agx,
}

impl TonemapOperator {
    /// Every variant in display order, for building UI menus.
    pub const ALL: [TonemapOperator; 5] = [
        TonemapOperator::PbrNeutral,
        TonemapOperator::Linear,
        TonemapOperator::Reinhard,
        TonemapOperator::Aces,
        TonemapOperator::Agx,
    ];

    /// Short menu label.
    pub fn label(self) -> &'static str {
        match self {
            TonemapOperator::PbrNeutral => "PBR Neutral",
            TonemapOperator::Linear => "Linear",
            TonemapOperator::Reinhard => "Reinhard",
            TonemapOperator::Aces => "ACES",
            TonemapOperator::Agx => "AgX",
        }
    }

    /// The index the post shader's `apply_tonemap` switch reads. Must match the
    /// `case` arms in `post.wgsl` (invariant 11).
    pub fn shader_index(self) -> u32 {
        match self {
            TonemapOperator::PbrNeutral => 0,
            TonemapOperator::Linear => 1,
            TonemapOperator::Reinhard => 2,
            TonemapOperator::Aces => 3,
            TonemapOperator::Agx => 4,
        }
    }
}

/// Tone-mapping configuration for the composite pass. Read by [`SceneCallback`] to
/// drive the tone-map stage of the post shader.
///
/// `enabled` is the status-bar toggle; when off the composite skips the tone curve
/// (linear → sRGB, hard-clipping highlights). The default is on with the Khronos
/// PBR Neutral operator, matching the pre-existing fixed look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TonemapSettings {
    pub enabled: bool,
    pub operator: TonemapOperator,
}

impl Default for TonemapSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            operator: TonemapOperator::PbrNeutral,
        }
    }
}

/// How the 2D UV viewport draws the model's UV layout. Mutually exclusive (the
/// toolbar's UV-shading group is a radio selection); the UV edges are always
/// drawn, the fill underneath them changes per mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UvShadingMode {
    /// UV edges over the reference grid, with no fill underneath (the default
    /// wire-only layout view).
    #[default]
    Wire,
    /// The UV islands filled with one solid shaded color, edges drawn on top.
    Shaded,
    /// Each UV island filled with its own unique color, edges drawn on top.
    Islands,
}

/// Which built-in checker texture the UV-checker view samples.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CheckerTexture {
    #[default]
    Greyscale,
    Color,
}

/// How the vertex-color view interprets the mesh's vertex-color attribute.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VertexColorMode {
    /// Show the RGB channels only (alpha forced opaque).
    #[default]
    Rgb,
    /// Show the alpha channel as a 0..1 greyscale value.
    Alpha,
    /// Show RGB as color and alpha as surface opacity.
    RgbAlpha,
}

/// Which material the filled faces display. The choices are mutually exclusive
/// (the toolbar's material group is a radio selection). [`Source`] is the
/// model's imported material; the other two replace it for inspection and apply
/// in every filled-face mode (unlit / shaded).
///
/// [`Source`]: ActiveMaterial::Source
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ActiveMaterial {
    /// The material as imported from the model.
    #[default]
    Source,
    /// A built-in UV checker pattern sampled through the model's UVs.
    UvChecker,
    /// The mesh's per-vertex color attribute.
    VertexColors,
}

/// How the source-material faces are shaded — the "Material Mode" option behind
/// the Source Material button. [`Source`] keeps the imported (and user-edited)
/// materials; the other two replace every mesh part's material with a uniform
/// matte standard material so geometry can be read without texture/material
/// noise. The replacement is applied renderer-side as an *effective* material
/// table (the imported materials are untouched), so switching back is free.
///
/// [`Source`]: MaterialMode::Source
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MaterialMode {
    /// The material as imported from the source file plus any user edits (the
    /// default, the renderer's pre-existing behavior).
    #[default]
    Source,
    /// Every mesh part replaced by one uniform matte mid-grey material (fully
    /// rough, non-metallic, no emissive).
    Standard,
    /// Like [`Standard`], but each unique mesh part gets its own randomized hue
    /// at the same mid brightness — so the parts of the model read apart.
    ///
    /// [`Standard`]: MaterialMode::Standard
    Unique,
}

impl MaterialMode {
    /// Every variant in display order, for building the dropdown.
    pub const ALL: [MaterialMode; 3] = [
        MaterialMode::Source,
        MaterialMode::Standard,
        MaterialMode::Unique,
    ];

    /// Dropdown label.
    pub fn label(self) -> &'static str {
        match self {
            MaterialMode::Source => "Source Material",
            MaterialMode::Standard => "Standard Material",
            MaterialMode::Unique => "Unique Mesh",
        }
    }

    /// The next mode in display order, wrapping back to [`Source`] after
    /// [`Unique`] — for cycling by re-clicking the active Source Material button.
    ///
    /// [`Source`]: MaterialMode::Source
    /// [`Unique`]: MaterialMode::Unique
    pub fn next(self) -> MaterialMode {
        match self {
            MaterialMode::Source => MaterialMode::Standard,
            MaterialMode::Standard => MaterialMode::Unique,
            MaterialMode::Unique => MaterialMode::Source,
        }
    }
}

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
    /// Which material the filled faces show (source / UV checker / vertex colors).
    /// Mutually exclusive; applies in every filled-face mode.
    pub active_material: ActiveMaterial,
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
    pub face_normals: bool,
    pub vertex_normals: bool,
    pub face_normal_length: f32,
    pub vertex_normal_length: f32,
    pub face_normal_color: [f32; 4],
    pub vertex_normal_color: [f32; 4],
    /// Color of the model wireframe overlay, baked into the line vertex buffer
    /// and rebuilt when it changes.
    pub wireframe_color: [f32; 4],
    /// Color of the bounding-box edges, baked into its line buffer and rebuilt
    /// when it changes.
    pub bounding_box_color: [f32; 4],
    /// Which geometry the bounding box wraps (whole model / only the selection /
    /// only the visible meshes). Baked into the box line buffer alongside the
    /// inputs the chosen scope depends on, so it rebuilds when they change.
    pub bounding_box_scope: BoundingBoxScope,
    /// The Outliner selection the box wraps in [`BoundingBoxScope::OnlySelection`]
    /// mode (ignored otherwise). Carried here so the box can be baked from the
    /// scene callback without threading the selection through separately.
    pub bounding_box_selection: Selection,
    /// When `true` back-facing triangles are drawn (the mesh is double-sided);
    /// when `false` (the default) they are culled, so only camera-facing surfaces
    /// are rendered. The renderer keeps two mesh pipelines — culling vs.
    /// double-sided — and picks one per frame from this flag, so toggling it
    /// allocates nothing.
    pub render_backfaces: bool,
}

impl Default for SceneDebugOptions {
    fn default() -> Self {
        Self {
            shading_mode: ShadingMode::Shaded,
            wireframe_overlay: false,
            active_material: ActiveMaterial::Source,
            material_mode: MaterialMode::Source,
            uv_checker_texture: CheckerTexture::Greyscale,
            vertex_color_mode: VertexColorMode::Rgb,
            uv_checker_tiling: 4,
            uv_channel: 0,
            show_grid: true,
            show_bounding_box: false,
            face_normals: false,
            vertex_normals: false,
            face_normal_length: 0.03,
            vertex_normal_length: 0.03,
            face_normal_color: [1.0, 0.1, 0.1, 0.95],
            vertex_normal_color: [0.14, 0.92, 0.96, 0.95],
            wireframe_color: [0.6, 0.6, 0.6, 1.0],
            bounding_box_color: [1.0, 0.803_921_6, 0.250_980_4, 1.0],
            bounding_box_scope: BoundingBoxScope::default(),
            bounding_box_selection: Selection::None,
            render_backfaces: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RendererConfig {
    pub preferred_backends: wgpu::Backends,
    pub clear_color: wgpu::Color,
}

impl Default for RendererConfig {
    fn default() -> Self {
        // On Windows, request DX12 only: adapter/device creation is ~100 ms
        // cheaper than bringing up the Vulkan loader + ICD (measured on an RTX
        // 4080), and DX12 is guaranteed on Windows 10+. Other platforms keep
        // Vulkan/Metal. This is the "prefer DX12 on Windows" decision in
        // CLAUDE.md §4, now enforced rather than left to adapter selection.
        #[cfg(windows)]
        let preferred_backends = wgpu::Backends::DX12;
        #[cfg(not(windows))]
        let preferred_backends = wgpu::Backends::VULKAN | wgpu::Backends::METAL;

        Self {
            preferred_backends,
            clear_color: wgpu::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
        }
    }
}
