//! Turning the sampled distance field back into a surface.
//!
//! ## Marching tetrahedra, not marching cubes
//!
//! Each cell is cut into six tetrahedra around its main diagonal, and each
//! tetrahedron is triangulated from its four corner signs. That is a different
//! choice from the textbook 256-case cube table, for one reason: **the result
//! has to be a closed manifold**, because the whole point of the operation is to
//! hand a quad solver something it can run on, and that is exactly what a closed
//! manifold means here.
//!
//! Marching cubes does not promise one. Its classic table is watertight but
//! leaves non-manifold *vertices* at the ambiguous cases — two cones meeting at
//! a point — and resolving those needs the extended 33-case table and a face
//! test. A tetrahedron has no ambiguous case at all: four corners, sixteen sign
//! patterns, one triangle or two, and no configuration where the surface could
//! have gone either way.
//!
//! The decomposition is consistent across cells, which is what makes the patches
//! meet. Every cell splits around the same corner-0-to-corner-7 diagonal, and on
//! any shared face the two cells' tetrahedra cut it along the same diagonal —
//! `the_cell_decomposition_agrees_across_a_shared_face` is that claim.
//!
//! The price is roughly twice the triangles of a cube-table extraction, and some
//! long thin ones. Neither matters: the shell exists to be remeshed or used as a
//! proxy, and a Remesh below the Shrinkwrap replaces every triangle it emits.

use std::collections::HashMap;

use glam::{IVec3, Vec3};

use super::grid::{Field, GridDesc};

/// The eight cell corners as `(x, y, z)` offsets, corner `i` being bit 0, 1 and
/// 2 of `i` — so corner 0 is the low corner and corner 7 the high one, and the
/// main diagonal is 0–7.
const CORNER_OFFSET: [IVec3; 8] = [
    IVec3::new(0, 0, 0),
    IVec3::new(1, 0, 0),
    IVec3::new(0, 1, 0),
    IVec3::new(1, 1, 0),
    IVec3::new(0, 0, 1),
    IVec3::new(1, 0, 1),
    IVec3::new(0, 1, 1),
    IVec3::new(1, 1, 1),
];

/// The six tetrahedra one cell is cut into, each naming four cell corners.
///
/// All six share the 0–7 diagonal and fan around it through the cell's other
/// six corners in a ring, which is what leaves no gaps and no overlaps. Each is
/// written in **positive** orientation (`(b-a) · ((c-a) × (d-a)) > 0`), which
/// `the_tetrahedra_are_positively_oriented` pins.
const TETRAHEDRA: [[usize; 4]; 6] = [
    [0, 7, 1, 3],
    [0, 7, 3, 2],
    [0, 7, 2, 6],
    [0, 7, 6, 4],
    [0, 7, 4, 5],
    [0, 7, 5, 1],
];

/// A surface extracted from the field.
#[derive(Debug, Default)]
pub(super) struct Surface {
    pub positions: Vec<Vec3>,
    /// Triangles over `positions`, wound so each normal points out of the solid.
    pub indices: Vec<u32>,
}

