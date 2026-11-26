//! Settings for the Convex Hull operation.

use serde::{Deserialize, Serialize};

/// Fewest vertices a hull may be limited to: a tetrahedron.
pub const HULL_VERTICES_MIN: u32 = 4;
/// Most vertices a hull may keep: the limit PhysX and Unreal put on a convex
/// collision mesh.
pub const HULL_VERTICES_MAX: u32 = 255;

/// Settings for the [`OpKind::ConvexHull`](crate::stack::OpKind::ConvexHull)
/// operation: which objects become hulls, and how many vertices each keeps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HullParams {
    /// Name patterns (`*`, `?`, `[...]`) of the objects replaced by their hull.
    /// To start with, the collision meshes' conventions: `*_hull` and Unreal's
    /// `UCX_*`. `*` takes every object.
    pub objects: Vec<String>,
    /// The most vertices a hull keeps. A hull with more is built from the
    /// points that stick out furthest, one at a time, until it has this many.
    pub max_vertices: u32,
}

impl Default for HullParams {
    fn default() -> Self {
        Self {
            objects: vec!["*_hull".to_owned(), "UCX_*".to_owned()],
            max_vertices: 32,
        }
    }
}
