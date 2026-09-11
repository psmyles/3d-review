//! CPU ambient-occlusion bake: deterministic cosine-weighted hemisphere
//! raycasts against bake-scoped occluder models, written into
//! [`Vertex::vertex_color`].
//!
//! AO is inherently a whole-scene query — a wall in one submesh must occlude a
//! floor in another — so unlike the per-piece operations this one takes every
//! submesh at once (dispatched from `process::apply_op`'s whole-mesh path,
//! beside `Reduce`). Excluded objects still *occlude*; they just aren't
//! written.
//!
//! ## What counts as "the scene"
//!
//! Game FBXs routinely carry a whole LOD chain and a collision shell as
//! co-located sibling nodes, and raycasting against those near-coincident
//! surfaces shreds the bake. Two rules keep it idiot-proof:
//!
//! * **Outliner-hidden nodes are out entirely** — neither occluding nor baked
//!   (hide the collision hulls).
//! * **LOD identity partitions the occluders.** A node whose name (or an
//!   ancestor's) carries the conventional `_LOD<n>` suffix bakes only against
//!   its own LOD's geometry plus every suffix-less node — a suffix-less mesh
//!   has no LOD variants, so it exists at every level. Suffix-less nodes
//!   themselves bake against the lowest LOD present (the ground-truth
//!   geometry). A scene with no suffixes anywhere is one group, i.e. the
//!   plain whole-scene bake. This is what lets an artist keep the whole chain
//!   visible and bake every LOD in one run.
//!
//! ## Where rays start
//!
//! A vertex's rays are **not** cast from the vertex itself. A corner sample
//! point sits exactly where surfaces meet, so a vertex tucked into a
//! contact/overlap gap (a shell rim between two walls of the same mesh) reads
//! fully occluded — and interpolation then drags whole faces black even though
//! most of each face is open. Instead the ray budget is dealt round-robin
//! across **surface-neighborhood origins**: two per incident triangle
//! ([`CornerAdjacency`], position-bit keyed over the occluder scene), inset
//! [`ORIGIN_INSETS`] of the way toward that triangle's centroid and lifted
//! off the surface by the bias — the vertex-bake analog of a texture baker
//! sampling texel centers strictly inside faces, never on edges. Incident
//! triangles whose geometric normal is near-perpendicular to the vertex
//! normal (|dot| ≤ [`NEIGHBOR_ALIGNMENT_MIN`]) are skipped — the far side of a
//! hard edge, and zero-area slivers; a vertex with no surviving neighbor falls
//! back to the bare corner origin. Genuine crevices still darken: insets near
//! a real crevice are themselves occluded.
//!
//! Each occluder model is a transient concatenation of submesh geometry, not a
//! copy of anything shared (invariant 1 in spirit — it is bake scratch, like
//! the BVH builder's own centroid arrays). It is also what makes the parallel
//! bake safely expressible: the worker threads mutate `vertex_color` on the
//! live submesh vertices while raycasting, so the read side has to be a
//! separate immutable snapshot rather than a borrow of the same arrays.
//!
//! Everything here is a pure function of the vertex bits and the occluder set
//! (the adjacency included — every input to the origin set is bit-derived):
//! two runs produce bit-identical colors, and vertices duplicated across
//! submesh seams (byte-identical position + normal, copied from the same
//! source corner) share their adjacency key and so get identical AO — seams
//! never show.

use std::collections::{BTreeMap, HashMap};

use glam::{Vec3, Vec4};
use review_model::{Bounds, Bvh, ModelData, Vertex};

use crate::process::{is_excluded, resolve_op};
use crate::stack::{AoTarget, BakeAoParams, OpInstance, OpKind, OptStack};
use crate::submesh::Submesh;

/// Vertices per job in the thread-scope work list — small enough to balance
/// uneven submesh sizes across workers, large enough that job bookkeeping is
/// noise next to the raycasts.
const CHUNK: usize = 1024;

/// The two rings of ray origins per incident triangle, as fractions of the
/// way from the vertex toward the triangle's centroid. The near ring keeps
/// the sample local to the corner, so genuine crevices still darken; the far
/// ring sits in the face's interior and is the guarantee the whole scheme
/// exists for — a face that is mostly visible can never bake near-black,
/// however deeply its corner is buried, because half its rays start out in
/// the open. Two rings also blunt triangulation sensitivity: with one ring a
/// low-valence corner (one incident triangle) insets a shorter world distance
/// than its two-triangle neighbors and can stay buried while they escape.
const ORIGIN_INSETS: [f32; 2] = [0.25, 0.75];

/// Minimum |dot(triangle geometric normal, vertex normal)| for a triangle to
/// contribute a ray origin. Rejects the far side of a near-perpendicular hard
/// edge (whose inset origin would hover off-surface with a half-buried
/// hemisphere) and — via `normalize_or_zero` — zero-area slivers. The
/// absolute value is load-bearing: winding orientation is arbitrary, so the
/// geometric normal may legitimately oppose the shading normal. As a side
/// effect the floor guarantees the vertex-normal lift clears every surviving
/// triangle's plane by at least `0.1 × bias`.
const NEIGHBOR_ALIGNMENT_MIN: f32 = 0.1;

/// One bake group's raycast world: the occluder geometry, its hierarchy, the
/// corner adjacency the ray origins come from, and the self-intersection bias
/// sized to it.
struct OccluderScene {
    occluders: ModelData,
    bvh: Bvh,
    adjacency: CornerAdjacency,
    bias: f32,
}

