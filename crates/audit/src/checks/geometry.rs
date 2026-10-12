//! Geometry and topology: degenerate and inverted faces, non-manifold and hard
//! edges, stray and duplicate vertices, n-gons, missing shading data, budgets.

use std::collections::{HashMap, HashSet};

use glam::Vec3;
use review_model::topology::{edge_uses_of_faces, group_edge_uses};

use super::{Outcome, count_param, largest};
use crate::context::Context;
use crate::finding::{ElementSet, Measured, Offender, RangeSet, Skip, Threshold};
use crate::profile::RuleConfig;

/// A triangle is degenerate when its area is this small against its longest
/// edge squared — near-collinear or collapsed, invisible as a surface.
const DEGENERATE_RATIO: f32 = 1.0e-7;

/// How firmly a corner normal must oppose its face's winding to count toward
/// an inverted face (the cosine of the angle past perpendicular).
const INVERTED_COS: f32 = 0.2;

/// Two normals more than this far apart (as `1 - cos`) make an edge hard.
const HARD_EDGE_COS: f32 = 0.9999;

/// The render corners of polygon `face` (or triangle, for a model without face
/// topology).
fn face_corners(ctx: &Context<'_>, face: u32) -> Vec<u32> {
    let model = ctx.model;
    if model.faces.is_empty() {
        return model
            .indices
            .get(face as usize * 3..face as usize * 3 + 3)
            .map(<[u32]>::to_vec)
            .unwrap_or_default();
    }
    model
        .faces
        .get(face as usize)
        .map_or_else(Vec::new, |polygon| {
            (polygon.first_index..polygon.first_index + polygon.index_count).collect()
        })
}

/// The triangles of `node` whose polygon `flagged` accepts, as a range set.
fn triangles_of_faces(ctx: &Context<'_>, node: usize, flagged: impl Fn(u32) -> bool) -> RangeSet {
    let to_face = &ctx.model.triangles.to_face;
    let by_triangle = ctx.model.faces.is_empty() || to_face.is_empty();
    RangeSet::from_sorted(
        ctx.node_triangles()[node]
            .iter()
            .copied()
            .filter(|&triangle| {
                let face = if by_triangle {
                    triangle
                } else {
                    to_face[triangle as usize]
                };
                flagged(face)
            }),
    )
}

pub(super) fn degenerate_triangles(ctx: &Context<'_>) -> Outcome {
    let Some(per_node) = ctx.par_nodes(|node| {
        RangeSet::from_sorted(
            ctx.node_triangles()[node]
                .iter()
                .copied()
                .filter(|&triangle| {
                    review_model::measure::triangle_corners(ctx.model, triangle as usize)
                        .is_some_and(|[a, b, c]| {
                            let longest = (b - a)
                                .length_squared()
                                .max((c - b).length_squared())
                                .max((a - c).length_squared());
                            let doubled_area = (b - a).cross(c - a).length();
                            longest == 0.0 || doubled_area <= DEGENERATE_RATIO * longest
                        })
                }),
        )
    }) else {
        return Outcome::skip(Skip::Cancelled);
    };
    let offenders = per_node
        .into_iter()
        .enumerate()
        .filter(|(_, set)| !set.is_empty())
        .map(|(node, set)| Offender::elements(Some(node), ElementSet::Triangles(set)))
        .collect();
    Outcome::judged(offenders, Threshold::None)
}

/// Per node, the edges used by more than two faces, as `wire` corner pairs.
pub(super) fn non_manifold_edges(ctx: &Context<'_>) -> Outcome {
    let logical = ctx.logical();
    let Some(per_node) = ctx.par_nodes(|node| {
        let mut edges = edge_uses_of_faces(ctx.model, logical, &ctx.node_faces()[node]);
        group_edge_uses(&mut edges)
            .filter(|group| group.len() > 2)
            .map(|group| [group[0].low, group[0].high])
            .collect::<Vec<_>>()
    }) else {
        return Outcome::skip(Skip::Cancelled);
    };
    let offenders = per_node
        .into_iter()
        .enumerate()
        .filter(|(_, edges)| !edges.is_empty())
        .map(|(node, edges)| Offender::elements(Some(node), ElementSet::Edges(edges)))
        .collect();
    Outcome::judged(offenders, Threshold::None)
}

