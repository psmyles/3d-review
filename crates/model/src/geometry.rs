//! The geometry primitives every other module is written in terms of.
//!
//! A [`Vertex`] is one render corner — import splits every face corner into its
//! own, which is why a mesh reaches the viewer with no shared vertices at all.
//! [`TriangleData`] is the per-triangle metadata beside the index buffer, held
//! as parallel arrays that must stay in lockstep; its `validate` is the guard.

use glam::{Vec2, Vec3, Vec4};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub uv: Vec2,
    pub tangent: Vec4,
    /// Per-vertex RGBA color from the mesh's vertex-color attribute (the DCC
    /// color set). White when the mesh carries no vertex-color layer. Visualized
    /// by the vertex-color debug view. The resolved material base color and
    /// smoothness are no longer baked per vertex (Phase 1): they live on
    /// [`MaterialImportDefaults`] and drive the per-material draws via the renderer's
    /// material table.
    pub vertex_color: Vec4,
}

impl Default for Vertex {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            normal: Vec3::Y,
            uv: Vec2::ZERO,
            tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
            vertex_color: Vec4::ONE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl Bounds {
    pub const EMPTY: Self = Self {
        min: Vec3::splat(f32::INFINITY),
        max: Vec3::splat(f32::NEG_INFINITY),
    };

    pub fn include_point(&mut self, point: Vec3) {
        self.min = self.min.min(point);
        self.max = self.max.max(point);
    }

    pub fn center(self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn size(self) -> Vec3 {
        self.max - self.min
    }

    pub fn radius(self) -> f32 {
        self.size().length() * 0.5
    }

    pub fn is_empty(self) -> bool {
        self.min.x.is_infinite() || self.max.x.is_infinite()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TopologyFace {
    pub first_index: u32,
    pub index_count: u32,
}

/// Per-triangle metadata, grouped so the parallel arrays stay in lockstep. Each
/// array is either empty or exactly `triangle_count` (= `indices.len() / 3`) long
/// and ordered by triangle index; [`TriangleData::validate`] asserts this at the
/// import funnel (invariant 7) so a drifted array is caught once, not by every
/// reader's ad-hoc length guard. Future per-triangle audit data (Phase 7
/// centroids, etc.) lands here with the same guard.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TriangleData {
    /// Owning original polygon, indexing [`ModelData::faces`].
    pub to_face: Vec<u32>,
    /// Material slot, indexing [`ModelData::materials`], or `u32::MAX` for a
    /// triangle whose face carried no material. Drives the per-material draw
    /// grouping (Phase 1) without a per-vertex `material_id`.
    pub material: Vec<u32>,
    /// Owning scene-graph node, indexing [`ModelData::nodes`] — drives the
    /// Outliner's per-node selection / solo (Phase 2). Empty for models with no
    /// node hierarchy.
    pub node: Vec<u32>,
}

impl TriangleData {
    /// Number of triangles, inferred from the (mutually equal-length) arrays.
    pub fn len(&self) -> usize {
        self.to_face.len()
    }

    pub fn is_empty(&self) -> bool {
        self.to_face.is_empty()
    }

    /// Each present (non-empty) array must be exactly `triangle_count` long — an
    /// empty array means "this model carries no such per-triangle info" and is
    /// allowed — and every entry must index a real face / material / node
    /// (`u32::MAX` is the no-material sentinel). Returns `Err` describing the
    /// first array that drifted or the first out-of-range entry.
    pub fn validate(
        &self,
        triangle_count: usize,
        face_count: usize,
        material_count: usize,
        node_count: usize,
    ) -> Result<(), String> {
        for (name, array) in [
            ("to_face", &self.to_face),
            ("material", &self.material),
            ("node", &self.node),
        ] {
            if !array.is_empty() && array.len() != triangle_count {
                return Err(format!(
                    "tri_{name} has {} entries, expected {triangle_count}",
                    array.len()
                ));
            }
        }
        if let Some(&face) = self
            .to_face
            .iter()
            .find(|&&face| face as usize >= face_count)
        {
            return Err(format!(
                "tri_to_face references face {face} of {face_count}"
            ));
        }
        if let Some(&slot) = self
            .material
            .iter()
            .find(|&&slot| slot != u32::MAX && slot as usize >= material_count)
        {
            return Err(format!(
                "tri_material references material {slot} of {material_count}"
            ));
        }
        if let Some(&node) = self.node.iter().find(|&&node| node as usize >= node_count) {
            return Err(format!("tri_node references node {node} of {node_count}"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangle_data_validate_catches_drift() {
        // Equal-length arrays with in-range entries pass.
        let ok = TriangleData {
            to_face: vec![0, 0, 1],
            material: vec![0, 0, 0],
            node: vec![0, 0, 0],
        };
        assert!(ok.validate(3, 2, 1, 1).is_ok());

        // An empty array means "no such info" and is allowed.
        let no_nodes = TriangleData {
            to_face: vec![0, 0, 1],
            material: vec![0, 0, 0],
            node: Vec::new(),
        };
        assert!(no_nodes.validate(3, 2, 1, 0).is_ok());

        // A present-but-short array is a drift and is rejected with a clear message.
        let drifted = TriangleData {
            to_face: vec![0, 0, 1],
            material: vec![0, 0],
            node: vec![0, 0, 0],
        };
        let err = drifted.validate(3, 2, 1, 1).unwrap_err();
        assert!(err.contains("material"), "message names the drifted array");

        // All-empty (a model with no per-triangle info) passes for any count.
        assert!(TriangleData::default().validate(0, 0, 0, 0).is_ok());
    }

    #[test]
    fn triangle_data_validate_catches_out_of_range_indices() {
        let base = TriangleData {
            to_face: vec![0, 1, 1],
            material: vec![0, u32::MAX, 0],
            node: vec![0, 0, 0],
        };
        assert!(base.validate(3, 2, 1, 1).is_ok());

        // A face index past the face table is rejected.
        let bad_face = TriangleData {
            to_face: vec![0, 2, 1],
            ..base.clone()
        };
        let err = bad_face.validate(3, 2, 1, 1).unwrap_err();
        assert!(
            err.contains("to_face"),
            "message names the bad array: {err}"
        );

        // A material slot past the material table is rejected, but the
        // `u32::MAX` no-material sentinel stays allowed.
        let bad_material = TriangleData {
            material: vec![0, 1, 0],
            ..base.clone()
        };
        let err = bad_material.validate(3, 2, 1, 1).unwrap_err();
        assert!(
            err.contains("material"),
            "message names the bad array: {err}"
        );

        // A node index past the node table is rejected.
        let bad_node = TriangleData {
            node: vec![0, 0, 1],
            ..base
        };
        let err = bad_node.validate(3, 2, 1, 1).unwrap_err();
        assert!(err.contains("node"), "message names the bad array: {err}");
    }
}