/// Which triangles touch each distinct vertex position of an occluder model,
/// keyed by the position's *bit pattern* (matching [`scramble_hash`]'s
/// bit-stability discipline — ±0.0 differ by design; the seam guarantee is
/// about byte-identical copies). Keying by position alone deliberately unions
/// corner-split duplicates — the same physical corner split across
/// submeshes/materials — so every duplicate sees the same neighborhood.
///
/// CSR layout, built with the two-pass count→fill discipline: three flat
/// buffers rather than one `Vec` per corner. Deterministic without a
/// `BTreeMap` — slots are assigned in triangle-scan order and the map is only
/// ever *looked up*, never iterated.
struct CornerAdjacency {
    slots: HashMap<[u32; 3], u32>,
    /// Per-slot run boundaries into `tris`; length = slot count + 1.
    starts: Vec<u32>,
    /// Flat triangle indices, each slot's run in ascending triangle order.
    tris: Vec<u32>,
}

impl CornerAdjacency {
    fn build(occluders: &ModelData) -> Self {
        let triangle_count = occluders.indices.len() / 3;
        let mut slots: HashMap<[u32; 3], u32> = HashMap::new();
        let mut counts: Vec<u32> = Vec::new();
        for tri in 0..triangle_count {
            for position in triangle_positions(occluders, tri as u32) {
                let next = counts.len() as u32;
                let slot = *slots.entry(position_key(position)).or_insert(next);
                if slot == next {
                    counts.push(0);
                }
                counts[slot as usize] += 1;
            }
        }

        let mut starts = Vec::with_capacity(counts.len() + 1);
        let mut total = 0u32;
        starts.push(0);
        for &count in &counts {
            total += count;
            starts.push(total);
        }

        let mut cursor = starts.clone();
        let mut tris = vec![0u32; total as usize];
        for tri in 0..triangle_count {
            for position in triangle_positions(occluders, tri as u32) {
                let slot = slots[&position_key(position)] as usize;
                tris[cursor[slot] as usize] = tri as u32;
                cursor[slot] += 1;
            }
        }

        Self {
            slots,
            starts,
            tris,
        }
    }

    /// The triangles touching `position` (bit-exact), in triangle order —
    /// empty for a position no triangle references. A degenerate triangle
    /// with two coincident corners lists twice; harmless, since the origin
    /// filter rejects it anyway.
    fn incident(&self, position: Vec3) -> &[u32] {
        let Some(&slot) = self.slots.get(&position_key(position)) else {
            return &[];
        };
        let from = self.starts[slot as usize] as usize;
        let to = self.starts[slot as usize + 1] as usize;
        &self.tris[from..to]
    }
}

fn position_key(position: Vec3) -> [u32; 3] {
    [
        position.x.to_bits(),
        position.y.to_bits(),
        position.z.to_bits(),
    ]
}

/// The three positions of triangle `tri` in the occluder model, falling back
/// to the origin for any out-of-range index (mirroring the BVH's own
/// defensive read — a malformed mesh degrades, never panics).
fn triangle_positions(occluders: &ModelData, tri: u32) -> [Vec3; 3] {
    let base = tri as usize * 3;
    let position = |slot: usize| {
        occluders
            .indices
            .get(base + slot)
            .and_then(|&index| occluders.vertices.get(index as usize))
            .map(|vertex| vertex.position)
            .unwrap_or(Vec3::ZERO)
    };
    [position(0), position(1), position(2)]
}

/// Bake AO into every non-excluded, non-hidden submesh's vertex colors, each
/// against its own LOD group's occluders (see the module docs). `model` is the
/// source model, read only for node names/hierarchy — the LOD classification.
/// Infallible: there is no per-submesh failure mode — an empty occluder set
/// simply yields full visibility everywhere.
pub(crate) fn bake_submeshes(
    submeshes: &mut [Submesh],
    op: &OpInstance,
    stack: &OptStack,
    model: &ModelData,
    hidden_nodes: &[u32],
) {
    let _z = crate::prof::zone!("Bake AO");

    let OpKind::BakeAo(base_params) = &op.kind else {
        return;
    };

    // Classify every piece by source LOD identity, once. Suffix-less pieces
    // bake against the lowest LOD present — the ground-truth geometry — and
    // when no suffixes exist anywhere the single `None` group is the whole
    // visible scene.
    let piece_lods: Vec<Option<u32>> = submeshes
        .iter()
        .map(|piece| node_lod(model, piece.node))
        .collect();
    let lowest_lod = piece_lods.iter().flatten().copied().min();

    // One occluder scene per bake group actually written. `BTreeMap` for
    // deterministic build order.
    let mut scenes: BTreeMap<Option<u32>, OccluderScene> = BTreeMap::new();
    for (piece, lod) in submeshes.iter().zip(&piece_lods) {
        if is_excluded(stack, piece.node) || hidden_nodes.contains(&piece.node) {
            continue;
        }
        scenes
            .entry(lod.or(lowest_lod))
            .or_insert_with_key(|&group| {
                occluder_scene(submeshes, &piece_lods, hidden_nodes, group)
            });
    }

    // Resolve settings per submesh up front, then flatten into fixed-size
    // vertex-chunk jobs, each carrying its group's scene. Disjoint `&mut`
    // slices — the borrow checker is what proves the parallel section below
    // data-race-free.
    let mut jobs: Vec<(&mut [Vertex], BakeAoParams, &OccluderScene)> = Vec::new();
    for (piece, lod) in submeshes.iter_mut().zip(&piece_lods) {
        if is_excluded(stack, piece.node) || hidden_nodes.contains(&piece.node) {
            continue;
        }
        let Some(scene) = scenes.get(&lod.or(lowest_lod)) else {
            continue;
        };
        let params = match resolve_op(stack, op, piece.node) {
            OpKind::BakeAo(resolved) => *resolved,
            // An override can only ever hold the same kind as the operation it
            // overrides; fall back to the global settings if one somehow doesn't.
            _ => *base_params,
        };
        for chunk in piece.vertices.chunks_mut(CHUNK) {
            jobs.push((chunk, params, scene));
        }
    }
    if jobs.is_empty() {
        return;
    }

    // Deal the jobs round-robin into per-worker buckets *before* spawning: no
    // shared queue, no atomics, and the fixed chunk size keeps the static
    // partition balanced. Scheduling cannot affect the result — each vertex is
    // a pure function of its own bits.
    let workers = std::thread::available_parallelism()
        .map_or(1, std::num::NonZero::get)
        .min(jobs.len());
    let mut buckets: Vec<Vec<(&mut [Vertex], BakeAoParams, &OccluderScene)>> =
        (0..workers).map(|_| Vec::new()).collect();
    for (slot, job) in jobs.into_iter().enumerate() {
        buckets[slot % workers].push(job);
    }
    std::thread::scope(|scope| {
        for bucket in buckets {
            scope.spawn(move || {
                for (chunk, params, scene) in bucket {
                    bake_chunk(chunk, &params, scene);
                }
            });
        }
    });
}

