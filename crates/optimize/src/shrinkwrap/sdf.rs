//! Building the narrow-band signed distance field: deciding which blocks exist,
//! then filling them.
//!
//! "Narrow band" is the whole reason a wrap at a useful resolution fits in
//! memory and finishes in seconds. Only the lattice near the surface is
//! allocated or evaluated; everything else is implicitly outside the band, which
//! is all the extraction needs (see [`super::grid`]).
//!
//! ## The surface is not only where the geometry is
//!
//! Near the triangles is enough for the shell that *follows* the geometry. It is
//! not enough for the part that closes a gap: the field's sign is the winding
//! number's (see [`super::winding`]), and across an opening - the mouth of a
//! cup, a box with a face missing - that crosses ½ on a surface spanning the
//! opening, as far from any triangle as the opening is wide. Blocks chosen by
//! triangle alone held only a ring of that cap near the rim, and the extraction
//! skips a cell with a corner in a block nobody allocated, so the "watertight"
//! shell came back with a hole the width of the opening less a few voxels.
//!
//! So a second pass reads the inside test once per block corner, which is
//! 1/512th of the lattice, and allocates every block whose corners disagree:
//! one the sign surface passes through. A plane through a cube always has
//! corners on both sides of it, so a cap that is smooth at the scale of a block
//! is found in every block it crosses.
//!
//! Every other block gets a value too, without being allocated. A block the
//! geometry does not reach has no triangle within the band's reach of any of its
//! points, so what [`voxel::sample`] would write at each of them is the band
//! itself, signed by the inside test - one number for the whole block, which its
//! agreeing corners already gave. Stored as that (see [`Field::set_uniform`]),
//! it means **every cell of the lattice has eight values**, and an extraction
//! over a field with no holes in it is closed by construction: there is no cell
//! left for a patch to stop at. A cap that bulges into a block between its
//! corners is the one thing the uniform value gets wrong, and it gets the cap's
//! shape wrong there, not its closure.

use glam::{IVec3, Vec3};
use review_model::{Bvh, ModelData};

use crate::cancel::{CancelToken, cancelled};

use super::grid::{BLOCK, Field, GridDesc};
use super::voxel::{self, Source};
use super::winding::Tree;

/// Sample the field around `model`'s surface, or `None` when the run was
/// cancelled part way.
///
/// The blocks near the geometry are chosen from the triangles' own bounding
/// boxes rather than by walking the lattice: a triangle knows where it is, and
/// the alternative is a distance query per lattice point of the whole box to find
/// out that almost all of them are empty. The rest of the sign surface, and the
/// value of every block left unallocated, come from [`block_signs`].
pub(super) fn build(
    desc: GridDesc,
    model: &ModelData,
    bvh: &Bvh,
    winding: &Tree,
    offset: f32,
    threads: usize,
    cancel: Option<&CancelToken>,
) -> Option<Field> {
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
    if cancelled(cancel) {
        return None;
    }
    let signs = block_signs(&desc, winding, threads);
    if cancelled(cancel) {
        return None;
    }
    for (touch, sign) in touched.iter_mut().zip(&signs) {
        if sign.is_none() {
            *touch = true;
        }
    }

    let mut field = Field::new(desc, &touched);
    for (index, (&touch, sign)) in touched.iter().zip(&signs).enumerate() {
        if let (false, Some(inside)) = (touch, *sign) {
            field.set_uniform(index, inside);
        }
    }
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
        return Some(field);
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
                    // A block at a time: 512 distance and winding queries is a
                    // fraction of a millisecond, and the fill is most of a wrap.
                    if cancelled(cancel) {
                        return;
                    }
                    voxel::fill_block(&desc, &source, block * BLOCK, BLOCK, values);
                }
            });
        }
    });

    (!cancelled(cancel)).then_some(field)
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

/// Per block, in block-lattice order: which side of the winding number's ½
/// surface it is on when all eight corners of its cube agree, or `None` when
/// they do not and the surface passes through it. See the module docs.
///
/// One inside test per block corner, in parallel: each answer is a pure
/// function of where the corner is, so the result cannot depend on scheduling.
fn block_signs(desc: &GridDesc, winding: &Tree, threads: usize) -> Vec<Option<bool>> {
    let blocks = desc.block_dims();
    // Block corners: one more than the blocks along each axis, the last one
    // closing the final block's cube even where it hangs past the lattice.
    let corners = blocks + IVec3::ONE;
    let corner_index = |at: IVec3| {
        (at.z as usize * corners.y as usize + at.y as usize) * corners.x as usize + at.x as usize
    };
    let mut inside = vec![false; corners.element_product() as usize];
    crate::parallel::sweep(&mut inside, threads, |base, chunk| {
        for (offset, slot) in chunk.iter_mut().enumerate() {
            let index = base + offset;
            let at = IVec3::new(
                (index % corners.x as usize) as i32,
                ((index / corners.x as usize) % corners.y as usize) as i32,
                (index / (corners.x as usize * corners.y as usize)) as i32,
            );
            *slot = winding.is_inside(desc.position(at * BLOCK));
        }
    });

    let mut signs = Vec::with_capacity(blocks.element_product() as usize);
    for z in 0..blocks.z {
        for y in 0..blocks.y {
            for x in 0..blocks.x {
                let block = IVec3::new(x, y, z);
                let first = inside[corner_index(block)];
                let crossed = (1..8).any(|corner| {
                    let step = IVec3::new(corner & 1, (corner >> 1) & 1, (corner >> 2) & 1);
                    inside[corner_index(block + step)] != first
                });
                signs.push((!crossed).then_some(first));
            }
        }
    }
    signs
}

