//! UV layouts: missing sets, coordinates outside the tile, overlaps, mirrored
//! islands, and the lightmap set's own requirements (present, non-overlapping,
//! padded).
//!
//! Overlap and padding are measured by rasterizing the layout at a fixed
//! resolution over texel centres — what a baker or a lightmapper actually
//! samples. The rasterizer works in fixed point with an exact fill rule, so two
//! triangles sharing an edge never both claim a texel: any texel claimed twice
//! is a real overlap.

use std::collections::{BTreeMap, BTreeSet};

use glam::Vec2;
use review_model::extras::UvSetExtras;
use review_model::uv_islands::{UvIslands, uv_islands};

use super::{Outcome, count_param};
use crate::context::Context;
use crate::finding::{ElementSet, Measured, Offender, RangeSet, Skip, Threshold};
use crate::profile::RuleConfig;

/// How far outside `0..=1` a UV may sit before it counts — float noise on a
/// coordinate meant to be exactly on the border.
const RANGE_EPSILON: f32 = 1.0e-4;
/// Fixed-point subpixel steps per texel.
const SUBPIXEL: i64 = 256;

/// Whether the model carries UV set `channel` at all.
fn has_channel(ctx: &Context<'_>, channel: usize) -> bool {
    channel
        < ctx
            .model
            .stats
            .uv_set_count
            .max(usize::from(!ctx.model.vertices.is_empty()))
}

fn triangle_uvs(ctx: &Context<'_>, triangle: usize, channel: usize) -> Option<[Vec2; 3]> {
    let corners = ctx.model.indices.get(triangle * 3..triangle * 3 + 3)?;
    Some([0, 1, 2].map(|i| ctx.model.uv_for_channel(corners[i] as usize, channel)))
}

/// One offender per node from a per-triangle flag.
fn offenders_from_flags(ctx: &Context<'_>, flagged: &[bool]) -> Vec<Offender> {
    ctx.node_triangles()
        .iter()
        .enumerate()
        .filter_map(|(node, triangles)| {
            let set = RangeSet::from_sorted(
                triangles
                    .iter()
                    .copied()
                    .filter(|&triangle| flagged.get(triangle as usize).copied().unwrap_or(false)),
            );
            (!set.is_empty()).then(|| Offender::elements(Some(node), ElementSet::Triangles(set)))
        })
        .collect()
}

/// Visit every texel whose centre triangle `uv` covers, on an
/// `resolution`×`resolution` grid over the `0..1` tile. Fill rule: a centre
/// exactly on an edge belongs to the triangle on one fixed side of it, so
/// adjacent triangles split their shared edge's texels between them exactly.
pub(crate) fn rasterize(uv: [Vec2; 3], resolution: u32, mut visit: impl FnMut(u32, u32)) {
    let scale = f64::from(resolution) * SUBPIXEL as f64;
    let snap = |p: Vec2| -> (i64, i64) {
        (
            (f64::from(p.x) * scale).round() as i64,
            (f64::from(p.y) * scale).round() as i64,
        )
    };
    let (a, mut b, mut c) = (snap(uv[0]), snap(uv[1]), snap(uv[2]));
    let edge = |a: (i64, i64), b: (i64, i64), p: (i64, i64)| {
        (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
    };
    let area = edge(a, b, c);
    if area == 0 {
        return;
    }
    if area < 0 {
        std::mem::swap(&mut b, &mut c);
    }
    // An edge owns the centres lying exactly on it when it runs this way.
    let owns = |a: (i64, i64), b: (i64, i64)| b.1 < a.1 || (b.1 == a.1 && b.0 > a.0);
    let (own_bc, own_ca, own_ab) = (owns(b, c), owns(c, a), owns(a, b));

    let max = i64::from(resolution) - 1;
    let to_texel = |value: i64| (value.div_euclid(SUBPIXEL)).clamp(0, max);
    let min_x = to_texel(a.0.min(b.0).min(c.0));
    let max_x = to_texel(a.0.max(b.0).max(c.0));
    let min_y = to_texel(a.1.min(b.1).min(c.1));
    let max_y = to_texel(a.1.max(b.1).max(c.1));
    // A triangle wholly outside the tile clamps onto its border row/column;
    // the edge tests below reject those centres.
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let p = (x * SUBPIXEL + SUBPIXEL / 2, y * SUBPIXEL + SUBPIXEL / 2);
            let inside = |w: i64, own: bool| w > 0 || (w == 0 && own);
            if inside(edge(b, c, p), own_bc)
                && inside(edge(c, a, p), own_ca)
                && inside(edge(a, b, p), own_ab)
            {
                visit(x as u32, y as u32);
            }
        }
    }
}

