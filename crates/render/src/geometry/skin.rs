//! The skin-weight heat map's CPU geometry: a vertex buffer parallel to the mesh's
//! own, carrying the selected bones' influence baked into the vertex color.
//!
//! Deliberately **corner-parallel** with [`model_mesh`]'s output — one entry per
//! render vertex, same order — so the existing index buffer, per-material draw
//! ranges, and the solo / visibility index lists all keep working unchanged; the
//! renderer simply binds this buffer in place of the mesh's.
//!
//! Vertices keep the mesh's **real normals**, because the shader Lambert-shades a
//! dark matte base and tints the ramp over it — a flat heat map would lose the
//! silhouette entirely against a dark background, since an uninfluenced region is
//! near-black. The ramp color rides in `vertex_color.rgb` and the influence
//! fraction in `vertex_color.a`; `review.glsl` does the blend and the shading, keyed
//! off `projection_params.w`.
//!
//! [`model_mesh`]: super::mesh::model_mesh

use review_model::ModelData;

use crate::scene::SceneVertex;

use super::deform::corner_deform;
use super::vertex::push_shaded_vertex;

/// The heat ramp: blue (0) → cyan → green → yellow → red (1), the convention every
/// DCC weight-paint tool uses, so an artist reads it without a legend.
///
/// Emitted in **gamma space**, like every other overlay color — the scene shader
/// decodes it before blending it over the shaded base.
pub(crate) fn weight_ramp(weight: f32) -> [f32; 4] {
    // Four equal segments, each a linear blend between two stops.
    const STOPS: [[f32; 3]; 5] = [
        [0.0, 0.0, 1.0], // 0.00 blue
        [0.0, 1.0, 1.0], // 0.25 cyan
        [0.0, 1.0, 0.0], // 0.50 green
        [1.0, 1.0, 0.0], // 0.75 yellow
        [1.0, 0.0, 0.0], // 1.00 red
    ];
    let weight = if weight.is_finite() {
        weight.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let scaled = weight * (STOPS.len() - 1) as f32;
    // `min` keeps weight == 1.0 (scaled == 4.0) inside the last segment.
    let index = (scaled.floor() as usize).min(STOPS.len() - 2);
    let t = scaled - index as f32;
    let (low, high) = (STOPS[index], STOPS[index + 1]);
    [
        low[0] + (high[0] - low[0]) * t,
        low[1] + (high[1] - low[1]) * t,
        low[2] + (high[2] - low[2]) * t,
        1.0,
    ]
}

/// Build the heat-map vertex buffer for `selected` (sorted, deduplicated node
/// indices).
///
/// Each render vertex carries the ramp color in `rgb` and, in `a`, the *fraction*
/// of its logical source vertex's total influence that the selected bones account
/// for — see [`SkinData::influence_fraction`], which normalizes against the
/// vertex's own total so an un-normalized rig still reads truthfully. The shader
/// blends from the neutral base toward the ramp by that alpha, so a vertex no
/// selected bone touches (and every vertex when nothing is selected) renders as
/// plain shaded grey rather than as ramp-blue — an untouched vertex is *outside*
/// the 0..1 weight range, not a zero inside it.
///
/// Returns an empty vector when the model carries no skin, which the caller treats
/// as "nothing to draw, fall back to the mesh".
///
/// [`SkinData::influence_fraction`]: review_model::SkinData::influence_fraction
pub(crate) fn skin_weight_vertices(
    model: &ModelData,
    lanes: &[[u32; 4]],
    selected: &[u32],
) -> Vec<SceneVertex> {
    let Some(skin) = model.skin.as_ref() else {
        return Vec::new();
    };

    let mut vertices = Vec::with_capacity(model.vertices.len());
    for (index, vertex) in model.vertices.iter().enumerate() {
        let fraction = match model.corner_to_logical.get(index) {
            Some(&logical) if !selected.is_empty() => {
                skin.influence_fraction(logical as usize, selected)
            }
            _ => 0.0,
        };
        let ramp = weight_ramp(fraction);
        push_shaded_vertex(
            &mut vertices,
            vertex.position.to_array(),
            vertex.normal.to_array(),
            [ramp[0], ramp[1], ramp[2], fraction],
            corner_deform(lanes, index),
        );
    }
    vertices
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use review_model::{SkinData, Vertex};

    /// The color a vertex gets when the selection doesn't touch it: the ramp's
    /// zero stop, but with a zero influence alpha, which is what tells the shader
    /// to leave it at the neutral shaded base.
    fn untouched() -> [f32; 4] {
        [0.0, 0.0, 1.0, 0.0]
    }

    /// The expected vertex color for an influence `fraction`: ramp hue, fraction
    /// in alpha.
    fn tinted(fraction: f32) -> [f32; 4] {
        let ramp = weight_ramp(fraction);
        [ramp[0], ramp[1], ramp[2], fraction]
    }

    /// Three render corners over two logical vertices:
    ///
    /// * logical 0 — 0.75 from bone 1, 0.25 from bone 2 (corners 0 and 1)
    /// * logical 1 — 1.0 from bone 2 (corner 2)
    fn skinned_model() -> ModelData {
        ModelData {
            vertices: vec![
                Vertex {
                    position: Vec3::X,
                    ..Default::default()
                },
                Vertex {
                    position: Vec3::Y,
                    ..Default::default()
                },
                Vertex {
                    position: Vec3::Z,
                    ..Default::default()
                },
            ],
            corner_to_logical: vec![0, 0, 1],
            skin: Some(SkinData {
                offsets: vec![0, 2, 3],
                bones: vec![1, 2, 2],
                weights: vec![0.75, 0.25, 1.0],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn ramp_hits_its_stops() {
        assert_eq!(weight_ramp(0.0), [0.0, 0.0, 1.0, 1.0], "blue at zero");
        assert_eq!(weight_ramp(0.25), [0.0, 1.0, 1.0, 1.0], "cyan");
        assert_eq!(weight_ramp(0.5), [0.0, 1.0, 0.0, 1.0], "green at half");
        assert_eq!(weight_ramp(0.75), [1.0, 1.0, 0.0, 1.0], "yellow");
        assert_eq!(weight_ramp(1.0), [1.0, 0.0, 0.0, 1.0], "red at full");
    }

    #[test]
    fn ramp_clamps_out_of_range_and_non_finite_input() {
        assert_eq!(weight_ramp(-1.0), weight_ramp(0.0));
        assert_eq!(weight_ramp(5.0), weight_ramp(1.0));
        assert_eq!(weight_ramp(f32::NAN), weight_ramp(0.0));
    }

    #[test]
    fn ramp_moves_from_cold_to_hot_across_the_range() {
        // Red rises and blue falls monotonically — the property that makes the
        // ramp readable, independent of the exact stops.
        let samples: Vec<[f32; 4]> = (0..=10).map(|i| weight_ramp(i as f32 / 10.0)).collect();
        for pair in samples.windows(2) {
            assert!(pair[1][0] >= pair[0][0], "red must not fall: {pair:?}");
            assert!(pair[1][2] <= pair[0][2], "blue must not rise: {pair:?}");
        }
    }

    #[test]
    fn the_buffer_is_parallel_to_the_mesh_vertices() {
        let model = skinned_model();
        let vertices = skin_weight_vertices(&model, &[], &[1]);
        assert_eq!(vertices.len(), model.vertices.len());
        // Positions are copied straight through, so the shared index buffer,
        // material ranges and visibility lists stay valid.
        assert_eq!(vertices[0].position, [1.0, 0.0, 0.0]);
        assert_eq!(vertices[2].position, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn a_single_bone_paints_only_its_own_influence() {
        let model = skinned_model();
        let vertices = skin_weight_vertices(&model, &[], &[1]);
        // Bone 1 owns 75% of logical vertex 0 -> yellow, on both its corners.
        assert_eq!(vertices[0].vertex_color, tinted(0.75));
        assert_eq!(vertices[1].vertex_color, tinted(0.75));
        // It doesn't touch logical vertex 1 at all, so that corner carries a zero
        // influence alpha and the shader leaves it on the neutral base.
        assert_eq!(vertices[2].vertex_color, untouched());
    }

    #[test]
    fn selecting_both_bones_sums_to_full_influence() {
        let model = skinned_model();
        // This is the multi-select payoff: overlapping regions read hotter.
        let vertices = skin_weight_vertices(&model, &[], &[1, 2]);
        assert_eq!(vertices[0].vertex_color, tinted(1.0));
        assert_eq!(vertices[2].vertex_color, tinted(1.0));
    }

    #[test]
    fn nothing_selected_paints_the_whole_mesh_inert() {
        let model = skinned_model();
        let vertices = skin_weight_vertices(&model, &[], &[]);
        // Every vertex reads as zero influence, so the whole mesh renders as the
        // plain shaded base rather than a wall of ramp-blue.
        assert!(vertices.iter().all(|v| v.vertex_color == untouched()));
    }

    #[test]
    fn an_unskinned_model_produces_no_buffer() {
        let mut model = skinned_model();
        model.skin = None;
        assert!(skin_weight_vertices(&model, &[], &[1]).is_empty());
    }

    #[test]
    fn heat_map_vertices_keep_the_mesh_normals() {
        let mut model = skinned_model();
        model.vertices[0].normal = Vec3::new(0.0, 1.0, 0.0);
        let vertices = skin_weight_vertices(&model, &[], &[1]);
        // The shader Lambert-shades this buffer, so a zero normal here would drop
        // it onto the flat overlay path and lose the silhouette entirely.
        assert_eq!(vertices[0].normal, [0.0, 1.0, 0.0]);
        assert!(vertices.iter().all(|v| v.normal != [0.0, 0.0, 0.0]));
    }

    #[test]
    fn a_short_corner_map_falls_back_instead_of_panicking() {
        // A corner the map doesn't cover can't be colored; it must read inert
        // rather than index out of bounds. (`SkinData::validate` rejects this at
        // import, so this is belt-and-braces for a hand-built model.)
        let mut model = skinned_model();
        model.corner_to_logical.truncate(1);
        let vertices = skin_weight_vertices(&model, &[], &[1]);
        assert_eq!(vertices.len(), 3);
        assert_eq!(vertices[1].vertex_color, untouched());
        assert_eq!(vertices[2].vertex_color, untouched());
    }
}
