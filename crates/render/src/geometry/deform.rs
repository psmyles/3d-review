//! The per-model deform layout: what every render vertex's `deform` lane points
//! at, and the two structured-buffer tables those lanes index.
//!
//! Built once per model alongside the mesh upload. The influence table starts
//! with one single-entry run per scene node (entry `i`, weight 1) — what a rigid
//! mesh vertex and the skeleton overlay reference — followed by the skin's CSR
//! rows re-targeted at the cluster entries that follow the node entries in the
//! palette (`review_model::anim::build_palette`). Blend-shape deltas are the
//! model's own per-logical-vertex CSR, copied into GPU form; a corner's morph
//! lane simply names its logical vertex's row.
//!
//! Every vertex builder that derives geometry from the mesh copies the source
//! corner's lane, so wireframe edges, normal lines, the heat map and the
//! selection flash all deform with the mesh through the one vertex shader.

use review_model::ModelData;

use crate::scene::{InfluenceEntry, MorphEntry};

/// The deform lane of geometry that never deforms.
pub(crate) const NO_DEFORM: [u32; 4] = [0; 4];

/// The lane for geometry rigidly attached to scene node `node` — the node's own
/// single-entry run in the influence table (entry `node`, weight 1).
pub(crate) fn node_deform(node: usize) -> [u32; 4] {
    [node as u32, 1, 0, 0]
}

/// The lane of render vertex `corner` in `lanes`, or [`NO_DEFORM`] when the
/// model carries no layout (an empty slice) or the corner is out of range.
pub(crate) fn corner_deform(lanes: &[[u32; 4]], corner: usize) -> [u32; 4] {
    lanes.get(corner).copied().unwrap_or(NO_DEFORM)
}

/// The deform layout of one model — see the module docs.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct DeformLayout {
    /// Per render vertex, its `deform` lane.
    pub(crate) corner: Vec<[u32; 4]>,
    /// The influence table (`t12`).
    pub(crate) influences: Vec<InfluenceEntry>,
    /// The blend-shape delta table (`t14`), empty when the model has none.
    pub(crate) morph: Vec<MorphEntry>,
    /// Palette entries the shader may index: nodes + clusters.
    pub(crate) palette_len: usize,
    /// Blend shapes the weight buffer (`t15`) must hold.
    pub(crate) shape_count: usize,
}

