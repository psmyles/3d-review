//! Normal generation settings: the Recalculate Normals operation's parameters,
//! and the choice a rebuilding operation offers between keeping the source's
//! shading and generating fresh normals from its own surface.

use serde::{Deserialize, Serialize};

/// How much each face around a point counts toward the point's normal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NormalWeighting {
    /// The corner's angle times the two edges' lengths there: meshoptimizer's
    /// own, which also offers smoothing.
    #[default]
    AngleAndEdges,
    /// The face's area: a large flat face keeps its own normal right up to its
    /// edge, and the narrow fillet or chamfer beside it bends instead — what
    /// hard-surface parts at low triangle counts need, so flats do not pinch.
    Area,
    /// The corner's angle, so how finely a face is cut does not matter.
    Angle,
    /// Both: the area times the angle.
    AreaAndAngle,
}

impl NormalWeighting {
    pub const ALL: [NormalWeighting; 4] = [
        NormalWeighting::AngleAndEdges,
        NormalWeighting::Area,
        NormalWeighting::Angle,
        NormalWeighting::AreaAndAngle,
    ];
}

/// How normals are generated from positions alone.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NormalParams {
    /// In degrees. Where two faces meet at less than this the edge is smoothed
    /// across; sharper edges stay hard. 180 smooths everything.
    pub crease_angle: f32,
    /// Extra relaxation over neighbouring triangles, 0 for none. Useful on a
    /// surface faceted at a small scale — a voxel or distance-field surface —
    /// where plain averaging still shows the steps. Applies to the
    /// [`NormalWeighting::AngleAndEdges`] weighting.
    pub smoothing: f32,
    pub weighting: NormalWeighting,
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
            weighting: NormalWeighting::AngleAndEdges,
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

    /// The generation settings to use, or `None` to project.
    pub fn generation(self, params: NormalParams) -> Option<NormalParams> {
        (self == NormalMode::Generate).then_some(params)
    }
}
