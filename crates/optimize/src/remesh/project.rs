//! Reading the source surface's attributes back onto a regenerated one.
//!
//! The retopologized mesh shares nothing with the input, so every attribute has
//! to be *sampled*: find where each new face and corner lies on the old surface
//! and read the material, UVs, colors and normal there.
//!
//! ## Why a face's material and a corner's attributes are found differently
//!
//! A **face** takes the material of the source triangle nearest its centroid.
//! The centroid is the one point of a face that is unambiguously inside it, so a
//! face straddling a material boundary picks the material it mostly covers
//! rather than whichever corner happened to be sampled first.
//!
//! A **corner** takes the nearest source triangle to itself — but restricted to
//! its face's own *attribute region* when the two disagree. That restriction is
//! the whole mechanism behind seams, and it turns on one property of the source:
//! import splits every face corner into its own vertex, so the index buffer is
//! already cut along every UV, material and smoothing seam, and the lossless
//! index pass rejoins exactly the corners that match in every attribute. The
//! connected components of *that* adjacency are therefore the regions across
//! which an attribute is continuous.
//!
//! Two output faces meeting inside one region both find the same nearest source
//! triangle, sample bit-identical attributes, and [`crate::ops::weld`] merges
//! their corners into one vertex. Two faces meeting *across* a seam each stay in
//! their own region and their corners stay split — which is what puts the
//! source's seams onto output edges instead of tearing a face across one.
//!
//! The in-region fallback is a bounded best-first walk from the face's anchor
//! rather than a fixed ring around it. A ring cannot work: the rebuilt mesh is
//! routinely *coarser* than the source, so a corner sits several source
//! triangles away from its own face's centroid, and a ring small enough to stay
//! inside one region is too small to reach it. The walk expands by adjacency in
//! order of distance and stops improving where the surface does.
//!
//! ## Extrapolation
//!
//! Barycentric coordinates are taken on the plane of the chosen triangle and
//! left **unclamped** (within a bound). A regenerated vertex lies within a
//! fraction of an edge length of the source surface, so a corner that falls just
//! off its patch is better served by continuing the attribute gradient than by
//! being pinned to the patch's rim, which would flatten a UV island's border
//! into a ridge.

use glam::{Vec2, Vec3, Vec4};
use review_model::{Bvh, ModelData, Vertex, closest_point_on_triangle, triangle_positions};

use crate::submesh::{NO_MATERIAL, Submesh};

use super::RemeshOutput;

/// How far a barycentric coordinate may be extrapolated past the triangle it
/// belongs to. Half a triangle's width in any direction — beyond that the sample
/// is describing somewhere else.
const EXTRAPOLATION_LIMIT: f32 = 0.5;

/// The source surface, indexed for projection: the node's pieces concatenated
/// **without** welding (so every attribute discontinuity import authored is
/// still a vertex split), plus the adjacency and the hierarchy a query needs.
pub(crate) struct ProjectionSource {
    /// A scratch model carrying only what a projection reads: positions,
    /// normals, UVs and colors per vertex, and a triangle index buffer.
    model: ModelData,
    /// Extra UV sets, parallel to `model.vertices`.
    uv_channels: Vec<Vec<Vec2>>,
    /// Vertex-color sets beyond the first, parallel to `model.vertices`.
    color_channels: Vec<Vec<Vec4>>,
    /// Per triangle, the material slot of the piece it came from.
    triangle_material: Vec<u32>,
    /// Per triangle, the neighbour across each of its three edges, or
    /// [`u32::MAX`]. Built from shared *index-buffer* edges, so it stops at
    /// every seam the source authored.
    adjacency: Vec<[u32; 3]>,
    /// Per triangle, the attribute region it belongs to: the connected component
    /// of [`Self::adjacency`]. Two triangles share a region exactly when a path
    /// of index-shared edges joins them, which is exactly when every attribute
    /// is continuous along that path.
    region: Vec<u32>,
    bvh: Bvh,
}

