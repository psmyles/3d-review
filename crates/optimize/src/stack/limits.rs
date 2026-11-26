//! The range every numeric operation setting may take.
//!
//! One source for two readers: the Opt inspector's sliders (which `ui` builds
//! from these) and [`super::OptStack::sanitize`], which clamps a preset loaded
//! from disk into the same ranges. A preset is a file anyone can edit, and a value
//! the inspector could never have produced - a million LOD levels, a negative
//! weight - would otherwise reach the operations unchecked.
//!
//! The weld tolerance and the prune size threshold are both fractions of the
//! mesh's overall size; the attribute weights scale a simplifier's per-attribute
//! error; the overdraw threshold is the vertex-cache efficiency it may give up
//! (1.0 = none); the LOD rows are the per-level triangle target (a share, or a
//! count) and error limit.

pub const WELD_TOLERANCE_MIN: f32 = 0.0;
pub const WELD_TOLERANCE_MAX: f32 = 0.1;
pub const PRUNE_THRESHOLD_MIN: f32 = 0.0;
pub const PRUNE_THRESHOLD_MAX: f32 = 0.5;
pub const ATTRIBUTE_WEIGHT_MIN: f32 = 0.0;
pub const ATTRIBUTE_WEIGHT_MAX: f32 = 4.0;
pub const OVERDRAW_THRESHOLD_MIN: f32 = 1.0;
pub const OVERDRAW_THRESHOLD_MAX: f32 = 3.0;
pub const LOD_RATIO_MIN: f32 = 0.01;
pub const LOD_RATIO_MAX: f32 = 1.0;
pub const LOD_ERROR_MIN: f32 = 0.0;
pub const LOD_ERROR_MAX: f32 = 1.0;
/// A triangle target given as a count (`TargetMode::Triangles`). The floor is one
/// triangle — the simplifier stops where the error limit says long before — and the
/// ceiling is past any game asset; a count above an object's own triangles leaves
/// it as it is. The ratio's 1% floor does not apply to the share a count works out
/// to: a million-triangle scan can be asked for 1500.
pub const LOD_TRIANGLES_MIN: u32 = 1;
pub const LOD_TRIANGLES_MAX: u32 = 100_000_000;
/// The quality limits (`QualityLimits`): the most an object's volume may change, as
/// a share of it, and the farthest its surface may move, as a share of its size.
pub const VOLUME_CHANGE_MIN: f32 = 0.001;
pub const VOLUME_CHANGE_MAX: f32 = 1.0;
pub const DISTANCE_LIMIT_MIN: f32 = 0.0001;
pub const DISTANCE_LIMIT_MAX: f32 = 0.5;

/// The most levels a LOD chain may hold. Past this the chain stops being a
/// pipeline decision and starts being an experiment; the cost is one full
/// simplify of the whole model per level, per edit.
pub const MAX_LOD_LEVELS: usize = 8;

/// Remesh: the density given either as a share of the object's current
/// triangles or as an absolute face budget, the crease angle in degrees, and the
/// smoothing pass count. The ratio's ceiling is above 1.0 so a coarse object can
/// be *asked* for more faces - a rebuild only ever merges, so it comes back at
/// about its current density and says so, which is better than a slider that
/// silently refuses the question.
pub const REMESH_RATIO_MIN: f32 = 0.05;
pub const REMESH_RATIO_MAX: f32 = 2.0;
pub const REMESH_FACES_MIN: u32 = 100;
pub const REMESH_FACES_MAX: u32 = 200_000;
pub const REMESH_CREASE_MIN: f32 = 5.0;
pub const REMESH_CREASE_MAX: f32 = 90.0;
pub const REMESH_SMOOTH_MAX: u32 = 10;
/// Remesh: how far face size follows curvature. The top of the range is 1
/// rather than something larger because past it the quad layout resolves the
/// transition with singularities instead of a gradient.
pub const REMESH_ADAPTIVE_MIN: f32 = 0.0;
pub const REMESH_ADAPTIVE_MAX: f32 = 1.0;

