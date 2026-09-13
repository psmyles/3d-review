// Host-agnostic data only (invariant 10) — and fully safe (invariant 9): all
// `unsafe`/FFI lives in `import`/`psd` and the sanctioned D3D11 sites.
#![forbid(unsafe_code)]

use glam::{Vec2, Vec3};

pub mod anim;
mod bvh;
mod clip;
pub mod color;
mod demo;
pub mod extras;
mod geometry;
mod morph;
pub mod pick;
mod scene;
mod skin;
mod stats;

// The crate's whole surface is re-exported here, so every `review_model::X`
// path other crates use resolves exactly as it did when this was one file.
pub use anim::{AnimContext, DeformPose, Pose};
pub use bvh::{Bvh, Hit, SceneBvh, SceneHit, ray_triangle_t, triangle_positions};
pub use clip::{
    AnimationClip, DEFAULT_FRAME_RATE, Key, MorphTrack, NodeTrack, frame_rate_or_default,
};
pub use demo::demo_cube_model;
pub use extras::{ExtrasCounts, SourceExtras};
pub use geometry::{Bounds, TopologyFace, TriangleData, Vertex};
pub use morph::{MorphChannel, MorphData, MorphKeyframe, MorphShape};
pub use pick::PosedScene;
pub use scene::{BoneInfo, LocalTransform, MaterialImportDefaults, NodeKind, SceneNode};
pub use skin::{SkinCluster, SkinData, SkinDeformerInfo, SkinningMethod};
pub use stats::{MeshGroupStats, ModelStats, ScopeStats, StatsScope};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelData {
    pub name: String,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub faces: Vec<TopologyFace>,
    /// Per-triangle metadata (face / material / node), grouped so the parallel
    /// arrays stay in lockstep — see [`TriangleData`].
    pub triangles: TriangleData,
    /// The imported scene-graph hierarchy (every node, mesh-bearing or not), for
    /// the Outliner. Empty for procedurally-built models with no hierarchy.
    pub nodes: Vec<SceneNode>,
    /// Per-vertex UV coordinates for every UV set the model carries, one inner
    /// vector per channel (each `vertices.len()` long). Only populated when the
    /// model has **more than one** UV set; single-set models leave this empty
    /// and use [`Vertex::uv`] (which always holds channel 0). Channel 0 is thus
    /// mirrored here for multi-set models — a deliberate trade so the renderer
    /// can pick a channel by index without special-casing channel 0.
    pub uv_channels: Vec<Vec<Vec2>>,
    /// Names of the model's UV sets in source-file order (e.g. `"UVMap"`,
    /// `"UVMap.001"`), one per UV set. May be shorter than the UV-set count, or
    /// hold an empty string for an unnamed set; use [`ModelData::uv_set_label`]
    /// for a display label that falls back to a generated name.
    pub uv_set_names: Vec<String>,
    pub bounds: Option<Bounds>,
    pub stats: ModelStats,
    pub materials: Vec<MaterialImportDefaults>,
    /// Per render vertex (parallel to [`ModelData::vertices`]), the logical
    /// source vertex it was expanded from — what projects the per-logical-vertex
    /// skin and blend-shape tables onto the render mesh. Always filled by import;
    /// empty for a procedurally-built or optimizer-rebuilt mesh, which then
    /// carries no skin or morph data either.
    pub corner_to_logical: Vec<u32>,
    /// Skeletal binding data, `Some` only when the source carried a skin
    /// deformer whose clusters resolved to real bone nodes. See [`SkinData`].
    pub skin: Option<SkinData>,
    /// Blend-shape data, `Some` only when a mesh carried a blend deformer with at
    /// least one usable offset. See [`MorphData`].
    pub morph: Option<MorphData>,
    /// Every animation clip the file carries, in source order.
    pub animations: Vec<AnimationClip>,
    /// The file's declared frame rate; `0.0` when it declared none (see
    /// [`ModelData::frame_rate_or_default`]).
    pub frame_rate: f64,
}

impl ModelData {
    /// True when drawing this model needs the GPU deform path at all: it carries
    /// skin or blend-shape data (always deformed, even at rest, since the rest
    /// pose is the file's default pose, not the bind pose) or any clip that
    /// could move a node.
    pub fn needs_deform(&self) -> bool {
        self.skin.is_some() || self.morph.is_some() || !self.animations.is_empty()
    }

