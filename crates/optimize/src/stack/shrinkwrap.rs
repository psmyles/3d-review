//! The shrinkwrap operation's settings.
//!
//! Shrinkwrap replaces an object with a single closed shell that hugs it: the
//! surface is voxelized into a signed distance field and re-extracted. That
//! fuses a kitbash of interpenetrating parts into one watertight object, which
//! is the one thing a quad solver cannot do for itself and cannot run without.

use serde::{Deserialize, Serialize};

use super::{NormalMode, NormalParams};

/// Settings for the [`crate::stack::OpKind::Shrinkwrap`] operation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShrinkwrapParams {
    /// Voxels across the object's longest side. Everything about the result —
    /// how much detail survives, how small a gap gets bridged, how long it takes
    /// — follows from this one number, which is why it is the only size knob.
    ///
    /// It is also what the *cost* follows from, cubically, and the extraction
    /// emits roughly six triangles per surface cell. Measured on a real prop:
    /// 48 gives a 170 000-triangle shell in a quarter of a second, 64 gives
    /// 300 000 in half of one, and 128 gives **1.3 million** in two — dense
    /// enough that the quad solver below it then gives up. Hence the default.
    pub resolution: u32,
    /// Push the shell out (or, negative, pull it in) by this many world meters.
    /// A small positive offset closes gaps a plain wrap leaves open; a negative
    /// one shrinks the shell inside the original, for a collision proxy.
    pub offset: f32,
    /// Keep only the largest closed shell the extraction produced. On by
    /// default: a wrap of a messy object routinely leaves small satellites
    /// around specks of stray geometry, and none of them is wanted.
    pub keep_largest_shell: bool,
    /// Where the shell's normals come from: projected off the original by
    /// default, or generated from the shell itself.
    pub normals: NormalMode,
    /// How they are generated, when [`Self::normals`] says to.
    pub normal_params: NormalParams,
}

impl Default for ShrinkwrapParams {
    fn default() -> Self {
        Self {
            // Measured rather than chosen: at 64 a prop keeps its silhouette,
            // the wrap takes half a second, and — the part that matters — the
            // shell is clean enough for a Remesh below it to even out. At
            // 128 the same prop's shell is four times the triangles and the
            // solver gives up on it, which is the whole pairing failing.
            resolution: 64,
            offset: 0.0,
            keep_largest_shell: true,
            normals: NormalMode::Project,
            normal_params: NormalParams::default(),
        }
    }
}