/// Flag every triangle (of `triangles`) that shares a texel with another.
fn mark_overlaps(
    ctx: &Context<'_>,
    triangles: &[u32],
    channel: usize,
    resolution: u32,
    owner: &mut Vec<u32>,
    flagged: &mut [bool],
) {
    let size = resolution as usize * resolution as usize;
    owner.clear();
    owner.resize(size, u32::MAX);
    for &triangle in triangles {
        let Some(uv) = triangle_uvs(ctx, triangle as usize, channel) else {
            continue;
        };
        rasterize(uv, resolution, |x, y| {
            let slot = &mut owner[y as usize * resolution as usize + x as usize];
            if *slot == u32::MAX {
                *slot = triangle;
            } else if *slot != triangle {
                flagged[*slot as usize] = true;
                flagged[triangle as usize] = true;
            }
        });
    }
}

pub(super) fn missing(ctx: &Context<'_>) -> Outcome {
    let model = ctx.model;
    let Some(extras) = ctx.extras else {
        // Without the capture, the one tell is a mesh whose UVs are all zero —
        // what import fills a missing set with.
        let offenders = ctx
            .node_triangles()
            .iter()
            .enumerate()
            .filter(|(_, triangles)| !triangles.is_empty())
            .filter(|(_, triangles)| {
                triangles.iter().all(|&triangle| {
                    model.indices[triangle as usize * 3..triangle as usize * 3 + 3]
                        .iter()
                        .all(|&corner| model.vertices[corner as usize].uv == Vec2::ZERO)
                })
            })
            .map(|(node, _)| Offender::node(node, Some(Measured::Count(0))))
            .collect();
        return Outcome::judged(offenders, Threshold::None);
    };
    let layouts: Vec<&[UvSetExtras]> = extras
        .meshes
        .iter()
        .map(|mesh| mesh.uv_sets.as_slice())
        .collect();
    let offenders = parts_missing_uvs(&layouts)
        .into_iter()
        .map(|part| {
            let mesh = &extras.meshes[part];
            Offender::node(
                mesh.node as usize,
                Some(Measured::Count(filled_channels(&mesh.uv_sets).len() as u64)),
            )
        })
        .collect();
    Outcome::judged(offenders, Threshold::None)
}

/// The parts (indices into `layouts`, each part's UV sets in the file's order)
/// that lack UVs another part has.
///
/// An engine reads UV sets by channel, never by name - `map1` in one part and
/// `UVMap` in the next are both its channel 0 - and a set the file declares but
/// leaves empty is no set at all. So a part is reported when it fills no
/// channel, when it leaves empty a channel some other part fills, or when it
/// holds its sets in a different order from another part's.
fn parts_missing_uvs(layouts: &[&[UvSetExtras]]) -> Vec<usize> {
    let used: BTreeSet<usize> = layouts
        .iter()
        .flat_map(|sets| filled_channels(sets))
        .collect();
    let reordered = reordered_parts(layouts);
    (0..layouts.len())
        .filter(|&part| {
            let channels = filled_channels(layouts[part]);
            channels.is_empty()
                || used
                    .iter()
                    .any(|channel| channels.binary_search(channel).is_err())
                || reordered.contains(&part)
        })
        .collect()
}

/// The channels a part fills with UVs, ascending.
fn filled_channels(sets: &[UvSetExtras]) -> Vec<usize> {
    sets.iter()
        .enumerate()
        .filter(|(_, set)| set.has_values)
        .map(|(channel, _)| channel)
        .collect()
}