/// The source LOD identity of `node`: the index parsed from the conventional
/// `_LOD<n>` name suffix, on the node itself or the nearest named ancestor
/// (LOD grouping is sometimes on a parent group node). `None` for a node with
/// no LOD identity. The parent walk is bounded so a malformed cycle in the
/// hierarchy terminates rather than spins.
fn node_lod(model: &ModelData, node: u32) -> Option<u32> {
    let mut index = node as usize;
    for _ in 0..=model.nodes.len() {
        let node = model.nodes.get(index)?;
        if let Some(lod) = lod_suffix(&node.name) {
            return Some(lod);
        }
        index = node.parent?;
    }
    None
}

/// Parse a node name's trailing LOD index: `..._LOD<n>` in any case, or a name
/// that is entirely `LOD<n>` (FBX LOD-group children are often named just
/// that). Anchored at the end so `"Wall_LOD2"` is 2 but `"LOD_Test_Wall"` is
/// nothing.
fn lod_suffix(name: &str) -> Option<u32> {
    let bytes = name.as_bytes();
    let digits_start = bytes.iter().rposition(|byte| !byte.is_ascii_digit())? + 1;
    if digits_start == bytes.len() {
        return None; // no trailing digits
    }
    let head = &bytes[..digits_start];
    let tagged = (head.len() >= 4 && head[head.len() - 4..].eq_ignore_ascii_case(b"_lod"))
        || head.eq_ignore_ascii_case(b"lod");
    if !tagged {
        return None;
    }
    name[digits_start..].parse().ok()
}

/// Build one bake group's occluder scene: every *visible* submesh belonging to
/// `group` — pieces of that LOD, plus every suffix-less piece (no LOD variants
/// means it exists at every level) — concatenated into a positions-meaningful
/// model (vertices + rebased indices only; bake-scoped scratch, see the module
/// docs). The hidden set is the handful of Outliner-hidden meshes, so the
/// linear membership test is cheaper than building a set (mirroring
/// `SceneBvh`). A `None` group only ever exists when no piece carries a
/// suffix, so it is simply the whole visible scene.
fn occluder_scene(
    submeshes: &[Submesh],
    piece_lods: &[Option<u32>],
    hidden_nodes: &[u32],
    group: Option<u32>,
) -> OccluderScene {
    let members = || {
        submeshes
            .iter()
            .zip(piece_lods)
            .filter(|(piece, lod)| {
                !hidden_nodes.contains(&piece.node)
                    && match (**lod, group) {
                        (None, _) => true,
                        (Some(lod), Some(group)) => lod == group,
                        // Unreachable: a `None` group means no piece anywhere
                        // carries a suffix.
                        (Some(_), None) => false,
                    }
            })
            .map(|(piece, _)| piece)
    };
    let total_vertices: usize = members().map(|piece| piece.vertices.len()).sum();
    let total_indices: usize = members().map(|piece| piece.indices.len()).sum();
    let mut vertices = Vec::with_capacity(total_vertices);
    let mut indices = Vec::with_capacity(total_indices);
    for piece in members() {
        let base = vertices.len() as u32;
        vertices.extend_from_slice(&piece.vertices);
        indices.extend(piece.indices.iter().map(|&index| base + index));
    }

    let mut bounds = Bounds::EMPTY;
    for vertex in &vertices {
        bounds.include_point(vertex.position);
    }
    let radius = if bounds.is_empty() {
        0.0
    } else {
        bounds.radius()
    };

    let occluders = ModelData {
        vertices,
        indices,
        ..Default::default()
    };
    let bvh = Bvh::build(&occluders);
    let adjacency = CornerAdjacency::build(&occluders);
    // Scale-aware self-intersection bias: models are normalized to meters but
    // spans still vary wildly, so tie it to the scene size, with a floor for a
    // degenerate (near-point) scene.
    let bias = (radius * 1.0e-4).max(1.0e-6);
    OccluderScene {
        occluders,
        bvh,
        adjacency,
        bias,
    }
}

fn bake_chunk(chunk: &mut [Vertex], params: &BakeAoParams, scene: &OccluderScene) {
    // One origin buffer reused across the chunk keeps the per-vertex loop
    // allocation-free.
    let mut origins: Vec<Vec3> = Vec::new();
    for vertex in chunk {
        let ao = vertex_ao(vertex.position, vertex.normal, params, scene, &mut origins);
        write_target(&mut vertex.vertex_color, ao, params.target, params.srgb);
    }
}

