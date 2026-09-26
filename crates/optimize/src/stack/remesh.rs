//! The retopology operation's settings.
//!
//! Unlike every other operation in the stack, Remesh does not *edit* the mesh it
//! is given — it lays a new surface over the old one and brings the materials,
//! UVs and colors back by projection. So its parameters describe the mesh to
//! produce (what it is made of, how dense) rather than what to remove.

use serde::{Deserialize, Serialize};

use super::{NormalMode, NormalParams};

/// What the rebuilt surface is made of.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemeshTopology {
    /// Evenly sized, curvature-aligned triangles.
    #[default]
    Triangles,
    /// The same surface with its triangles paired into quads wherever a quad
    /// describes it honestly, and left as triangles where one would not.
    ///
    /// Quad *dominant*, not pure. A closed surface cannot be all quads without
    /// either moving the vertices somewhere they fit better — which would cost
    /// the shape this rebuild is careful about — or flattening the creases the
    /// odd faces sit on.
    #[serde(alias = "QuadDominant", alias = "PureQuads")]
    Quads,
}

impl RemeshTopology {
    pub const ALL: [RemeshTopology; 2] = [RemeshTopology::Triangles, RemeshTopology::Quads];

    pub fn label(self) -> &'static str {
        match self {
            RemeshTopology::Triangles => "Triangles",
            RemeshTopology::Quads => "Quads",
        }
    }

    /// How many output *faces* one source triangle is worth when the density is
    /// given as a ratio.
    ///
    /// One for triangles. A half for quads, since a quad covers roughly two
    /// triangles' worth of surface — so the same ratio asks for the same amount
    /// of surface detail either way, which is what a ratio is for.
    pub(crate) fn faces_per_triangle(self) -> f32 {
        match self {
            RemeshTopology::Triangles => 1.0,
            RemeshTopology::Quads => 0.5,
        }
    }
}

/// How the target face count for each object is arrived at.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemeshDensity {
    /// A fraction of what the object has now. Every object keeps its relative
    /// detail, which is what you want when remeshing a whole asset at once.
    #[default]
    Ratio,
    /// One face budget for the whole selection, split between objects by
    /// surface area. An object whose own override names a count takes exactly
    /// that and leaves the shared pool.
    Absolute,
}

impl RemeshDensity {
    pub const ALL: [RemeshDensity; 2] = [RemeshDensity::Ratio, RemeshDensity::Absolute];

    pub fn label(self) -> &'static str {
        match self {
            RemeshDensity::Ratio => "Ratio of current",
            RemeshDensity::Absolute => "Face count",
        }
    }
}

/// Settings for the [`crate::stack::OpKind::Remesh`] operation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemeshParams {
    pub topology: RemeshTopology,
    pub density: RemeshDensity,
    /// [`RemeshDensity::Ratio`]: the fraction of the object's current triangle
    /// count to aim for. Read against triangles, not faces, so it lines up with
    /// the Tris figure on the stats card.
    pub ratio: f32,
    /// [`RemeshDensity::Absolute`]: faces for the whole selection.
    pub faces: u32,
    /// Detect and hold sharp edges rather than letting the field run over them.
    /// Off by default: on organic geometry it only fragments the result.
    pub sharp_edges: bool,
    /// Dihedral angle, in degrees, above which an edge counts as sharp. Only
    /// read when `sharp_edges` is on.
    pub crease_angle: f32,
    /// Pin the field to open borders, so a boundary comes back as one straight
    /// edge loop rather than a ragged fringe.
    pub align_to_boundaries: bool,
    /// Rounds of tidying over the rebuilt mesh — a valence-improving flip pass
    /// and a relaxation along the surface. Evens out face sizes; too many round
    /// off detail.
    pub smooth_iterations: u32,
    /// How far face size may follow curvature, 0 (one face size everywhere) to
    /// 1 (as far as the layout will carry it). What it buys is where the budget
    /// goes: a flat panel gets large faces and a tight fillet small ones, for
    /// the same count.
    ///
    /// It is read as an exponent, and the two ends of it are named rules rather
    /// than arbitrary settings. At 0.5 face size goes as `1 / sqrt(curvature)`,
    /// which spends the same chordal error everywhere — the geometrically
    /// correct answer, and a gentle one. At 1 it goes as `1 / curvature`, so
    /// every face turns through the same angle; that is what a hand retopology
    /// looks like and it is much more aggressive. Measured on a driftwood
    /// branch, the spread of output face sizes across the object runs 1.7x /
    /// 4.2x / 6.1x at 0 / 0.5 / 1.
    ///
    /// At 0 no field is built at all and one size is used everywhere — the same
    /// answer as a field of ones, by a shorter road.
    pub adaptive_strength: f32,
    /// Where the rebuilt surface's normals come from. Projected by default,
    /// which carries the artist's shading across.
    pub normals: NormalMode,
    /// How they are generated, when [`Self::normals`] says to.
    pub normal_params: NormalParams,
}

impl Default for RemeshParams {
    fn default() -> Self {
        Self {
            topology: RemeshTopology::default(),
            density: RemeshDensity::default(),
            // The same face count as the source: a like-for-like retopology
            // rather than a reduction. Reducing is what the ratio is for.
            ratio: 1.0,
            faces: 5_000,
            sharp_edges: false,
            crease_angle: 30.0,
            align_to_boundaries: true,
            smooth_iterations: 2,
            // The geometric rule (see the field doc above), which is the
            // strongest setting that costs nothing. Full strength is worth
            // reaching for on a silhouette-critical object, not by default.
            adaptive_strength: 0.5,
            normals: NormalMode::Project,
            normal_params: NormalParams::default(),
        }
    }
}
