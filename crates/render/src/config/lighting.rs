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
/// scene renderer to drive the GTAO passes and the composite's ambient darkening.
/// (User-facing UI calls this "Ambient Occlusion"; the internal implementation is
/// GTAO.)
///
/// GTAO samples a single-sample view-space normal + depth G-buffer, marches the
/// screen-space horizon per slice to estimate the cosine-weighted visible arc,
/// denoises the result, and applies it only to the scene's ambient radiance in the
/// composite — darkening contact creases and cavities without muting
/// direct/specular light. The horizon search follows Intel's XeGTAO.
///
/// The radius is **derived from what the viewport is showing**, not stored here
/// — see [`GtaoSettings::effective_radius`] for why, and note that [`Self::radius`]
/// is only a multiplier over it. `enabled` is the toolbar toggle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GtaoSettings {
    pub enabled: bool,
    /// Scales the radius the viewer derives for the current view
    /// ([`GtaoSettings::effective_radius`]). `1.0` is the automatic value; the
    /// slider exists to taste it up or down, not to carry the scene's scale.
    pub radius: f32,
    /// Strength of the darkening: a power on the GTAO visibility (1 = ground
    /// truth, >1 darkens, 0 disables).
    pub intensity: f32,
    /// Thin-occluder compensation (0..1). A horizon search that only ever climbs
    /// assumes every occluder is infinitely deep, so a railing or a chair leg
    /// shadows everything behind it; at 1 the horizon follows the last sample back
    /// down and thin geometry stops over-occluding, at 0 occluders are solid.
    ///
    /// Named "Thickness" in the UI, which reads the right way round: more of it
    /// means the shader treats occluders as *less* thick.
    pub thickness: f32,
    /// Sampling quality — the slice / step counts of the horizon search and the
    /// number of denoise passes over the result.
    pub quality: GtaoQuality,
}

impl Default for GtaoSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            radius: 1.0,
            // Above ground truth on purpose: the composite darkens only the diffuse
            // ambient, so a physically exact 1.0 reads weaker on screen than the
            // occlusion actually is.
            intensity: 1.5,
            // Near zero, i.e. occluders are treated as solid. Letting the horizon
            // fall back toward a thin occluder is the more correct model, but it
            // lightens every contact shadow to buy back a case that game assets
            // rarely hit; the knob is there when they do.
            thickness: 0.01,
            quality: GtaoQuality::Medium,
        }
    }
}

/// The share of the viewport's world-space height the automatic radius aims for.
///
/// Choosing it as a fraction of the *view* rather than of the model is what makes
/// the occlusion scale-free, and it has a second property that matters more than it
/// looks: the radius's size in **pixels** is then this fraction times the viewport
/// height, whatever the scene's units and however far the camera has zoomed. The
/// horizon march always spans the same number of pixels, so its fixed step count
/// always resolves it — the failure mode where a large scene spread six samples over
/// half the screen and found nothing cannot happen.
///
/// A tenth of the viewport height was picked by eye against the alternatives: 0.04
/// and 0.06 read as too timid on real assets, and going much wider starts to spread
/// the fixed step count thinly enough that near-field contact is undersampled again.
/// At a 1400 px viewport this is ~140 px, which the squared step distribution covers
/// from about 2 px out.
const RADIUS_VIEW_FRACTION: f32 = 0.10;

/// Ceiling on the automatic radius as a share of the model's bounding sphere.
///
/// Only binds when the camera is pulled well back from the content, where the view
/// extent keeps growing but the model does not. Occlusion sampled over a distance
/// larger than the object itself finds nothing but its own silhouette, so the
/// ceiling is what stops a zoomed-out view paying for a radius that cannot report
/// anything.
const RADIUS_MODEL_CEILING: f32 = 0.5;

impl GtaoSettings {
    /// The occlusion radius in world units for the view described by
    /// `view_extent` ([`crate::OrbitCamera::view_extent`]) and `scene_radius`.
    ///
    /// The whole scale problem lives in this function. Ambient occlusion is a
    /// *local* effect — it shades the crease between two surfaces, the contact
    /// under a chair leg — so its radius belongs to the scale of detail being
    /// looked at, not to the extent of the scene. Deriving it from the model's
    /// bounding sphere conflates those: a 10 cm prop and a 10 km landscape want
    /// almost the same radius when you are examining a feature of the same
    /// apparent size, and quite different ones when you are not.
    ///
    /// So the radius follows the viewport instead, with the model only as a
    /// ceiling, and [`Self::radius`] left as a multiplier over the result.
    pub fn effective_radius(&self, view_extent: f32, scene_radius: f32) -> f32 {
        let from_view = RADIUS_VIEW_FRACTION * view_extent.max(1e-6);
        let ceiling = RADIUS_MODEL_CEILING * scene_radius.max(1e-6);
        // The multiplier is applied *after* the ceiling so that asking for a wider
        // radius than the model is still possible — that is an explicit choice,
        // while the ceiling exists to stop an automatic value drifting there.
        (self.radius.max(0.0) * from_view.min(ceiling)).max(1e-6)
    }
}

