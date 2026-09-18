//! The retopology operation's settings.
//!
//! Unlike every other operation in the stack, Remesh does not *edit* the mesh it
//! is given — it regenerates the surface from a field solved over it and brings
//! the materials, UVs and colors back by projection. So its parameters describe
//! the mesh to produce (what topology, how dense) rather than what to remove.

use serde::{Deserialize, Serialize};

/// What the regenerated surface is made of.
///
/// Both map onto the engine's symmetry settings: a 6-RoSy / 3-PoSy field gives
/// evenly sized triangles, and a 4/4 field gives quads wherever the field admits
/// them and triangles where it does not.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemeshTopology {
    /// Evenly sized, curvature-aligned triangles.
    Triangles,
    /// Quads wherever the field allows, triangles at the singularities. The
    /// default: it is what an artist retopologizing by hand produces, and it
    /// works on any input.
    #[default]
    QuadDominant,
}

impl RemeshTopology {
    pub const ALL: [RemeshTopology; 2] = [RemeshTopology::Triangles, RemeshTopology::QuadDominant];

    pub fn label(self) -> &'static str {
        match self {
            RemeshTopology::Triangles => "Triangles",
            RemeshTopology::QuadDominant => "Mostly quads",
        }
    }

    /// The `(rosy, posy)` symmetry pair the field is solved with.
    pub(crate) fn rosy_posy(self) -> (u32, u32) {
        match self {
            RemeshTopology::Triangles => (6, 3),
            RemeshTopology::QuadDominant => (4, 4),
        }
    }

    /// How many output *faces* one source triangle is worth when the density is
    /// given as a ratio: a quad covers roughly two triangles' worth of surface,
    /// so "100%" of a 10 000-triangle mesh is 5 000 quads, not 10 000.
    pub(crate) fn faces_per_triangle(self) -> f32 {
        match self {
            RemeshTopology::Triangles => 1.0,
            RemeshTopology::QuadDominant => 0.5,
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
    /// Laplacian passes over the extracted mesh, each re-projected onto the
    /// source surface. Evens out face sizes; too many round off detail.
    pub smooth_iterations: u32,
    /// Take the reproducible path through every order-sensitive stage, so the
    /// same input gives the same bytes on any machine. On by default: a
    /// preview that changes under you while you drag a slider is worse than a
    /// slightly slower one.
    pub deterministic: bool,
}

impl Default for RemeshParams {
    fn default() -> Self {
        Self {
            topology: RemeshTopology::default(),
            density: RemeshDensity::default(),
            // Same face *area* as the source, which for quads is half its
            // triangle count — a like-for-like retopology rather than a
            // reduction. Reducing is what the ratio is for.
            ratio: 1.0,
            faces: 5_000,
            sharp_edges: false,
            crease_angle: 30.0,
            align_to_boundaries: true,
            smooth_iterations: 2,
            deterministic: true,
        }
    }
}