pub(super) fn isolated_vertices(ctx: &Context<'_>) -> Outcome {
    let Some(extras) = ctx.extras else {
        return Outcome::skip(Skip::SourcePropertiesUnavailable);
    };
    let offenders = extras
        .meshes
        .iter()
        .filter(|mesh| !mesh.unused_vertices.is_empty())
        .map(|mesh| {
            Offender::elements(
                Some(mesh.node as usize),
                ElementSet::Points(
                    mesh.unused_vertices
                        .iter()
                        .map(|(logical, position)| (position.to_array(), *logical))
                        .collect(),
                ),
            )
        })
        .collect();
    Outcome::judged(offenders, Threshold::None)
}

/// Distinct logical vertices of one object closer than the tolerance — points
/// that should have been welded.
pub(super) fn duplicate_vertices(ctx: &Context<'_>, config: &RuleConfig) -> Outcome {
    let tolerance = config.number("tolerance").unwrap_or(0.0).max(0.0) as f32;
    let model = ctx.model;
    let logical = ctx.logical();
    let representative = ctx.logical_corner();
    let Some(per_node) = ctx.par_nodes(|node| {
        // The logical vertices this object uses, one representative corner each.
        let mut seen = HashSet::new();
        let mut points: Vec<(u32, Vec3)> = Vec::new();
        for &triangle in &ctx.node_triangles()[node] {
            for &corner in &model.indices[triangle as usize * 3..triangle as usize * 3 + 3] {
                let Some(&id) = logical.get(corner as usize) else {
                    continue;
                };
                if id != u32::MAX && seen.insert(id) {
                    let corner = representative.get(id as usize).copied().unwrap_or(corner);
                    if let Some(vertex) = model.vertices.get(corner as usize) {
                        points.push((corner, vertex.position));
                    }
                }
            }
        }
        flag_close_points(&points, tolerance)
    }) else {
        return Outcome::skip(Skip::Cancelled);
    };
    let offenders = per_node
        .into_iter()
        .enumerate()
        .filter(|(_, corners)| !corners.is_empty())
        .map(|(node, corners)| Offender::elements(Some(node), ElementSet::Vertices(corners)))
        .collect();
    Outcome::judged(
        offenders,
        Threshold::Max(Measured::Meters(f64::from(tolerance))),
    )
}

/// The corners of every point within `tolerance` of another, via a uniform
/// grid with cells one tolerance wide (exact matches when the tolerance is 0).
fn flag_close_points(points: &[(u32, Vec3)], tolerance: f32) -> Vec<u32> {
    let cell = tolerance.max(1.0e-9);
    let key = |p: Vec3| {
        let q = (p / cell).floor();
        (q.x as i64, q.y as i64, q.z as i64)
    };
    let mut grid: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    for (index, &(_, position)) in points.iter().enumerate() {
        grid.entry(key(position)).or_default().push(index);
    }
    let limit = tolerance * tolerance;
    let mut flagged = vec![false; points.len()];
    for (index, &(_, position)) in points.iter().enumerate() {
        let (x, y, z) = key(position);
        'search: for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let Some(bucket) = grid.get(&(x + dx, y + dy, z + dz)) else {
                        continue;
                    };
                    for &other in bucket {
                        if other != index && points[other].1.distance_squared(position) <= limit {
                            flagged[index] = true;
                            break 'search;
                        }
                    }
                }
            }
        }
    }
    let mut corners: Vec<u32> = points
        .iter()
        .zip(flagged)
        .filter(|(_, flagged)| *flagged)
        .map(|((corner, _), _)| *corner)
        .collect();
    corners.sort_unstable();
    corners
}

