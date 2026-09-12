//! The lighting and post options: the environment, the occlusion, the operator
//! the composite tone maps with.

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
/// the scene renderer to choose + precompute the IBL maps and drive the PBR shaded
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

/// Ground-Truth Ambient Occlusion configuration for the shaded view. Read by the
/// scene renderer to drive the GTAO + blur passes and the composite multiply.
/// (User-facing UI calls this "Ambient Occlusion"; the internal implementation is
/// GTAO.)
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
    /// `case` arms in `post.hlsl` (invariant 11).
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

/// Tone-mapping configuration for the composite pass. Read by the scene renderer to
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
