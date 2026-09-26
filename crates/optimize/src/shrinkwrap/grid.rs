//! The lattice the distance field is sampled on, and the block-sparse storage
//! that holds it.
//!
//! ## Why sparse
//!
//! A dense lattice at the resolutions that keep a prop's silhouette is mostly
//! empty: at 256 across, a thin-walled object touches a percent or two of the
//! 16 million points. Storing the rest costs hundreds of megabytes to hold the
//! same two numbers everywhere (`+band`, and a sign nothing reads).
//!
//! So the lattice is cut into 8×8×8 **blocks**, and a block exists only where
//! some triangle's bounding box — dilated by the band, so the surface's whole
//! neighbourhood is covered — reaches it. Everything outside is *implicitly*
//! farther than the band, which is all the extraction needs to know: a cell that
//! crosses the surface has all eight corners within a voxel diagonal of it, and
//! the band is wider than that, so such a cell is always fully allocated.

use glam::{IVec3, Vec3};
use review_model::Bounds;

/// Points per block along each axis. 512 points per block: big enough that the
/// per-block bookkeeping disappears, small enough that a thin shell does not
/// drag much empty space in with it.
pub(super) const BLOCK: i32 = 8;

/// How far from the surface the field is evaluated, in voxels.
///
/// Two, because a cell that crosses the surface has corners at most one voxel
/// diagonal (≈1.74 voxels) away from it — so a two-voxel band guarantees every
/// such corner is inside it and carries a real distance rather than a clamp.
pub(super) const BAND_VOXELS: f32 = 2.0;

/// Most lattice points a run will allocate. Past this the wrap is asking for
/// more memory than the result could justify, and the caller lowers the
/// resolution and says so rather than trying.
pub(super) const MAX_POINTS: usize = 64 * 1024 * 1024;

/// Where the lattice sits and how fine it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct GridDesc {
    /// World position of lattice point `(0, 0, 0)`.
    pub origin: Vec3,
    /// Edge length of one cell, in world units. Cubic: an anisotropic voxel
    /// would make the same feature survive along one axis and not another.
    pub voxel: f32,
    /// Lattice **points** along each axis; there is one fewer cell.
    pub dims: IVec3,
}

impl GridDesc {
    /// A lattice covering `bounds` with three voxels of padding on every side.
    ///
    /// The padding is what lets the shell close around the object: marching
    /// cubes only emits a face where the sign changes *inside* a cell, so a
    /// surface flush against the lattice's edge would come back open. Three
    /// voxels is the band plus one.
    ///
    /// `resolution` counts voxels across the object's **longest** side, so the
    /// same number means the same amount of detail whatever the object's shape.
    pub(super) fn cover(bounds: Bounds, resolution: u32, offset: f32) -> Self {
        let resolution = resolution.max(4) as f32;
        let size = bounds.size();
        // An offset pushes the surface outward, so the lattice has to reach that
        // much farther or the offset shell would be cut off at the edge.
        let grown = size.max_element().max(1.0e-6) + offset.max(0.0) * 2.0;
        let voxel = grown / resolution;
        let padding = Vec3::splat(voxel * 3.0 + offset.max(0.0));
        let origin = bounds.min - padding;
        let extent = size + padding * 2.0;
        let dims = IVec3::new(
            (extent.x / voxel).ceil() as i32 + 1,
            (extent.y / voxel).ceil() as i32 + 1,
            (extent.z / voxel).ceil() as i32 + 1,
        )
        .max(IVec3::splat(2));
        Self {
            origin,
            voxel,
            dims,
        }
    }

    /// World position of lattice point `point`.
    pub(super) fn position(&self, point: IVec3) -> Vec3 {
        self.origin + point.as_vec3() * self.voxel
    }

    /// The band's width in world units.
    pub(super) fn band(&self) -> f32 {
        self.voxel * BAND_VOXELS
    }

    /// Lattice points this grid would hold if it were dense — the figure
    /// [`MAX_POINTS`] bounds.
    pub(super) fn point_count(&self) -> usize {
        self.dims.x as usize * self.dims.y as usize * self.dims.z as usize
    }

    /// Blocks along each axis.
    pub(super) fn block_dims(&self) -> IVec3 {
        // `i32::div_ceil` is still unstable, and the dimensions are positive
        // by construction, so the plain form is exact here.
        let ceil = |value: i32| (value + BLOCK - 1) / BLOCK;
        IVec3::new(ceil(self.dims.x), ceil(self.dims.y), ceil(self.dims.z))
    }
}

/// The signed distances, held one block at a time.
///
/// A block is `BLOCK³` values in x-fastest order. `blocks` is dense over the
/// block lattice and holds an index into `values`, or [`u32::MAX`] where no
/// block was allocated — one `u32` per 512 points, which is small enough to keep
/// dense even at the maximum resolution.
pub(super) struct Field {
    pub(super) desc: GridDesc,
    block_dims: IVec3,
    blocks: Vec<u32>,
    /// `BLOCK³` values per allocated block.
    values: Vec<f32>,
}

