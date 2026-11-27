//! Cutting a piece's triangles into islands along the seams the settings name.
//!
//! Two triangles are in one island when a chain of shared edges joins them
//! and no edge of the chain is a seam. Edges are matched by *position*, not by
//! vertex index: a piece's vertices are already split wherever a normal or a
//! UV changes, and a hard edge is no reason to cut an island. An edge is a seam
//! when the triangles either side of it differ in the seam key (a CAD face id),
//! when the reference UVs are discontinuous across it and the settings say to
//! cut there ([`ExistingSeams::Cut`]) — by more than [`REFERENCE_SEAM_GAP`],
//! since the simplifier refits each copy of a vertex at a hard edge on its own
//! and they drift apart by a few ulps — when they meet sharper than the crease
//! angle (when that is asked for), or when it is not shared by exactly two
//! triangles.
//!
//! **No seam runs inside a source polygon.** The two triangles of a quad are one
//! face to the artist and to the exporter, which writes the quad back with one
//! UV per corner: cut along its diagonal, and one triangle's corners would have
//! to take the other's UVs (`Submesh::split_by_corner` makes the corners of one
//! face agree), stretching it across the atlas. So the triangles of one polygon
//! are always joined, whatever lies between them, and so are their corners at
//! each point. [`Cut::face`] numbers the polygons, so the seams placed later
//! keep to their edges too.
//!
//! With [`ExistingSeams::Prefer`] the reference UVs' seams cut nothing; each
//! edge along one is marked in [`Cut::preferred`], and a seam the unwrap has to
//! place costs less there.
//!
//! The same joins, made at the corners either end of each joined edge, number
//! the cut surface's vertices: two corners share one only where a chain of
//! joined edges round their position links them, so a seam that runs into an
//! island and stops (a cylinder's slit) stays cut even though the island does
//! not fall apart along it.
//!
//! Islands are numbered in the order of their first triangle, and vertices and
//! faces in the order of their first corner, so the numbering is a function of
//! the piece alone.

use std::collections::HashMap;

use glam::{Vec2, Vec3};

use super::SeamKey;
use crate::stack::{ExistingSeams, UnwrapParams};
use crate::submesh::{NO_FACE, Submesh};

/// How far apart, in UV units, the reference UVs either side of an edge must be
/// for it to be a seam: a tenth of a millimetre at one unit per metre. An
/// authored seam opens by whole fractions of the texture; a refit moves a copy
/// by far less.
pub(super) const REFERENCE_SEAM_GAP: f32 = 1.0e-4;

/// A piece cut along its seams.
pub(super) struct Cut {
    /// Per triangle, its island.
    pub island: Vec<u32>,
    /// Per corner, its vertex in the cut surface: two corners share one only
    /// where they are at one position and joined round it by edges no seam
    /// runs along, or are corners of one polygon there. Numbered in order of
    /// first corner.
    pub vertex: Vec<u32>,
    /// How many cut vertices there are.
    pub vertex_count: u32,
    /// Per triangle, the polygon it was cut from — shared by every triangle of
    /// one source polygon, and a triangle's own where the piece carries no
    /// polygons. Numbered in order of first triangle.
    pub face: Vec<u32>,
    /// How many faces there are.
    pub face_count: u32,
    /// Per triangle, a bit per edge (bit `k` for corner `k` to `k + 1`) the
    /// reference UVs are cut along, where seams are to be preferred there.
    pub preferred: Vec<u8>,
}