#[cfg(test)]
mod tests {
    use review_model::{Bounds, Vertex, demo_cube_model};

    use super::*;
    use crate::remesh::proxy;
    use crate::submesh::partition as split;

    fn cube_parts() -> (ModelData, Bvh, Tree, Bounds) {
        box_parts(false)
    }

    /// The demo cube, closed or with its top face taken away.
    fn box_parts(open_top: bool) -> (ModelData, Bvh, Tree, Bounds) {
        let source = demo_cube_model();
        let (pieces, _) = split(&source, None);
        let borrowed: Vec<_> = pieces.iter().collect();
        let mut proxy = proxy::build(&borrowed);
        if open_top {
            let top = proxy.bounds.max.y;
            let height = |vertex: u32| proxy.positions[vertex as usize * 3 + 1];
            let kept: Vec<u32> = proxy
                .indices
                .as_chunks::<3>()
                .0
                .iter()
                .filter(|corners| corners.iter().any(|&corner| height(corner) < top))
                .flatten()
                .copied()
                .collect();
            proxy.indices = kept;
        }
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

    /// The blocks the triangles alone reach, as the first pass of [`build`]
    /// counts them.
    fn blocks_near_the_geometry(desc: &GridDesc, model: &ModelData) -> usize {
        let block_dims = desc.block_dims();
        let mut touched = vec![false; block_dims.element_product() as usize];
        let reach = desc.band() + desc.voxel;
        for triangle in 0..(model.indices.len() / 3) as u32 {
            let corners = review_model::triangle_positions(model, triangle);
            let lo = corners[0].min(corners[1]).min(corners[2]) - Vec3::splat(reach);
            let hi = corners[0].max(corners[1]).max(corners[2]) + Vec3::splat(reach);
            mark_blocks(desc, block_dims, lo, hi, &mut touched);
        }
        touched.iter().filter(|&&touch| touch).count()
    }

    /// Every directed edge used once, and its reverse once: closed and wound
    /// one way.
    fn assert_closed(surface: &super::super::mc::Surface) {
        use std::collections::HashSet;
        let mut directed: HashSet<(u32, u32)> = HashSet::new();
        for corners in surface.indices.as_chunks::<3>().0 {
            for corner in 0..3 {
                let edge = (corners[corner], corners[(corner + 1) % 3]);
                assert!(directed.insert(edge), "edge {edge:?} is used twice one way");
            }
        }
        let open = directed
            .iter()
            .filter(|&&(a, b)| !directed.contains(&(b, a)))
            .count();
        assert_eq!(open, 0, "{open} edges have no face on their other side");
    }

    /// A box with a face missing still wraps to one closed shell: the cap across
    /// the opening is where the winding number crosses ½, a long way from any
    /// triangle, and it used to come back as a ring round the rim with a hole
    /// the width of the opening in it.
    #[test]
    fn an_opening_wider_than_the_band_is_capped() {
        let (model, bvh, winding, bounds) = box_parts(true);
        // The opening is the cube's whole top: 64 voxels across, eight blocks,
        // where the band reaches three voxels in from each wall.
        let desc = GridDesc::cover(bounds, 64, 0.0);

        let field = build(desc, &model, &bvh, &winding, 0.0, 4, None).expect("nothing cancels it");
        let surface = super::super::mc::extract(&field, None).expect("nothing cancels it");

        assert!(surface.triangle_count() > 0);
        assert_closed(&surface);
        // And the cap really is across the top: something sits over the
        // middle of the opening, nowhere near a wall.
        let middle = Vec3::new(bounds.center().x, bounds.max.y, bounds.center().z);
        assert!(
            surface
                .positions
                .iter()
                .any(|point| (*point - middle).length() < desc.voxel * 4.0),
            "nothing caps the middle of the opening"
        );
    }

    /// Closing an opening must not cost a closed object anything: its sign
    /// changes only where its triangles already are.
    #[test]
    fn a_closed_object_allocates_only_near_its_geometry() {
        let (model, bvh, winding, bounds) = cube_parts();
        let desc = GridDesc::cover(bounds, 64, 0.0);

        let field = build(desc, &model, &bvh, &winding, 0.0, 4, None).expect("nothing cancels it");

        assert_eq!(
            field.allocated_blocks(),
            blocks_near_the_geometry(&desc, &model)
        );
        let surface = super::super::mc::extract(&field, None).expect("nothing cancels it");
        assert_closed(&surface);
    }

    #[test]
    fn a_cancelled_sampling_returns_nothing() {
        use std::sync::Arc;
        use std::sync::atomic::AtomicU64;
        let (model, bvh, winding, bounds) = cube_parts();
        let desc = GridDesc::cover(bounds, 32, 0.0);
        let dead = CancelToken::new(Arc::new(AtomicU64::new(2)), 1);

        assert!(build(desc, &model, &bvh, &winding, 0.0, 4, Some(&dead)).is_none());
    }

    #[test]
    fn only_the_blocks_near_the_surface_are_allocated() {
        let (model, bvh, winding, bounds) = cube_parts();
        let desc = GridDesc::cover(bounds, 48, 0.0);

        let field = build(desc, &model, &bvh, &winding, 0.0, 4, None).expect("nothing cancels it");

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

        let field = build(desc, &model, &bvh, &winding, 0.0, 4, None).expect("nothing cancels it");

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
