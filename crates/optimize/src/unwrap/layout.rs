//! Placing the flattened charts: packed into the 0-1 square, or tiled at a
//! texel density and turned to the reference UVs.

use std::collections::HashMap;

use glam::{Vec2, Vec3};

use super::charts::Chart;
use super::islands::{REFERENCE_SEAM_GAP, canonical, root, union};
use crate::stack::{UnwrapParams, UvLayout};
use crate::xatlas::PackOptions;

/// Below this, an area is no area: a chart this small has no shape to scale.
const TINY: f32 = 1.0e-20;

/// The texels per model unit an atlas of `resolution` texels a side is packed
/// at: charts cover about three quarters of an atlas, so this many fill one.
/// The result is scaled to 0-1 afterwards, so only the padding's scale
/// depends on it.
fn texels_per_unit(resolution: u32, area: f32) -> f32 {
    resolution as f32 * 0.75f32.sqrt() / area.max(TINY).sqrt()
}

/// The surface area of `indices`' triangles.
pub(super) fn surface_area(positions: &[Vec3], indices: &[u32]) -> f32 {
    indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|corners| {
            let [a, b, c] = corners.map(|corner| positions[corner as usize]);
            0.5 * (b - a).cross(c - a).length()
        })
        .sum()
}

/// Per corner (three per triangle, `count` triangles), the UV `params` lays the
/// charts out at.
pub(super) fn lay_out(
    charts: &[Chart],
    count: usize,
    positions: &[Vec3],
    indices: &[u32],
    reference: Option<&[Vec2]>,
    params: &UnwrapParams,
) -> Result<Vec<Vec2>, String> {
    let area = surface_area(positions, indices);
    match params.layout {
        UvLayout::Atlas => {
            let options = PackOptions {
                resolution: 0,
                create_image: false,
                brute_force: false,
                block_align: false,
                texels_per_unit: 1.0,
                padding: params.padding,
                bilinear: true,
                rotate_charts: params.rotate,
                rotate_charts_to_axis: params.rotate,
                ..PackOptions::default()
            };
            let scale = texels_per_unit(params.resolution, area);
            let (packed, side) = pack(charts, count, scale, &options)?;
            Ok(packed.into_iter().map(|uv| uv / side).collect())
        }
        UvLayout::Tiled => match reference.filter(|_| params.align_to_reference) {
            Some(reference) => Ok(tiled(charts, count, positions, indices, reference, params)),
            // Nothing to place the charts by: packed, at the density, apart.
            None => {
                let options = PackOptions {
                    resolution: 0,
                    create_image: false,
                    brute_force: false,
                    block_align: false,
                    texels_per_unit: 1.0,
                    padding: 1,
                    bilinear: false,
                    rotate_charts: params.rotate,
                    rotate_charts_to_axis: params.rotate,
                    ..PackOptions::default()
                };
                let scale = texels_per_unit(256, area);
                let (packed, _) = pack(charts, count, scale, &options)?;
                Ok(packed
                    .into_iter()
                    .map(|uv| uv / scale * params.density)
                    .collect())
            }
        },
    }
}

/// Pack the charts, scaled by `scale` texels per model unit: per corner its
/// texel, and the atlas's side.
fn pack(
    charts: &[Chart],
    count: usize,
    scale: f32,
    options: &PackOptions,
) -> Result<(Vec<Vec2>, f32), String> {
    // A vertex per chart and UV, so faces of a chart share them as the chart's
    // own vertices do (a zero-area face goes where its neighbours go).
    let mut vertex: HashMap<(usize, [u32; 2]), u32> = HashMap::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut indices = vec![0u32; count * 3];
    let mut chart_of = vec![0u32; count];
    for (at, chart) in charts.iter().enumerate() {
        for (&triangle, corners) in chart.triangles.iter().zip(&chart.uvs) {
            chart_of[triangle as usize] = at as u32;
            for (k, uv) in corners.iter().enumerate() {
                let texel = *uv * scale;
                let key = (at, [texel.x.to_bits(), texel.y.to_bits()]);
                indices[triangle as usize * 3 + k] = *vertex.entry(key).or_insert_with(|| {
                    uvs.push(texel.to_array());
                    uvs.len() as u32 - 1
                });
            }
        }
    }
    let packed = crate::xatlas::pack(&uvs, &indices, &chart_of, options)?;
    let side = packed.width.max(packed.height).max(1) as f32;
    Ok((
        indices
            .iter()
            .map(|&index| packed.uvs[index as usize].map_or(Vec2::ZERO, Vec2::from_array))
            .collect(),
        side,
    ))
}