/// Cap on how many triangles the in-region fallback will look at.
///
/// It only runs for a corner whose globally nearest triangle is in another
/// region — a corner within one source triangle of a seam — so the walk is short
/// in practice. The cap is what stops a pathological region from turning a
/// per-corner query into a scan of the whole mesh.
const REGION_WALK_LIMIT: usize = 256;

impl ProjectionSource {
    pub(crate) fn build(pieces: &[&Submesh]) -> Self {
        let _z = crate::prof::zone!("Remesh Projection Source");

        let uv_channel_count = pieces
            .iter()
            .map(|piece| piece.uv_channels.len())
            .max()
            .unwrap_or(0);
        let color_channel_count = pieces
            .iter()
            .map(|piece| piece.color_channels.len())
            .max()
            .unwrap_or(0);

        let mut vertices: Vec<Vertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut uv_channels: Vec<Vec<Vec2>> = vec![Vec::new(); uv_channel_count];
        let mut color_channels: Vec<Vec<Vec4>> = vec![Vec::new(); color_channel_count];
        let mut triangle_material: Vec<u32> = Vec::new();

        for piece in pieces {
            let base = vertices.len() as u32;
            vertices.extend_from_slice(&piece.vertices);
            indices.extend(piece.indices.iter().map(|&index| index + base));
            triangle_material.extend(std::iter::repeat_n(piece.material, piece.triangle_count()));
            for (channel, destination) in uv_channels.iter_mut().enumerate() {
                match piece.uv_channels.get(channel) {
                    Some(source) => destination.extend_from_slice(source),
                    // A piece with fewer sets than its sibling pads with its own
                    // channel 0, which is what `Vertex::uv` already holds.
                    None => destination.extend(piece.vertices.iter().map(|vertex| vertex.uv)),
                }
            }
            for (channel, destination) in color_channels.iter_mut().enumerate() {
                match piece.color_channels.get(channel) {
                    Some(source) => destination.extend_from_slice(source),
                    None => destination.resize(vertices.len(), Vec4::ONE),
                }
            }
        }

        let adjacency = build_adjacency(&indices);
        let region = build_regions(&adjacency);
        let model = ModelData {
            vertices,
            indices,
            ..ModelData::default()
        };
        let bvh = Bvh::build(&model);

        Self {
            model,
            uv_channels,
            color_channels,
            triangle_material,
            adjacency,
            region,
            bvh,
        }
    }

    pub(crate) fn triangle_count(&self) -> usize {
        self.model.indices.len() / 3
    }

    /// The material slot of the piece `triangle` came from.
    fn material_of(&self, triangle: u32) -> u32 {
        self.triangle_material
            .get(triangle as usize)
            .copied()
            .unwrap_or(NO_MATERIAL)
    }

    /// The geometric normal of `triangle`, or `None` for a degenerate one.
    fn face_normal(&self, triangle: u32) -> Option<Vec3> {
        let [a, b, c] = triangle_positions(&self.model, triangle);
        (b - a).cross(c - a).try_normalize()
    }

    /// The source triangle to read `query`'s attributes from, for a face whose
    /// centroid resolved to `anchor`.
    ///
    /// The globally nearest triangle when it shares `anchor`'s attribute region
    /// — which is the common case and, being a pure function of the position,
    /// is what lets two faces of one region weld their shared corner. Otherwise
    /// the nearest triangle the walk below reaches without leaving the region,
    /// so a corner beside a seam reads its own side of it.
    fn sample_triangle(&self, anchor: u32, query: Vec3, range: f32) -> u32 {
        let region = self
            .region
            .get(anchor as usize)
            .copied()
            .unwrap_or(u32::MAX);
        if let Some(hit) = self.bvh.closest_point(&self.model, query, range)
            && self.region.get(hit.triangle as usize).copied() == Some(region)
        {
            return hit.triangle;
        }
        self.nearest_in_region(anchor, query)
    }