pub(super) fn ngons(ctx: &Context<'_>) -> Outcome {
    let model = ctx.model;
    if model.faces.is_empty() {
        // A mesh without polygons is triangles only.
        return Outcome::judged(Vec::new(), Threshold::None);
    }
    let offenders = (0..model.nodes.len())
        .filter_map(|node| {
            let set = triangles_of_faces(ctx, node, |face| {
                model
                    .faces
                    .get(face as usize)
                    .is_some_and(|polygon| polygon.index_count > 4)
            });
            let faces = ctx.node_faces()[node]
                .iter()
                .filter(|&&face| model.faces[face as usize].index_count > 4)
                .count();
            (!set.is_empty()).then(|| {
                let mut offender = Offender::elements(Some(node), ElementSet::Triangles(set));
                offender.count = faces as u64;
                offender
            })
        })
        .collect();
    Outcome::judged(offenders, Threshold::Max(Measured::Count(4)))
}

fn mesh_flag_rule(
    ctx: &Context<'_>,
    authored: impl Fn(&review_model::extras::MeshExtras) -> bool,
) -> Outcome {
    let Some(extras) = ctx.extras else {
        return Outcome::skip(Skip::SourcePropertiesUnavailable);
    };
    let offenders = extras
        .meshes
        .iter()
        .filter(|mesh| !authored(mesh))
        .map(|mesh| Offender::node(mesh.node as usize, None))
        .collect();
    Outcome::judged(offenders, Threshold::None)
}

pub(super) fn missing_normals(ctx: &Context<'_>) -> Outcome {
    mesh_flag_rule(ctx, |mesh| mesh.normals_authored)
}

pub(super) fn missing_tangents(ctx: &Context<'_>) -> Outcome {
    mesh_flag_rule(ctx, |mesh| mesh.tangents_authored)
}

/// Faces whose winding disagrees with their own normals. A mirrored object's
/// winding is flipped by its transform while its normals are not (ufbx
/// sign-corrects them), so the comparison is made in the object's own
/// handedness.
pub(super) fn inverted_normals(ctx: &Context<'_>) -> Outcome {
    let model = ctx.model;
    let Some(per_node) = ctx.par_nodes(|node| {
        let mirror = if ctx.geometry_to_world(node).determinant() < 0.0 {
            -1.0
        } else {
            1.0
        };
        let inverted: HashSet<u32> = ctx.node_faces()[node]
            .iter()
            .copied()
            .filter(|&face| {
                let corners = face_corners(ctx, face);
                if corners.len() < 3 {
                    return false;
                }
                // Newell's method: robust for any planar-ish polygon.
                let mut winding = Vec3::ZERO;
                for (index, &corner) in corners.iter().enumerate() {
                    let next = corners[(index + 1) % corners.len()];
                    let (Some(a), Some(b)) = (
                        model.vertices.get(corner as usize),
                        model.vertices.get(next as usize),
                    ) else {
                        return false;
                    };
                    winding += Vec3::new(
                        (a.position.y - b.position.y) * (a.position.z + b.position.z),
                        (a.position.z - b.position.z) * (a.position.x + b.position.x),
                        (a.position.x - b.position.x) * (a.position.y + b.position.y),
                    );
                }
                if winding.length_squared() <= 1.0e-20 {
                    return false;
                }
                let facing = (winding * mirror).normalize();
                // Inverted means *every* corner normal points against the
                // winding. A smooth surface's averaged normals routinely lean
                // across a small face, so one disagreeing corner — or a mean
                // that dips just below zero — is shading, not a flipped face.
                corners.iter().all(|&corner| {
                    model
                        .vertices
                        .get(corner as usize)
                        .is_some_and(|vertex| vertex.normal.dot(facing) < -INVERTED_COS)
                })
            })
            .collect();
        if inverted.is_empty() {
            return None;
        }
        let set = triangles_of_faces(ctx, node, |face| inverted.contains(&face));
        let mut offender = Offender::elements(Some(node), ElementSet::Triangles(set));
        offender.count = inverted.len() as u64;
        Some(offender)
    }) else {
        return Outcome::skip(Skip::Cancelled);
    };
    Outcome::judged(per_node.into_iter().flatten().collect(), Threshold::None)
}

