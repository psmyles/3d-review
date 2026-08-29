// Host-agnostic data only (invariant 10) — and fully safe (invariant 9): all
// `unsafe`/FFI lives in `import`/`psd` and the sanctioned D3D11 sites.
#![forbid(unsafe_code)]

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

/// What a scene-graph node *is*, as classified by the importer from the source
/// file's node attribute. Drives the Outliner's per-type icons / filter and the
/// skeleton overlay's "which nodes are joints" question. [`Other`] covers both
/// attribute types we don't surface individually and anything a future importer
/// hands us that this enum predates, so an unknown code is never an error.
///
/// [`Other`]: NodeKind::Other
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum NodeKind {
    /// Carries renderable geometry (`mesh_part` is `Some`).
    Mesh,
    /// A skeleton joint — the unit the skeleton overlay draws and skin weights
    /// reference.
    Bone,
    Light,
    Camera,
    /// A transform-only null / group node (the DCC's "empty").
    Empty,
    #[default]
    Other,
}

impl NodeKind {
    /// Every kind, in Outliner filter-row order. Iterated to build the type
    /// filter, so the order is deterministic.
    pub const ALL: [NodeKind; 6] = [
        NodeKind::Mesh,
        NodeKind::Bone,
        NodeKind::Light,
        NodeKind::Camera,
        NodeKind::Empty,
        NodeKind::Other,
    ];

    /// Display label for the Outliner filter tooltip and the Inspector's Type row.
    pub fn label(self) -> &'static str {
        match self {
            NodeKind::Mesh => "Mesh",
            NodeKind::Bone => "Bone",
            NodeKind::Light => "Light",
            NodeKind::Camera => "Camera",
            NodeKind::Empty => "Empty",
            NodeKind::Other => "Other",
        }
    }
}

/// A bone node's display parameters, as authored in the source file. Used to
/// size the skeleton overlay's leaf/root joint markers; `0.0` in either field
/// means the file declared nothing useful and the overlay falls back to a
/// model-extent fraction.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BoneInfo {
    /// The bone's authored radius, in the same (post-import, meters) world units
    /// as the geometry.
    pub radius: f32,
    /// The bone's length as a fraction of its parent's, as authored.
    pub relative_length: f32,
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
    /// What this node is, from the source file's node attribute.
    pub kind: NodeKind,
    /// Authored bone display parameters, `Some` only when `kind` is
    /// [`NodeKind::Bone`].
    pub bone: Option<BoneInfo>,
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
    /// Number of [`NodeKind::Bone`] nodes in the scene graph — a measured count
    /// (invariant 5), shown in the stats panel only when non-zero.
    pub bone_count: usize,
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

/// Skin (skeletal binding) data: which bones move each vertex, and how strongly.
///
/// Stored **per logical source vertex** — the DCC's own vertex count, before the
/// importer expands each face corner into its own render vertex — in a compressed
/// sparse-row layout: vertex `v`'s influences are `bones[offsets[v]..offsets[v+1]]`
/// paired with `weights[..]` over the same range. Per-corner storage would multiply
/// every influence by the 3–6× corner expansion, so instead a single
/// [`corner_to_logical`] map (4 bytes per render vertex) projects the render mesh
/// back onto the logical vertices.
///
/// [`bones`] entries index [`ModelData::nodes`] directly (**not** a separate bone
/// table), so a skin influence and an Outliner row name the same thing with the
/// same number.
///
/// [`corner_to_logical`]: SkinData::corner_to_logical
/// [`bones`]: SkinData::bones
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkinData {
    /// Per render vertex (parallel to [`ModelData::vertices`]), the logical
    /// source vertex it was expanded from — the index into [`SkinData::offsets`].
    pub corner_to_logical: Vec<u32>,
    /// CSR row starts, `logical_vertex_count + 1` long: vertex `v`'s influences
    /// occupy `offsets[v]..offsets[v + 1]`. Monotonic non-decreasing, and the
    /// last entry equals the influence count (a vertex with no influences is a
    /// legal empty row).
    pub offsets: Vec<u32>,
    /// Flat influence bones, indexing [`ModelData::nodes`]. Same length as
    /// [`SkinData::weights`].
    pub bones: Vec<u32>,
    /// Flat influence weights, parallel to [`SkinData::bones`]. Non-negative and
    /// finite, but **not** guaranteed to sum to 1 per vertex — FBX does not
    /// require normalized weights and the importer stores what was authored. Use
    /// [`SkinData::influence_fraction`] for display, which normalizes against the
    /// vertex's own total.
    pub weights: Vec<f32>,
}