/// Shrinkwrap: grid steps across the object's longest side, and how far the
/// shell is pushed out (or, negative, pulled in) in world meters. The offset
/// range is deliberately symmetric - a shell *inside* the object is a collision
/// proxy, which is as normal a thing to ask for as one outside it.
pub const SHRINKWRAP_RESOLUTION_MIN: u32 = 16;
pub const SHRINKWRAP_RESOLUTION_MAX: u32 = 512;
pub const SHRINKWRAP_OFFSET_MIN: f32 = -0.5;
pub const SHRINKWRAP_OFFSET_MAX: f32 = 0.5;
/// Shrinkwrap's voxel method: meshoptimizer's own resolution window, and the
/// built-in reduction's target as a share of the object's triangles (above 1 is
/// allowed - a coarse source can want a denser shell) or as a per-object count
/// on Remesh's scale.
pub const SHRINKWRAP_VOXEL_RESOLUTION_MIN: u32 = 4;
pub const SHRINKWRAP_VOXEL_RESOLUTION_MAX: u32 = 256;
pub const SHRINKWRAP_TARGET_RATIO_MIN: f32 = 0.01;
pub const SHRINKWRAP_TARGET_RATIO_MAX: f32 = 2.0;
pub const SHRINKWRAP_TARGET_TRIANGLES_MIN: u32 = 100;
pub const SHRINKWRAP_TARGET_TRIANGLES_MAX: u32 = 200_000;

/// Normal generation (Recalculate Normals, and a rebuild told to generate): the
/// crease angle in degrees and the smoothing amount. The smoothing ceiling is
/// meshoptimizer's recommended range; the library accepts more, but past it
/// every pass relaxes shape into mush.
pub const NORMAL_CREASE_MIN: f32 = 0.0;
pub const NORMAL_CREASE_MAX: f32 = 180.0;
pub const NORMAL_SMOOTHING_MIN: f32 = 0.0;
pub const NORMAL_SMOOTHING_MAX: f32 = 5.0;

/// Bake AO: the max ray distance in world meters (0 = unlimited) and the power
/// on visibility (matching the viewport AO panel's Intensity, whose range is
/// deliberately wider here - a bake is worth over-driving).
pub const AO_BAKE_DISTANCE_MIN: f32 = 0.0;
pub const AO_BAKE_DISTANCE_MAX: f32 = 10.0;
pub const AO_BAKE_INTENSITY_MIN: f32 = 0.1;
pub const AO_BAKE_INTENSITY_MAX: f32 = 4.0;

/// Unwrap: the UV set written or referred to (glTF and FBX meshes rarely carry
/// more than a handful), the texel density in UV units per metre, the crease
/// angle in degrees, the atlas resolution and padding in texels.
pub const UV_SET_MAX: u32 = 7;
pub const UNWRAP_DENSITY_MIN: f32 = 0.001;
pub const UNWRAP_DENSITY_MAX: f32 = 1000.0;
pub const UNWRAP_CREASE_MIN: f32 = 1.0;
pub const UNWRAP_CREASE_MAX: f32 = 179.0;
pub const UNWRAP_RESOLUTION_MIN: u32 = 64;
pub const UNWRAP_RESOLUTION_MAX: u32 = 16384;
pub const UNWRAP_PADDING_MAX: u32 = 64;
pub const UNWRAP_ISLAND_STRETCH_MIN: f32 = 1.0;
pub const UNWRAP_ISLAND_STRETCH_MAX: f32 = 10.0;

/// Bake Edge Wear: the angles in degrees, and the power on the ramp.
pub const WEAR_ANGLE_MIN: f32 = 0.0;
pub const WEAR_ANGLE_MAX: f32 = 180.0;
pub const WEAR_FALLOFF_MIN: f32 = 0.1;
pub const WEAR_FALLOFF_MAX: f32 = 8.0;

/// `value` inside `min..=max`, or `fallback` when it is not a number at all.
pub(crate) fn fit(value: &mut f32, min: f32, max: f32, fallback: f32) {
    *value = if value.is_nan() {
        fallback
    } else {
        value.clamp(min, max)
    };
}