/// Fill `origins` with the vertex's surface-neighborhood ray starts (see the
/// module docs): two rings per incident triangle passing the alignment
/// filter, inset toward its centroid and lifted off the surface along the
/// vertex normal — sign-free (winding is arbitrary) and consistent with the
/// hemisphere's own orientation, with clearance of every surviving plane
/// guaranteed by the filter floor. Falls back to the bare corner origin when
/// nothing survives (an orphan or fully-degenerate neighborhood).
fn sample_origins(scene: &OccluderScene, position: Vec3, n: Vec3, origins: &mut Vec<Vec3>) {
    origins.clear();
    for &tri in scene.adjacency.incident(position) {
        let [a, b, c] = triangle_positions(&scene.occluders, tri);
        let geometric = (b - a).cross(c - a).normalize_or_zero();
        if geometric.dot(n).abs() <= NEIGHBOR_ALIGNMENT_MIN {
            continue;
        }
        let centroid = (a + b + c) / 3.0;
        for inset in ORIGIN_INSETS {
            origins.push(position.lerp(centroid, inset) + n * scene.bias);
        }
    }
    if origins.is_empty() {
        origins.push(position + n * scene.bias);
    }
}

/// One vertex's ambient occlusion in `0.0..=1.0` (1 = fully open): a
/// deterministic cosine-weighted hemisphere visibility estimate, its rays
/// dealt round-robin across the vertex's neighborhood origins. With a cosine
/// pdf every ray carries equal weight, so the estimate is simply the visible
/// fraction, then shaped by the intensity power. (At Low quality a vertex
/// whose valence exceeds the ray count leaves some origins unsampled —
/// deterministic, and the sampled subset is still an unbiased neighborhood.)
fn vertex_ao(
    position: Vec3,
    normal: Vec3,
    params: &BakeAoParams,
    scene: &OccluderScene,
    origins: &mut Vec<Vec3>,
) -> f32 {
    let length_squared = normal.length_squared();
    if length_squared < 1.0e-12 {
        // A degenerate normal has no hemisphere to sample (overlay-style
        // vertices) — leave it fully open rather than fabricating one.
        return 1.0;
    }
    let n = normal / length_squared.sqrt();
    let (tangent, bitangent) = orthonormal_basis(n);
    sample_origins(scene, position, n, origins);
    // `max_distance` is measured from each origin; the inset moves an origin
    // by a fraction of one edge length, immaterial at world scale.
    let t_max = if params.max_distance > 0.0 {
        params.max_distance
    } else {
        f32::INFINITY
    };

    // Hammersley points with a per-vertex Cranley–Patterson rotation: the same
    // well-stratified set every run, decorrelated between vertices so the
    // estimator's residual error dithers instead of banding.
    let rays = params.quality.rays();
    let rotation = scramble_hash(position, normal);
    let mut hits = 0usize;
    for ray in 0..rays {
        let origin = origins[ray % origins.len()];
        let u1 = (ray as f32 + 0.5) / rays as f32;
        let mut u2 = radical_inverse_base2(ray as u32) + rotation;
        if u2 >= 1.0 {
            u2 -= 1.0;
        }
        // Cosine-weighted mapping: uniform disk, projected up onto the
        // hemisphere around +z, then rotated into the vertex's frame.
        let disk_radius = u1.sqrt();
        let phi = std::f32::consts::TAU * u2;
        let direction = tangent * (disk_radius * phi.cos())
            + bitangent * (disk_radius * phi.sin())
            + n * (1.0 - u1).sqrt();
        if scene
            .bvh
            .ray_occluded(&scene.occluders, origin, direction, t_max)
        {
            hits += 1;
        }
    }

    let occlusion = hits as f32 / rays as f32;
    (1.0 - occlusion)
        .powf(params.intensity.max(0.01))
        .clamp(0.0, 1.0)
}

/// Write the baked value into its target channels. Alpha is always linear; the
/// sRGB encode applies only to the RGB-family targets. `MultiplyRgb`
/// multiplies the stored components by the encoded factor directly — the
/// stored RGB's own encoding is unknown to this crate, so the AO factor
/// honestly follows the user's chosen encoding.
fn write_target(color: &mut Vec4, ao: f32, target: AoTarget, srgb: bool) {
    let encoded = if srgb && target.is_rgb() {
        linear_to_srgb(ao)
    } else {
        ao
    };
    match target {
        AoTarget::Alpha => color.w = ao,
        AoTarget::Rgb => {
            color.x = encoded;
            color.y = encoded;
            color.z = encoded;
        }
        AoTarget::MultiplyRgb => {
            color.x *= encoded;
            color.y *= encoded;
            color.z *= encoded;
        }
        AoTarget::Red => color.x = encoded,
        AoTarget::Green => color.y = encoded,
        AoTarget::Blue => color.z = encoded,
    }
}

/// The van der Corput sequence in base 2: `i`'s bits mirrored across the
/// binary point, in `[0, 1)`.
fn radical_inverse_base2(i: u32) -> f32 {
    // 2^-32 — the mirrored bits land in [0, 2^32).
    i.reverse_bits() as f32 * 2.328_306_4e-10
}

/// A rotation offset in `[0, 1)` from the vertex's position + normal *bit
/// patterns* (never the values through arithmetic), so byte-identical seam
/// duplicates hash identically and results are bit-stable across runs.
fn scramble_hash(position: Vec3, normal: Vec3) -> f32 {
    let mut hash: u32 = 0x9E37_79B9;
    for component in [
        position.x, position.y, position.z, normal.x, normal.y, normal.z,
    ] {
        // Wang/murmur-style avalanche per component.
        hash ^= component.to_bits();
        hash = hash.wrapping_mul(0x85EB_CA6B);
        hash ^= hash >> 13;
        hash = hash.wrapping_mul(0xC2B2_AE35);
        hash ^= hash >> 16;
    }
    // 24 mantissa-safe bits into [0, 1).
    (hash >> 8) as f32 / (1u32 << 24) as f32
}

/// Branchless orthonormal basis around unit `n` (Duff et al., "Building an
/// Orthonormal Basis, Revisited").
fn orthonormal_basis(n: Vec3) -> (Vec3, Vec3) {
    let sign = 1.0_f32.copysign(n.z);
    let a = -1.0 / (sign + n.z);
    let b = n.x * n.y * a;
    (
        Vec3::new(1.0 + sign * n.x * n.x * a, sign * b, -sign * n.x),
        Vec3::new(b, sign + n.y * n.y * a, -n.y),
    )
}