    /// The import-funnel lockstep guard for everything the deform path reads:
    /// the corner→logical map, the skin, the blend shapes and every clip. Called
    /// once at import (invariant 7) after [`TriangleData::validate`].
    pub fn validate_deform(&self) -> Result<(), String> {
        let logical_count = self.stats.vertex_count;
        let node_count = self.nodes.len();
        if !self.corner_to_logical.is_empty() {
            if self.corner_to_logical.len() != self.vertices.len() {
                return Err(format!(
                    "corner_to_logical has {} entries, expected {}",
                    self.corner_to_logical.len(),
                    self.vertices.len()
                ));
            }
            if let Some(&logical) = self
                .corner_to_logical
                .iter()
                .find(|&&logical| logical as usize >= logical_count)
            {
                return Err(format!(
                    "corner_to_logical references source vertex {logical} of {logical_count}"
                ));
            }
        }
        if (self.skin.is_some() || self.morph.is_some())
            && self.corner_to_logical.len() != self.vertices.len()
        {
            return Err(
                "a skinned or morphed model must carry its corner_to_logical map".to_owned(),
            );
        }
        if let Some(skin) = &self.skin {
            skin.validate(logical_count, node_count)?;
        }
        if let Some(morph) = &self.morph {
            morph.validate(logical_count, node_count)?;
        }
        let channel_count = self.morph.as_ref().map_or(0, |morph| morph.channels.len());
        for clip in &self.animations {
            clip.validate(node_count, channel_count)?;
        }
        Ok(())
    }

    /// UV coordinates for `vertex_index` in `channel`, falling back to the
    /// vertex's own [`Vertex::uv`] when the requested channel isn't stored
    /// (single-set models, or an out-of-range channel).
    pub fn uv_for_channel(&self, vertex_index: usize, channel: usize) -> Vec2 {
        self.uv_channels
            .get(channel)
            .and_then(|channel_uvs| channel_uvs.get(vertex_index).copied())
            .unwrap_or_else(|| {
                self.vertices
                    .get(vertex_index)
                    .map(|vertex| vertex.uv)
                    .unwrap_or(Vec2::ZERO)
            })
    }

    /// Display label for UV set `channel`: its source name when present and
    /// non-empty, otherwise a generated `"UV {channel}"` fallback.
    pub fn uv_set_label(&self, channel: usize) -> String {
        self.uv_set_names
            .get(channel)
            .filter(|name| !name.is_empty())
            .cloned()
            .unwrap_or_else(|| format!("UV {channel}"))
    }

    /// Display labels for every UV set the model carries, in source-file order.
    /// Empty when the model has no UV sets.
    pub fn uv_set_labels(&self) -> Vec<String> {
        (0..self.stats.uv_set_count)
            .map(|channel| self.uv_set_label(channel))
            .collect()
    }

    pub fn recompute_bounds(&mut self) {
        let mut bounds = Bounds::EMPTY;

        for vertex in &self.vertices {
            bounds.include_point(vertex.position);
        }

        self.bounds = (!bounds.is_empty()).then_some(bounds);
    }

    /// True when the mesh carries no usable per-vertex tangent basis — every
    /// vertex tangent is degenerate (zero length). The importer leaves a zero
    /// tangent when the source FBX has UVs but no tangent layer (common for Maya
    /// exports), which is the signal to synthesize one before normal mapping.
    pub fn has_degenerate_tangents(&self) -> bool {
        !self.vertices.is_empty()
            && self
                .vertices
                .iter()
                .all(|vertex| vertex.tangent.truncate().length_squared() < 1e-12)
    }

    /// Recompute per-vertex normals as the area-weighted average of the faces
    /// meeting at each vertex.
    ///
    /// Needed after any operation that *merges* vertices which carried different
    /// normals — a position-only weld, say. Merging keeps one of the originals
    /// arbitrarily, so the surviving normal describes one of the faces rather than
    /// the surface, and the mesh shades as noise until this runs.
    ///
    /// Smoothing is per *vertex*, so it never crosses a split the mesh still has:
    /// a hard edge whose two sides remain separate vertices keeps its two normals.
    /// Using the uncross product rather than a normalized face normal weights each
    /// face by twice its area, which is what keeps a fan of thin triangles from
    /// outvoting the large face beside it.
    ///
    /// A vertex whose incident faces cancel out (or that no face references) keeps
    /// the normal it had, so this can never introduce a zero-length one.
    pub fn generate_normals(&mut self) {
        if self.vertices.is_empty() || self.indices.len() < 3 {
            return;
        }

        let mut accumulated = vec![Vec3::ZERO; self.vertices.len()];
        for triangle in self.indices.as_chunks::<3>().0 {
            let [i0, i1, i2] = [
                triangle[0] as usize,
                triangle[1] as usize,
                triangle[2] as usize,
            ];
            let (Some(v0), Some(v1), Some(v2)) = (
                self.vertices.get(i0),
                self.vertices.get(i1),
                self.vertices.get(i2),
            ) else {
                continue;
            };
            // Unnormalized: its length is twice the triangle's area, which is the
            // weighting we want.
            let face = (v1.position - v0.position).cross(v2.position - v0.position);
            for &index in &[i0, i1, i2] {
                accumulated[index] += face;
            }
        }

        for (vertex, normal) in self.vertices.iter_mut().zip(accumulated) {
            if normal.length_squared() > 1e-20 {
                vertex.normal = normal.normalize();
            }
        }
    }

