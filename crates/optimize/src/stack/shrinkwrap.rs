//! The shrinkwrap operation's settings.
//!
//! Shrinkwrap replaces an object with a single closed shell that hugs it. Two
//! methods produce that shell. The default samples a signed distance field whose
//! sign is a winding number and re-extracts it: robust to holes and inverted
//! parts, which is what fuses a kitbash into one watertight object. The voxel
//! method is meshoptimizer's remesher: it keeps thin sheets (a leaf card, a
//! cape) that a distance field loses below its voxel size, fits its vertices to
//! the original's features, and can reduce its own output before the
//! attributes are copied back.

use serde::{Deserialize, Serialize};

use super::{NormalMode, NormalParams};

/// Which surface extraction a Shrinkwrap runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShrinkwrapMethod {
    /// Narrow-band signed distance field, sign from the generalized winding
    /// number, extracted by marching tetrahedra. Handles holes and inside-out
    /// parts; loses anything thinner than a voxel.
    #[default]
    Winding,
    /// meshoptimizer's voxel remesher (experimental upstream). Keeps thin
    /// sheets at any resolution and fits vertices to features; fills a solid by
    /// flood fill, so a hole wider than a voxel lets the outside in.
    Voxel,
}

impl ShrinkwrapMethod {
    pub const ALL: [ShrinkwrapMethod; 2] = [ShrinkwrapMethod::Winding, ShrinkwrapMethod::Voxel];
}

/// How far the voxel method reduces its own output, before the attributes are
/// projected back on. Reducing first is what lets the simplifier work freely:
/// the bare shell has no UV or material seams yet to hold it back.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoxelTarget {
    /// Keep every triangle the remesher emits.
    Keep,
    /// A share of the object's own triangles before the wrap.
    #[default]
    Ratio,
    /// A triangle count per object.
    Triangles,
}

impl VoxelTarget {
    pub const ALL: [VoxelTarget; 3] = [
        VoxelTarget::Keep,
        VoxelTarget::Ratio,
        VoxelTarget::Triangles,
    ];
}

/// Settings for the [`crate::stack::OpKind::Shrinkwrap`] operation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShrinkwrapParams {
    /// Which extraction builds the shell. Every field below `method` up to the
    /// `voxel_*` ones belongs to [`ShrinkwrapMethod::Winding`].
    pub method: ShrinkwrapMethod,
    /// Voxels across the object's longest side. Everything about the result —
    /// how much detail survives, how small a gap gets bridged, how long it takes
    /// — follows from this one number, which is why it is the only size knob.
    ///
    /// It is also what the *cost* follows from, cubically, and the extraction
    /// emits roughly six triangles per surface cell. Measured on a real prop:
    /// 48 gives a 170 000-triangle shell in a quarter of a second, 64 gives
    /// 300 000 in half of one, and 128 gives **1.3 million** in two — dense
    /// enough that the quad solver Remesh was then built on gave up on it, and
    /// four times the work for anything below it still. Hence the default.
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
    /// [`ShrinkwrapMethod::Voxel`]: grid steps across the object's longest
    /// side, in the library's `4..=256`. Its own field rather than
    /// [`Self::resolution`] so switching methods never silently clamps a
    /// distance-field setting above 256.
    pub voxel_resolution: u32,
    /// [`ShrinkwrapMethod::Voxel`]: place vertices to fit the original's
    /// surface rather than on the grid. Upstream recommends it.
    pub voxel_solve: bool,
    /// [`ShrinkwrapMethod::Voxel`]: a two-sided skin around every surface
    /// instead of a filled solid. Doubles the triangles and can self-intersect;
    /// upstream calls it generally not recommended.
    pub voxel_shell: bool,
    /// [`ShrinkwrapMethod::Voxel`]: how far to reduce the shell before the
    /// attributes come back.
    pub voxel_target: VoxelTarget,
    /// [`VoxelTarget::Ratio`]: triangles to keep as a share of the object's own
    /// before the wrap — the same basis a Remesh ratio reads against.
    pub voxel_ratio: f32,
    /// [`VoxelTarget::Triangles`]: triangles to keep, per object.
    pub voxel_triangles: u32,
    /// [`ShrinkwrapMethod::Voxel`]: reduce toward evenly shaped triangles, at a
    /// small cost to accuracy.
    pub voxel_regularize: bool,
}

impl ShrinkwrapParams {
    /// Switch method, resetting the normals to that method's default: a voxel
    /// shell's projected normals merge a thin sheet's two sides, so it generates
    /// them (with enough smoothing to hide the voxel steps), while the distance
    /// field keeps projecting as it always has.
    pub fn set_method(&mut self, method: ShrinkwrapMethod) {
        if self.method == method {
            return;
        }
        self.method = method;
        match method {
            ShrinkwrapMethod::Winding => {
                self.normals = NormalMode::Project;
                self.normal_params = NormalParams::default();
            }
            ShrinkwrapMethod::Voxel => {
                self.normals = NormalMode::Generate;
                self.normal_params = NormalParams {
                    smoothing: NormalParams::VOXEL_SMOOTHING,
                    ..NormalParams::default()
                };
            }
        }
    }
}

impl Default for ShrinkwrapParams {
    fn default() -> Self {
        Self {
            // Measured rather than chosen: at 64 a prop keeps its silhouette,
            // the wrap takes half a second, and — the part that matters — the
            // shell is clean enough for a Remesh below it to even out. At
            // 128 the same prop's shell is four times the triangles, which the
            // quad solver Remesh was then built on gave up on outright — the
            // whole pairing failing — and which still costs four times as much
            // to rebuild.
            resolution: 64,
            offset: 0.0,
            keep_largest_shell: true,
            normals: NormalMode::Project,
            normal_params: NormalParams::default(),
            method: ShrinkwrapMethod::Winding,
            // meshoptimizer's own example resolution: enough to keep a prop's
            // features, and the built-in target reduces what it emits.
            voxel_resolution: 100,
            voxel_solve: true,
            voxel_shell: false,
            // Like for like: as many triangles as the object had. A wrap is
            // rarely wanted denser than its source.
            voxel_target: VoxelTarget::Ratio,
            voxel_ratio: 1.0,
            voxel_triangles: 5_000,
            voxel_regularize: false,
        }
    }
}
