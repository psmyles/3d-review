//! One lattice point's value, as a pure function of where it is.
//!
//! Deliberately shaped this way rather than as a loop that fills the grid: the
//! sampling is the expensive half of a wrap, it is embarrassingly parallel, and
//! the day it moves to a compute shader the kernel is this function with the
//! index arithmetic already separated out. Today the caller runs it over a block
//! at a time on a worker thread; nothing about the result depends on which.

use glam::IVec3;
use review_model::{Bvh, ModelData};

use super::grid::GridDesc;
use super::winding::Tree;

/// What the sampling pass reads. All of it immutable, which is what makes the
/// parallel fill sound without a lock — and `Copy`, so every worker gets its own
/// handle on the same borrowed data.
#[derive(Clone, Copy)]
pub(super) struct Source<'a> {
    /// The welded proxy, as a model the hierarchy can be queried against.
    pub model: &'a ModelData,
    pub bvh: &'a Bvh,
    pub winding: &'a Tree,
    /// Pushed out (or, negative, pulled in) by this much, in world units.
    pub offset: f32,
}

/// The signed distance at lattice point `point`, clamped to the band.
///
/// Negative inside. The magnitude is exact within the band and clamped outside
/// it, which is all the extraction reads: a cell that crosses the surface has
/// every corner inside the band (see [`super::grid`]), so no interpolation ever
/// touches a clamped value.
///
/// The **sign** is the generalized winding number's, not the nearest triangle's
/// facing. That is the whole point of the operation: a kitbash has no consistent
/// facing to read, and a normal-based sign would put the boundary of the shell
/// wherever the nearest scrap of geometry happened to point.
pub(super) fn sample(desc: &GridDesc, source: &Source<'_>, point: IVec3) -> f32 {
    let query = desc.position(point);
    // Enough range that the clamp below is the only thing that truncates: the
    // offset moves the zero crossing, so the band has to reach past it.
    let range = desc.band() + source.offset.abs() + desc.voxel;
    let distance = match source.bvh.closest_point(source.model, query, range) {
        Some(hit) => hit.distance_squared.max(0.0).sqrt(),
        None => range,
    };
    let signed = if source.winding.is_inside(query) {
        -distance
    } else {
        distance
    };
    // A positive offset pushes the zero crossing outward, which is a *subtraction*
    // from the signed distance: the shell moves to where the distance equals the
    // offset.
    let band = desc.band();
    (signed - source.offset).clamp(-band, band)
}

/// Every lattice point of one block, filled in place.
///
/// `values` is the block's `BLOCK³` slice in x-fastest order — the layout
/// [`super::grid::Field`] hands out — and `base` is the lattice coordinate of
/// its `(0, 0, 0)` corner. Points past the lattice's edge are left as they came
/// (`NaN`), so the extraction skips the cells that would have used them.
pub(super) fn fill_block(
    desc: &GridDesc,
    source: &Source<'_>,
    base: IVec3,
    block: i32,
    values: &mut [f32],
) {
    for z in 0..block {
        for y in 0..block {
            for x in 0..block {
                let point = base + IVec3::new(x, y, z);
                if point.cmpge(desc.dims).any() {
                    continue;
                }
                let slot = ((z * block + y) * block + x) as usize;
                values[slot] = sample(desc, source, point);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;
    use review_model::demo_cube_model;

    use super::*;
    use crate::remesh::proxy;
    use crate::shrinkwrap::grid::{BLOCK, GridDesc};
    use crate::submesh::partition as split;

    /// The world position of a lattice point from a flat index into a dense
    /// lattice of `desc.dims` — the other half of "a pure function of an
    /// index", which the tests below walk the lattice with.
    fn point_of_index(desc: &GridDesc, index: usize) -> IVec3 {
        let width = desc.dims.x as usize;
        let height = desc.dims.y as usize;
        IVec3::new(
            (index % width) as i32,
            ((index / width) % height) as i32,
            (index / (width * height)) as i32,
        )
    }

    struct Fixture {
        model: ModelData,
        bvh: Bvh,
        winding: Tree,
        desc: GridDesc,
    }

    fn cube() -> Fixture {
        let source = demo_cube_model();
        let (pieces, _) = split(&source, None);
        let borrowed: Vec<_> = pieces.iter().collect();
        let proxy = proxy::build(&borrowed);
        let mut model = ModelData::default();
        for values in proxy.positions.as_chunks::<3>().0 {
            model.vertices.push(review_model::Vertex {
                position: Vec3::new(values[0], values[1], values[2]),
                ..review_model::Vertex::default()
            });
        }
        model.indices = proxy.indices.clone();
        let bvh = Bvh::build(&model);
        let winding = Tree::build(&proxy.positions, &proxy.indices);
        let desc = GridDesc::cover(proxy.bounds, 16, 0.0);
        Fixture {
            model,
            bvh,
            winding,
            desc,
        }
    }

    #[test]
    fn a_point_inside_the_surface_samples_negative() {
        let fixture = cube();
        let source = Source {
            model: &fixture.model,
            bvh: &fixture.bvh,
            winding: &fixture.winding,
            offset: 0.0,
        };
        // The lattice's middle point is inside the cube it was built around.
        let middle = fixture.desc.dims / 2;

        assert!(
            sample(&fixture.desc, &source, middle) < 0.0,
            "the middle of the object is inside it"
        );
        assert!(
            sample(&fixture.desc, &source, IVec3::ZERO) > 0.0,
            "the lattice's padded corner is outside it"
        );
    }

    #[test]
    fn voxel_sample_is_a_pure_function_of_its_index() {
        let fixture = cube();
        let source = Source {
            model: &fixture.model,
            bvh: &fixture.bvh,
            winding: &fixture.winding,
            offset: 0.0,
        };

        // What a block fill writes has to equal what a point-at-a-time walk
        // does — which is what lets the fill be split across threads however the
        // scheduler likes, and what a compute-shader port would have to match.
        let mut values = vec![f32::NAN; (BLOCK * BLOCK * BLOCK) as usize];
        let base = IVec3::new(BLOCK, BLOCK, BLOCK);
        fill_block(&fixture.desc, &source, base, BLOCK, &mut values);

        for z in 0..BLOCK {
            for y in 0..BLOCK {
                for x in 0..BLOCK {
                    let point = base + IVec3::new(x, y, z);
                    if point.cmpge(fixture.desc.dims).any() {
                        continue;
                    }
                    let slot = ((z * BLOCK + y) * BLOCK + x) as usize;
                    assert_eq!(
                        values[slot],
                        sample(&fixture.desc, &source, point),
                        "block fill and point sample disagree at {point:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn an_offset_moves_the_surface_outward() {
        let fixture = cube();
        let plain = Source {
            model: &fixture.model,
            bvh: &fixture.bvh,
            winding: &fixture.winding,
            offset: 0.0,
        };
        let grown = Source {
            offset: fixture.desc.voxel,
            ..plain
        };
        // A point just outside the surface: positive without an offset, and
        // inside the grown shell with one.
        let mut outside = None;
        for index in 0..fixture.desc.point_count() {
            let point = point_of_index(&fixture.desc, index);
            let value = sample(&fixture.desc, &plain, point);
            if value > 0.0 && value < fixture.desc.voxel * 0.9 {
                outside = Some(point);
                break;
            }
        }
        let outside = outside.expect("some lattice point sits just outside the cube");

        assert!(sample(&fixture.desc, &plain, outside) > 0.0);
        assert!(
            sample(&fixture.desc, &grown, outside) < sample(&fixture.desc, &plain, outside),
            "growing the shell moves the zero crossing past this point"
        );
    }
}