/// Per corner, the tiled UV: each chart at the density, turned to its
/// reference UVs and centred where they were, or the reference itself where it
/// stretches less.
///
/// The reference only says how a chart should lie where it covers the chart in
/// one piece. A chart cut along a seam attribute coarser than the reference's
/// own seams (a `_CAD_REGION` that joins a fillet to the faces either side,
/// each face with parameters of its own) spans several pieces of it, which
/// disagree: fitted to all of them at once it lay at whatever angle averaged
/// them — 45 degrees across the flat sides of a filleted part — and the reference "kept" in its
/// place was cut back into the pieces the region was there to join, its seams
/// with it. Such a chart is turned to its largest piece alone, and never
/// swapped for the reference. A chart the reference covers whole (every chart,
/// when the reference's seams cut) is laid out as it always was.
fn tiled(
    charts: &[Chart],
    count: usize,
    positions: &[Vec3],
    indices: &[u32],
    reference: &[Vec2],
    params: &UnwrapParams,
) -> Vec<Vec2> {
    let mut out = vec![Vec2::ZERO; count * 3];
    let reference_at = |corner: usize| reference[indices[corner] as usize];
    for chart in charts {
        let corners: Vec<usize> = chart
            .triangles
            .iter()
            .flat_map(|&triangle| triangle as usize * 3..triangle as usize * 3 + 3)
            .collect();
        let flat: Vec<Vec2> = chart.uvs.iter().flatten().copied().collect();
        let mean = |values: &dyn Fn(usize) -> Vec2| {
            (0..corners.len()).map(values).sum::<Vec2>() / corners.len() as f32
        };
        // Where the chart is fitted to its reference: all of it, or the slots
        // of its largest continuous piece.
        let piece = largest_reference_piece(chart, positions, indices, reference);
        let fit_mean = |values: &dyn Fn(usize) -> Vec2| match &piece {
            None => mean(values),
            Some(slots) => {
                slots.iter().map(|&slot| values(slot)).sum::<Vec2>() / slots.len() as f32
            }
        };
        // The chart is in model units already; the density scales it.
        let flat_mean = fit_mean(&|slot| flat[slot]);
        let centred: Vec<Vec2> = flat
            .iter()
            .map(|&uv| (uv - flat_mean) * params.density)
            .collect();
        let reference_mean = fit_mean(&|slot| reference_at(corners[slot]));
        let target: Vec<Vec2> = corners
            .iter()
            .map(|&corner| reference_at(corner) - reference_mean)
            .collect();
        let turn = match &piece {
            None => procrustes(&centred, &target),
            Some(slots) => {
                let from: Vec<Vec2> = slots.iter().map(|&slot| centred[slot]).collect();
                let to: Vec<Vec2> = slots.iter().map(|&slot| target[slot]).collect();
                procrustes(&from, &to)
            }
        };
        let fresh: Vec<Vec2> = centred
            .iter()
            .map(|&point| turn(point) + reference_mean)
            .collect();

        // The reference UVs scaled to the density about their centre, where
        // they stretch less than the fresh unwrap does — and cover the chart
        // in one piece, so keeping them adds no seam the chart does not have.
        let chosen = if params.keep_better && piece.is_none() {
            let kept: Vec<Vec2> = corners.iter().map(|&corner| reference_at(corner)).collect();
            let kept_area: f32 = kept
                .as_chunks::<3>()
                .0
                .iter()
                .map(|triangle| triangle_area2(triangle))
                .sum();
            let area3: f32 = chart
                .triangles
                .iter()
                .map(|&face| triangle_area3(positions, indices, face as usize))
                .sum();
            if kept_area > TINY && area3 > TINY {
                let centre = kept.iter().copied().sum::<Vec2>() / kept.len() as f32;
                let kept_scale = (area3 / kept_area).sqrt() * params.density;
                let kept: Vec<Vec2> = kept
                    .iter()
                    .map(|&uv| (uv - centre) * kept_scale + centre)
                    .collect();
                let fresh_stretch = mean_stretch(positions, indices, &chart.triangles, &fresh);
                let kept_stretch = mean_stretch(positions, indices, &chart.triangles, &kept);
                if fresh_stretch < kept_stretch {
                    fresh
                } else {
                    kept
                }
            } else {
                fresh
            }
        } else {
            fresh
        };
        for (slot, &corner) in corners.iter().enumerate() {
            out[corner] = chosen[slot];
        }
    }
    out
}

