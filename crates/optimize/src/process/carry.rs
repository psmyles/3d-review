//! What a processed level carries beside its mesh.
//!
//! The renderer has no use for any of it — the corner-run layout it assumes
//! cannot survive a weld — so the polygons, color sets, creases, extra skins and
//! DQ weights ride here and reach only the exporter.

use review_model::ModelData;

use crate::submesh::NO_FACE;

use super::*;

/// One output level: the mesh, and what measuring it produced.
#[derive(Debug, Clone)]
pub struct ProcessedLod {
    /// 0 for the base mesh, then one per configured LOD level.
    pub level: usize,
    pub model: ModelData,
    pub metrics: AnalysisMetrics,
    /// What the source authored about this level's vertices and faces that the
    /// stack did not change — for the export; the viewport reads none of it.
    pub carry: LevelCarry,
}

/// The polygon topology of one processed piece (a node's triangles of one
/// material), in the assembled level's vertex and triangle numbering.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodePolygons {
    pub node: u32,
    /// Material slot, or `NO_MATERIAL`.
    pub material: u32,
    /// The assembled index of this piece's first triangle; `triangle_face` is
    /// parallel to the piece's triangles from there.
    pub triangle_first: u32,
    /// Per triangle of the piece, the local face or [`NO_FACE`].
    pub triangle_face: Vec<u32>,
    /// `face_count + 1` starts into `corners`.
    pub face_offsets: Vec<u32>,
    /// Assembled vertex indices.
    pub corners: Vec<u32>,
    /// Per face, the source face it came from (`ModelData::faces` of the source).
    pub source_face: Vec<u32>,
    pub face_smoothing: Vec<bool>,
    pub face_hole: Vec<bool>,
    pub face_group: Vec<u32>,
    /// Assembled vertex pairs.
    pub edges: Vec<[u32; 2]>,
    /// Per edge, the source edge index (`MeshExtras::edges` of the part).
    pub source_edge: Vec<u32>,
    pub edge_smoothing: Vec<bool>,
    pub edge_crease: Vec<f32>,
    pub edge_visibility: Vec<bool>,
}

impl NodePolygons {
    pub fn face_count(&self) -> usize {
        self.face_offsets.len().saturating_sub(1)
    }

    pub fn face(&self, face: usize) -> &[u32] {
        let first = self.face_offsets[face] as usize;
        let end = self.face_offsets[face + 1] as usize;
        &self.corners[first..end]
    }
}

/// Everything a level carries for the export beyond its `ModelData`. The
/// model itself stays a pure triangle mesh with the layout the renderer
/// expects; this is where the polygons the operations preserved, the extra
/// color sets and the vertex creases live, all in the level's own numbering.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LevelCarry {
    /// One entry per piece that still knows its source polygons.
    pub polygons: Vec<NodePolygons>,
    /// Vertex-color sets beyond the first, each parallel to the level's
    /// vertices (white where a piece had none). Empty when no piece had any.
    pub color_channels: Vec<Vec<glam::Vec4>>,
    /// Per vertex; empty when no piece carried creases.
    pub vertex_crease: Vec<f32>,
    /// Skin layers beyond the first, per node: `(level vertex, cluster, weight)`
    /// with the cluster indexing that layer's cluster list in the source's
    /// `MeshExtras::extra_skins`.
    pub extra_skins: Vec<LevelSkinLayer>,
    /// The primary skin's dual-quaternion weights: `(level vertex, weight)`.
    pub dq_weights: Vec<(u32, f32)>,
    /// Per level vertex, a source corner it came from (`u32::MAX` unknown).
    pub source_corner: Vec<u32>,
}

impl LevelCarry {
    /// The level's polygon count: the faces that survived plus every triangle
    /// that belongs to none.
    pub fn polygon_count(&self, triangle_count: usize) -> usize {
        let faces: usize = self.polygons.iter().map(NodePolygons::face_count).sum();
        let faced: usize = self
            .polygons
            .iter()
            .map(|piece| {
                piece
                    .triangle_face
                    .iter()
                    .filter(|&&face| face != NO_FACE)
                    .count()
            })
            .sum();
        faces + triangle_count.saturating_sub(faced)
    }
}

/// One skin layer beyond the first, over a level's vertices.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LevelSkinLayer {
    pub node: u32,
    /// Index into the node's `MeshExtras::extra_skins`.
    pub layer: usize,
    pub influences: Vec<(u32, u32, f32)>,
}
