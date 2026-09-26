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
//! (1.0 = none); the LOD rows are the per-level triangle target and error limit.

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

/// `value` inside `min..=max`, or `fallback` when it is not a number at all.
pub(crate) fn fit(value: &mut f32, min: f32, max: f32, fallback: f32) {
    *value = if value.is_nan() {
        fallback
    } else {
        value.clamp(min, max)
    };
}