impl Field {
    /// Allocate every block `touched` marks, in block-lattice order.
    ///
    /// Values start at `NaN`, which is never a distance: it is what
    /// [`Self::get`] reads as "not evaluated", so a point the sampling pass
    /// somehow skipped fails the extraction's own check rather than passing as a
    /// plausible zero.
    pub(super) fn new(desc: GridDesc, touched: &[bool]) -> Self {
        let block_dims = desc.block_dims();
        let mut blocks = vec![u32::MAX; touched.len()];
        let mut allocated = 0u32;
        for (index, &touch) in touched.iter().enumerate() {
            if touch {
                blocks[index] = allocated;
                allocated += 1;
            }
        }
        let per_block = (BLOCK * BLOCK * BLOCK) as usize;
        Self {
            desc,
            block_dims,
            blocks,
            values: vec![f32::NAN; allocated as usize * per_block],
        }
    }

    pub(super) fn allocated_blocks(&self) -> usize {
        self.values.len() / (BLOCK * BLOCK * BLOCK) as usize
    }

    /// The block-lattice index of `block`, or `None` when it is off the lattice.
    pub(super) fn block_index(&self, block: IVec3) -> Option<usize> {
        if block.cmplt(IVec3::ZERO).any() || block.cmpge(self.block_dims).any() {
            return None;
        }
        Some(
            (block.z as usize * self.block_dims.y as usize + block.y as usize)
                * self.block_dims.x as usize
                + block.x as usize,
        )
    }

    /// Where point `point`'s value lives, or `None` when its block is not
    /// allocated.
    fn slot(&self, point: IVec3) -> Option<usize> {
        if point.cmplt(IVec3::ZERO).any() || point.cmpge(self.desc.dims).any() {
            return None;
        }
        let block = IVec3::new(point.x / BLOCK, point.y / BLOCK, point.z / BLOCK);
        let base = self.blocks[self.block_index(block)?];
        if base == u32::MAX {
            return None;
        }
        let inside = point - block * BLOCK;
        let offset = (inside.z * BLOCK + inside.y) * BLOCK + inside.x;
        Some(base as usize * (BLOCK * BLOCK * BLOCK) as usize + offset as usize)
    }

    /// The signed distance at `point`, or `None` outside the allocated blocks.
    pub(super) fn get(&self, point: IVec3) -> Option<f32> {
        let value = self.values[self.slot(point)?];
        (!value.is_nan()).then_some(value)
    }

    /// Every allocated block's lattice coordinate, in allocation order — which
    /// is also the order [`Self::block_values_mut`] hands them out, so a
    /// parallel pass can pair the two by index.
    pub(super) fn block_coords(&self) -> Vec<IVec3> {
        let mut coords = vec![IVec3::ZERO; self.allocated_blocks()];
        for (index, &base) in self.blocks.iter().enumerate() {
            if base == u32::MAX {
                continue;
            }
            let x = index % self.block_dims.x as usize;
            let y = (index / self.block_dims.x as usize) % self.block_dims.y as usize;
            let z = index / (self.block_dims.x as usize * self.block_dims.y as usize);
            coords[base as usize] = IVec3::new(x as i32, y as i32, z as i32);
        }
        coords
    }

    /// The value array split into one mutable slice per allocated block, in the
    /// same order as [`Self::block_coords`].
    ///
    /// Disjoint `&mut` slices, which is what lets the sampling pass run them in
    /// parallel with the borrow checker as the proof that it is sound.
    pub(super) fn block_values_mut(&mut self) -> Vec<&mut [f32]> {
        self.values
            .chunks_mut((BLOCK * BLOCK * BLOCK) as usize)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_bounds() -> Bounds {
        Bounds {
            min: Vec3::ZERO,
            max: Vec3::ONE,
        }
    }

    #[test]
    fn a_grid_pads_the_object_on_every_side() {
        let desc = GridDesc::cover(unit_bounds(), 16, 0.0);

        assert!(
            desc.origin.cmplt(Vec3::ZERO).all(),
            "the lattice starts outside the object: {:?}",
            desc.origin
        );
        let far = desc.position(desc.dims - IVec3::ONE);
        assert!(
            far.cmpgt(Vec3::ONE).all(),
            "the lattice ends outside the object: {far:?}"
        );
        // Three voxels of padding either side of a 16-voxel object.
        assert!((16..=24).contains(&desc.dims.x), "{:?}", desc.dims);
    }

    #[test]
    fn a_block_is_allocated_only_where_it_is_asked_for() {
        let desc = GridDesc::cover(unit_bounds(), 16, 0.0);
        let mut touched = vec![false; desc.block_dims().element_product() as usize];
        touched[0] = true;

        let field = Field::new(desc, &touched);

        assert_eq!(field.allocated_blocks(), 1);
        assert!(field.get(IVec3::ZERO).is_none(), "nothing is sampled yet");
        assert!(
            field.get(desc.dims - IVec3::ONE).is_none(),
            "the far corner's block was never allocated"
        );
        assert_eq!(field.block_coords(), vec![IVec3::ZERO]);
    }

    #[test]
    fn a_value_written_into_a_block_reads_back_at_its_point() {
        let desc = GridDesc::cover(unit_bounds(), 16, 0.0);
        let mut touched = vec![false; desc.block_dims().element_product() as usize];
        touched[0] = true;
        let mut field = Field::new(desc, &touched);

        // Point (3, 2, 1) of block (0, 0, 0), in the x-fastest layout.
        let offset = (BLOCK + 2) * BLOCK + 3;
        field.block_values_mut()[0][offset as usize] = 0.25;

        assert_eq!(field.get(IVec3::new(3, 2, 1)), Some(0.25));
        assert_eq!(field.get(IVec3::new(3, 2, 2)), None, "still NaN");
    }
}