/// The parts (indices into `layouts`) holding the same sets as another part in
/// a different order.
///
/// A name is the one thing a channel cannot say: two parts that both fill
/// channels 0 and 1, one with `map1` then `lightmap` and the other the other
/// way round, pass every channel check while each textures from the other's
/// lightmap layout. Only that counts - a part that merely lacks a channel is
/// already reported, and a name another part spells differently is only a
/// name - so a conflict needs both parts to fill both channels.
fn reordered_parts(layouts: &[&[UvSetExtras]]) -> BTreeSet<usize> {
    let fills =
        |part: usize, channel: usize| layouts[part].get(channel).is_some_and(|set| set.has_values);
    // Name -> channel -> the parts filling that channel under that name. A
    // name a part gives two of its sets says nothing about order.
    let mut by_name: BTreeMap<&str, BTreeMap<usize, Vec<usize>>> = BTreeMap::new();
    for (part, sets) in layouts.iter().enumerate() {
        for (channel, set) in sets.iter().enumerate() {
            let unique = sets.iter().filter(|other| other.name == set.name).count() == 1;
            if set.has_values && !set.name.is_empty() && unique {
                by_name
                    .entry(set.name.as_str())
                    .or_default()
                    .entry(channel)
                    .or_default()
                    .push(part);
            }
        }
    }
    let mut flagged = BTreeSet::new();
    for channels in by_name.values() {
        for (&first, at_first) in channels {
            for (&second, at_second) in channels.range(first + 1..) {
                let one: Vec<usize> = at_first
                    .iter()
                    .copied()
                    .filter(|&part| fills(part, second))
                    .collect();
                let other: Vec<usize> = at_second
                    .iter()
                    .copied()
                    .filter(|&part| fills(part, first))
                    .collect();
                if !one.is_empty() && !other.is_empty() {
                    flagged.extend(one);
                    flagged.extend(other);
                }
            }
        }
    }
    flagged
}

pub(super) fn out_of_range(ctx: &Context<'_>, channel: usize) -> Outcome {
    if !has_channel(ctx, channel) {
        return Outcome::skip(Skip::NoUvSet);
    }
    let outside = |value: f32| !(-RANGE_EPSILON..=1.0 + RANGE_EPSILON).contains(&value);
    let flagged: Vec<bool> = (0..ctx.model.indices.len() / 3)
        .map(|triangle| {
            triangle_uvs(ctx, triangle, channel)
                .is_some_and(|uvs| uvs.iter().any(|uv| outside(uv.x) || outside(uv.y)))
        })
        .collect();
    Outcome::judged(
        offenders_from_flags(ctx, &flagged),
        Threshold::Range {
            min: Measured::Ratio(0.0),
            max: Measured::Ratio(1.0),
        },
    )
}

/// UV0 overlaps within one material: two parts of the surface reading the
/// same texels of the same texture. (Different materials read different
/// textures, so overlap across them is fine.)
pub(super) fn overlap_per_material(ctx: &Context<'_>, config: &RuleConfig) -> Outcome {
    if !has_channel(ctx, 0) {
        return Outcome::skip(Skip::NoUvSet);
    }
    let resolution = count_param(config, "resolution", 1024).clamp(16, 4096) as u32;
    let model = ctx.model;
    let triangle_count = model.indices.len() / 3;
    let mut by_material: std::collections::BTreeMap<u32, Vec<u32>> = Default::default();
    for triangle in 0..triangle_count {
        let material = model.triangles.material.get(triangle).copied().unwrap_or(0);
        by_material
            .entry(material)
            .or_default()
            .push(triangle as u32);
    }
    let mut flagged = vec![false; triangle_count];
    let mut owner = Vec::new();
    for triangles in by_material.values() {
        if ctx.cancelled() {
            return Outcome::skip(Skip::Cancelled);
        }
        mark_overlaps(ctx, triangles, 0, resolution, &mut owner, &mut flagged);
    }
    Outcome::judged(offenders_from_flags(ctx, &flagged), Threshold::None)
}

/// Mirrored islands, and triangles folded against their own island.
pub(super) fn flipped(ctx: &Context<'_>) -> Outcome {
    if !has_channel(ctx, 0) {
        return Outcome::skip(Skip::NoUvSet);
    }
    let model = ctx.model;
    let islands: UvIslands = uv_islands(model, 0, |_| false);
    let triangle_count = model.indices.len() / 3;
    let areas: Vec<f32> = (0..triangle_count)
        .map(|triangle| review_model::measure::triangle_uv_area(model, triangle, 0))
        .collect();
    let mut island_area = vec![0.0_f64; islands.count as usize];
    let island_of: Vec<Option<u32>> = (0..triangle_count)
        .map(|triangle| islands.of_triangle(model, triangle))
        .collect();
    for (triangle, island) in island_of.iter().enumerate() {
        if let Some(island) = island {
            island_area[*island as usize] += f64::from(areas[triangle]);
        }
    }
    let flagged: Vec<bool> = (0..triangle_count)
        .map(|triangle| {
            let Some(island) = island_of[triangle] else {
                return false;
            };
            let net = island_area[island as usize];
            let area = f64::from(areas[triangle]);
            net < 0.0 || (area.abs() > 1.0e-12 && area.signum() != net.signum())
        })
        .collect();
    Outcome::judged(offenders_from_flags(ctx, &flagged), Threshold::None)
}

fn lightmap_channel(config: &RuleConfig) -> usize {
    count_param(config, "channel", 1) as usize
}