    /// The nearest triangle to `query` reachable from `anchor` by adjacency.
    ///
    /// Best-first: the frontier is always expanded at its closest triangle, so
    /// the walk goes toward `query` rather than flooding the region, and it
    /// stops as soon as a whole expansion fails to improve on the best found.
    /// Ties break on the lower triangle index, so the answer depends only on
    /// `(anchor, query)` and never on the order the adjacency was built in.
    fn nearest_in_region(&self, anchor: u32, query: Vec3) -> u32 {
        let distance_to = |triangle: u32| {
            let [a, b, c] = triangle_positions(&self.model, triangle);
            let (point, _) = closest_point_on_triangle(query, a, b, c);
            (point - query).length_squared()
        };

        let mut best = (distance_to(anchor), anchor);
        let mut visited = vec![anchor];
        let mut frontier = vec![(best.0, anchor)];
        while !frontier.is_empty() && visited.len() < REGION_WALK_LIMIT {
            // The closest unexpanded triangle; ties on the lower index.
            let Some(slot) = (0..frontier.len()).min_by(|&a, &b| {
                frontier[a]
                    .0
                    .total_cmp(&frontier[b].0)
                    .then(frontier[a].1.cmp(&frontier[b].1))
            }) else {
                break;
            };
            let (distance, current) = frontier.swap_remove(slot);
            // Everything still on the frontier is at least this far away, and a
            // neighbour cannot be nearer than the triangle it borders by more
            // than that triangle's own reach — so once the closest frontier
            // entry is no better than the best found, nothing left can be.
            if distance > best.0 {
                break;
            }
            let Some(neighbours) = self.adjacency.get(current as usize) else {
                continue;
            };
            for &neighbour in neighbours {
                if neighbour == u32::MAX || visited.contains(&neighbour) {
                    continue;
                }
                visited.push(neighbour);
                let distance = distance_to(neighbour);
                if distance < best.0 || (distance == best.0 && neighbour < best.1) {
                    best = (distance, neighbour);
                }
                frontier.push((distance, neighbour));
            }
        }
        best.1
    }

    /// Sample every attribute at `query`, interpolated across `triangle`.
    fn sample(
        &self,
        triangle: u32,
        query: Vec3,
        uv_channel_count: usize,
        color_channel_count: usize,
    ) -> CornerSample {
        let base = triangle as usize * 3;
        let corners: [usize; 3] = [
            self.model.indices.get(base).copied().unwrap_or(0) as usize,
            self.model.indices.get(base + 1).copied().unwrap_or(0) as usize,
            self.model.indices.get(base + 2).copied().unwrap_or(0) as usize,
        ];
        let weights = plane_barycentric(&self.model, triangle, query);
        let vertex_at = |slot: usize| {
            self.model
                .vertices
                .get(corners[slot])
                .copied()
                .unwrap_or_default()
        };

        let blend3 = |pick: &dyn Fn(Vertex) -> Vec3| {
            pick(vertex_at(0)) * weights.x
                + pick(vertex_at(1)) * weights.y
                + pick(vertex_at(2)) * weights.z
        };
        let blend2 = |pick: &dyn Fn(Vertex) -> Vec2| {
            pick(vertex_at(0)) * weights.x
                + pick(vertex_at(1)) * weights.y
                + pick(vertex_at(2)) * weights.z
        };
        let blend4 = |pick: &dyn Fn(Vertex) -> Vec4| {
            pick(vertex_at(0)) * weights.x
                + pick(vertex_at(1)) * weights.y
                + pick(vertex_at(2)) * weights.z
        };

        // An interpolated normal can cancel to zero where the three corners
        // disagree sharply; the triangle's own normal is the honest fallback.
        let normal = blend3(&|vertex: Vertex| vertex.normal)
            .try_normalize()
            .or_else(|| self.face_normal(triangle))
            .unwrap_or(Vec3::Y);

        let uvs = (0..uv_channel_count)
            .map(|channel| match self.uv_channels.get(channel) {
                Some(values) => {
                    let at = |slot: usize| values.get(corners[slot]).copied().unwrap_or(Vec2::ZERO);
                    at(0) * weights.x + at(1) * weights.y + at(2) * weights.z
                }
                None => blend2(&|vertex: Vertex| vertex.uv),
            })
            .collect();
        let colors = (0..color_channel_count)
            .map(|channel| match self.color_channels.get(channel) {
                Some(values) => {
                    let at = |slot: usize| values.get(corners[slot]).copied().unwrap_or(Vec4::ONE);
                    (at(0) * weights.x + at(1) * weights.y + at(2) * weights.z)
                        .clamp(Vec4::ZERO, Vec4::ONE)
                }
                None => Vec4::ONE,
            })
            .collect();

        CornerSample {
            position: query,
            normal,
            uv: blend2(&|vertex: Vertex| vertex.uv),
            uvs,
            color: blend4(&|vertex: Vertex| vertex.vertex_color).clamp(Vec4::ZERO, Vec4::ONE),
            colors,
        }
    }
}

