use glam::{Mat4, Vec2, Vec3, Vec4};

mod bvh;
pub use bvh::{Bvh, SceneBvh};

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

/// A material's *import-time defaults* — the immutable source values an FBX
/// declares, used only to seed the renderer's editable material table (the
/// load-bearing source-vs-editable split: edits live renderer-side, not here).
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialImportDefaults {
    pub name: String,
    // TODO: `draw_count` is a renderer-computed stat, not an import default — the
    // bridge fills a per-node tally that import then overwrites at the model level
    // (`ModelStats::draw_count`). Nothing reads this per-material copy; drop it
    // from here + the C FFI struct in a future FFI-touching phase.
    pub draw_count: usize,
    /// Import default base color (linear RGB), seeding the editable material
    /// table. White when the source material declared none.
    pub base_color: Vec3,
    /// Import default smoothness in `0.0..=1.0` (glossiness, `1 - roughness`).
    /// `0.5` when the source material declared neither glossiness nor roughness.
    pub smoothness: f32,
    /// Import default metalness in `0.0..=1.0`. `0.0` (dielectric) when the
    /// source material declared none.
    pub metallic: f32,
    /// Import default emissive color (linear RGB, `emission_color` scaled by
    /// `emission_factor`). Black when the source material declared none.
    pub emissive: Vec3,
}

