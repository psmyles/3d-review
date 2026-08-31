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
//! Each occluder model is a transient concatenation of submesh geometry, not a
//! copy of anything shared (invariant 1 in spirit — it is bake scratch, like
//! the BVH builder's own centroid arrays). It is also what makes the parallel
//! bake safely expressible: the worker threads mutate `vertex_color` on the
//! live submesh vertices while raycasting, so the read side has to be a
//! separate immutable snapshot rather than a borrow of the same arrays.
//!
//! Everything here is a pure function of the vertex bits and the occluder set:
//! two runs produce bit-identical colors, and vertices duplicated across
//! submesh seams (byte-identical position + normal, copied from the same
//! source corner) get identical AO, so seams never show.

use std::collections::BTreeMap;

use glam::{Vec3, Vec4};
use review_model::{Bounds, Bvh, ModelData, Vertex};

use crate::process::{is_excluded, resolve_op};
use crate::stack::{AoTarget, BakeAoParams, OpInstance, OpKind, OptStack};
use crate::submesh::Submesh;

/// Vertices per job in the thread-scope work list — small enough to balance
/// uneven submesh sizes across workers, large enough that job bookkeeping is
/// noise next to the raycasts.
const CHUNK: usize = 1024;

/// One bake group's raycast world: the occluder geometry, its hierarchy, and
/// the self-intersection bias sized to it.
struct OccluderScene {
    occluders: ModelData,
    bvh: Bvh,
    bias: f32,
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
    // Scale-aware self-intersection bias: models are normalized to meters but
    // spans still vary wildly, so tie it to the scene size, with a floor for a
    // degenerate (near-point) scene.
    let bias = (radius * 1.0e-4).max(1.0e-6);
    OccluderScene {
        occluders,
        bvh,
        bias,
    }
}

fn bake_chunk(chunk: &mut [Vertex], params: &BakeAoParams, scene: &OccluderScene) {
    for vertex in chunk {
        let ao = vertex_ao(
            vertex.position,
            vertex.normal,
            params,
            &scene.bvh,
            &scene.occluders,
            scene.bias,
        );
        write_target(&mut vertex.vertex_color, ao, params.target, params.srgb);
    }
}

/// One vertex's ambient occlusion in `0.0..=1.0` (1 = fully open): a
/// deterministic cosine-weighted hemisphere visibility estimate. With a cosine
/// pdf every ray carries equal weight, so the estimate is simply the visible
/// fraction, then shaped by the intensity power.
fn vertex_ao(
    position: Vec3,
    normal: Vec3,
    params: &BakeAoParams,
    bvh: &Bvh,
    occluders: &ModelData,
    bias: f32,
) -> f32 {
    let length_squared = normal.length_squared();
    if length_squared < 1.0e-12 {
        // A degenerate normal has no hemisphere to sample (overlay-style
        // vertices) — leave it fully open rather than fabricating one.
        return 1.0;
    }
    let n = normal / length_squared.sqrt();
    let (tangent, bitangent) = orthonormal_basis(n);
    let origin = position + n * bias;
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
        if bvh.ray_occluded(occluders, origin, direction, t_max) {
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
    use super::*;
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
            transform: Mat4::IDENTITY,
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
}
