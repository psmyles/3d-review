//! Placing xatlas's charts: tiled at a texel density and turned to the
//! reference UVs, or packed into the 0-1 square.

use glam::{Vec2, Vec3};

use crate::stack::{ChartGrowth, Flattening, UnwrapParams, UvLayout};
use crate::xatlas::{ChartOptions, NO_CHART, Output, PackOptions};

/// Below this, an area is no area: a chart this small has no shape to scale.
const TINY: f32 = 1.0e-20;

/// Chart growth as the settings ask: whole islands, or xatlas's own.
pub(super) fn chart_options(growth: ChartGrowth) -> ChartOptions {
    let mut options = ChartOptions::default();
    if growth == ChartGrowth::Islands {
        // No cost for anything: a chart grows over its whole island (the face
        // groups stop it at the seams) and is split only if its flattening
        // would fold over itself.
        options.normal_deviation_weight = 0.0;
        options.roundness_weight = 0.0;
        options.straightness_weight = 0.0;
        options.normal_seam_weight = 0.0;
        options.texture_seam_weight = 0.0;
        options.max_cost = 1.0e9;
    }
    options
}

/// The packing: for an atlas, the settings' resolution and padding; for tiled
/// UVs, a cheap layout whose only use is a place to start each chart from.
pub(super) fn pack_options(
    params: &UnwrapParams,
    positions: &[Vec3],
    indices: &[u32],
) -> PackOptions {
    let area: f32 = indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|corners| {
            let [a, b, c] = corners.map(|corner| positions[corner as usize]);
            0.5 * (b - a).cross(c - a).length()
        })
        .sum::<f32>()
        .max(TINY);
    let mut options = PackOptions {
        resolution: 0,
        create_image: false,
        brute_force: false,
        block_align: false,
        ..PackOptions::default()
    };
    match params.layout {
        UvLayout::Atlas => {
            // Charts cover about three quarters of an atlas, so this many texels
            // per unit fills one of the asked size; the result is scaled to 0-1
            // afterwards, so only the padding's scale depends on it.
            options.texels_per_unit = params.resolution as f32 * 0.75f32.sqrt() / area.sqrt();
            options.padding = params.padding;
            options.bilinear = true;
            options.rotate_charts = true;
            options.rotate_charts_to_axis = true;
        }
        UvLayout::Tiled => {
            options.texels_per_unit = 256.0 / area.sqrt();
            options.padding = 1;
            options.bilinear = false;
            options.rotate_charts = false;
            options.rotate_charts_to_axis = false;
        }
    }
    options
}

/// How much lower an alternative's mean stretch must be to be taken over what
/// the settings asked for.
pub(super) const CLEARLY_LESS: f32 = 0.01;

/// Per island, the area-weighted mean stretch of per-corner `uvs`.
pub(super) fn island_stretch(
    positions: &[Vec3],
    indices: &[u32],
    island: &[u32],
    uvs: &[Vec2],
) -> Vec<f32> {
    let count = island.iter().max().map_or(0, |&last| last as usize + 1);
    let mut sum = vec![0.0f32; count];
    let mut weight = vec![0.0f32; count];
    for (face, &own) in island.iter().enumerate() {
        let points = [0, 1, 2].map(|k| positions[indices[face * 3 + k] as usize]);
        let corners = [uvs[face * 3], uvs[face * 3 + 1], uvs[face * 3 + 2]];
        let area = triangle_area3(positions, indices, face);
        if let Some(value) = stretch(points, corners) {
            sum[own as usize] += value * area;
            weight[own as usize] += area;
        }
    }
    sum.iter()
        .zip(&weight)
        .map(|(&sum, &weight)| if weight > 0.0 { sum / weight } else { 1.0 })
        .collect()
}

