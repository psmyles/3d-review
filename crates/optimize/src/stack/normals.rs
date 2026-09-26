//! Normal generation settings: the Recalculate Normals operation's parameters,
//! and the choice a rebuilding operation offers between keeping the source's
//! shading and generating fresh normals from its own surface.

use serde::{Deserialize, Serialize};

/// How meshoptimizer generates normals from positions alone.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NormalParams {
    /// In degrees. Where two faces meet at less than this the edge is smoothed
    /// across; sharper edges stay hard. 180 smooths everything.
    pub crease_angle: f32,
    /// Extra relaxation over neighbouring triangles, 0 for none. Useful on a
    /// surface faceted at a small scale — a voxel or distance-field surface —
    /// where plain averaging still shows the steps.
    pub smoothing: f32,
}

impl NormalParams {
    /// The smoothing a voxel-derived surface defaults to: enough to hide the
    /// voxel-scale faceting, as meshoptimizer's own remesh demo does.
    pub const VOXEL_SMOOTHING: f32 = 1.5;
}

impl Default for NormalParams {
    fn default() -> Self {
        Self {
            // meshoptimizer's demo value and a common DCC default: keeps a
            // box's corners hard and a cylinder's facets soft.
            crease_angle: 60.0,
            // The library's own default. Relaxing an authored model's normals
            // is rarely what a recalculation is for.
            smoothing: 0.0,
        }
    }
}

/// Where a rebuilt surface's normals come from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NormalMode {
    /// Read off the source surface at each new corner — the artist's shading,
    /// carried across.
    #[default]
    Project,
    /// Generated from the new surface itself, with a crease angle.
    Generate,
}

impl NormalMode {
    pub const ALL: [NormalMode; 2] = [NormalMode::Project, NormalMode::Generate];
}
