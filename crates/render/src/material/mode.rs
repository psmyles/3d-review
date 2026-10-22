//! The [`MaterialMode`] override: the *effective* material table the renderer
//! draws when the Source Material button is set to Standard or Unique. The
//! imported [`MaterialState`]s are left untouched (invariant 1); these helpers
//! derive a replacement table (and, for Unique, a per-triangle mesh-part grouping
//! key) that the scene resources upload + group by instead.
//!
//! - **Standard** — one uniform matte mid-grey material for every part.
//! - **Unique** — that same matte material, but a distinct hue per mesh part
//!   (the model's per-triangle owning node), all at one OKLab lightness so only
//!   the hue distinguishes the parts.

use std::collections::HashMap;

use glam::Vec3;
use review_model::ModelData;
use review_model::color::oklch_to_linear_srgb;

use crate::config::MaterialMode;

use super::state::{AlphaMode, MaterialState, RoughnessWorkflow};

/// Mid-grey base color (linear RGB) of the standard material.
const STANDARD_VALUE: f32 = 0.5;
/// OKLab lightness every Unique-mode hue is generated at, so the parts read
/// apart by hue alone, not lightness — an HSV wheel at one value made its blues
/// far darker than its yellows. Just under the standard grey's 0.79, where the
/// sRGB gamut still holds [`UNIQUE_CHROMA`] at every hue.
const UNIQUE_LIGHTNESS: f32 = 0.75;
/// OKLab chroma of the Unique-mode per-part hues. Moderate, so the colors stay
/// readable rather than fully-saturated and garish.
const UNIQUE_CHROMA: f32 = 0.12;
/// Golden-ratio conjugate: stepping the hue by this per part spreads successive
/// parts maximally around the color wheel, so neighboring parts contrast even
/// though the sequence is deterministic (stable across frames / reloads).
const HUE_STEP: f32 = 0.618_034;

/// The matte standard material: a fully-rough, non-metallic, non-emissive surface
/// of `base_color` (linear RGB), with no texture slots bound.
fn standard_material(base_color: Vec3) -> MaterialState {
    MaterialState {
        base_color,
        metallic: 0.0,
        roughness: 1.0,
        emissive: Vec3::ZERO,
        textures: Default::default(),
        alpha_mode: AlphaMode::Opaque,
        alpha_cutoff: 0.5,
        workflow: RoughnessWorkflow::Roughness,
    }
}

/// The standard material at mid grey (Standard mode, and the Unique fallback when
/// the model carries no per-part info).
fn standard_grey() -> MaterialState {
    standard_material(Vec3::splat(STANDARD_VALUE))
}

/// The standard material for Unique mesh part `index`: the matte surface tinted
/// with a deterministic per-part hue at the shared lightness.
fn unique_material(index: usize) -> MaterialState {
    let hue = (index as f32 * HUE_STEP).fract();
    standard_material(Vec3::from(oklch_to_linear_srgb(
        UNIQUE_LIGHTNESS,
        UNIQUE_CHROMA,
        hue,
    )))
}

/// The effective material table for `mode`: the imported `source` materials in
/// Source mode; otherwise the standard/unique replacement table indexed by the
/// same key the draw ranges are grouped by (material slot for Standard, mesh part
/// for Unique). `part_count` is the number of unique mesh parts from
/// [`build_part_key`] (0 when the model carries none).
///
/// The replacement table always has at least one entry, so a model with no
/// per-triangle material info — drawn as one range on material 0 — still draws as
/// the standard material rather than the renderer's neutral fallback. Triangles
/// import tagged as having no material (`u32::MAX`) index past the end of any
/// table; they draw with [`unmaterialed_material`].
pub(crate) fn effective_materials(
    mode: MaterialMode,
    source: &[MaterialState],
    part_count: usize,
) -> Vec<MaterialState> {
    match mode {
        MaterialMode::Source => source.to_vec(),
        MaterialMode::Standard => vec![standard_grey(); source.len().max(1)],
        MaterialMode::Unique => {
            if part_count == 0 {
                // No per-part info to color by: degrade to the uniform standard
                // table (indexed by material slot, matching the None grouping key).
                vec![standard_grey(); source.len().max(1)]
            } else {
                (0..part_count).map(unique_material).collect()
            }
        }
    }
}