/// Per corner, the packed atlas UV, in the 0-1 square, V up.
pub(super) fn atlas_uvs(atlas: &Output) -> Vec<Vec2> {
    let side = atlas.width.max(atlas.height).max(1) as f32;
    atlas
        .indices
        .iter()
        .map(|&index| {
            let [u, v] = atlas.vertices[index as usize].uv;
            Vec2::new(u / side, 1.0 - v / side)
        })
        .collect()
}

/// Per corner, the tiled UV: each chart at the density, turned to its
/// reference UVs, or the reference itself where it stretches less.
pub(super) fn tiled_uvs(
    atlas: &Output,
    positions: &[Vec3],
    indices: &[u32],
    reference: Option<&[Vec2]>,
    params: &UnwrapParams,
    texels_per_unit: f32,
) -> Vec<Vec2> {
    let faces = indices.len() / 3;
    // The packed layout in world units: each chart's shape, and a place.
    let mut packed: Vec<Vec2> = atlas
        .indices
        .iter()
        .map(|&index| {
            Vec2::from_array(atlas.vertices[index as usize].uv) / texels_per_unit.max(TINY)
        })
        .collect();
    // Faces by chart, charts in order of their first face.
    let mut chart_of: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    let mut charts: Vec<Vec<usize>> = Vec::new();
    for face in 0..faces {
        let chart = atlas.vertices[atlas.indices[face * 3] as usize].chart;
        let at = *chart_of.entry(chart).or_insert_with(|| {
            charts.push(Vec::new());
            charts.len() - 1
        });
        charts[at].push(face);
    }

    let mut out = packed.clone();
    let reference_at = |corner: usize| reference.map(|uvs| uvs[indices[corner] as usize]);
    for faces in &charts {
        let corners: Vec<usize> = faces
            .iter()
            .flat_map(|&face| face * 3..face * 3 + 3)
            .collect();
        // Faces xatlas left out (no area to flatten): their reference UVs, or
        // nothing.
        if atlas.vertices[atlas.indices[faces[0] * 3] as usize].chart == NO_CHART {
            for &corner in &corners {
                out[corner] = reference_at(corner).unwrap_or_default();
            }
            continue;
        }
        if params.flattening == Flattening::MinimumStretch {
            refine_chart(atlas, positions, &corners, &mut packed);
        }
        let area3: f32 = faces
            .iter()
            .map(|&face| triangle_area3(positions, indices, face))
            .sum();
        let area2: f32 = faces
            .iter()
            .map(|&face| triangle_area2(&packed[face * 3..face * 3 + 3]))
            .sum();
        if area3 <= TINY || area2 <= TINY {
            for &corner in &corners {
                out[corner] = packed[corner] * params.density;
            }
            continue;
        }
        let scale = (area3 / area2).sqrt() * params.density;
        let mean = |values: &dyn Fn(usize) -> Vec2| {
            corners.iter().map(|&corner| values(corner)).sum::<Vec2>() / corners.len() as f32
        };
        let packed_mean = mean(&|corner| packed[corner]);
        let centred: Vec<Vec2> = corners
            .iter()
            .map(|&corner| (packed[corner] - packed_mean) * scale)
            .collect();

        let fresh: Vec<Vec2> = match reference.filter(|_| params.align_to_reference) {
            Some(_) => {
                let reference_mean = mean(&|corner| reference_at(corner).unwrap_or_default());
                let target: Vec<Vec2> = corners
                    .iter()
                    .map(|&corner| reference_at(corner).unwrap_or_default() - reference_mean)
                    .collect();
                let turn = procrustes(&centred, &target);
                centred
                    .iter()
                    .map(|&point| turn(point) + reference_mean)
                    .collect()
            }
            None => {
                let turn = longest_axis_to_u(&centred);
                let place = packed_mean * params.density;
                centred.iter().map(|&point| turn(point) + place).collect()
            }
        };

        // The reference UVs scaled to the density about their centre, where
        // they stretch less than the fresh unwrap does.
        let chosen = match reference.filter(|_| params.keep_better) {
            Some(_) => {
                let kept: Vec<Vec2> = corners
                    .iter()
                    .map(|&corner| reference_at(corner).unwrap_or_default())
                    .collect();
                let kept_area: f32 = kept
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|triangle| triangle_area2(triangle))
                    .sum();
                if kept_area > TINY {
                    let centre = kept.iter().copied().sum::<Vec2>() / kept.len() as f32;
                    let kept_scale = (area3 / kept_area).sqrt() * params.density;
                    let kept: Vec<Vec2> = kept
                        .iter()
                        .map(|&uv| (uv - centre) * kept_scale + centre)
                        .collect();
                    let fresh_stretch = mean_stretch(positions, indices, faces, &fresh);
                    let kept_stretch = mean_stretch(positions, indices, faces, &kept);
                    if fresh_stretch < kept_stretch {
                        fresh
                    } else {
                        kept
                    }
                } else {
                    fresh
                }
            }
            None => fresh,
        };
        for (slot, &corner) in corners.iter().enumerate() {
            out[corner] = chosen[slot];
        }
    }
    out
}