    /// Synthesize a per-vertex tangent basis from positions, UVs and normals
    /// (Lengyel's method): accumulate each triangle's UV-gradient tangent onto its
    /// corners, then Gram-Schmidt-orthonormalize against the vertex normal and
    /// store the bitangent handedness sign in `tangent.w`. Needed for correct
    /// normal mapping when the FBX omits a tangent layer — a constant placeholder
    /// tangent produces a garbage TBN and smeared shading. No-op for an empty or
    /// index-less mesh. Uses the primary UV set ([`Vertex::uv`]); a vertex whose
    /// accumulated tangent is degenerate (no UV area) falls back to an arbitrary
    /// orthonormal vector so the basis is never zero.
    pub fn generate_tangents(&mut self) {
        let vertex_count = self.vertices.len();
        if vertex_count == 0 || self.indices.len() < 3 {
            return;
        }

        let mut tangents = vec![Vec3::ZERO; vertex_count];
        let mut bitangents = vec![Vec3::ZERO; vertex_count];
        for triangle in self.indices.as_chunks::<3>().0 {
            let [i0, i1, i2] = [
                triangle[0] as usize,
                triangle[1] as usize,
                triangle[2] as usize,
            ];
            let (Some(v0), Some(v1), Some(v2)) = (
                self.vertices.get(i0),
                self.vertices.get(i1),
                self.vertices.get(i2),
            ) else {
                continue;
            };
            let edge1 = v1.position - v0.position;
            let edge2 = v2.position - v0.position;
            let delta_uv1 = v1.uv - v0.uv;
            let delta_uv2 = v2.uv - v0.uv;
            // 1 / determinant of the UV-gradient matrix; skip a triangle with no UV
            // area (collinear UVs) — it contributes no direction.
            let determinant = delta_uv1.x * delta_uv2.y - delta_uv2.x * delta_uv1.y;
            if determinant.abs() < 1e-12 {
                continue;
            }
            let inverse = 1.0 / determinant;
            let tangent = (edge1 * delta_uv2.y - edge2 * delta_uv1.y) * inverse;
            let bitangent = (edge2 * delta_uv1.x - edge1 * delta_uv2.x) * inverse;
            for &index in &[i0, i1, i2] {
                tangents[index] += tangent;
                bitangents[index] += bitangent;
            }
        }

        for (index, vertex) in self.vertices.iter_mut().enumerate() {
            let normal = vertex.normal;
            // Gram-Schmidt: project the accumulated tangent off the normal so the
            // stored tangent is exactly perpendicular to it.
            let projected = tangents[index] - normal * normal.dot(tangents[index]);
            let tangent = if projected.length_squared() > 1e-12 {
                projected.normalize()
            } else {
                normal.any_orthonormal_vector()
            };
            // Handedness: which way the bitangent runs relative to N×T (negative for
            // mirrored UVs). The shader reconstructs bitangent = w * cross(N, T).
            let handedness = if normal.cross(tangent).dot(bitangents[index]) < 0.0 {
                -1.0
            } else {
                1.0
            };
            vertex.tangent = tangent.extend(handedness);
        }
    }

