//! The voxel method: meshoptimizer's remesher as a Shrinkwrap extraction.
//!
//! It stands in for the distance field + marching tetrahedra of the default
//! method and hands back the same [`Surface`], so everything after extraction —
//! keeping the largest shell, projecting the attributes back, the per-material
//! split — is shared. Two things are its own:
//!
//! * **The weld.** The library emits an unindexed soup, three positions per
//!   triangle. It is welded by exact position here, since keeping the largest
//!   shell and simplifying both need shared vertices to see connectivity.
//! * **The reduction.** A voxel shell is dense, and reducing it *before* the
//!   attributes come back is what lets the simplifier work freely: the bare
//!   shell has no UV or material seams yet, which is exactly what would stall a
//!   Reduce placed below the Shrinkwrap. `PreserveFolds` keeps the rim of a thin
//!   sheet — which the remesher emits as two coincident, opposite-facing
//!   surfaces — from eroding as it goes.

use glam::Vec3;

use crate::Warnings;
use crate::meshopt::{self, SimplifyAttributes, options};
use crate::notice::OptWarning;
use crate::remesh::proxy::Proxy;
use crate::stack::{ShrinkwrapParams, VoxelTarget};

use super::mc::Surface;

/// The largest shell the counting pass may promise before the resolution is
/// lowered: about 150 MB of soup, over the grid itself. Halving the resolution
/// roughly quarters the output, so this converges in a step or two.
const VOXEL_MAX_TRIANGLES: usize = 4_000_000;

/// The lowest resolution the memory guard will fall back to.
const VOXEL_GUARD_FLOOR: u32 = 16;

/// Fewest triangles a reduction may aim for: below this there is no closed
/// shell left to speak of.
const MIN_TARGET_TRIANGLES: usize = 12;

/// Remesh the proxy into a welded shell, and the resolution it actually ran at.
/// `None` (with a warning) when the library refused.
pub(super) fn extract(
    proxy: &Proxy,
    params: &ShrinkwrapParams,
    name: &str,
    warnings: &mut Warnings,
) -> Option<(Surface, u32)> {
    let _z = crate::prof::zone!("Voxel Remesh");
    let mut flags = 0;
    if params.voxel_solve {
        flags |= options::remesh::SOLVE;
    }
    if params.voxel_shell {
        flags |= options::remesh::SHELL;
    }
    let vertex_count = proxy.positions.len() / 3;
    let refused = |error: crate::OptError, warnings: &mut Warnings| {
        warnings.push(OptWarning::ShrinkwrapVoxelFailed {
            object: name.to_string(),
            detail: error.to_string(),
        });
    };

    let requested = params
        .voxel_resolution
        .clamp(meshopt::VOXEL_RESOLUTION_MIN, meshopt::VOXEL_RESOLUTION_MAX);
    let mut resolution = requested;
    let bound_at = |resolution: u32| {
        meshopt::remesh_bound(
            &proxy.indices,
            &proxy.positions,
            vertex_count,
            resolution,
            flags,
        )
    };
    let mut bound = match bound_at(resolution) {
        Ok(bound) => bound,
        Err(error) => {
            refused(error, warnings);
            return None;
        }
    };
    while bound > VOXEL_MAX_TRIANGLES && resolution > VOXEL_GUARD_FLOOR {
        resolution = (resolution / 2).max(VOXEL_GUARD_FLOOR);
        bound = match bound_at(resolution) {
            Ok(bound) => bound,
            Err(error) => {
                refused(error, warnings);
                return None;
            }
        };
    }
    if resolution != requested {
        warnings.push(OptWarning::ShrinkwrapResolutionLowered {
            object: name.to_string(),
            requested,
            resolution,
        });
    }

    let soup = match meshopt::remesh(
        &proxy.indices,
        &proxy.positions,
        vertex_count,
        resolution,
        flags,
        bound,
    ) {
        Ok(soup) => soup,
        Err(error) => {
            refused(error, warnings);
            return None;
        }
    };
    Some((weld_soup(&soup), resolution))
}

/// Weld an unindexed triangle soup (nine floats per triangle) by exact
/// position, dropping any triangle the weld collapses.
fn weld_soup(soup: &[f32]) -> Surface {
    let corners = soup.len() / 3;
    if corners < 3 {
        return Surface::default();
    }
    // Exact bits, with -0 folded onto +0 so the two spellings of one point merge.
    let mut key = Vec::with_capacity(corners * 12);
    for value in soup {
        let value = if *value == 0.0 { 0.0f32 } else { *value };
        key.extend_from_slice(&value.to_bits().to_ne_bytes());
    }
    let identity: Vec<u32> = (0..corners as u32).collect();
    let Ok((remap, unique)) = meshopt::generate_vertex_remap(&identity, &key, corners, 12) else {
        return Surface::default();
    };

    let mut positions = vec![Vec3::ZERO; unique];
    for (corner, &slot) in remap.iter().enumerate() {
        positions[slot as usize] =
            Vec3::new(soup[corner * 3], soup[corner * 3 + 1], soup[corner * 3 + 2]);
    }
    let mut indices = Vec::with_capacity(corners);
    for triangle in remap.as_chunks::<3>().0 {
        let [a, b, c] = *triangle;
        if a != b && b != c && a != c {
            indices.extend_from_slice(triangle);
        }
    }
    Surface { positions, indices }
}

/// The triangle count the voxel method's target asks for, or `None` to keep
/// the shell as it came.
pub(super) fn target_triangles(proxy: &Proxy, params: &ShrinkwrapParams) -> Option<usize> {
    let target = match params.voxel_target {
        VoxelTarget::Keep => return None,
        VoxelTarget::Ratio => (proxy.source_triangles as f64
            * f64::from(params.voxel_ratio.max(0.0)))
        .round() as usize,
        VoxelTarget::Triangles => params.voxel_triangles as usize,
    };
    Some(target.max(MIN_TARGET_TRIANGLES))
}

/// Simplify the bare shell toward `target` triangles, positions only. The
/// error limit is left wide open — the target is the only stop, as it is in
/// upstream's own remesh pipeline.
pub(super) fn reduce(surface: &mut Surface, target: usize, regularize: bool) {
    let _z = crate::prof::zone!("Reduce Voxel Shell");
    if surface.triangle_count() <= target {
        return;
    }
    let positions: Vec<f32> = surface
        .positions
        .iter()
        .flat_map(|p| p.to_array())
        .collect();
    let mut flags = options::simplify::PRESERVE_FOLDS;
    if regularize {
        flags |= options::simplify::REGULARIZE_LIGHT;
    }
    if let Ok(outcome) = meshopt::simplify(
        &surface.indices,
        &positions,
        surface.positions.len(),
        &SimplifyAttributes::default(),
        target * 3,
        1.0,
        flags,
    ) && !outcome.indices.is_empty()
    {
        surface.indices = outcome.indices;
    }
}