/// One corner of a rebuilt face, with everything it needs to become a [`Vertex`].
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct CornerSample {
    pub position: Vec3,
    pub normal: Vec3,
    /// UV set 0, which always lives on [`Vertex::uv`].
    pub uv: Vec2,
    /// Every UV set, when the source carried more than one.
    pub uvs: Vec<Vec2>,
    /// Vertex-color set 0, which lives on [`Vertex::vertex_color`].
    pub color: Vec4,
    /// Vertex-color sets beyond the first.
    pub colors: Vec<Vec4>,
}

/// One rebuilt face: which material it belongs to, and its corners in winding
/// order.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct FaceSample {
    pub material: u32,
    pub corners: Vec<CornerSample>,
}

/// Where a rebuilt face's winding comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Winding {
    /// Take it from the source surface: flip a face whose normal opposes the
    /// source's at the point under its centroid. A field extraction's winding is
    /// its own business and carries no relation to which way the object faces,
    /// so it has to be re-derived.
    FromSource,
    /// Leave it exactly as the soup has it. A [`crate::shrinkwrap`] shell's
    /// winding is derived from the distance field's sign and is *more* reliable
    /// than the source's — the source being a kitbash whose parts face whichever
    /// way they were modelled. Re-deriving it there would flip individual faces
    /// of a consistently oriented shell, which is the one thing that turns a
    /// closed manifold back into a broken one.
    Keep,
}

/// Project every face of `output` onto `source`.
///
/// Parallel over faces, dealt round-robin into per-worker buckets before the
/// threads start — the same static partition the AO bake uses, and for the same
/// reason: each face is a pure function of its own corners and the immutable
/// source, so scheduling cannot reach the result.
pub(crate) fn project_faces(
    output: &RemeshOutput,
    source: &ProjectionSource,
    range: f32,
    uv_channel_count: usize,
    color_channel_count: usize,
    winding: Winding,
    threads: usize,
) -> Vec<FaceSample> {
    let _z = crate::prof::zone!("Remesh Projection");

    let mut faces: Vec<FaceSample> = vec![FaceSample::default(); output.face_count()];
    if faces.is_empty() || source.triangle_count() == 0 {
        return faces;
    }

    const CHUNK: usize = 256;
    let mut jobs: Vec<(usize, &mut [FaceSample])> = Vec::new();
    for (index, chunk) in faces.chunks_mut(CHUNK).enumerate() {
        jobs.push((index * CHUNK, chunk));
    }
    // The object's own share of the machine: this runs inside `solve_nodes`,
    // which already puts one object on each core.
    let workers = threads.min(jobs.len()).max(1);
    let mut buckets: Vec<Vec<(usize, &mut [FaceSample])>> =
        (0..workers).map(|_| Vec::new()).collect();
    for (slot, job) in jobs.into_iter().enumerate() {
        buckets[slot % workers].push(job);
    }

    std::thread::scope(|scope| {
        for bucket in buckets {
            scope.spawn(move || {
                for (first, chunk) in bucket {
                    for (offset, destination) in chunk.iter_mut().enumerate() {
                        *destination = project_face(
                            output,
                            source,
                            first + offset,
                            range,
                            uv_channel_count,
                            color_channel_count,
                            winding,
                        );
                    }
                }
            });
        }
    });

    faces
}