fn linear_to_srgb(value: f32) -> f32 {
    if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use review_model::{ModelData, TriangleData, Vertex};

    use super::*;
    use crate::process::{ProcessInput, ProcessedResult, process};
    use crate::stack::{
        BakeAoParams, LodLevel, OpKind, OptStack, ReduceParams, SimplifyAlgorithm, SimplifySettings,
    };

    /// The demo cube through a stack, for the tests that only need a baseline.
    fn run(stack: &OptStack) -> ProcessedResult {
        run_model(&review_model::demo_cube_model(), stack)
    }
    use glam::Mat4;
    use review_model::{NodeKind, SceneNode};

    #[test]
    fn lod_suffixes_parse_by_convention() {
        assert_eq!(lod_suffix("Wall_LOD2"), Some(2));
        assert_eq!(lod_suffix("wall_lod10"), Some(10));
        assert_eq!(lod_suffix("SM_Speaker_01a_LOD0"), Some(0));
        assert_eq!(lod_suffix("LOD3"), Some(3), "a bare LOD-group child name");
        assert_eq!(lod_suffix("Wall"), None);
        assert_eq!(
            lod_suffix("Wall2"),
            None,
            "trailing digits alone are not a LOD"
        );
        assert_eq!(lod_suffix("LOD_Test_Wall"), None, "not anchored at the end");
        assert_eq!(
            lod_suffix("Palod3"),
            None,
            "no underscore, not a bare LOD name"
        );
        assert_eq!(lod_suffix("Wall_LOD"), None, "no digits");
        assert_eq!(lod_suffix(""), None);
    }

    #[test]
    fn a_lod_identity_is_inherited_from_an_ancestor_group() {
        let node = |name: &str, parent: Option<usize>| SceneNode {
            name: name.to_owned(),
            parent,
            mesh_part: None,
            source_vertex_count: 0,
            transform: Mat4::IDENTITY,
            rest_local: Default::default(),
            kind: NodeKind::Mesh,
            bone: None,
        };
        let model = ModelData {
            nodes: vec![
                node("Asset", None),
                node("LOD1", Some(0)),
                node("WheelMesh", Some(1)),
            ],
            ..ModelData::default()
        };
        assert_eq!(node_lod(&model, 2), Some(1), "the group names the LOD");
        assert_eq!(node_lod(&model, 0), None, "the root has no LOD identity");
        assert_eq!(node_lod(&model, 99), None, "an out-of-range node has none");
    }

    /// One corner position seen by four triangles: two corner-split floor
    /// copies (as a material seam leaves behind), a perpendicular wall, and a
    /// zero-area sliver. The adjacency must union them all; the origin filter
    /// must then keep only the ones aligned with the querying vertex's normal.
    #[test]
    fn corner_adjacency_unions_duplicates_and_the_filter_drops_edges() {
        let corner = Vec3::ZERO;
        let positions = [
            // tri 0: a +Y floor triangle.
            corner,
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            // tri 1: a second floor triangle over a corner-split duplicate.
            corner,
            Vec3::new(-1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, -1.0),
            // tri 2: a vertical wall (geometric normal +X).
            corner,
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            // tri 3: a zero-area sliver (two coincident corners).
            corner,
            corner,
            Vec3::new(1.0, 0.0, 0.0),
        ];
        let occluders = ModelData {
            vertices: positions
                .iter()
                .map(|&position| review_model::Vertex {
                    position,
                    ..review_model::Vertex::default()
                })
                .collect(),
            indices: (0..positions.len() as u32).collect(),
            ..ModelData::default()
        };
        let scene = OccluderScene {
            bvh: Bvh::build(&occluders),
            adjacency: CornerAdjacency::build(&occluders),
            occluders,
            bias: 1.0e-6,
        };

        // The union, in triangle order; the degenerate lists once per
        // coincident corner.
        assert_eq!(scene.adjacency.incident(corner), &[0, 1, 2, 3, 3]);
        assert_eq!(
            scene.adjacency.incident(Vec3::new(9.0, 9.0, 9.0)),
            &[] as &[u32],
            "an unseen position has no neighbors"
        );

        // A +Y vertex normal keeps only the two floor triangles (two ring
        // origins each): the wall is near-perpendicular, the sliver's
        // geometric normal is zero.
        let mut origins = Vec::new();
        sample_origins(&scene, corner, Vec3::Y, &mut origins);
        assert_eq!(origins.len(), 4);
        let expected_0 = corner.lerp(Vec3::new(1.0 / 3.0, 0.0, 1.0 / 3.0), ORIGIN_INSETS[0]);
        assert!(
            (origins[0].x - expected_0.x).abs() < 1.0e-6
                && (origins[0].z - expected_0.z).abs() < 1.0e-6
                && origins[0].y > 0.0,
            "the origin is inset toward the centroid and lifted: {:?}",
            origins[0]
        );

        // A +X vertex normal keeps only the wall.
        sample_origins(&scene, corner, Vec3::X, &mut origins);
        assert_eq!(origins.len(), 2);

        // No incident triangles at all: the bare corner origin.
        sample_origins(&scene, Vec3::new(9.0, 9.0, 9.0), Vec3::Y, &mut origins);
        assert_eq!(origins.len(), 1);
        assert_eq!(origins[0], Vec3::new(9.0, 9.0, 9.0) + Vec3::Y * scene.bias);
    }

    /// A stand-in for the renderer's `SceneVertex` size in tests.
    const VERTEX_SIZE: usize = 48;

    fn run_model(model: &ModelData, stack: &OptStack) -> ProcessedResult {
        run_model_hiding(model, stack, &[])
    }

    fn run_model_hiding(model: &ModelData, stack: &OptStack, hidden: &[u32]) -> ProcessedResult {
        process(ProcessInput {
            model,
            stack,
            render_vertex_size: VERTEX_SIZE,
            hidden_nodes: hidden,
            extras: None,
        })
        .expect("processing succeeds")
    }

    /// A model of named, upward-facing horizontal quads for the AO tests — one
    /// scene node per `(name, half-extent, height)` entry. Every vertex carries
    /// a distinctive source color so the tests can tell "written as fully open"
    /// apart from "never written".
    fn named_quads_model(quads: &[(&str, f32, f32)]) -> ModelData {
        let placed: Vec<(&str, f32, Vec3)> = quads
            .iter()
            .map(|&(name, half, y)| (name, half, Vec3::new(0.0, y, 0.0)))
            .collect();
        placed_quads_model(&placed)
    }

    /// The general form of [`named_quads_model`]: each quad centered anywhere,
    /// not only on the y-axis.
    fn placed_quads_model(quads: &[(&str, f32, Vec3)]) -> ModelData {
        use glam::Mat4;
        use review_model::{NodeKind, SceneNode};

        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut node_tags = Vec::new();
        let mut nodes = Vec::new();
        for (index, &(name, half, center)) in quads.iter().enumerate() {
            let base = vertices.len() as u32;
            for position in [
                center + Vec3::new(-half, 0.0, -half),
                center + Vec3::new(half, 0.0, -half),
                center + Vec3::new(half, 0.0, half),
                center + Vec3::new(-half, 0.0, half),
            ] {
                vertices.push(Vertex {
                    position,
                    normal: Vec3::Y,
                    vertex_color: Vec4::new(0.2, 0.4, 0.6, 0.8),
                    ..Vertex::default()
                });
            }
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            node_tags.extend_from_slice(&[index as u32, index as u32]);
            nodes.push(SceneNode {
                name: name.to_owned(),
                parent: None,
                mesh_part: Some(index),
                source_vertex_count: 0,
                transform: Mat4::IDENTITY,
                rest_local: Default::default(),
                kind: NodeKind::Mesh,
                bone: None,
            });
        }
        ModelData {
            vertices,
            indices,
            triangles: TriangleData {
                node: node_tags,
                ..TriangleData::default()
            },
            nodes,
            ..ModelData::default()
        }
    }

    /// Two nodes with no LOD identity: node 0 is a 1×1 quad at y = 0 facing
    /// +Y, node 1 a 2×2 cover hovering at y = 0.5 above it. The geometry is
    /// sized so node 0's vertices end up *partially* occluded (strictly
    /// between 0 and 1), which is what the target/encode/intensity comparisons
    /// need.
    fn covered_quad_model() -> ModelData {
        named_quads_model(&[("lower", 0.5, 0.0), ("cover", 1.0, 0.5)])
    }

    /// The processed vertices of the lower (covered) quad / the upper cover,
    /// identified by height rather than order so the tests survive reordering.
    fn split_by_height(model: &ModelData) -> (Vec<Vertex>, Vec<Vertex>) {
        model
            .vertices
            .iter()
            .partition(|vertex| vertex.position.y < 0.25)
    }

    /// A convex mesh occludes none of its own hemispheres: every vertex must
    /// come out fully open — any value below 1 would be a self-intersection
    /// artifact the ray-origin bias exists to prevent.
    #[test]
    fn bake_ao_leaves_a_convex_mesh_fully_open() {
        let mut baseline = OptStack::default();
        baseline.push_op(OpKind::FilterTriangles);
        let mut baked = OptStack::default();
        baked.push_op(OpKind::FilterTriangles);
        baked.push_op(OpKind::BakeAo(BakeAoParams::default()));

        let baseline = run(&baseline);
        let baked = run(&baked);
        for (before, after) in baseline.lods[0]
            .model
            .vertices
            .iter()
            .zip(&baked.lods[0].model.vertices)
        {
            assert!(
                after.vertex_color.w > 0.99,
                "a convex surface is fully open, got {}",
                after.vertex_color.w
            );
            assert_eq!(
                after.vertex_color.truncate(),
                before.vertex_color.truncate(),
                "the alpha target leaves RGB untouched"
            );
        }
    }

    #[test]
    fn bake_ao_darkens_a_covered_surface() {
        let model = covered_quad_model();
        let mut stack = OptStack::default();
        stack.push_op(OpKind::BakeAo(BakeAoParams::default()));

        let result = run_model(&model, &stack);
        let (covered, cover) = split_by_height(&result.lods[0].model);
        assert!(!covered.is_empty() && !cover.is_empty());
        for vertex in &covered {
            let ao = vertex.vertex_color.w;
            assert!(
                ao < 0.5 && ao > 0.0,
                "a covered vertex is mostly but not fully occluded, got {ao}"
            );
        }
        for vertex in &cover {
            assert!(
                vertex.vertex_color.w > 0.99,
                "nothing hangs over the cover, got {}",
                vertex.vertex_color.w
            );
        }
    }

    #[test]
    fn bake_ao_max_distance_releases_distant_occluders() {
        let model = covered_quad_model();
        let mut stack = OptStack::default();
        stack.push_op(OpKind::BakeAo(BakeAoParams {
            // The cover hangs 0.5 above; a shorter reach never finds it.
            max_distance: 0.4,
            ..BakeAoParams::default()
        }));

        let (covered, _) = split_by_height(&run_model(&model, &stack).lods[0].model);
        for vertex in &covered {
            assert!(
                vertex.vertex_color.w > 0.99,
                "the cover is out of reach, got {}",
                vertex.vertex_color.w
            );
        }
    }

    /// Exclusion means "don't touch this object's data" — not "pretend it
    /// isn't there": an excluded object still occludes its neighbours.
    #[test]
    fn an_excluded_node_still_occludes_but_is_not_written() {
        let model = covered_quad_model();
        let mut stack = OptStack::default();
        stack.push_op(OpKind::BakeAo(BakeAoParams {
            target: AoTarget::Rgb,
            ..BakeAoParams::default()
        }));
        stack.node_override_mut(1).exclude = true;

        let (covered, cover) = split_by_height(&run_model(&model, &stack).lods[0].model);
        for vertex in &covered {
            assert!(
                vertex.vertex_color.x < 0.5,
                "the excluded cover still darkens the quad below, got {}",
                vertex.vertex_color.x
            );
        }
        for vertex in &cover {
            assert_eq!(
                vertex.vertex_color,
                Vec4::new(0.2, 0.4, 0.6, 0.8),
                "the excluded object's own colors are untouched"
            );
        }
    }

    /// A hidden object is out of the bake's scene entirely: it neither occludes
    /// (unlike an *excluded* one, which does) nor receives colors. This is what
    /// keeps assets carrying hidden co-located LOD copies and collision shells
    /// bakeable at all — those invisible near-coincident surfaces would
    /// otherwise shadow every vertex of the visible mesh.
    #[test]
    fn a_hidden_node_neither_occludes_nor_is_baked() {
        let model = covered_quad_model();
        let mut stack = OptStack::default();
        stack.push_op(OpKind::BakeAo(BakeAoParams::default()));

        // Hiding the cover releases the quad below it...
        let (covered, cover) =
            split_by_height(&run_model_hiding(&model, &stack, &[1]).lods[0].model);
        for vertex in &covered {
            assert!(
                vertex.vertex_color.w > 0.99,
                "a hidden cover casts no occlusion, got {}",
                vertex.vertex_color.w
            );
        }
        // ...and the hidden cover itself keeps its source colors.
        for vertex in &cover {
            assert_eq!(
                vertex.vertex_color,
                Vec4::new(0.2, 0.4, 0.6, 0.8),
                "a hidden object is not baked"
            );
        }
    }

    /// The classic per-vertex-AO failure this bake explicitly guards against:
    /// a face whose corners sit in tight contact gaps used to bake black
    /// across its whole area, because occlusion was sampled exactly at the
    /// buried corner point (visibility ≈ 0.03 under a ±0.06 cap hovering
    /// 0.005 above — measured 0.0156 before the fix). With neighborhood
    /// sampling the ray origins are inset onto the incident faces — out from
    /// under the caps — so the mostly-open floor stays open. The caps sit on
    /// the floor's *diagonal* corners too, the ones with a single incident
    /// triangle, so this also pins the low-valence case the two-ring inset
    /// exists for.
    #[test]
    fn a_corner_buried_under_a_tight_cap_stays_open() {
        let model = placed_quads_model(&[
            ("floor", 0.5, Vec3::ZERO),
            ("cap0", 0.06, Vec3::new(-0.5, 0.005, -0.5)),
            ("cap1", 0.06, Vec3::new(0.5, 0.005, -0.5)),
            ("cap2", 0.06, Vec3::new(0.5, 0.005, 0.5)),
            ("cap3", 0.06, Vec3::new(-0.5, 0.005, 0.5)),
        ]);
        let mut stack = OptStack::default();
        stack.push_op(OpKind::BakeAo(BakeAoParams::default()));

        for vertex in &run_model(&model, &stack).lods[0].model.vertices {
            let ao = vertex.vertex_color.w;
            if vertex.position.y < 0.005 {
                assert!(
                    ao > 0.85,
                    "a corner buried under a tight cap samples its open \
                     neighborhood, got {ao}"
                );
            } else {
                assert!(ao > 0.99, "nothing hangs over a cap, got {ao}");
            }
        }
    }

    /// Same geometry as [`covered_quad_model`], but the nodes carry LOD
    /// suffixes — the co-located-LOD-chain layout every game FBX ships. Each
    /// LOD bakes only against its own group, so the "cover" (a different LOD)
    /// casts nothing on the quad below it and both come out fully open. This
    /// is what lets an artist keep the whole chain visible and bake every LOD
    /// in one run.
    #[test]
    fn co_located_lod_copies_do_not_shadow_each_other() {
        let model = named_quads_model(&[("Thing_LOD0", 0.5, 0.0), ("Thing_lod1", 1.0, 0.5)]);
        let mut stack = OptStack::default();
        stack.push_op(OpKind::BakeAo(BakeAoParams::default()));

        for vertex in &run_model(&model, &stack).lods[0].model.vertices {
            assert!(
                vertex.vertex_color.w > 0.99,
                "a different LOD never occludes, got {}",
                vertex.vertex_color.w
            );
        }
    }

    /// A suffix-less mesh has no LOD variants, so it exists at every level: it
    /// occludes each LOD group, and the LOD quads still ignore each other.
    #[test]
    fn an_unsuffixed_object_occludes_every_lod() {
        let model = named_quads_model(&[
            ("Q_LOD0", 0.5, 0.0),
            ("Q_LOD1", 0.5, 0.05),
            ("Cover", 2.0, 0.5),
        ]);
        let mut stack = OptStack::default();
        stack.push_op(OpKind::BakeAo(BakeAoParams::default()));

        for vertex in &run_model(&model, &stack).lods[0].model.vertices {
            let ao = vertex.vertex_color.w;
            if vertex.position.y < 0.2 {
                assert!(
                    ao < 0.5,
                    "the shared cover darkens both LOD quads, got {ao}"
                );
            } else {
                assert!(ao > 0.99, "nothing hangs over the cover, got {ao}");
            }
        }
    }

    /// Suffix-less meshes bake against the lowest LOD present — the
    /// ground-truth geometry — so a floor under a LOD'd canopy still darkens.
    #[test]
    fn an_unsuffixed_object_is_occluded_by_the_lowest_lod() {
        let model = named_quads_model(&[
            ("Floor", 0.5, 0.0),
            ("Canopy_LOD0", 1.0, 0.5),
            ("Canopy_LOD1", 1.5, 0.6),
        ]);
        let mut stack = OptStack::default();
        stack.push_op(OpKind::BakeAo(BakeAoParams::default()));

        let baked = run_model(&model, &stack);
        for vertex in &baked.lods[0].model.vertices {
            if vertex.position.y < 0.2 {
                assert!(
                    vertex.vertex_color.w < 0.5,
                    "the LOD0 canopy darkens the suffix-less floor, got {}",
                    vertex.vertex_color.w
                );
            }
        }
    }

    #[test]
    fn bake_ao_is_deterministic() {
        let model = covered_quad_model();
        let mut stack = OptStack::default();
        stack.push_op(OpKind::BakeAo(BakeAoParams::default()));

        let first = run_model(&model, &stack);
        let second = run_model(&model, &stack);
        for (a, b) in first.lods[0]
            .model
            .vertices
            .iter()
            .zip(&second.lods[0].model.vertices)
        {
            assert_eq!(
                a.vertex_color.to_array(),
                b.vertex_color.to_array(),
                "two runs are bit-identical"
            );
        }
    }

    /// Every write target touches exactly its own channels, leaving the
    /// authored color in the rest.
    #[test]
    fn each_bake_target_touches_only_its_channels() {
        let model = covered_quad_model();
        let bake = |target: AoTarget| {
            let mut stack = OptStack::default();
            stack.push_op(OpKind::BakeAo(BakeAoParams {
                target,
                ..BakeAoParams::default()
            }));
            let (covered, _) = split_by_height(&run_model(&model, &stack).lods[0].model);
            covered[0].vertex_color
        };
        let source = Vec4::new(0.2, 0.4, 0.6, 0.8);

        let alpha = bake(AoTarget::Alpha);
        assert_eq!(alpha.truncate(), source.truncate());
        assert!(alpha.w < 0.5, "alpha carries the occlusion");

        let rgb = bake(AoTarget::Rgb);
        assert!(rgb.x == rgb.y && rgb.y == rgb.z, "grayscale");
        assert!(rgb.x < 0.5);
        assert_eq!(rgb.w, source.w);

        let multiplied = bake(AoTarget::MultiplyRgb);
        assert!(multiplied.x < source.x && multiplied.y < source.y);
        assert_eq!(multiplied.w, source.w);
        let ratio_x = multiplied.x / source.x;
        let ratio_y = multiplied.y / source.y;
        assert!(
            (ratio_x - ratio_y).abs() < 1.0e-6,
            "one factor multiplies every channel: {ratio_x} vs {ratio_y}"
        );

        let red = bake(AoTarget::Red);
        assert!(red.x < 0.5);
        assert_eq!(red.y, source.y);
        assert_eq!(red.z, source.z);
        assert_eq!(red.w, source.w);

        let green = bake(AoTarget::Green);
        assert_eq!(green.x, source.x);
        assert!(green.y < 0.5);
        assert_eq!(green.w, source.w);

        let blue = bake(AoTarget::Blue);
        assert_eq!(blue.x, source.x);
        assert!(blue.z < 0.5);
        assert_eq!(blue.w, source.w);
    }

    /// The sRGB option encodes the RGB-family writes (brightening any value in
    /// (0, 1)) and never applies to the alpha target.
    #[test]
    fn bake_ao_srgb_encodes_rgb_but_never_alpha() {
        let model = covered_quad_model();
        let bake = |target: AoTarget, srgb: bool| {
            let mut stack = OptStack::default();
            stack.push_op(OpKind::BakeAo(BakeAoParams {
                target,
                srgb,
                ..BakeAoParams::default()
            }));
            let (covered, _) = split_by_height(&run_model(&model, &stack).lods[0].model);
            covered[0].vertex_color
        };

        let linear = bake(AoTarget::Rgb, false);
        let encoded = bake(AoTarget::Rgb, true);
        assert!(
            encoded.x > linear.x,
            "sRGB encoding brightens a mid value: {} vs {}",
            encoded.x,
            linear.x
        );

        assert_eq!(
            bake(AoTarget::Alpha, true).w,
            bake(AoTarget::Alpha, false).w,
            "alpha is always written linear"
        );
    }

    #[test]
    fn bake_ao_intensity_darkens() {
        let model = covered_quad_model();
        let bake = |intensity: f32| {
            let mut stack = OptStack::default();
            stack.push_op(OpKind::BakeAo(BakeAoParams {
                intensity,
                ..BakeAoParams::default()
            }));
            let (covered, _) = split_by_height(&run_model(&model, &stack).lods[0].model);
            covered[0].vertex_color.w
        };

        assert!(bake(2.0) < bake(1.0), "a higher power darkens mid values");
    }

    #[test]
    fn a_bake_above_a_simplifier_gets_an_advisory() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::BakeAo(BakeAoParams::default()));
        stack.push_op(OpKind::Reduce(ReduceParams {
            simplify: SimplifySettings {
                algorithm: SimplifyAlgorithm::Sloppy,
                ..SimplifySettings::default()
            },
            target: LodLevel {
                target_ratio: 0.5,
                target_error: 1.0,
            },
        }));
        assert!(
            run(&stack)
                .warnings
                .iter()
                .any(|warning| warning.contains("Bake AO runs before a simplifier")),
            "the ordering advisory is raised"
        );

        stack.reorder(0, 1);
        assert!(
            !run(&stack)
                .warnings
                .iter()
                .any(|warning| warning.contains("Bake AO runs before a simplifier")),
            "baking after the simplifier is the recommended order"
        );
    }
}