impl DeformLayout {
    /// Build the layout for `model`, or `None` when it needs no deform path
    /// (no skin, no blend shapes, no clips) or has no nodes to attach to.
    pub(crate) fn build(model: &ModelData) -> Option<Self> {
        if !model.needs_deform() || model.nodes.is_empty() {
            return None;
        }
        let node_count = model.nodes.len();

        // Node entries first: entry `i` is node `i`'s rigid delta.
        let mut influences: Vec<InfluenceEntry> = (0..node_count)
            .map(|node| InfluenceEntry {
                entry: node as u32,
                weight: 1.0,
            })
            .collect();

        // Each corner's owning node, from the per-triangle node info.
        let mut corner_node = vec![u32::MAX; model.vertices.len()];
        let triangle_count = model.indices.len() / 3;
        if model.triangles.node.len() == triangle_count {
            for (triangle, corners) in model.indices.chunks_exact(3).enumerate() {
                for &corner in corners {
                    if let Some(slot) = corner_node.get_mut(corner as usize) {
                        *slot = model.triangles.node[triangle];
                    }
                }
            }
        }

        // Skin runs are emitted once per logical vertex and shared by every
        // corner expanded from it.
        let logical_count = model.stats.vertex_count;
        let mut logical_run: Vec<Option<(u32, u32)>> = vec![None; logical_count];
        let skin = model.skin.as_ref();
        let morph = model.morph.as_ref();

        let corner = (0..model.vertices.len())
            .map(|index| {
                let logical = model.corner_to_logical.get(index).map(|&v| v as usize);
                let mut lane = NO_DEFORM;

                let mut skinned = false;
                if let (Some(skin), Some(logical)) = (skin, logical) {
                    if let Some(Some(run)) = logical_run.get(logical) {
                        lane[0] = run.0;
                        lane[1] = run.1;
                        skinned = run.1 > 0;
                    } else {
                        let range = skin.influence_range(logical);
                        let first = influences.len() as u32;
                        for influence in range {
                            influences.push(InfluenceEntry {
                                entry: (node_count + skin.influence_cluster[influence] as usize)
                                    as u32,
                                weight: skin.weights[influence],
                            });
                        }
                        let count = influences.len() as u32 - first;
                        if let Some(slot) = logical_run.get_mut(logical) {
                            *slot = Some((first, count));
                        }
                        lane[0] = first;
                        lane[1] = count;
                        skinned = count > 0;
                    }
                }
                if !skinned {
                    // Unskinned (or a skinned vertex with an empty row): ride the
                    // owning node rigidly, when there is one.
                    let node = corner_node[index];
                    if (node as usize) < node_count {
                        lane[0] = node;
                        lane[1] = 1;
                    } else {
                        lane[0] = 0;
                        lane[1] = 0;
                    }
                }

                if let (Some(morph), Some(logical)) = (morph, logical) {
                    let range = morph.entry_range(logical);
                    lane[2] = range.start as u32;
                    lane[3] = range.len() as u32;
                }
                lane
            })
            .collect();

        let morph_entries = morph
            .map(|morph| {
                morph
                    .shape
                    .iter()
                    .zip(&morph.position)
                    .zip(&morph.normal)
                    .map(|((&shape, position), normal)| MorphEntry {
                        shape,
                        position: position.to_array(),
                        normal: normal.to_array(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Some(Self {
            corner,
            influences,
            morph: morph_entries,
            palette_len: review_model::anim::palette_len(model),
            shape_count: morph.map_or(0, |morph| morph.shapes.len()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Vec3};
    use review_model::{
        AnimationClip, ModelStats, MorphData, MorphShape, SceneNode, SkinCluster, SkinData,
        TriangleData, Vertex,
    };

    /// Three nodes; one triangle owned by node 1; corners 0..3 map to logical
    /// 0..3.
    fn base_model() -> ModelData {
        ModelData {
            vertices: vec![Vertex::default(); 3],
            indices: vec![0, 1, 2],
            triangles: TriangleData {
                to_face: Vec::new(),
                material: Vec::new(),
                node: vec![1],
            },
            nodes: (0..3).map(|_| SceneNode::default()).collect(),
            stats: ModelStats {
                vertex_count: 3,
                ..ModelStats::default()
            },
            corner_to_logical: vec![0, 1, 2],
            ..ModelData::default()
        }
    }

    #[test]
    fn a_static_model_has_no_layout() {
        assert!(DeformLayout::build(&base_model()).is_none());
    }

    #[test]
    fn rigid_corners_reference_their_node_entry() {
        let mut model = base_model();
        model.animations.push(AnimationClip::default());
        let layout = DeformLayout::build(&model).expect("a clip needs the deform path");
        assert_eq!(layout.influences.len(), 3);
        assert_eq!(layout.influences[1].entry, 1);
        assert_eq!(layout.influences[1].weight, 1.0);
        for lane in &layout.corner {
            assert_eq!(*lane, node_deform(1));
        }
        assert_eq!(layout.palette_len, 3);
        assert!(layout.morph.is_empty());
    }

    #[test]
    fn skinned_corners_reference_cluster_runs_and_share_them() {
        let mut model = base_model();
        // Corners 0 and 1 both expand from logical 0 (2 influences); corner 2 is
        // logical 1 with an empty row and falls back to its node.
        model.corner_to_logical = vec![0, 0, 1];
        let cluster = |bone: u32| SkinCluster {
            bone,
            mesh_node: 1,
            world_to_bone_bind: Mat4::IDENTITY,
        };
        model.skin = Some(SkinData {
            offsets: vec![0, 2, 2, 2],
            bones: vec![0, 2],
            weights: vec![0.25, 0.75],
            influence_cluster: vec![0, 1],
            clusters: vec![cluster(0), cluster(2)],
            deformers: Vec::new(),
        });
        let layout = DeformLayout::build(&model).expect("skinned");
        // 3 node entries + one shared 2-entry run.
        assert_eq!(layout.influences.len(), 5);
        assert_eq!(layout.corner[0], [3, 2, 0, 0]);
        assert_eq!(layout.corner[1], [3, 2, 0, 0]);
        assert_eq!(layout.corner[2], node_deform(1));
        assert_eq!(
            layout.influences[3],
            InfluenceEntry {
                entry: 3,
                weight: 0.25
            }
        );
        assert_eq!(
            layout.influences[4],
            InfluenceEntry {
                entry: 4,
                weight: 0.75
            }
        );
        assert_eq!(layout.palette_len, 5);
    }

    #[test]
    fn morph_lanes_name_the_logical_row() {
        let mut model = base_model();
        model.morph = Some(MorphData {
            channels: Vec::new(),
            shapes: vec![MorphShape::default(), MorphShape::default()],
            offsets: vec![0, 0, 2, 2],
            shape: vec![0, 1],
            position: vec![Vec3::X, Vec3::Y],
            normal: vec![Vec3::ZERO, Vec3::ZERO],
        });
        let layout = DeformLayout::build(&model).expect("morphed");
        assert_eq!(layout.corner[0][2..], [0, 0]);
        assert_eq!(layout.corner[1][2..], [0, 2]);
        assert_eq!(layout.corner[2][2..], [2, 0]);
        assert_eq!(layout.morph.len(), 2);
        assert_eq!(layout.morph[1].shape, 1);
        assert_eq!(layout.morph[1].position, [0.0, 1.0, 0.0]);
        assert_eq!(layout.shape_count, 2);
        assert_eq!(corner_deform(&layout.corner, 9), NO_DEFORM);
        assert_eq!(corner_deform(&[], 0), NO_DEFORM);
    }
}