/// Project one face. An unreachable source surface yields an empty face, which
/// the caller drops.
fn project_face(
    output: &RemeshOutput,
    source: &ProjectionSource,
    face: usize,
    range: f32,
    uv_channel_count: usize,
    color_channel_count: usize,
    winding: Winding,
) -> FaceSample {
    let corners = output.face(face);
    let positions: Vec<Vec3> = corners
        .iter()
        .map(|&corner| output.positions[corner as usize])
        .collect();
    let centroid = positions.iter().copied().sum::<Vec3>() / positions.len() as f32;

    let Some(anchor) = source.bvh.closest_point(&source.model, centroid, range) else {
        return FaceSample::default();
    };
    let material = source.material_of(anchor.triangle);

    // A field extraction's winding is its own; a face whose normal opposes the
    // surface it was built from would draw back-to-front. Newell's formula
    // rather than one corner's cross product, so a slightly non-planar quad
    // still answers. See [`Winding`] for why a wrapped shell opts out.
    let mut ordered = positions;
    if winding == Winding::FromSource
        && let (Some(face_normal), Some(source_normal)) =
            (newell_normal(&ordered), source.face_normal(anchor.triangle))
        && face_normal.dot(source_normal) < 0.0
    {
        ordered.reverse();
    }

    let sampled = ordered
        .into_iter()
        .map(|position| {
            let triangle = source.sample_triangle(anchor.triangle, position, range);
            source.sample(triangle, position, uv_channel_count, color_channel_count)
        })
        .collect();

    FaceSample {
        material,
        corners: sampled,
    }
}

/// Newell's normal of a polygon: robust for a non-planar face, where a single
/// corner's cross product is not.
fn newell_normal(positions: &[Vec3]) -> Option<Vec3> {
    let mut normal = Vec3::ZERO;
    for (index, &current) in positions.iter().enumerate() {
        let next = positions[(index + 1) % positions.len()];
        normal.x += (current.y - next.y) * (current.z + next.z);
        normal.y += (current.z - next.z) * (current.x + next.x);
        normal.z += (current.x - next.x) * (current.y + next.y);
    }
    normal.try_normalize()
}

/// Barycentric coordinates of `query`'s projection onto `triangle`'s plane,
/// **not** clamped to the triangle — see the module docs. Each component is held
/// within [`EXTRAPOLATION_LIMIT`] of the triangle so a query that somehow landed
/// far away cannot produce a wild attribute.
fn plane_barycentric(model: &ModelData, triangle: u32, query: Vec3) -> Vec3 {
    let [a, b, c] = triangle_positions(model, triangle);
    let ab = b - a;
    let ac = c - a;
    let normal = ab.cross(ac);
    let denominator = normal.length_squared();
    if denominator <= 0.0 {
        // Degenerate: every corner is the same point as far as attributes go.
        return Vec3::new(1.0, 0.0, 0.0);
    }
    let projected = query - normal * (normal.dot(query - a) / denominator);
    let v = normal.dot(ab.cross(projected - a)) / denominator;
    let u = normal.dot((c - projected).cross(b - projected)) / denominator;
    let weights = Vec3::new(u, 1.0 - u - v, v).clamp(
        Vec3::splat(-EXTRAPOLATION_LIMIT),
        Vec3::splat(1.0 + EXTRAPOLATION_LIMIT),
    );
    // Clamping can break the partition of unity; renormalize so an attribute is
    // still an average rather than a scaled one.
    let total = weights.x + weights.y + weights.z;
    if total.abs() < 1.0e-6 {
        Vec3::new(1.0, 0.0, 0.0)
    } else {
        weights / total
    }
}