/// The corner slots (three per triangle, in `chart.triangles`' order) of the
/// largest piece of `chart` its reference UVs cover continuously, or `None`
/// when they cover all of it.
///
/// Continuous is what [`super::islands`] cuts by: two triangles are joined
/// across an edge they share — matched by position, used by exactly those two —
/// where their reference UVs agree at both ends within
/// [`REFERENCE_SEAM_GAP`]. The largest piece is the one of most surface area,
/// the first in triangle order on a tie.
fn largest_reference_piece(
    chart: &Chart,
    positions: &[Vec3],
    indices: &[u32],
    reference: &[Vec2],
) -> Option<Vec<usize>> {
    let point = |corner: usize| {
        positions[indices[corner] as usize]
            .to_array()
            .map(canonical)
    };
    // Each edge, by its two points, with the chart's triangles (by slot) that
    // use it and the corners at its ends, in key order.
    type Point = [u32; 3];
    type Use = (u32, usize, usize);
    let mut edges: HashMap<(Point, Point), Vec<Use>> = HashMap::new();
    let edge = |slot: usize, k: usize| {
        let base = chart.triangles[slot] as usize * 3;
        let (a, b) = (base + k, base + (k + 1) % 3);
        let (pa, pb) = (point(a), point(b));
        if pa < pb {
            ((pa, pb), a, b)
        } else {
            ((pb, pa), b, a)
        }
    };
    for slot in 0..chart.triangles.len() {
        for k in 0..3 {
            let (key, a, b) = edge(slot, k);
            if key.0 != key.1 {
                edges.entry(key).or_default().push((slot as u32, a, b));
            }
        }
    }
    let agree = |x: usize, y: usize| {
        reference[indices[x] as usize].distance(reference[indices[y] as usize])
            <= REFERENCE_SEAM_GAP
    };
    let mut parent: Vec<u32> = (0..chart.triangles.len() as u32).collect();
    // Walk the triangles' own edges in order; the map is only looked up.
    for slot in 0..chart.triangles.len() {
        for k in 0..3 {
            let (key, _, _) = edge(slot, k);
            let Some([(s0, a0, b0), (s1, a1, b1)]) = edges.get(&key).map(Vec::as_slice) else {
                continue;
            };
            if agree(*a0, *a1) && agree(*b0, *b1) {
                union(&mut parent, *s0, *s1);
            }
        }
    }
    let roots: Vec<u32> = (0..parent.len() as u32)
        .map(|slot| root(&mut parent, slot))
        .collect();
    if roots.iter().all(|&at| at == roots[0]) {
        return None;
    }
    let mut area = vec![0.0f32; roots.len()];
    for (slot, &at) in roots.iter().enumerate() {
        area[at as usize] += triangle_area3(positions, indices, chart.triangles[slot] as usize);
    }
    // A root is its piece's first slot, so the first largest is the earliest.
    let mut largest = roots[0];
    for &at in &roots {
        if area[at as usize] > area[largest as usize] {
            largest = at;
        }
    }
    Some(
        roots
            .iter()
            .enumerate()
            .filter(|&(_, &at)| at == largest)
            .flat_map(|(slot, _)| slot * 3..slot * 3 + 3)
            .collect(),
    )
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
fn procrustes(from: &[Vec2], to: &[Vec2]) -> impl Fn(Vec2) -> Vec2 + use<> {
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

/// [`review_model::measure::uv_stretch`]: one measure for the unwrap and the
/// viewer's stretch map.
pub(crate) fn stretch(points: [Vec3; 3], uvs: [Vec2; 3]) -> Option<f32> {
    review_model::measure::uv_stretch(points, uvs)
}

/// The area-weighted mean stretch of `faces` under per-corner `uvs` (laid out
/// as the chart's corners are: three per face, in `faces`' order).
fn mean_stretch(positions: &[Vec3], indices: &[u32], faces: &[u32], uvs: &[Vec2]) -> f32 {
    let (mut sum, mut weight) = (0.0f32, 0.0f32);
    for (slot, &face) in faces.iter().enumerate() {
        let face = face as usize;
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