    /// Bounds over only the geometry whose owning node is *not* in `hidden_nodes`
    /// — the box for the Outliner's currently-visible meshes. Falls back to the
    /// full [`ModelData::bounds`] when nothing is hidden or the model carries no
    /// per-triangle node info (so visibility can't be resolved). `None` when no
    /// visible geometry remains (every mesh hidden, or an empty model).
    pub fn visible_bounds(&self, hidden_nodes: &[u32]) -> Option<Bounds> {
        let triangle_count = self.indices.len() / 3;
        if hidden_nodes.is_empty() || self.triangles.node.len() != triangle_count {
            return self.bounds;
        }
        let hidden: std::collections::HashSet<u32> = hidden_nodes.iter().copied().collect();
        let mut bounds = Bounds::EMPTY;
        for (triangle_index, triangle) in self.indices.as_chunks::<3>().0.iter().enumerate() {
            if hidden.contains(&self.triangles.node[triangle_index]) {
                continue;
            }
            for &corner in triangle {
                if let Some(vertex) = self.vertices.get(corner as usize) {
                    bounds.include_point(vertex.position);
                }
            }
        }
        (!bounds.is_empty()).then_some(bounds)
    }
}

#[cfg(test)]
mod tests {
    use glam::{Vec2, Vec3, Vec4};

    use super::*;

    /// A minimal model wrapping `vertices` + `indices`, for the normal/tangent
    /// generation tests.
    fn bare_model(vertices: Vec<Vertex>, indices: Vec<u32>) -> ModelData {
        ModelData {
            name: "bare".to_owned(),
            vertices,
            indices,
            ..ModelData::default()
        }
    }

    #[test]
    fn generate_normals_averages_the_faces_meeting_at_a_vertex() {
        // Two triangles forming a 90° fold along the shared edge (0,0,0)-(0,1,0):
        // one in the XY plane (normal +Z), one in the ZY plane (normal +X). The
        // two shared vertices should come out at the 45° bisector.
        let vertex = |pos: Vec3| Vertex {
            position: pos,
            normal: Vec3::ZERO,
            ..Vertex::default()
        };
        let mut model = bare_model(
            vec![
                vertex(Vec3::new(0.0, 0.0, 0.0)),
                vertex(Vec3::new(0.0, 1.0, 0.0)),
                vertex(Vec3::new(1.0, 0.0, 0.0)),
                vertex(Vec3::new(0.0, 0.0, 1.0)),
            ],
            // Wound so the first faces +Z and the second faces +X.
            vec![0, 2, 1, 0, 1, 3],
        );
        model.generate_normals();

        let bisector = Vec3::new(1.0, 0.0, 1.0).normalize();
        for shared in [0, 1] {
            assert!(
                model.vertices[shared].normal.distance(bisector) < 1.0e-5,
                "shared vertex {shared} should bisect the fold, got {:?}",
                model.vertices[shared].normal
            );
        }
        // The corners belonging to only one face keep that face's own normal.
        assert!(model.vertices[2].normal.distance(Vec3::Z) < 1.0e-5);
        assert!(model.vertices[3].normal.distance(Vec3::X) < 1.0e-5);
    }

    #[test]
    fn generate_normals_weights_faces_by_area() {
        // Two coplanar triangles of very different size sharing vertex 0. Both
        // face +Z, so area weighting can't change the direction — what this pins
        // down is that the result stays unit length rather than summing to a
        // long vector or cancelling.
        let vertex = |x: f32, y: f32| Vertex {
            position: Vec3::new(x, y, 0.0),
            normal: Vec3::ZERO,
            ..Vertex::default()
        };
        let mut model = bare_model(
            vec![
                vertex(0.0, 0.0),
                vertex(0.01, 0.0),
                vertex(0.0, 0.01),
                vertex(10.0, 0.0),
                vertex(0.0, 10.0),
            ],
            vec![0, 1, 2, 0, 3, 4],
        );
        model.generate_normals();

        for (index, vertex) in model.vertices.iter().enumerate() {
            assert!(
                (vertex.normal.length() - 1.0).abs() < 1.0e-5,
                "vertex {index} normal is not unit length: {:?}",
                vertex.normal
            );
            assert!(vertex.normal.distance(Vec3::Z) < 1.0e-5);
        }
    }

    #[test]
    fn generate_normals_leaves_an_unreferenced_vertex_alone() {
        // A vertex no triangle mentions accumulates nothing; it must keep the
        // normal it had rather than become zero-length.
        let mut model = bare_model(
            vec![
                Vertex {
                    position: Vec3::ZERO,
                    normal: Vec3::Y,
                    ..Vertex::default()
                };
                4
            ],
            Vec::new(),
        );
        model.vertices[0].position = Vec3::new(1.0, 0.0, 0.0);
        model.vertices[1].position = Vec3::new(0.0, 1.0, 0.0);
        model.indices = vec![0, 1, 2];

        model.generate_normals();
        assert_eq!(
            model.vertices[3].normal,
            Vec3::Y,
            "an unreferenced vertex keeps its normal"
        );
    }