/// Per triangle, the neighbour across each of its three edges.
///
/// Keyed on the index-buffer edge rather than on positions, so it stops dead at
/// every attribute seam — which is the property the corner sampling relies on.
/// An edge shared by more than two triangles links none of them: there is no
/// single neighbour, and guessing one would grow the patch across a branch.
///
/// Grouped by sorting one `(edge, triangle corner)` entry per corner rather than
/// by hashing: a hash map of every edge with a small `Vec` per entry is several
/// times the memory at ten million triangles (see [`super::topology`] for why
/// that shape is ruled out). Sorting on the corner as well keeps each group in
/// triangle order, so the pairs come out exactly as the map's insertion order had
/// them.
fn build_adjacency(indices: &[u32]) -> Vec<[u32; 3]> {
    let triangle_count = indices.len() / 3;
    let mut owners: Vec<(u64, u32)> = Vec::with_capacity(triangle_count * 3);
    for triangle in 0..triangle_count {
        for corner in 0..3 {
            let a = indices[triangle * 3 + corner];
            let b = indices[triangle * 3 + (corner + 1) % 3];
            let (low, high) = if a < b { (a, b) } else { (b, a) };
            let key = (u64::from(low) << 32) | u64::from(high);
            owners.push((key, (triangle * 3 + corner) as u32));
        }
    }
    owners.sort_unstable();

    let mut adjacency = vec![[u32::MAX; 3]; triangle_count];
    for sharing in owners.chunk_by(|a, b| a.0 == b.0) {
        let [(_, first), (_, second)] = sharing else {
            continue;
        };
        let (first, first_edge) = (first / 3, first % 3);
        let (second, second_edge) = (second / 3, second % 3);
        adjacency[first as usize][first_edge as usize] = second;
        adjacency[second as usize][second_edge as usize] = first;
    }
    adjacency
}

/// Per triangle, the connected component of `adjacency` it belongs to.
///
/// Union-find with path halving and union by size: near-linear, and — unlike a
/// flood fill — independent of the order the triangles are visited in, so the
/// component *ids* are a function of the mesh rather than of the walk.
fn build_regions(adjacency: &[[u32; 3]]) -> Vec<u32> {
    let count = adjacency.len();
    let mut parent: Vec<u32> = (0..count as u32).collect();
    let mut size: Vec<u32> = vec![1; count];

    fn find(parent: &mut [u32], mut node: u32) -> u32 {
        while parent[node as usize] != node {
            // Path halving: point each node at its grandparent as we climb.
            parent[node as usize] = parent[parent[node as usize] as usize];
            node = parent[node as usize];
        }
        node
    }

    for (triangle, neighbours) in adjacency.iter().enumerate() {
        for &neighbour in neighbours {
            if neighbour == u32::MAX {
                continue;
            }
            let (mut a, mut b) = (
                find(&mut parent, triangle as u32),
                find(&mut parent, neighbour),
            );
            if a == b {
                continue;
            }
            if size[a as usize] < size[b as usize] {
                std::mem::swap(&mut a, &mut b);
            }
            parent[b as usize] = a;
            size[a as usize] += size[b as usize];
        }
    }

    (0..count as u32)
        .map(|triangle| find(&mut parent, triangle))
        .collect()
}

#[cfg(test)]
mod tests {
    use review_model::demo_cube_model;

    use super::*;
    use crate::submesh::partition;

    fn cube_source() -> ProjectionSource {
        let model = demo_cube_model();
        let (pieces, _) = partition(&model, None);
        let borrowed: Vec<&Submesh> = pieces.iter().collect();
        ProjectionSource::build(&borrowed)
    }