impl Surface {
    pub(super) fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub(super) fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

/// Extract the zero crossing of `field`.
///
/// Only cells whose eight corners all carry a sampled value are visited. That is
/// not a restriction in practice: a cell the surface passes through has every
/// corner within a voxel diagonal of it, the band is wider than that, and the
/// blocks cover the band — so a skipped cell is one the surface does not reach.
pub(super) fn extract(field: &Field) -> Surface {
    let _z = crate::prof::zone!("Shrinkwrap Extract");

    let desc = field.desc;
    let mut surface = Surface::default();
    // Every crossing is found once per edge of the lattice, so the edge is the
    // vertex's identity: two tetrahedra either side of it, in this cell or the
    // next, produce the same vertex and the patches join.
    let mut vertex_of_edge: HashMap<(u32, u32), u32> = HashMap::new();

    let cells = desc.dims - IVec3::ONE;
    for z in 0..cells.z {
        for y in 0..cells.y {
            for x in 0..cells.x {
                let base = IVec3::new(x, y, z);
                let mut corners = [IVec3::ZERO; 8];
                let mut values = [0.0f32; 8];
                let mut complete = true;
                for slot in 0..8 {
                    corners[slot] = base + CORNER_OFFSET[slot];
                    match field.get(corners[slot]) {
                        Some(value) => values[slot] = value,
                        None => {
                            complete = false;
                            break;
                        }
                    }
                }
                if !complete {
                    continue;
                }
                // Every corner on one side: the surface is elsewhere.
                let inside = values.iter().filter(|&&value| value < 0.0).count();
                if inside == 0 || inside == 8 {
                    continue;
                }
                for tetrahedron in TETRAHEDRA {
                    emit_tetrahedron(
                        &desc,
                        &corners,
                        &values,
                        tetrahedron,
                        &mut vertex_of_edge,
                        &mut surface,
                    );
                }
            }
        }
    }
    weld_coincident(&mut surface);
    surface
}

/// Merge crossings that landed on the same point, and drop the triangles that
/// collapse when they do.
///
/// A guard rather than a step: the inset in [`vertex_on_edge`] is what stops two
/// distinct lattice edges yielding the same point in the first place, and with
/// it this finds nothing. It stays because the alternative to finding nothing is
/// handing a zero-area triangle to whatever runs next, and a merge that *does*
/// fire is cheaper than that.
///
/// Merging by position turns a collapsed triangle into one with a repeated
/// index, and dropping it keeps the surface closed: its two real edges were the
/// same edge traversed both ways, so the neighbours that used them now pair with
/// each other.
fn weld_coincident(surface: &mut Surface) {
    let mut slot_of: HashMap<[u32; 3], u32> = HashMap::new();
    let mut positions: Vec<Vec3> = Vec::with_capacity(surface.positions.len());
    let mut remap: Vec<u32> = Vec::with_capacity(surface.positions.len());
    for &position in &surface.positions {
        let key = [
            position.x.to_bits(),
            position.y.to_bits(),
            position.z.to_bits(),
        ];
        let next = positions.len() as u32;
        let slot = *slot_of.entry(key).or_insert_with(|| {
            positions.push(position);
            next
        });
        remap.push(slot);
    }
    if positions.len() == surface.positions.len() {
        return;
    }

    let mut indices = Vec::with_capacity(surface.indices.len());
    for corners in surface.indices.as_chunks::<3>().0 {
        let welded = [
            remap[corners[0] as usize],
            remap[corners[1] as usize],
            remap[corners[2] as usize],
        ];
        if welded[0] == welded[1] || welded[1] == welded[2] || welded[0] == welded[2] {
            continue;
        }
        indices.extend_from_slice(&welded);
    }
    surface.positions = positions;
    surface.indices = indices;
}

/// One tetrahedron's contribution.
///
/// The winding is derived, not measured. Put the tetrahedron's corners in a
/// canonical order — the lone corner first for a 1–3 split, the two inside ones
/// first for a 2–2 — and the table below emits an outward-facing patch *provided
/// that reordering was an even permutation of a positively oriented tetrahedron*
/// ([`TETRAHEDRA`] guarantees the second half). An odd permutation mirrors the
/// tetrahedron, so the patch faces the other way and is reversed.
///
/// The obvious alternative — emit in any order, then flip whichever triangles
/// have a normal pointing the wrong way — does not work. These tetrahedra are
/// slivers, so a patch routinely contains a triangle whose normal is numerically
/// zero; the test's answer for that one is noise, and one wrong answer inside a
/// quad is a non-manifold edge down its diagonal.
fn emit_tetrahedron(
    desc: &GridDesc,
    corners: &[IVec3; 8],
    values: &[f32; 8],
    tetrahedron: [usize; 4],
    vertex_of_edge: &mut HashMap<(u32, u32), u32>,
    surface: &mut Surface,
) {
    let mut inside: Vec<usize> = Vec::with_capacity(4);
    let mut outside: Vec<usize> = Vec::with_capacity(4);
    for slot in tetrahedron {
        if values[slot] < 0.0 {
            inside.push(slot);
        } else {
            outside.push(slot);
        }
    }

    // The canonical order, and whether the table's own winding faces inward in
    // it. A lone *outside* corner is the mirror of a lone inside one: same
    // triangle, opposite side.
    let (order, mirrored): ([usize; 4], bool) = match (inside.len(), outside.len()) {
        (1, 3) => ([inside[0], outside[0], outside[1], outside[2]], false),
        (3, 1) => ([outside[0], inside[0], inside[1], inside[2]], true),
        (2, 2) => ([inside[0], inside[1], outside[0], outside[1]], false),
        // Every corner on one side: no surface here.
        _ => return,
    };

    // Reordering a positively oriented tetrahedron by an odd permutation mirrors
    // it, and mirrors the patch with it.
    let mut permutation = [0usize; 4];
    for (slot, &corner) in order.iter().enumerate() {
        permutation[slot] = tetrahedron
            .iter()
            .position(|&candidate| candidate == corner)
            .expect("the canonical order is a permutation of the tetrahedron");
    }
    let mut odd = false;
    for first in 0..4 {
        for second in first + 1..4 {
            if permutation[first] > permutation[second] {
                odd = !odd;
            }
        }
    }
    let flip = mirrored != odd;

    let mut crossing = |a: usize, b: usize| -> u32 {
        vertex_on_edge(
            desc,
            corners,
            values,
            order[a],
            order[b],
            vertex_of_edge,
            surface,
        )
    };

    let mut patch: Vec<[u32; 3]> = Vec::with_capacity(2);
    if inside.len() == 2 {
        // The quad separating `{0, 1}` from `{2, 3}`. Consecutive corners share
        // a tetrahedron vertex, which is what makes it a cycle and not a bow
        // tie; the fan below is therefore consistent within itself.
        let quad = [
            crossing(0, 2),
            crossing(0, 3),
            crossing(1, 3),
            crossing(1, 2),
        ];
        patch.push([quad[0], quad[1], quad[2]]);
        patch.push([quad[0], quad[2], quad[3]]);
    } else {
        patch.push([crossing(0, 1), crossing(0, 2), crossing(0, 3)]);
    }

    for triangle in patch {
        // A crossing that landed on the same vertex twice is a line, not a
        // surface: its two real edges are one edge traversed both ways, so
        // dropping it leaves the shell exactly as closed as it was.
        if triangle[0] == triangle[1] || triangle[1] == triangle[2] || triangle[0] == triangle[2] {
            continue;
        }
        if flip {
            surface
                .indices
                .extend_from_slice(&[triangle[0], triangle[2], triangle[1]]);
        } else {
            surface.indices.extend_from_slice(&triangle);
        }
    }
}

/// The vertex where the surface crosses the edge between cell corners `a` and
/// `b`, created on first use.
fn vertex_on_edge(
    desc: &GridDesc,
    corners: &[IVec3; 8],
    values: &[f32; 8],
    a: usize,
    b: usize,
    vertex_of_edge: &mut HashMap<(u32, u32), u32>,
    surface: &mut Surface,
) -> u32 {
    // Keyed on the two *lattice* points, in ascending order, so the same
    // crossing found from any of the cells or tetrahedra that share the edge is
    // the same vertex. Nothing else makes the patches join.
    let (first, second) = (
        lattice_index(desc, corners[a]),
        lattice_index(desc, corners[b]),
    );
    let (key, low, high) = if first < second {
        ((first, second), a, b)
    } else {
        ((second, first), b, a)
    };
    if let Some(&vertex) = vertex_of_edge.get(&key) {
        return vertex;
    }

    // Interpolated from the ordered pair, so the position is a function of the
    // edge rather than of which side reached it first — a difference of one
    // float would split the shell along that edge.
    //
    // Held strictly *inside* the edge. A lattice point whose value is exactly
    // zero — which axis-aligned geometry lands on constantly, since a crate's
    // face sits on a lattice plane — would otherwise put the crossing exactly on
    // it, and every edge meeting there would produce the same point. Merging
    // those coincident vertices is what turns a closed shell into one with
    // non-manifold edges; keeping them a hair apart means distinct edges always
    // yield distinct points, which is the property the extraction's closure
    // rests on. The displacement is a hundred-thousandth of a voxel.
    const INSET: f32 = 1.0e-5;
    let (va, vb) = (values[low], values[high]);
    let span = vb - va;
    let t = if span.abs() > f32::EPSILON {
        (-va / span).clamp(INSET, 1.0 - INSET)
    } else {
        0.5
    };
    let position = desc
        .position(corners[low])
        .lerp(desc.position(corners[high]), t);

    let vertex = surface.positions.len() as u32;
    surface.positions.push(position);
    vertex_of_edge.insert(key, vertex);
    vertex
}

/// A lattice point's index in the dense lattice — the identity an edge key is
/// built from.
fn lattice_index(desc: &GridDesc, point: IVec3) -> u32 {
    ((point.z * desc.dims.y + point.y) * desc.dims.x + point.x) as u32
}

/// Drop everything but the largest connected shell.
///
/// A wrap of a messy object routinely leaves satellites around specks of stray
/// geometry; none of them is wanted, and each is one more thing the quad solver
/// would refuse to run on. Connectivity is through shared *vertices*, which on
/// a welded extraction is the same as through shared edges.
pub(super) fn keep_largest_shell(surface: &mut Surface) {
    let triangles = surface.triangle_count();
    if triangles < 2 {
        return;
    }
    let mut parent: Vec<u32> = (0..surface.positions.len() as u32).collect();
    fn find(parent: &mut [u32], mut node: u32) -> u32 {
        while parent[node as usize] != node {
            parent[node as usize] = parent[parent[node as usize] as usize];
            node = parent[node as usize];
        }
        node
    }
    for corners in surface.indices.as_chunks::<3>().0 {
        let root = find(&mut parent, corners[0]);
        for &corner in &corners[1..] {
            let other = find(&mut parent, corner);
            if other != root {
                parent[other as usize] = root;
            }
        }
    }

    // The component with the most triangles wins; ties break on the lower root,
    // so the choice does not depend on the walk order.
    let mut counts: HashMap<u32, usize> = HashMap::new();
    for corners in surface.indices.as_chunks::<3>().0 {
        let root = find(&mut parent, corners[0]);
        *counts.entry(root).or_insert(0) += 1;
    }
    let Some(&keep) = counts
        .iter()
        .max_by_key(|&(root, &count)| (count, std::cmp::Reverse(*root)))
        .map(|(root, _)| root)
    else {
        return;
    };
    if counts.len() == 1 {
        return;
    }

    let mut indices = Vec::with_capacity(surface.indices.len());
    for corners in surface.indices.as_chunks::<3>().0 {
        if find(&mut parent, corners[0]) == keep {
            indices.extend_from_slice(corners);
        }
    }
    surface.indices = indices;

    // Drop the vertices nothing references any more, so the shell's own vertex
    // count is what it looks like.
    let mut remap = vec![u32::MAX; surface.positions.len()];
    let mut positions = Vec::new();
    for index in &mut surface.indices {
        let slot = &mut remap[*index as usize];
        if *slot == u32::MAX {
            *slot = positions.len() as u32;
            positions.push(surface.positions[*index as usize]);
        }
        *index = *slot;
    }
    surface.positions = positions;
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::shrinkwrap::grid::BLOCK;

    #[test]
    fn the_tetrahedra_are_positively_oriented() {
        for tetrahedron in TETRAHEDRA {
            let [a, b, c, d] = tetrahedron.map(|slot| CORNER_OFFSET[slot].as_vec3());
            let volume = (b - a).dot((c - a).cross(d - a));
            assert!(
                volume > 0.0,
                "{tetrahedron:?} is inside out or degenerate (volume {volume})"
            );
        }
        // Six tetrahedra of a sixth of the cell each: the decomposition covers
        // the cell exactly, with nothing left over and nothing counted twice.
        let total: f32 = TETRAHEDRA
            .iter()
            .map(|tetrahedron| {
                let [a, b, c, d] = tetrahedron.map(|slot| CORNER_OFFSET[slot].as_vec3());
                (b - a).dot((c - a).cross(d - a)) / 6.0
            })
            .sum();
        assert!((total - 1.0).abs() < 1.0e-6, "the six fill the unit cell");
    }

    #[test]
    fn the_cell_decomposition_agrees_across_a_shared_face() {
        // On the face between a cell and its +x neighbour, both have to cut the
        // quad along the same diagonal or the two patches leave a crack.
        //
        // The face is `x == 1` of the first cell (corners 1, 3, 5, 7) and
        // `x == 0` of the second (corners 0, 2, 4, 6). The neighbour's corner
        // `i` is this cell's corner `i + 1`.
        let diagonals = |on_face: fn(IVec3) -> bool| -> HashSet<(usize, usize)> {
            let mut found = HashSet::new();
            for tetrahedron in TETRAHEDRA {
                let on: Vec<usize> = tetrahedron
                    .into_iter()
                    .filter(|&slot| on_face(CORNER_OFFSET[slot]))
                    .collect();
                // A tetrahedron touching the face in three corners contributes a
                // triangle there; its diagonal is the pair not on the cell's own
                // face edges, but for this test the *set* of pairs is enough.
                if on.len() == 3 {
                    for first in 0..3 {
                        for second in first + 1..3 {
                            let (a, b) = (on[first].min(on[second]), on[first].max(on[second]));
                            if !found.insert((a, b)) {
                                found.remove(&(a, b));
                            }
                        }
                    }
                }
            }
            found
        };

        // Pairs that appear an *odd* number of times across the face's triangles
        // are its boundary edges plus its diagonal; the diagonal is the one
        // joining two corners that are not adjacent on the face.
        let mine = diagonals(|offset| offset.x == 1);
        let theirs: HashSet<(usize, usize)> = diagonals(|offset| offset.x == 0)
            .into_iter()
            .map(|(a, b)| {
                // Translate the neighbour's corner numbering into this cell's.
                let shift = |slot: usize| {
                    CORNER_OFFSET
                        .iter()
                        .position(|&offset| offset == CORNER_OFFSET[slot] + IVec3::X)
                        .expect("every low-face corner has a high-face twin")
                };
                let (a, b) = (shift(a), shift(b));
                (a.min(b), a.max(b))
            })
            .collect();

        assert_eq!(
            mine, theirs,
            "the two cells cut their shared face differently"
        );
    }

    /// A field that is a sphere, sampled densely enough to have a real surface.
    fn sphere_field(resolution: u32) -> Field {
        let bounds = review_model::Bounds {
            min: Vec3::splat(-1.0),
            max: Vec3::splat(1.0),
        };
        let desc = GridDesc::cover(bounds, resolution, 0.0);
        let blocks = desc.block_dims();
        let touched = vec![true; blocks.element_product() as usize];
        let mut field = Field::new(desc, &touched);
        let coords = field.block_coords();
        let band = desc.band();
        for (block, values) in coords.iter().zip(field.block_values_mut()) {
            for z in 0..BLOCK {
                for y in 0..BLOCK {
                    for x in 0..BLOCK {
                        let point = *block * BLOCK + IVec3::new(x, y, z);
                        if point.cmpge(desc.dims).any() {
                            continue;
                        }
                        let world = desc.position(point);
                        let slot = ((z * BLOCK + y) * BLOCK + x) as usize;
                        values[slot] = (world.length() - 0.75).clamp(-band, band);
                    }
                }
            }
        }
        field
    }

    #[test]
    fn marching_tetrahedra_of_a_sphere_sdf_is_closed_and_manifold() {
        let surface = extract(&sphere_field(24));
        assert!(surface.triangle_count() > 100, "the sphere has a surface");

        // Every *directed* edge appears exactly once, which is the strongest
        // statement available: it says the surface is closed (each edge has two
        // faces) and consistently oriented (they traverse it opposite ways).
        let mut directed: HashSet<(u32, u32)> = HashSet::new();
        for corners in surface.indices.as_chunks::<3>().0 {
            for corner in 0..3 {
                let edge = (corners[corner], corners[(corner + 1) % 3]);
                assert!(directed.insert(edge), "edge {edge:?} is used twice one way");
            }
        }
        for &(a, b) in &directed {
            assert!(
                directed.contains(&(b, a)),
                "edge {a}-{b} has no matching face on its other side"
            );
        }

        // Euler characteristic 2: one closed shell of genus zero.
        let vertices = surface.positions.len() as i64;
        let faces = surface.triangle_count() as i64;
        let edges = (directed.len() / 2) as i64;
        assert_eq!(
            vertices - edges + faces,
            2,
            "V {vertices} - E {edges} + F {faces}"
        );
    }

    #[test]
    fn the_extracted_sphere_sits_where_the_field_says_it_does() {
        let surface = extract(&sphere_field(32));
        for position in &surface.positions {
            let radius = position.length();
            assert!(
                (radius - 0.75).abs() < 0.05,
                "a vertex at radius {radius} is not on the 0.75 sphere"
            );
        }
        // Outward orientation: each face's normal points away from the centre.
        for corners in surface.indices.as_chunks::<3>().0 {
            let [a, b, c] = corners.map(|index| surface.positions[index as usize]);
            let normal = (b - a).cross(c - a);
            let centroid = (a + b + c) / 3.0;
            assert!(
                normal.dot(centroid) > 0.0,
                "a face of the sphere faces inward"
            );
        }
    }

    #[test]
    fn keeping_the_largest_shell_drops_the_satellites() {
        let mut surface = Surface {
            positions: vec![
                // A tetrahedron: four triangles.
                Vec3::ZERO,
                Vec3::X,
                Vec3::Y,
                Vec3::Z,
                // A lone triangle somewhere else.
                Vec3::splat(10.0),
                Vec3::splat(11.0),
                Vec3::new(10.0, 11.0, 10.0),
            ],
            indices: vec![0, 1, 2, 0, 2, 3, 0, 3, 1, 1, 3, 2, 4, 5, 6],
        };

        keep_largest_shell(&mut surface);

        assert_eq!(surface.triangle_count(), 4, "the satellite is gone");
        assert_eq!(surface.positions.len(), 4, "and so are its vertices");
    }
}