/// Refine one chart's flattening as rigidly as possible ([`super::arap`]), on
/// its own vertices (xatlas's output vertices, each a chart's own).
fn refine_chart(atlas: &Output, positions: &[Vec3], corners: &[usize], packed: &mut [Vec2]) {
    let mut local: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    let mut points = Vec::new();
    let mut uvs = Vec::new();
    let mut slot_of = Vec::with_capacity(corners.len());
    for &corner in corners {
        let vertex = atlas.indices[corner];
        let next = local.len() as u32;
        let slot = *local.entry(vertex).or_insert_with(|| {
            points.push(positions[atlas.vertices[vertex as usize].xref as usize]);
            uvs.push(packed[corner]);
            next
        });
        slot_of.push(slot);
    }
    let triangles: Vec<[u32; 3]> = slot_of.as_chunks::<3>().0.to_vec();
    super::arap::refine(&points, &triangles, &mut uvs);
    for (&corner, &slot) in corners.iter().zip(&slot_of) {
        packed[corner] = uvs[slot as usize];
    }
}

fn triangle_area3(positions: &[Vec3], indices: &[u32], face: usize) -> f32 {
    let [a, b, c] = [0, 1, 2].map(|k| positions[indices[face * 3 + k] as usize]);
    0.5 * (b - a).cross(c - a).length()
}

fn triangle_area2(uv: &[Vec2]) -> f32 {
    0.5 * (uv[1] - uv[0]).perp_dot(uv[2] - uv[0]).abs()
}

/// The rotation, or mirror, that best maps `from` onto `to` (both centred):
/// orthogonal Procrustes, closed form in two dimensions. With `M = Σ fromᵀ to`,
/// the best rotation's (cos, sin) is along `(M00 + M11, M01 - M10)` and the best
/// mirror's along `(M00 - M11, M01 + M10)`; whichever reaches further wins.
fn procrustes(from: &[Vec2], to: &[Vec2]) -> impl Fn(Vec2) -> Vec2 {
    let (mut m00, mut m01, mut m10, mut m11) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for (a, b) in from.iter().zip(to) {
        m00 += a.x * b.x;
        m01 += a.x * b.y;
        m10 += a.y * b.x;
        m11 += a.y * b.y;
    }
    let rotation = Vec2::new(m00 + m11, m01 - m10);
    let mirror = Vec2::new(m00 - m11, m01 + m10);
    let mirrored = mirror.length() > rotation.length();
    let (c, s) = match (if mirrored { mirror } else { rotation }).try_normalize() {
        Some(direction) => (direction.x, direction.y),
        None => (1.0, 0.0),
    };
    move |point: Vec2| {
        if mirrored {
            // Row vector times [[c, s], [s, -c]].
            Vec2::new(point.x * c + point.y * s, point.x * s - point.y * c)
        } else {
            // Row vector times [[c, s], [-s, c]].
            Vec2::new(point.x * c - point.y * s, point.x * s + point.y * c)
        }
    }
}