pub(super) fn lightmap_missing(ctx: &Context<'_>, config: &RuleConfig) -> Outcome {
    let channel = lightmap_channel(config);
    let offenders = match ctx.extras {
        Some(extras) => extras
            .meshes
            .iter()
            .filter(|mesh| mesh.uv_sets.iter().filter(|set| set.has_values).count() <= channel)
            .map(|mesh| Offender::node(mesh.node as usize, None))
            .collect(),
        None if ctx.model.stats.uv_set_count <= channel => ctx
            .node_triangles()
            .iter()
            .enumerate()
            .filter(|(_, triangles)| !triangles.is_empty())
            .map(|(node, _)| Offender::node(node, None))
            .collect(),
        None => Vec::new(),
    };
    Outcome::judged(
        offenders,
        Threshold::Min(Measured::Count(channel as u64 + 1)),
    )
}

pub(super) fn lightmap_out_of_range(ctx: &Context<'_>, config: &RuleConfig) -> Outcome {
    let channel = lightmap_channel(config);
    if ctx.model.stats.uv_set_count <= channel {
        return Outcome::skip(Skip::NoUvSet);
    }
    out_of_range(ctx, channel)
}

/// Lightmaps are per object, so overlap is measured within each object.
pub(super) fn lightmap_overlap(ctx: &Context<'_>, config: &RuleConfig) -> Outcome {
    let channel = lightmap_channel(config);
    if ctx.model.stats.uv_set_count <= channel {
        return Outcome::skip(Skip::NoUvSet);
    }
    let resolution = count_param(config, "resolution", 64).clamp(16, 4096) as u32;
    let Some(per_node) = ctx.par_nodes(|node| {
        let triangles = &ctx.node_triangles()[node];
        let (Some(&first), Some(&last)) = (triangles.first(), triangles.last()) else {
            return RangeSet::default();
        };
        // Flags are indexed by global triangle, offset to this object's span.
        let base = first as usize;
        let mut flagged = vec![false; last as usize + 1 - base];
        let mut owner = Vec::new();
        let size = resolution as usize * resolution as usize;
        owner.resize(size, u32::MAX);
        for &triangle in triangles {
            let Some(uv) = triangle_uvs(ctx, triangle as usize, channel) else {
                continue;
            };
            rasterize(uv, resolution, |x, y| {
                let slot = &mut owner[y as usize * resolution as usize + x as usize];
                if *slot == u32::MAX {
                    *slot = triangle;
                } else if *slot != triangle {
                    flagged[*slot as usize - base] = true;
                    flagged[triangle as usize - base] = true;
                }
            });
        }
        RangeSet::from_sorted(
            triangles
                .iter()
                .copied()
                .filter(|&triangle| flagged[triangle as usize - base]),
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

/// Islands of one object's lightmap closer than the padding: light bleeds
/// across the gap when the lightmap is filtered or mipped.
pub(super) fn lightmap_padding(ctx: &Context<'_>, config: &RuleConfig) -> Outcome {
    let channel = lightmap_channel(config);
    if ctx.model.stats.uv_set_count <= channel {
        return Outcome::skip(Skip::NoUvSet);
    }
    let resolution = count_param(config, "resolution", 64).clamp(16, 4096) as u32;
    let padding = config.number("min_padding").unwrap_or(2.0).max(0.0).ceil() as i64;
    let model = ctx.model;
    let islands = uv_islands(model, channel, |_| false);
    let island_of = |triangle: u32| {
        islands
            .of_triangle(model, triangle as usize)
            .unwrap_or(u32::MAX)
    };
    let Some(per_node) = ctx.par_nodes(|node| {
        let triangles = &ctx.node_triangles()[node];
        if triangles.is_empty() || padding == 0 {
            return RangeSet::default();
        }
        let size = resolution as usize;
        let mut owner = vec![u32::MAX; size * size];
        for &triangle in triangles {
            if let Some(uv) = triangle_uvs(ctx, triangle as usize, channel) {
                rasterize(uv, resolution, |x, y| {
                    let slot = &mut owner[y as usize * size + x as usize];
                    if *slot == u32::MAX {
                        *slot = triangle;
                    }
                });
            }
        }
        let mut flagged = std::collections::BTreeSet::new();
        for y in 0..size as i64 {
            for x in 0..size as i64 {
                let here = owner[y as usize * size + x as usize];
                if here == u32::MAX {
                    continue;
                }
                let island = island_of(here);
                for dy in -padding..=padding {
                    for dx in -padding..=padding {
                        let (nx, ny) = (x + dx, y + dy);
                        if nx < 0 || ny < 0 || nx >= size as i64 || ny >= size as i64 {
                            continue;
                        }
                        let there = owner[ny as usize * size + nx as usize];
                        if there != u32::MAX && island_of(there) != island {
                            flagged.insert(here);
                            flagged.insert(there);
                        }
                    }
                }
            }
        }
        RangeSet::from_sorted(flagged)
    }) else {
        return Outcome::skip(Skip::Cancelled);
    };
    let offenders = per_node
        .into_iter()
        .enumerate()
        .filter(|(_, set)| !set.is_empty())
        .map(|(node, set)| Offender::elements(Some(node), ElementSet::Triangles(set)))
        .collect();
    Outcome::judged(offenders, Threshold::Min(Measured::Pixels(padding as f64)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covered(uv: [Vec2; 3], resolution: u32) -> Vec<(u32, u32)> {
        let mut texels = Vec::new();
        rasterize(uv, resolution, |x, y| texels.push((x, y)));
        texels.sort_unstable();
        texels
    }

    #[test]
    fn two_triangles_of_a_quad_tile_its_texels_exactly_once() {
        let (a, b, c, d) = (
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        );
        let mut all = covered([a, b, c], 8);
        all.extend(covered([a, c, d], 8));
        all.sort_unstable();
        let expected: Vec<(u32, u32)> = (0..8).flat_map(|x| (0..8).map(move |y| (x, y))).collect();
        assert_eq!(
            all, expected,
            "every texel once, the diagonal split exactly"
        );
    }

    #[test]
    fn winding_does_not_change_coverage() {
        let uv = [
            Vec2::new(0.1, 0.1),
            Vec2::new(0.9, 0.2),
            Vec2::new(0.4, 0.8),
        ];
        assert_eq!(covered(uv, 16), covered([uv[0], uv[2], uv[1]], 16));
    }

    /// One part's UV sets, each `(name, has values)`.
    fn sets(layout: &[(&str, bool)]) -> Vec<UvSetExtras> {
        layout
            .iter()
            .enumerate()
            .map(|(index, &(name, has_values))| UvSetExtras {
                name: name.to_owned(),
                index: index as u32,
                has_values,
            })
            .collect()
    }

    fn missing(parts: &[&[(&str, bool)]]) -> Vec<usize> {
        let owned: Vec<Vec<UvSetExtras>> = parts.iter().map(|layout| sets(layout)).collect();
        let layouts: Vec<&[UvSetExtras]> = owned.iter().map(Vec::as_slice).collect();
        parts_missing_uvs(&layouts)
    }

    #[test]
    fn a_declared_but_empty_set_is_no_set_and_asks_nothing_of_the_others() {
        // Every part declares a second set and leaves it empty, the one with
        // the most sets included: nothing is missing.
        let declared = [("map1", true), ("map2", false)];
        assert!(missing(&[&declared, &declared, &[("map1", true)]]).is_empty());
        // A part that leaves a channel empty that another fills is reported.
        assert_eq!(
            missing(&[&[("map1", true), ("map2", true)], &declared]),
            [1]
        );
        assert_eq!(missing(&[&[("map1", true)], &[("map1", false)]]), [1]);
    }

    #[test]
    fn uv_sets_are_matched_by_channel_not_by_name() {
        // Maya's map1 and Blender's UVMap are both channel 0 to an engine.
        assert!(missing(&[&[("map1", true)], &[("UVMap", true)]]).is_empty());
        assert!(
            missing(&[
                &[("map1", true), ("lightmap", true)],
                &[("UVMap", true), ("UVMap.001", true)],
            ])
            .is_empty()
        );
    }

    #[test]
    fn the_same_sets_in_another_order_are_reported_on_both_parts() {
        let swapped = missing(&[
            &[("map1", true), ("lightmap", true)],
            &[("lightmap", true), ("map1", true)],
            &[("map1", true), ("lightmap", true)],
        ]);
        assert_eq!(swapped, [0, 1, 2]);
        // A part lacking a channel is reported for that alone: the part that
        // has both, in its own order, is not out of order with it.
        assert_eq!(
            missing(&[&[("map1", true), ("map2", true)], &[("map2", true)]]),
            [1]
        );
    }

    #[test]
    fn a_triangle_outside_the_tile_covers_nothing() {
        let uv = [
            Vec2::new(1.5, 1.5),
            Vec2::new(2.5, 1.5),
            Vec2::new(2.0, 2.5),
        ];
        assert!(covered(uv, 16).is_empty());
    }
}