/// What a triangle with no material of its own (import tags it `u32::MAX`, which
/// no table reaches) draws with in `mode`, or `None` for the renderer's neutral
/// fallback.
///
/// Source mode shows what the file says, and a triangle the file gave no material
/// has none, so it keeps the neutral fallback. Standard and Unique replace every
/// material with the standard grey, and a triangle without one is no exception:
/// it used to fall through to the fallback (a lighter, glossier grey), so the one
/// mode meant to make every surface read the same drew those a different shade.
/// In Unique mode every triangle has a part, so the only ones that miss the table
/// are those of a model with no per-part info, which degrades to the standard
/// table anyway.
pub(super) fn unmaterialed_material(mode: MaterialMode) -> Option<MaterialState> {
    match mode {
        MaterialMode::Source => None,
        MaterialMode::Standard | MaterialMode::Unique => Some(standard_grey()),
    }
}

/// A per-triangle mesh-part grouping key: each triangle's owning node remapped to
/// a dense part index in first-seen order, plus the part count. The draw ranges
/// (main mesh, solo, visibility) group by this so each part is one draw bound to
/// its own hued material, and the part index doubles as the table index.
///
/// Returns `(empty, 0)` when the model carries no per-triangle node info — the
/// caller then falls back to the uniform standard table grouped by material slot.
pub(crate) fn build_part_key(model: &ModelData) -> (Vec<u32>, usize) {
    let triangle_count = model.indices.len() / 3;
    if triangle_count == 0 || model.triangles.node.len() != triangle_count {
        return (Vec::new(), 0);
    }
    let mut next: u32 = 0;
    let mut dense: HashMap<u32, u32> = HashMap::new();
    let key: Vec<u32> = model
        .triangles
        .node
        .iter()
        .map(|&node| {
            *dense.entry(node).or_insert_with(|| {
                let part = next;
                next += 1;
                part
            })
        })
        .collect();
    (key, next as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use review_model::{ModelData, TriangleData};

    #[test]
    fn part_key_densifies_nodes_in_first_seen_order() {
        // 4 triangles owned by nodes 5, 5, 2, 9 -> dense parts 0, 0, 1, 2.
        let model = ModelData {
            indices: vec![0; 12],
            triangles: TriangleData {
                node: vec![5, 5, 2, 9],
                ..Default::default()
            },
            ..Default::default()
        };
        let (key, count) = build_part_key(&model);
        assert_eq!(key, vec![0, 0, 1, 2]);
        assert_eq!(count, 3);
    }

    #[test]
    fn part_key_empty_without_node_info() {
        let model = ModelData {
            indices: vec![0; 9],
            ..Default::default()
        };
        let (key, count) = build_part_key(&model);
        assert!(key.is_empty());
        assert_eq!(count, 0);
    }

    #[test]
    fn effective_materials_table_sizes() {
        let source = vec![MaterialState::default(); 3];
        // Source returns the imported table unchanged.
        assert_eq!(
            effective_materials(MaterialMode::Source, &source, 4).len(),
            3
        );
        // Standard: one grey per source material.
        let standard = effective_materials(MaterialMode::Standard, &source, 4);
        assert_eq!(standard.len(), 3);
        assert!(
            standard
                .iter()
                .all(|m| m.roughness == 1.0 && m.metallic == 0.0)
        );
        // Unique: one hued material per part.
        assert_eq!(
            effective_materials(MaterialMode::Unique, &source, 4).len(),
            4
        );
        // Unique with no part info degrades to the standard table.
        assert_eq!(
            effective_materials(MaterialMode::Unique, &source, 0).len(),
            3
        );
        // Always at least one entry, even with no source materials.
        assert_eq!(effective_materials(MaterialMode::Standard, &[], 0).len(), 1);
    }

    /// A triangle with no material draws as every other triangle does in the
    /// modes that replace materials, and as the file left it in Source mode.
    #[test]
    fn an_unmaterialed_triangle_gets_the_standard_grey() {
        let standard = effective_materials(MaterialMode::Standard, &[], 0)[0].clone();
        for mode in [MaterialMode::Standard, MaterialMode::Unique] {
            assert_eq!(
                unmaterialed_material(mode),
                Some(standard.clone()),
                "{mode:?}"
            );
        }
        assert_eq!(standard.base_color, Vec3::splat(STANDARD_VALUE));
        assert_eq!(standard.roughness, 1.0);
        assert_eq!(unmaterialed_material(MaterialMode::Source), None);
    }

    #[test]
    fn unique_hues_are_distinct_but_share_lightness() {
        // Different parts get different colors, so no two consecutive parts
        // collide.
        let a = unique_material(0).base_color;
        let b = unique_material(1).base_color;
        assert_ne!(a, b);
        // Every hue is generated at the shared OKLab lightness, the blues
        // included.
        for index in 0..24 {
            let c = unique_material(index).base_color;
            let [lightness, ..] = review_model::color::linear_srgb_to_oklab(c.into());
            assert!(
                (lightness - UNIQUE_LIGHTNESS).abs() < 1e-3,
                "part {index} lightness {lightness}"
            );
        }
    }
}