    #[test]
    fn a_face_centroid_finds_its_source_triangle_and_material() {
        let source = cube_source();
        // The demo cube spans -0.5..0.5 in x and z and 0.03..1.03 in y; this is
        // a point on its +Z face.
        let query = Vec3::new(0.1, 0.5, 0.5);

        let hit = source
            .bvh
            .closest_point(&source.model, query, 1.0)
            .expect("the cube is within range");

        assert!(hit.distance_squared < 1.0e-6, "the query is on the surface");
        assert_eq!(source.material_of(hit.triangle), 0);
    }

    #[test]
    fn every_face_of_a_corner_split_cube_is_its_own_region() {
        let source = cube_source();
        // Import gives every face corner its own vertex, so no edge of the demo
        // cube is shared between two *faces* — only the two triangles of one
        // face share the diagonal. Each face is therefore its own attribute
        // region, which is what keeps its normals and UVs off its neighbours.
        let neighbours: usize = source
            .adjacency
            .iter()
            .flatten()
            .filter(|&&neighbour| neighbour != u32::MAX)
            .count();
        assert_eq!(
            neighbours, 12,
            "each of the six faces links its own two triangles, both ways"
        );

        let regions: std::collections::HashSet<u32> = source.region.iter().copied().collect();
        assert_eq!(regions.len(), 6, "one region per cube face");
        assert_eq!(
            source.region[0], source.region[1],
            "a face's two triangles share a region"
        );
    }

    #[test]
    fn two_faces_of_one_region_sample_a_shared_corner_identically() {
        let source = cube_source();
        let query = Vec3::new(0.25, 0.25, 0.5);

        // Both triangles of the +Z face, standing in for two output faces whose
        // centroids landed in different source triangles of the same region.
        let from_first = source.sample_triangle(0, query, 1.0);
        let from_second = source.sample_triangle(1, query, 1.0);

        assert_eq!(
            source.sample(from_first, query, 0, 0),
            source.sample(from_second, query, 0, 0),
            "two faces of one attribute region must project bit-identically, \
             or their shared corner never welds"
        );
    }

    #[test]
    fn a_corner_on_a_hard_edge_keeps_each_side_s_own_normal() {
        let source = cube_source();
        // A point on the cube's +X/+Z edge: equidistant from two faces that are
        // separate regions, so each side reads its own.
        let on_edge = Vec3::new(0.5, 0.5, 0.5);
        let plus_x = (0..source.triangle_count() as u32)
            .find(|&triangle| {
                source
                    .face_normal(triangle)
                    .is_some_and(|normal| normal.x > 0.9)
            })
            .expect("the cube has a +X face");

        let from_z = source.sample_triangle(0, on_edge, 1.0);
        let from_x = source.sample_triangle(plus_x, on_edge, 1.0);

        assert_eq!(source.region[from_z as usize], source.region[0]);
        assert_eq!(
            source.region[from_x as usize],
            source.region[plus_x as usize]
        );
        assert_ne!(
            source.sample(from_z, on_edge, 0, 0).normal,
            source.sample(from_x, on_edge, 0, 0).normal,
            "the two sides of a hard edge must keep their own normals"
        );
    }

    #[test]
    fn the_in_region_walk_reaches_across_a_whole_region() {
        let source = cube_source();
        // The far corner of the +Z face, two triangles from the one the walk
        // starts at — the case a fixed small ring could not reach on a mesh
        // whose faces are larger than the rebuilt ones.
        let corner = Vec3::new(-0.5, 1.03, 0.5);

        let reached = source.nearest_in_region(0, corner);

        assert_eq!(
            source.region[reached as usize], source.region[0],
            "the walk must not leave the region it started in"
        );
        let [a, b, c] = triangle_positions(&source.model, reached);
        let (point, _) = closest_point_on_triangle(corner, a, b, c);
        assert!(
            (point - corner).length() < 1.0e-5,
            "the walk must find the triangle the corner actually sits on"
        );
    }
}