/// The rotation that lays `points`' (centred) longest axis along U.
fn longest_axis_to_u(points: &[Vec2]) -> impl Fn(Vec2) -> Vec2 {
    let (mut xx, mut xy, mut yy) = (0.0f32, 0.0f32, 0.0f32);
    for point in points {
        xx += point.x * point.x;
        xy += point.x * point.y;
        yy += point.y * point.y;
    }
    // The principal direction's angle is half of atan2(2xy, xx - yy).
    let angle = 0.5 * review_model::exact::atan2(2.0 * xy, xx - yy);
    let (c, s) = (
        review_model::exact::cos(-angle),
        review_model::exact::sin(-angle),
    );
    move |point: Vec2| Vec2::new(point.x * c - point.y * s, point.x * s + point.y * c)
}

/// The ratio of the larger to the smaller singular value of the map from a
/// triangle (in its own plane) to its UVs: 1 where the UVs are a scaled copy of
/// the shape, larger the more they stretch it one way. Capped at 50, as the
/// measures in the pipeline plan are; `None` for a triangle
/// with no area.
pub(crate) fn stretch(points: [Vec3; 3], uvs: [Vec2; 3]) -> Option<f32> {
    let (e1, e2) = (points[1] - points[0], points[2] - points[0]);
    let normal = e1.cross(e2);
    let (Some(x), Some(n)) = (e1.try_normalize(), normal.try_normalize()) else {
        return None;
    };
    let y = n.cross(x);
    // The triangle in its own plane, and its UV edges.
    let (p00, p01, p10, p11) = (e1.dot(x), e2.dot(x), e1.dot(y), e2.dot(y));
    let det = p00 * p11 - p01 * p10;
    if det.abs() <= TINY {
        return None;
    }
    let (q1, q2) = (uvs[1] - uvs[0], uvs[2] - uvs[0]);
    // J = U * P^-1, with U's columns the UV edges.
    let inverse = [[p11 / det, -p01 / det], [-p10 / det, p00 / det]];
    let a = q1.x * inverse[0][0] + q2.x * inverse[1][0];
    let b = q1.x * inverse[0][1] + q2.x * inverse[1][1];
    let c = q1.y * inverse[0][0] + q2.y * inverse[1][0];
    let d = q1.y * inverse[0][1] + q2.y * inverse[1][1];
    // A 2x2 matrix's singular values, in closed form.
    let energy = 0.5 * (a * a + b * b + c * c + d * d);
    let determinant = a * d - b * c;
    let root = (energy * energy - determinant * determinant)
        .max(0.0)
        .sqrt();
    let larger = (energy + root).sqrt();
    let smaller = (energy - root).max(0.0).sqrt();
    Some(if smaller <= larger / 50.0 {
        50.0
    } else {
        larger / smaller
    })
}

/// The area-weighted mean stretch of `faces` under per-corner `uvs` (laid out
/// as the chart's corners are: three per face, in `faces`' order).
fn mean_stretch(positions: &[Vec3], indices: &[u32], faces: &[usize], uvs: &[Vec2]) -> f32 {
    let (mut sum, mut weight) = (0.0f32, 0.0f32);
    for (slot, &face) in faces.iter().enumerate() {
        let points = [0, 1, 2].map(|k| positions[indices[face * 3 + k] as usize]);
        let corners = [uvs[slot * 3], uvs[slot * 3 + 1], uvs[slot * 3 + 2]];
        let area = triangle_area3(positions, indices, face);
        if let Some(value) = stretch(points, corners) {
            sum += value * area;
            weight += area;
        }
    }
    if weight > 0.0 { sum / weight } else { f32::MAX }
}