/// Cut a piece along its seams: its islands, and its corners' vertices in the
/// cut surface.
pub(super) fn islands(
    piece: &Submesh,
    positions: &[Vec3],
    reference: Option<&[Vec2]>,
    key: SeamKey,
    params: &UnwrapParams,
    uv_sets: usize,
) -> Cut {
    let triangles = piece.indices.as_chunks::<3>().0;

    // Each vertex's position, as an id shared by every vertex there.
    let mut ids: HashMap<[u32; 3], u32> = HashMap::new();
    let point: Vec<u32> = positions
        .iter()
        .map(|position| {
            let next = ids.len() as u32;
            *ids.entry(position.to_array().map(canonical))
                .or_insert(next)
        })
        .collect();

    // Each triangle's face: the carried polygon's, renumbered, or its own.
    let carried = piece
        .polygons
        .as_ref()
        .map(|carry| carry.triangle_face.as_slice())
        .filter(|faces| faces.len() == triangles.len());
    let mut face_number: HashMap<u32, u32> = HashMap::new();
    let mut face_count = 0u32;
    let face: Vec<u32> = (0..triangles.len())
        .map(|triangle| {
            let source = carried.map_or(NO_FACE, |faces| faces[triangle]);
            let mut fresh = || {
                face_count += 1;
                face_count - 1
            };
            if source == NO_FACE {
                fresh()
            } else {
                *face_number.entry(source).or_insert_with(fresh)
            }
        })
        .collect();

    // Every edge, by its two points, with the triangles that use it.
    let mut edges: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    for (triangle, corners) in triangles.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (
                point[corners[k] as usize],
                point[corners[(k + 1) % 3] as usize],
            );
            if a == b {
                continue;
            }
            edges
                .entry((a.min(b), a.max(b)))
                .or_default()
                .push(triangle as u32);
        }
    }

    let seam_of = |triangle: usize| -> Vec<u32> {
        let corner = triangles[triangle][0] as usize;
        match key {
            SeamKey::None => Vec::new(),
            SeamKey::Custom { offset, width } => piece
                .custom
                .row(corner)
                .get(offset..offset + width)
                .map(<[u32]>::to_vec)
                .unwrap_or_default(),
            SeamKey::UvSetU(set) if set < uv_sets.max(piece.uv_channels.len()) => {
                let u = match piece.uv_channels.get(set) {
                    Some(channel) => channel[corner].x,
                    None => piece.vertices[corner].uv.x,
                };
                // An id carried as a coordinate is a whole number, and the
                // simplifier's refit of the set moves it off one by a little.
                vec![u.round().to_bits()]
            }
            SeamKey::UvSetU(_) => Vec::new(),
        }
    };
    let seam: Vec<Vec<u32>> = (0..triangles.len()).map(seam_of).collect();
    let normal: Vec<Vec3> = triangles
        .iter()
        .map(|corners| {
            let [a, b, c] = corners.map(|corner| positions[corner as usize]);
            (b - a).cross(c - a).normalize_or_zero()
        })
        .collect();
    let crease_cosine = params
        .split_at_creases
        .then(|| review_model::exact::cos(params.crease_angle.to_radians()));
    let existing = if reference.is_some() {
        params.existing_seams
    } else {
        ExistingSeams::Ignore
    };

    // The reference UV a triangle's corner at point `at` has.
    let uv_at = |triangle: usize, at: u32| -> Option<Vec2> {
        let reference = reference?;
        triangles[triangle]
            .iter()
            .find(|&&corner| point[corner as usize] == at)
            .map(|&corner| reference[corner as usize])
    };
    // The edge of `triangle` from point `a` to point `b`, either way round.
    let edge_of = |triangle: usize, a: u32, b: u32| -> Option<usize> {
        (0..3).find(|&k| {
            let (x, y) = (
                point[triangles[triangle][k] as usize],
                point[triangles[triangle][(k + 1) % 3] as usize],
            );
            (x, y) == (a, b) || (x, y) == (b, a)
        })
    };

    let mut parent: Vec<u32> = (0..triangles.len() as u32).collect();
    // Corners, joined as the triangles are: across each edge, the corners at
    // either end of it.
    let mut corner_parent: Vec<u32> = (0..piece.indices.len() as u32).collect();
    let mut preferred = vec![0u8; triangles.len()];
    let corner_at = |triangle: usize, at: u32| -> u32 {
        let k = (0..3)
            .find(|&k| point[triangles[triangle][k] as usize] == at)
            .unwrap_or(0);
        (triangle * 3 + k) as u32
    };
    // Walk the triangles' own edges in order, so the joins are made in an
    // order fixed by the piece (the map is only looked up).
    for (triangle, corners) in triangles.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (
                point[corners[k] as usize],
                point[corners[(k + 1) % 3] as usize],
            );
            if a == b {
                continue;
            }
            let Some(users) = edges.get(&(a.min(b), a.max(b))) else {
                continue;
            };
            // A border, or a non-manifold edge: a seam either way.
            let [first, second] = users.as_slice() else {
                continue;
            };
            let other = if *first as usize == triangle {
                *second
            } else {
                *first
            } as usize;
            if other <= triangle {
                continue;
            }
            // Inside one polygon nothing cuts.
            if face[triangle] != face[other] {
                if seam[triangle] != seam[other] {
                    continue;
                }
                let discontinuous = existing != ExistingSeams::Ignore
                    && [a, b]
                        .iter()
                        .any(|&at| match (uv_at(triangle, at), uv_at(other, at)) {
                            (Some(mine), Some(theirs)) => {
                                mine.distance(theirs) > REFERENCE_SEAM_GAP
                            }
                            (mine, theirs) => mine.is_some() != theirs.is_some(),
                        });
                if discontinuous {
                    if existing == ExistingSeams::Cut {
                        continue;
                    }
                    preferred[triangle] |= 1 << k;
                    if let Some(j) = edge_of(other, a, b) {
                        preferred[other] |= 1 << j;
                    }
                }
                if let Some(cosine) = crease_cosine
                    && normal[triangle].dot(normal[other]) < cosine
                {
                    continue;
                }
            }
            union(&mut parent, triangle as u32, other as u32);
            for at in [a, b] {
                union(
                    &mut corner_parent,
                    corner_at(triangle, at),
                    corner_at(other, at),
                );
            }
        }
    }
    // The triangles of one polygon, and its corners at one point, are one
    // whatever their edges say: a polygon is never cut.
    let mut first_of_face: HashMap<u32, u32> = HashMap::new();
    let mut first_corner: HashMap<(u32, u32), u32> = HashMap::new();
    for (triangle, corners) in triangles.iter().enumerate() {
        let own = face[triangle];
        let first = *first_of_face.entry(own).or_insert(triangle as u32);
        union(&mut parent, first, triangle as u32);
        for (k, &corner) in corners.iter().enumerate() {
            let at = (triangle * 3 + k) as u32;
            let first = *first_corner
                .entry((own, point[corner as usize]))
                .or_insert(at);
            union(&mut corner_parent, first, at);
        }
    }

    // Number the islands by their first triangle, the vertices by their first
    // corner.
    let mut number: HashMap<u32, u32> = HashMap::new();
    let island = (0..triangles.len() as u32)
        .map(|triangle| {
            let at = root(&mut parent, triangle);
            let next = number.len() as u32;
            *number.entry(at).or_insert(next)
        })
        .collect();
    let mut number: HashMap<u32, u32> = HashMap::new();
    let vertex = (0..piece.indices.len() as u32)
        .map(|corner| {
            let at = root(&mut corner_parent, corner);
            let next = number.len() as u32;
            *number.entry(at).or_insert(next)
        })
        .collect();
    Cut {
        island,
        vertex,
        vertex_count: number.len() as u32,
        face,
        face_count,
        preferred,
    }
}

fn root(parent: &mut [u32], mut at: u32) -> u32 {
    while parent[at as usize] != at {
        parent[at as usize] = parent[parent[at as usize] as usize];
        at = parent[at as usize];
    }
    at
}

/// Join two sets; the lower root wins, so the result does not depend on the
/// order the joins are made in.
fn union(parent: &mut [u32], a: u32, b: u32) {
    let (x, y) = (root(parent, a), root(parent, b));
    if x != y {
        parent[x.max(y) as usize] = x.min(y);
    }
}

/// `-0.0` and `+0.0` as one position.
fn canonical(value: f32) -> u32 {
    if value == 0.0 { 0 } else { value.to_bits() }
}