impl SkinData {
    /// Number of logical source vertices this skin covers.
    pub fn logical_vertex_count(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    /// Total number of (bone, weight) influences across every vertex.
    pub fn influence_count(&self) -> usize {
        self.bones.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bones.is_empty()
    }

    /// The slice range into [`SkinData::bones`] / [`SkinData::weights`] holding
    /// `logical`'s influences. Empty for an out-of-range vertex, so callers can
    /// index without a bounds check of their own.
    pub fn influence_range(&self, logical: usize) -> std::ops::Range<usize> {
        match (self.offsets.get(logical), self.offsets.get(logical + 1)) {
            (Some(&start), Some(&end)) if end >= start => start as usize..end as usize,
            _ => 0..0,
        }
    }

    /// The raw summed weight `logical` receives from `bones`. `bones` must be
    /// **sorted ascending** (it's binary-searched once per influence); callers get
    /// that from the UI's sorted/deduped selection set. Returns 0.0 for a vertex
    /// with no influence from any of them.
    pub fn summed_weight(&self, logical: usize, bones: &[u32]) -> f32 {
        if bones.is_empty() {
            return 0.0;
        }
        let range = self.influence_range(logical);
        self.bones[range.clone()]
            .iter()
            .zip(&self.weights[range])
            .filter(|(bone, _)| bones.binary_search(bone).is_ok())
            .map(|(_, weight)| *weight)
            .sum()
    }

    /// The share of `logical`'s total influence that `bones` account for, in
    /// `0..=1` — the value the skin-weight heat map paints.
    ///
    /// Normalizing against the vertex's *own* total (rather than assuming 1.0) is
    /// what makes the display honest on rigs whose weights weren't normalized at
    /// export: "this bone owns 40% of this vertex" stays true either way, whereas a
    /// raw sum would read as full influence on a rig whose weights sum to 0.5. For
    /// the normalized common case the two are identical. Returns 0.0 for a vertex
    /// with no influences at all.
    pub fn influence_fraction(&self, logical: usize, bones: &[u32]) -> f32 {
        if bones.is_empty() {
            return 0.0;
        }
        let range = self.influence_range(logical);
        let mut selected = 0.0_f32;
        let mut total = 0.0_f32;
        for (bone, weight) in self.bones[range.clone()].iter().zip(&self.weights[range]) {
            total += *weight;
            if bones.binary_search(bone).is_ok() {
                selected += *weight;
            }
        }
        if total <= 0.0 {
            return 0.0;
        }
        (selected / total).clamp(0.0, 1.0)
    }

    /// The lockstep guard, mirroring [`TriangleData::validate`]: called once at the
    /// import funnel (invariant 7) so a drifted CSR is caught there rather than by
    /// every reader's ad-hoc bounds check. Verifies the corner map covers exactly
    /// the render mesh and stays in range, the row offsets are the right length,
    /// monotonic, and terminate at the influence count, that bones/weights are the
    /// same length, and that every bone indexes a real node with a finite,
    /// non-negative weight. There is deliberately **no** upper bound on a weight:
    /// FBX does not require normalized weights, so rejecting `> 1.0` would refuse
    /// files that every other tool loads.
    pub fn validate(
        &self,
        corner_count: usize,
        logical_count: usize,
        node_count: usize,
    ) -> Result<(), String> {
        if self.corner_to_logical.len() != corner_count {
            return Err(format!(
                "skin corner_to_logical has {} entries, expected {corner_count}",
                self.corner_to_logical.len()
            ));
        }
        if let Some(&logical) = self
            .corner_to_logical
            .iter()
            .find(|&&logical| logical as usize >= logical_count)
        {
            return Err(format!(
                "skin corner_to_logical references source vertex {logical} of {logical_count}"
            ));
        }
        if self.offsets.len() != logical_count + 1 {
            return Err(format!(
                "skin offsets has {} entries, expected {} (logical vertices + 1)",
                self.offsets.len(),
                logical_count + 1
            ));
        }
        if self.bones.len() != self.weights.len() {
            return Err(format!(
                "skin bones has {} entries but weights has {}",
                self.bones.len(),
                self.weights.len()
            ));
        }
        if let Some(window) = self.offsets.windows(2).find(|window| window[0] > window[1]) {
            return Err(format!(
                "skin offsets are not monotonic: {} then {}",
                window[0], window[1]
            ));
        }
        // `offsets` is non-empty here (length is `logical_count + 1`), so the last
        // entry exists and — given monotonicity — is the largest.
        let tail = self.offsets.last().copied().unwrap_or(0) as usize;
        if tail != self.bones.len() {
            return Err(format!(
                "skin offsets end at {tail} but there are {} influences",
                self.bones.len()
            ));
        }
        if let Some(&bone) = self.bones.iter().find(|&&bone| bone as usize >= node_count) {
            return Err(format!("skin bone references node {bone} of {node_count}"));
        }
        if let Some(&weight) = self
            .weights
            .iter()
            .find(|&&weight| !weight.is_finite() || weight < 0.0)
        {
            return Err(format!("skin weight {weight} is negative or not finite"));
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
    /// Skeletal binding data, `Some` only when the source carried a skin
    /// deformer whose clusters resolved to real bone nodes. See [`SkinData`].
    pub skin: Option<SkinData>,
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
            kind: NodeKind::Mesh,
            bone: None,
        }],
        uv_set_names: vec!["UVMap".to_owned()],
        stats: ModelStats {
            polygon_count: 6,
            triangle_count: 12,
            vertex_count: 24,
            uv_set_count: 1,
            material_count: 1,
            draw_count: 1,
            bone_count: 0,
            source_unit_meters: 1.0,
        },
        materials: vec![MaterialImportDefaults {
            name: "Default".to_owned(),
            base_color: Vec3::ONE,
            smoothness: 0.6,
            metallic: 0.0,
            emissive: Vec3::ZERO,
        }],
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
        assert!(
            tris.validate(
                model.stats.triangle_count,
                model.faces.len(),
                model.materials.len(),
                model.nodes.len(),
            )
            .is_ok()
        );
    }

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
                uv_set_count: 1,
                material_count: 0,
                draw_count: 0,
                bone_count: 0,
                source_unit_meters: 1.0,
            },
            materials: Vec::new(),
            skin: None,
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

    // ── SkinData ────────────────────────────────────────────────────────────
    //
    // A 2-logical-vertex skin over a 6-corner (2-triangle) mesh, bound to nodes
    // 1 and 2: vertex 0 is split 0.75/0.25 between them, vertex 1 rides node 2
    // alone.
    fn sample_skin() -> SkinData {
        SkinData {
            corner_to_logical: vec![0, 0, 1, 1, 0, 1],
            offsets: vec![0, 2, 3],
            bones: vec![1, 2, 2],
            weights: vec![0.75, 0.25, 1.0],
        }
    }

    #[test]
    fn skin_validate_accepts_a_consistent_skin() {
        assert_eq!(sample_skin().validate(6, 2, 3), Ok(()));
    }

    #[test]
    fn skin_influence_range_slices_each_vertex() {
        let skin = sample_skin();
        assert_eq!(skin.influence_range(0), 0..2);
        assert_eq!(skin.influence_range(1), 2..3);
        // Out of range reads as "no influences" rather than panicking.
        assert_eq!(skin.influence_range(7), 0..0);
        assert_eq!(skin.logical_vertex_count(), 2);
        assert_eq!(skin.influence_count(), 3);
    }

    #[test]
    fn skin_summed_weight_adds_only_the_selected_bones() {
        let skin = sample_skin();
        // `bones` must be sorted — that is what the UI hands us.
        assert_eq!(skin.summed_weight(0, &[1]), 0.75);
        assert_eq!(skin.summed_weight(0, &[2]), 0.25);
        // Both selected -> the influences sum (the multi-select heat map's core).
        assert_eq!(skin.summed_weight(0, &[1, 2]), 1.0);
        assert_eq!(skin.summed_weight(1, &[1]), 0.0);
        assert_eq!(skin.summed_weight(1, &[1, 2]), 1.0);
        // Nothing selected -> no weight anywhere.
        assert_eq!(skin.summed_weight(0, &[]), 0.0);
    }

    #[test]
    fn skin_validate_catches_a_drifted_corner_map() {
        let mut skin = sample_skin();
        skin.corner_to_logical.pop();
        let error = skin.validate(6, 2, 3).unwrap_err();
        assert!(error.contains("corner_to_logical has 5"), "{error}");
    }

    #[test]
    fn skin_validate_catches_an_out_of_range_corner_map() {
        let mut skin = sample_skin();
        skin.corner_to_logical[3] = 9;
        let error = skin.validate(6, 2, 3).unwrap_err();
        assert!(error.contains("source vertex 9 of 2"), "{error}");
    }

    #[test]
    fn skin_validate_catches_a_drifted_offsets_length() {
        let mut skin = sample_skin();
        skin.offsets.push(3);
        let error = skin.validate(6, 2, 3).unwrap_err();
        assert!(error.contains("offsets has 4"), "{error}");
    }

    #[test]
    fn skin_validate_catches_non_monotonic_offsets() {
        let mut skin = sample_skin();
        skin.offsets = vec![0, 3, 2];
        let error = skin.validate(6, 2, 3).unwrap_err();
        assert!(error.contains("not monotonic"), "{error}");
    }

    #[test]
    fn skin_validate_catches_offsets_that_miss_the_influences() {
        let mut skin = sample_skin();
        skin.offsets = vec![0, 2, 2];
        let error = skin.validate(6, 2, 3).unwrap_err();
        assert!(error.contains("end at 2 but there are 3"), "{error}");
    }

    #[test]
    fn skin_validate_catches_mismatched_bones_and_weights() {
        let mut skin = sample_skin();
        skin.weights.pop();
        let error = skin.validate(6, 2, 3).unwrap_err();
        assert!(
            error.contains("bones has 3 entries but weights has 2"),
            "{error}"
        );
    }

    #[test]
    fn skin_validate_catches_an_out_of_range_bone() {
        let mut skin = sample_skin();
        skin.bones[1] = 7;
        let error = skin.validate(6, 2, 3).unwrap_err();
        assert!(error.contains("bone references node 7 of 3"), "{error}");
    }

    #[test]
    fn skin_validate_catches_a_negative_or_non_finite_weight() {
        for bad in [-0.25_f32, f32::NAN, f32::INFINITY] {
            let mut skin = sample_skin();
            skin.weights[0] = bad;
            let error = skin.validate(6, 2, 3).unwrap_err();
            assert!(error.contains("negative or not finite"), "{bad}: {error}");
        }
    }

    #[test]
    fn skin_validate_accepts_unnormalized_weights() {
        // FBX does not require normalized weights; a weight above 1.0 is sloppy
        // but loadable, and refusing it would reject files other tools open.
        let mut skin = sample_skin();
        skin.weights[0] = 1.75;
        assert_eq!(skin.validate(6, 2, 3), Ok(()));
    }

    #[test]
    fn skin_influence_fraction_normalizes_against_the_vertex_total() {
        let skin = sample_skin();
        // Already normalized (0.75 + 0.25) -> the fraction equals the raw sum.
        assert_eq!(skin.influence_fraction(0, &[1]), 0.75);
        assert_eq!(skin.influence_fraction(0, &[1, 2]), 1.0);

        // An un-normalized vertex: raw weights 1.5 / 0.5 sum to 2.0, so bone 1
        // owns 75% of the vertex even though its raw weight exceeds 1.0.
        let unnormalized = SkinData {
            corner_to_logical: vec![0, 0, 0],
            offsets: vec![0, 2],
            bones: vec![1, 2],
            weights: vec![1.5, 0.5],
        };
        assert_eq!(unnormalized.summed_weight(0, &[1]), 1.5);
        assert_eq!(unnormalized.influence_fraction(0, &[1]), 0.75);
        assert_eq!(unnormalized.influence_fraction(0, &[1, 2]), 1.0);

        // A vertex with no influences at all reads as zero, not NaN.
        let empty = SkinData {
            corner_to_logical: vec![0],
            offsets: vec![0, 0],
            bones: Vec::new(),
            weights: Vec::new(),
        };
        assert_eq!(empty.influence_fraction(0, &[1]), 0.0);
    }

    #[test]
    fn demo_cube_node_is_typed_as_a_mesh() {
        let model = demo_cube_model();
        assert_eq!(model.nodes[0].kind, NodeKind::Mesh);
        assert!(model.nodes[0].bone.is_none());
        assert!(model.skin.is_none());
        assert_eq!(model.stats.bone_count, 0);
    }
}
