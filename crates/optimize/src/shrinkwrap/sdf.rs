//! Building the narrow-band signed distance field: deciding which blocks exist,
//! then filling them.
//!
//! "Narrow band" is the whole reason a wrap at a useful resolution fits in
//! memory and finishes in seconds. Only the lattice near the surface is
//! allocated or evaluated; everything else is implicitly outside the band, which
//! is all the extraction needs (see [`super::grid`]).

use glam::{IVec3, Vec3};
use review_model::{Bvh, ModelData};

use super::grid::{BLOCK, Field, GridDesc};
use super::voxel::{self, Source};
use super::winding::Tree;

/// Sample the field around `model`'s surface.
///
/// The blocks are chosen from the triangles' own bounding boxes rather than by
/// walking the lattice: a triangle knows where it is, and the alternative is a
/// distance query per lattice point of the whole box to find out that almost all
/// of them are empty.
pub(super) fn build(
    desc: GridDesc,
    model: &ModelData,
    bvh: &Bvh,
    winding: &Tree,
    offset: f32,
    threads: usize,
) -> Field {
    let _z = crate::prof::zone!("Shrinkwrap Sample");

    let block_dims = desc.block_dims();
    let mut touched = vec![false; block_dims.element_product() as usize];
    // The reach a triangle has to claim: the band the field is evaluated over,
    // plus whatever the offset moves the surface by, plus one voxel of slack so
    // a crossing never lands on an unallocated corner.
    let reach = desc.band() + offset.abs() + desc.voxel;
    for triangle in 0..(model.indices.len() / 3) as u32 {
        let corners = review_model::triangle_positions(model, triangle);
        let lo = corners[0].min(corners[1]).min(corners[2]) - Vec3::splat(reach);
        let hi = corners[0].max(corners[1]).max(corners[2]) + Vec3::splat(reach);
        mark_blocks(&desc, block_dims, lo, hi, &mut touched);
    }

    let mut field = Field::new(desc, &touched);
    let coords = field.block_coords();
    let source = Source {
        model,
        bvh,
        winding,
        offset,
    };

    // Deal the blocks round-robin into per-worker buckets before spawning: no
    // shared queue, no atomics, and — because a block's values are a pure
    // function of its own lattice points — scheduling cannot reach the result.
    // The borrow checker proves the slices disjoint, which is what makes this
    // sound without a lock.
    let mut jobs: Vec<(IVec3, &mut [f32])> =
        coords.into_iter().zip(field.block_values_mut()).collect();
    if jobs.is_empty() {
        return field;
    }
    // The object's own share of the machine; see `shrinkwrap_submeshes`.
    let workers = threads.min(jobs.len()).max(1);
    let mut buckets: Vec<Vec<(IVec3, &mut [f32])>> = (0..workers).map(|_| Vec::new()).collect();
    for (slot, job) in jobs.drain(..).enumerate() {
        buckets[slot % workers].push(job);
    }
    std::thread::scope(|scope| {
        for bucket in buckets {
            scope.spawn(move || {
                for (block, values) in bucket {
                    voxel::fill_block(&desc, &source, block * BLOCK, BLOCK, values);
                }
            });
        }
    });

    field
}

/// Mark every block the world-space box `[lo, hi]` reaches.
fn mark_blocks(desc: &GridDesc, block_dims: IVec3, lo: Vec3, hi: Vec3, touched: &mut [bool]) {
    let to_lattice = |point: Vec3| (point - desc.origin) / desc.voxel;
    let first = to_lattice(lo);
    let last = to_lattice(hi);
    let low = IVec3::new(
        (first.x.floor() as i32).div_euclid(BLOCK),
        (first.y.floor() as i32).div_euclid(BLOCK),
        (first.z.floor() as i32).div_euclid(BLOCK),
    )
    .max(IVec3::ZERO);
    let high = IVec3::new(
        (last.x.ceil() as i32).div_euclid(BLOCK),
        (last.y.ceil() as i32).div_euclid(BLOCK),
        (last.z.ceil() as i32).div_euclid(BLOCK),
    )
    .min(block_dims - IVec3::ONE);

    for z in low.z..=high.z {
        for y in low.y..=high.y {
            for x in low.x..=high.x {
                let index = (z as usize * block_dims.y as usize + y as usize)
                    * block_dims.x as usize
                    + x as usize;
                if let Some(slot) = touched.get_mut(index) {
                    *slot = true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use review_model::{Bounds, Vertex, demo_cube_model};

    use super::*;
    use crate::remesh::proxy;
    use crate::submesh::partition as split;

    fn cube_parts() -> (ModelData, Bvh, Tree, Bounds) {
        let source = demo_cube_model();
        let (pieces, _) = split(&source, None);
        let borrowed: Vec<_> = pieces.iter().collect();
        let proxy = proxy::build(&borrowed);
        let mut model = ModelData::default();
        for values in proxy.positions.as_chunks::<3>().0 {
            model.vertices.push(Vertex {
                position: Vec3::new(values[0], values[1], values[2]),
                ..Vertex::default()
            });
        }
        model.indices = proxy.indices.clone();
        let bvh = Bvh::build(&model);
        let winding = Tree::build(&proxy.positions, &proxy.indices);
        (model, bvh, winding, proxy.bounds)
    }

    #[test]
    fn only_the_blocks_near_the_surface_are_allocated() {
        let (model, bvh, winding, bounds) = cube_parts();
        let desc = GridDesc::cover(bounds, 48, 0.0);

        let field = build(desc, &model, &bvh, &winding, 0.0, 4);

        let dense = desc.block_dims().element_product() as usize;
        assert!(field.allocated_blocks() > 0, "the surface has blocks");
        assert!(
            field.allocated_blocks() <= dense,
            "more blocks than the lattice has"
        );
        // A hollow cube's shell is a fraction of its volume; the exact share
        // depends on the resolution, but it must not be all of it.
        assert!(
            field.allocated_blocks() < dense,
            "a narrow band allocated the whole lattice: {} of {dense}",
            field.allocated_blocks()
        );
    }

    #[test]
    fn the_sampled_field_changes_sign_across_the_surface() {
        let (model, bvh, winding, bounds) = cube_parts();
        let desc = GridDesc::cover(bounds, 32, 0.0);

        let field = build(desc, &model, &bvh, &winding, 0.0, 4);

        let mut negative = 0;
        let mut positive = 0;
        for z in 0..desc.dims.z {
            for y in 0..desc.dims.y {
                for x in 0..desc.dims.x {
                    match field.get(IVec3::new(x, y, z)) {
                        Some(value) if value < 0.0 => negative += 1,
                        Some(_) => positive += 1,
                        None => {}
                    }
                }
            }
        }
        assert!(negative > 0, "some sampled points are inside the cube");
        assert!(positive > 0, "and some are outside it");
    }
}