    #[test]
    fn generate_normals_is_a_no_op_without_geometry() {
        let mut model = ModelData::default();
        model.generate_normals();
        assert!(model.vertices.is_empty());
    }

    #[test]
    fn generate_tangents_builds_orthonormal_basis_from_uvs() {
        // A single triangle in the XY plane (normal +Z) with UVs aligned to X/Y:
        // the U direction (tangent) must come out ~ +X, unit length, perpendicular
        // to the normal, with a right-handed (+1) sign.
        let vertex = |pos: Vec3, uv: Vec2| Vertex {
            position: pos,
            normal: Vec3::Z,
            uv,
            tangent: Vec4::ZERO, // degenerate -> the regenerate signal
            vertex_color: Vec4::ONE,
        };
        let mut model = ModelData {
            name: "tri".to_owned(),
            vertices: vec![
                vertex(Vec3::new(0.0, 0.0, 0.0), Vec2::new(0.0, 0.0)),
                vertex(Vec3::new(1.0, 0.0, 0.0), Vec2::new(1.0, 0.0)),
                vertex(Vec3::new(0.0, 1.0, 0.0), Vec2::new(0.0, 1.0)),
            ],
            indices: vec![0, 1, 2],
            faces: vec![TopologyFace {
                first_index: 0,
                index_count: 3,
            }],
            triangles: TriangleData {
                to_face: vec![0],
                material: Vec::new(),
                node: Vec::new(),
            },
            nodes: Vec::new(),
            uv_channels: Vec::new(),
            uv_set_names: Vec::new(),
            bounds: None,
            stats: ModelStats {
                polygon_count: 1,
                triangle_count: 1,
                vertex_count: 3,
                gpu_vertex_count: 3,
                uv_set_count: 1,
                material_count: 0,
                draw_count: 0,
                bone_count: 0,
                clip_count: 0,
                source_unit_meters: 1.0,
            },
            materials: Vec::new(),
            corner_to_logical: Vec::new(),
            skin: None,
            morph: None,
            animations: Vec::new(),
            frame_rate: 0.0,
        };

        assert!(model.has_degenerate_tangents(), "seeded with zero tangents");
        model.generate_tangents();
        assert!(
            !model.has_degenerate_tangents(),
            "tangents are non-degenerate after generation"
        );

        for vertex in &model.vertices {
            let tangent = vertex.tangent.truncate();
            assert!(
                (tangent.length() - 1.0).abs() < 1e-4,
                "tangent is unit length, got {tangent:?}"
            );
            assert!(
                tangent.dot(Vec3::Z).abs() < 1e-4,
                "tangent perpendicular to the normal, got {tangent:?}"
            );
            assert!(
                (tangent - Vec3::X).length() < 1e-3,
                "tangent points along +U (+X) for this layout, got {tangent:?}"
            );
            assert!(
                (vertex.tangent.w - 1.0).abs() < 1e-4,
                "right-handed UV layout yields +1 handedness, got {}",
                vertex.tangent.w
            );
        }
    }

    #[test]
    fn visible_bounds_excludes_hidden_nodes() {
        // Two triangles: node 0 spans x in [0,2], node 1 spans x in [10,12].
        let positions = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(10.0, 0.0, 0.0),
            Vec3::new(11.0, 1.0, 0.0),
            Vec3::new(12.0, 0.0, 0.0),
        ];
        let mut model = ModelData {
            vertices: positions
                .iter()
                .map(|&position| Vertex {
                    position,
                    normal: Vec3::Y,
                    uv: Vec2::ZERO,
                    tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
                    vertex_color: Vec4::ONE,
                })
                .collect(),
            indices: vec![0, 1, 2, 3, 4, 5],
            triangles: TriangleData {
                node: vec![0, 1],
                ..Default::default()
            },
            ..Default::default()
        };
        model.recompute_bounds();

        // Nothing hidden -> the full extent (falls back to `bounds`).
        let all = model.visible_bounds(&[]).unwrap();
        assert_eq!(all.max.x, 12.0);

        // Hide node 1 -> the box stops at node 0's geometry.
        let visible = model.visible_bounds(&[1]).unwrap();
        assert_eq!(visible.min.x, 0.0);
        assert_eq!(visible.max.x, 2.0);

        // Hide every node -> no visible geometry, no box.
        assert!(model.visible_bounds(&[0, 1]).is_none());
    }
}