/// GTAO sampling quality: the number of slice directions and horizon steps per
/// slice, and how many denoise passes run over the result. More of each means
/// smoother, more accurate occlusion at higher cost.
///
/// The counts are lower than they look: the accumulation
/// ([`crate::scene`]'s `ao_accum`) averages 24 frames whenever the view is still,
/// so a settled image at Medium has seen 72 slice directions per pixel. These
/// numbers therefore buy quality for the frames *during* an orbit, and the denoise
/// pass count is what carries a moving frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GtaoQuality {
    /// 2 slices × 3 steps, 1 denoise pass — cheapest.
    Low,
    /// 3 slices × 4 steps, 2 denoise passes — the balanced default.
    #[default]
    Medium,
    /// 4 slices × 6 steps, 3 denoise passes — smoothest while moving, most
    /// expensive.
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
            GtaoQuality::Medium => (3, 4),
            GtaoQuality::High => (4, 6),
        }
    }

    /// How many times the edge-aware denoise runs over the occlusion.
    ///
    /// Iterating a narrow kernel beats widening it once: each pass re-reads its
    /// neighbours' *already smoothed* values, so support grows geometrically while
    /// every tap keeps being rejected by the same edge test. A single wide kernel
    /// would have to loosen that test to reach as far.
    pub fn denoise_passes(self) -> u32 {
        match self {
            GtaoQuality::Low => 1,
            GtaoQuality::Medium => 2,
            GtaoQuality::High => 3,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::OrbitCamera;
    use review_model::Bounds;

    /// A camera framing a cube `size` across, which is how the viewer actually
    /// arrives at a view: the user loads a model and it is framed.
    fn framed(size: f32) -> OrbitCamera {
        let half = size * 0.5;
        let mut camera = OrbitCamera {
            aspect_ratio: 16.0 / 9.0,
            ..OrbitCamera::default()
        };
        camera.frame_bounds(Bounds {
            min: glam::Vec3::splat(-half),
            max: glam::Vec3::splat(half),
        });
        camera
    }

    fn radius_at(size: f32) -> f32 {
        let camera = framed(size);
        GtaoSettings::default().effective_radius(camera.view_extent(), camera.scene_radius)
    }

    /// The point of the whole exercise: a model framed in the viewport gets an
    /// occlusion radius proportional to its own size, across every scale the
    /// viewer is expected to open — 10 cm to 10 km is five orders of magnitude.
    ///
    /// Proportional is what "looks the same" means here. The radius in *pixels* is
    /// the ratio below times the viewport height, so holding the ratio constant is
    /// exactly holding the on-screen size of the occlusion constant, which is what
    /// the eye judges.
    #[test]
    fn a_framed_model_gets_the_same_relative_radius_at_every_scale() {
        let sizes = [0.1_f32, 1.0, 100.0, 10_000.0];
        let reference = radius_at(sizes[0]) / sizes[0];
        for size in sizes {
            let ratio = radius_at(size) / size;
            assert!(
                (ratio - reference).abs() < reference * 1e-3,
                "a {size} m model resolved to {ratio} of its size, not {reference}"
            );
        }
    }

    /// And the absolute values are sane rather than merely consistent: a 10 cm prop
    /// must not ask for a metre of occlusion, nor a 10 km landscape for a
    /// millimetre.
    #[test]
    fn the_resolved_radius_is_a_believable_distance_at_each_scale() {
        assert!(
            (0.002..0.05).contains(&radius_at(0.1)),
            "{}",
            radius_at(0.1)
        );
        assert!((0.02..0.5).contains(&radius_at(1.0)), "{}", radius_at(1.0));
        assert!(
            (200.0..5000.0).contains(&radius_at(10_000.0)),
            "{}",
            radius_at(10_000.0)
        );
    }

    /// Pulling back from the content keeps growing the view extent while the model
    /// stays the same size, and occlusion sampled wider than the object itself
    /// reports nothing. The ceiling is what stops that.
    #[test]
    fn the_model_ceiling_binds_once_the_camera_pulls_away() {
        let mut camera = framed(1.0);
        let framed_radius =
            GtaoSettings::default().effective_radius(camera.view_extent(), camera.scene_radius);
        camera.distance *= 50.0;
        let pulled_back =
            GtaoSettings::default().effective_radius(camera.view_extent(), camera.scene_radius);
        assert!(
            pulled_back > framed_radius,
            "a wider view should still widen the radius somewhat"
        );
        assert!(
            pulled_back <= RADIUS_MODEL_CEILING * camera.scene_radius + 1e-6,
            "{pulled_back} exceeded the model ceiling"
        );
    }

    /// The slider is a multiplier over the automatic value and nothing else, so
    /// doubling it doubles the distance at any scale.
    #[test]
    fn the_slider_scales_the_automatic_radius() {
        let camera = framed(3.0);
        let auto = GtaoSettings::default();
        let doubled = GtaoSettings {
            radius: 2.0,
            ..auto
        };
        let a = auto.effective_radius(camera.view_extent(), camera.scene_radius);
        let b = doubled.effective_radius(camera.view_extent(), camera.scene_radius);
        assert!((b - a * 2.0).abs() < a * 1e-5, "{a} -> {b}");
    }
}