/// One node in the imported scene-graph hierarchy (every FBX node, mesh-bearing
/// or not), carried through for the Outliner. The transform is display metadata
/// only — geometry is world-baked at import (invariant 1).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneNode {
    pub name: String,
    /// Index into [`ModelData::nodes`] of this node's parent, or `None` for the
    /// root (and any node the importer left parentless).
    pub parent: Option<usize>,
    /// Running index among mesh-bearing nodes (in import traversal order), or
    /// `None` when this node carries no renderable mesh.
    pub mesh_part: Option<usize>,
    /// `node_to_world` transform. Display metadata only.
    pub transform: Mat4,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelWarning {
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TopologyFace {
    pub first_index: u32,
    pub index_count: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ModelStats {
    pub polygon_count: usize,
    pub triangle_count: usize,
    pub vertex_count: usize,
    pub uv_set_count: usize,
    pub material_count: usize,
    pub draw_count: usize,
    /// The model's authored world unit, in meters per source unit, as recorded
    /// in the file (e.g. `0.01` for a centimeter file like a Maya export). This
    /// is the *original* unit before import normalizes everything to meters, so
    /// the stats panel can show what the file claimed. `0.0` means the source
    /// declared no unit (e.g. the built-in demo, or a file missing the metadata).
    pub source_unit_meters: f32,
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

    /// Each present (non-empty) array must be exactly `triangle_count` long; an
    /// empty array means "this model carries no such per-triangle info" and is
    /// allowed. Returns `Err` describing the first array that drifted.
    pub fn validate(&self, triangle_count: usize) -> Result<(), String> {
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
        Ok(())
    }
}

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
    pub warnings: Vec<ModelWarning>,
}

impl ModelData {
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

    /// Number of per-material draw ranges the renderer issues for this mesh —
    /// one per distinct [`TriangleData::material`] slot, in first-seen order.
    /// Mirrors the grouping in the renderer's `model_mesh`, so the Draws stat
    /// reflects the real draw-call count (invariant 5). `0` when the mesh has no
    /// triangles; `1` when triangles exist but carry no per-triangle material.
    pub fn material_draw_count(&self) -> usize {
        let triangle_count = self.indices.len() / 3;
        if triangle_count == 0 {
            return 0;
        }
        if self.triangles.material.len() != triangle_count {
            return 1;
        }
        let mut seen: Vec<u32> = Vec::new();
        for &slot in &self.triangles.material {
            if !seen.contains(&slot) {
                seen.push(slot);
            }
        }
        seen.len()
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
        for triangle in self.indices.chunks_exact(3) {
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
        for (triangle_index, triangle) in self.indices.chunks_exact(3).enumerate() {
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

pub fn demo_cube_model() -> ModelData {
    let mut model = ModelData {
        name: "Demo Cube".to_owned(),
        vertices: demo_cube_vertices(),
        indices: demo_cube_indices(),
        faces: (0..6)
            .map(|face_index| TopologyFace {
                first_index: face_index * 4,
                index_count: 4,
            })
            .collect(),
        triangles: TriangleData {
            to_face: vec![0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5],
            material: vec![0; 12],
            // The demo cube is a single node, so every triangle belongs to node 0.
            node: vec![0; 12],
        },
        nodes: vec![SceneNode {
            name: "Demo Cube".to_owned(),
            parent: None,
            mesh_part: Some(0),
            transform: Mat4::IDENTITY,
        }],
        uv_set_names: vec!["UVMap".to_owned()],
        stats: ModelStats {
            polygon_count: 6,
            triangle_count: 12,
            vertex_count: 24,
            uv_set_count: 1,
            material_count: 1,
            draw_count: 1,
            source_unit_meters: 1.0,
        },
        materials: vec![MaterialImportDefaults {
            name: "Default".to_owned(),
            draw_count: 1,
            base_color: Vec3::ONE,
            smoothness: 0.6,
            metallic: 0.0,
            emissive: Vec3::ZERO,
        }],
        warnings: Vec::new(),
        ..Default::default()
    };
    model.recompute_bounds();
    model
}

fn demo_cube_vertices() -> Vec<Vertex> {
    let s = 0.5;
    let y0 = 0.03;
    let y1 = 1.03;

    let faces = [
        ([[-s, y0, s], [s, y0, s], [s, y1, s], [-s, y1, s]], Vec3::Z),
        (
            [[s, y0, -s], [-s, y0, -s], [-s, y1, -s], [s, y1, -s]],
            Vec3::NEG_Z,
        ),
        (
            [[-s, y0, -s], [-s, y0, s], [-s, y1, s], [-s, y1, -s]],
            Vec3::NEG_X,
        ),
        ([[s, y0, s], [s, y0, -s], [s, y1, -s], [s, y1, s]], Vec3::X),
        (
            [[-s, y1, s], [s, y1, s], [s, y1, -s], [-s, y1, -s]],
            Vec3::Y,
        ),
        (
            [[-s, y0, -s], [s, y0, -s], [s, y0, s], [-s, y0, s]],
            Vec3::NEG_Y,
        ),
    ];

    let uvs = [
        Vec2::new(0.0, 0.0),
        Vec2::new(1.0, 0.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(0.0, 1.0),
    ];

    // Distinct per-corner vertex colors so the vertex-color debug view has
    // something to show on the built-in demo: an R/G/B/yellow ring with a
    // 0.25→1.0 alpha ramp to exercise the alpha / RGB+A modes too.
    let vertex_colors = [
        Vec4::new(1.0, 0.0, 0.0, 0.25),
        Vec4::new(0.0, 1.0, 0.0, 0.50),
        Vec4::new(0.0, 0.0, 1.0, 0.75),
        Vec4::new(1.0, 1.0, 0.0, 1.00),
    ];

    let mut vertices = Vec::with_capacity(24);
    for (positions, normal) in faces {
        for (index, position) in positions.into_iter().enumerate() {
            vertices.push(Vertex {
                position: Vec3::from_array(position),
                normal,
                uv: uvs[index],
                tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
                vertex_color: vertex_colors[index],
            });
        }
    }

    vertices
}

fn demo_cube_indices() -> Vec<u32> {
    let mut indices = Vec::with_capacity(36);
    for face_index in 0..6 {
        let base = face_index * 4;
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    indices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_cube_carries_nodes_and_per_triangle_material() {
        let model = demo_cube_model();

        let tris = &model.triangles;
        assert!(!model.nodes.is_empty());
        assert_eq!(tris.material.len(), model.stats.triangle_count);
        assert_eq!(tris.material.len(), tris.to_face.len());
        assert!(tris.material.iter().all(|&slot| slot == 0));
        // Per-triangle node index runs parallel and points at the single node.
        assert_eq!(tris.node.len(), model.stats.triangle_count);
        assert!(tris.node.iter().all(|&node| node == 0));

        // The parallel per-triangle arrays stay mutually in lockstep, and every
        // index they carry points at a real face / material / node.
        assert!(tris.validate(model.stats.triangle_count).is_ok());
        for &face in &tris.to_face {
            assert!(
                (face as usize) < model.faces.len(),
                "tri_to_face out of range"
            );
        }
        for &slot in &tris.material {
            assert!(
                (slot as usize) < model.materials.len(),
                "tri_material out of range"
            );
        }
        for &node in &tris.node {
            assert!((node as usize) < model.nodes.len(), "tri_node out of range");
        }
    }

    #[test]
    fn triangle_data_validate_catches_drift() {
        // Equal-length arrays pass.
        let ok = TriangleData {
            to_face: vec![0, 0, 1],
            material: vec![0, 0, 0],
            node: vec![0, 0, 0],
        };
        assert!(ok.validate(3).is_ok());

        // An empty array means "no such info" and is allowed.
        let no_nodes = TriangleData {
            to_face: vec![0, 0, 1],
            material: vec![0, 0, 0],
            node: Vec::new(),
        };
        assert!(no_nodes.validate(3).is_ok());

        // A present-but-short array is a drift and is rejected with a clear message.
        let drifted = TriangleData {
            to_face: vec![0, 0, 1],
            material: vec![0, 0],
            node: vec![0, 0, 0],
        };
        let err = drifted.validate(3).unwrap_err();
        assert!(err.contains("material"), "message names the drifted array");

        // All-empty (a model with no per-triangle info) passes for any count.
        assert!(TriangleData::default().validate(0).is_ok());
    }

    #[test]
    fn material_draw_count_counts_distinct_slots() {
        let mut model = demo_cube_model();
        // Single material across every triangle -> one draw.
        assert_eq!(model.material_draw_count(), 1);

        // Three distinct slots -> three draws, regardless of ordering/repeats.
        model.triangles.material = vec![0, 0, 1, 1, 2, 2, 0, 1, 2, 2, 1, 0];
        assert_eq!(model.material_draw_count(), 3);

        // No triangles -> no draws.
        model.indices.clear();
        assert_eq!(model.material_draw_count(), 0);
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
                uv_set_count: 1,
                material_count: 0,
                draw_count: 0,
                source_unit_meters: 1.0,
            },
            materials: Vec::new(),
            warnings: Vec::new(),
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