/// Objects whose share of hard (normal-split) edges is above the limit. Each
/// hard edge splits a vertex on the GPU, so a mesh that is mostly hard edges is
/// mostly duplicated vertices.
pub(super) fn hard_edges(ctx: &Context<'_>, config: &RuleConfig) -> Outcome {
    let max_ratio = config.number("max_ratio").unwrap_or(0.5);
    let model = ctx.model;
    let logical = ctx.logical();
    let normal = |corner: u32| {
        model
            .vertices
            .get(corner as usize)
            .map_or(Vec3::ZERO, |vertex| vertex.normal)
    };
    let Some(per_node) = ctx.par_nodes(|node| {
        let mut edges = edge_uses_of_faces(model, logical, &ctx.node_faces()[node]);
        let mut interior = 0_u64;
        let mut hard = Vec::new();
        for group in group_edge_uses(&mut edges) {
            if group.len() != 2 {
                continue;
            }
            interior += 1;
            let (a, b) = (group[0], group[1]);
            let split = normal(a.low).dot(normal(b.low)) < HARD_EDGE_COS
                || normal(a.high).dot(normal(b.high)) < HARD_EDGE_COS;
            if split {
                hard.push([a.low, a.high]);
            }
        }
        if interior == 0 {
            return None;
        }
        let ratio = hard.len() as f64 / interior as f64;
        (ratio > max_ratio).then(|| {
            let mut offender = Offender::elements(Some(node), ElementSet::Edges(hard));
            offender.measured = Some(Measured::Ratio(ratio));
            offender
        })
    }) else {
        return Outcome::skip(Skip::Cancelled);
    };
    let offenders: Vec<Offender> = per_node.into_iter().flatten().collect();
    let measured = largest(&offenders, |m| match m {
        Measured::Ratio(r) => Some(*r),
        _ => None,
    });
    Outcome::judged(offenders, Threshold::Max(Measured::Ratio(max_ratio))).with_measured(measured)
}

pub(super) fn triangle_budget(ctx: &Context<'_>, config: &RuleConfig) -> Outcome {
    let per_object = count_param(config, "max_per_object", u64::MAX);
    let total_max = count_param(config, "max_total", u64::MAX);
    let mut offenders: Vec<Offender> = ctx
        .node_triangles()
        .iter()
        .enumerate()
        .filter(|(_, triangles)| triangles.len() as u64 > per_object)
        .map(|(node, triangles)| {
            Offender::node(node, Some(Measured::Count(triangles.len() as u64)))
        })
        .collect();
    let total = (ctx.model.indices.len() / 3) as u64;
    if total > total_max {
        offenders.insert(
            0,
            Offender {
                node: None,
                count: 1,
                measured: Some(Measured::Count(total)),
                detail: None,
                elements: ElementSet::None,
            },
        );
    }
    let measured = largest(&offenders, |m| match m {
        Measured::Count(c) => Some(*c as f64),
        _ => None,
    });
    Outcome::judged(offenders, Threshold::Max(Measured::Count(per_object))).with_measured(measured)
}

pub(super) fn draw_call_budget(ctx: &Context<'_>, config: &RuleConfig) -> Outcome {
    let max = count_param(config, "max", u64::MAX);
    let triangles = &ctx.model.triangles;
    let draws: HashSet<(u32, u32)> = triangles
        .node
        .iter()
        .zip(&triangles.material)
        .map(|(&node, &material)| (node, material))
        .collect();
    let count = draws.len() as u64;
    let offenders = if count > max {
        vec![Offender {
            node: None,
            count: 1,
            measured: Some(Measured::Count(count)),
            detail: None,
            elements: ElementSet::None,
        }]
    } else {
        Vec::new()
    };
    Outcome::judged(offenders, Threshold::Max(Measured::Count(max)))
        .with_measured(Some(Measured::Count(count)))
}
