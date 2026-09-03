// Host-agnostic data only (invariant 10) — and fully safe (invariant 9): all
// `unsafe`/FFI lives in `import`/`psd` and the sanctioned D3D11 sites.
#![forbid(unsafe_code)]

use glam::{Mat4, Quat, Vec2, Vec3, Vec4};

pub mod anim;
mod bvh;
pub use anim::{AnimContext, DeformPose, Pose};
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

/// A node's transform relative to its parent, as translation / rotation / scale
/// — the form animation keys replace channel by channel. The importer folds the
/// FBX pivots and rotation order in, so `parent_world * to_mat4()` reproduces the
/// node's `node_to_world` exactly (it loads with ufbx's helper-node inherit-mode
/// handling, which makes every node a plain parent × local product).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalTransform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl LocalTransform {
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    pub fn to_mat4(self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

impl Default for LocalTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// One node in the imported scene-graph hierarchy (every FBX node, mesh-bearing
/// or not), carried through for the Outliner. The transform is display metadata
/// only — geometry is world-baked at import (invariant 1); animation re-poses it
/// through the GPU deform palette (see [`anim`]), never by rewriting vertices.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneNode {
    pub name: String,
    /// Index into [`ModelData::nodes`] of this node's parent, or `None` for the
    /// root (and any node the importer left parentless).
    pub parent: Option<usize>,
    /// Running index among mesh-bearing nodes (in import traversal order), or
    /// `None` when this node carries no renderable mesh.
    pub mesh_part: Option<usize>,
    /// This node's own mesh's logical (DCC control-point) vertex count, as the
    /// source file authored it — the per-node share of
    /// [`ModelStats::vertex_count`], which is exactly their sum. `0` for a node
    /// carrying no mesh, and for a model whose producer doesn't track it (the
    /// demo cube, the Opt workspace's rebuilt meshes), in which case a scoped
    /// Verts stat reports nothing rather than a wrong number.
    pub source_vertex_count: usize,
    /// `node_to_world` transform at the file's default pose. Display metadata
    /// only — and the *rest* world every animated pose is a delta from.
    pub transform: Mat4,
    /// The node's rest transform relative to its parent — what an animation
    /// clip's keys override channel by channel. Identity for a procedurally-built
    /// model.
    pub rest_local: LocalTransform,
    /// What this node is, from the source file's node attribute.
    pub kind: NodeKind,
    /// Authored bone display parameters, `Some` only when `kind` is
    /// [`NodeKind::Bone`].
    pub bone: Option<BoneInfo>,
}

impl Default for SceneNode {
    fn default() -> Self {
        Self {
            name: String::new(),
            parent: None,
            mesh_part: None,
            source_vertex_count: 0,
            transform: Mat4::IDENTITY,
            rest_local: LocalTransform::IDENTITY,
            kind: NodeKind::Other,
            bone: None,
        }
    }
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
    /// The source file's own logical vertex count (control points), as the
    /// artist's DCC reports it.
    pub vertex_count: usize,
    /// What the mesh costs on the GPU: unique vertices per draw group, as
    /// measured by [`ModelData::count_gpu_vertices`] — the buffer any engine
    /// importer's lossless indexing would build. Neither the DCC count above nor
    /// this viewer's internal corner-split buffer, which is an implementation
    /// detail and reported nowhere.
    pub gpu_vertex_count: usize,
    pub uv_set_count: usize,
    pub material_count: usize,
    pub draw_count: usize,
    /// Number of [`NodeKind::Bone`] nodes in the scene graph — a measured count
    /// (invariant 5), shown in the stats panel only when non-zero.
    pub bone_count: usize,
    /// Number of animation clips (FBX animation stacks) the file carries — a
    /// measured count (invariant 5), shown in the stats panel only when non-zero.
    pub clip_count: usize,
    /// The model's authored world unit, in meters per source unit, as recorded
    /// in the file (e.g. `0.01` for a centimeter file like a Maya export). This
    /// is the *original* unit before import normalizes everything to meters, so
    /// the stats panel can show what the file claimed. `0.0` means the source
    /// declared no unit (e.g. the built-in demo, or a file missing the metadata).
    pub source_unit_meters: f32,
}

/// Measured counts for one draw group — the triangles sharing a
/// (scene node, material slot) pair, which is the finest unit the renderer
/// actually draws and the unit every scope the stats overlay reports is a union
/// of.
///
/// Built once per model by [`ModelData::mesh_group_stats`] so the overlay's
/// per-scope columns are sums of measured values, not a per-frame rescan: the
/// GPU-vertex measurement alone is an O(corners) hash walk that must never run
/// on the redraw path (invariant 6).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MeshGroupStats {
    /// Index into [`ModelData::nodes`] (`0` for a model with no node tags).
    pub node: u32,
    /// Index into [`ModelData::materials`], or `u32::MAX` for untagged triangles
    /// — the same sentinel [`TriangleData::material`] uses.
    pub material: u32,
    /// Source polygons whose triangles land in this group.
    pub polygon_count: usize,
    pub triangle_count: usize,
    /// This group's share of [`ModelStats::gpu_vertex_count`]. Groups never share
    /// a vertex (the count keys on the group), so the shares sum to the total.
    pub gpu_vertex_count: usize,
}

/// Which slice of the model a [`ScopeStats`] covers.
///
/// There is deliberately no "whole model" variant: the file's own figures are
/// what [`ModelStats`] already carries, straight from import, and re-deriving
/// them here would be a second answer to a question that has one (invariant 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatsScope<'a> {
    /// The nodes marked in a per-node inclusion mask (indexed by node index; a
    /// mask shorter than the node table excludes the rest).
    Nodes(&'a [bool]),
    /// Every triangle of one material slot, whichever node carries it.
    Material(u32),
}

/// A scope's measured totals, summed from the [`MeshGroupStats`] table.
///
/// Every field is a real measured count (invariant 5) — the sums of counts taken
/// off the mesh itself, never a proportional share of a whole-model figure.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScopeStats {
    pub polygon_count: usize,
    pub triangle_count: usize,
    /// The DCC control-point count, `None` when the scope has no honest one: a
    /// material slot is not a set of meshes, and the count is authored per mesh
    /// (it is likewise `None` for a model whose nodes carry no
    /// [`SceneNode::source_vertex_count`]).
    pub vertex_count: Option<usize>,
    pub gpu_vertex_count: usize,
    /// Distinct material slots in the scope — one draw call each, matching
    /// [`ModelData::material_draw_count`]'s grouping.
    pub draw_count: usize,
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
/// every influence by the 3–6× corner expansion, so instead the model-wide
/// [`ModelData::corner_to_logical`] map (4 bytes per render vertex) projects the
/// render mesh back onto the logical vertices.
///
/// [`bones`] entries index [`ModelData::nodes`] directly (**not** a separate bone
/// table), so a skin influence and an Outliner row name the same thing with the
/// same number. The parallel [`influence_cluster`] names the *cluster* — the
/// (mesh node, bone) binding whose [`SkinCluster::world_to_bone_bind`] the GPU
/// skinning palette multiplies by — since one bone can bind several meshes with
/// different bind matrices.
///
/// [`bones`]: SkinData::bones
/// [`influence_cluster`]: SkinData::influence_cluster
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkinData {
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
    /// Per influence (parallel to [`SkinData::bones`]), the index into
    /// [`SkinData::clusters`] of the binding it belongs to. Always satisfies
    /// `clusters[influence_cluster[i]].bone == bones[i]`.
    pub influence_cluster: Vec<u32>,
    /// Every (mesh node, bone) binding the skin deformers declare, with the bind
    /// matrix the palette needs. One entry per cluster per mesh-bearing *node*
    /// (an instanced mesh under two nodes gets two, since their
    /// `geometry_to_world` differ).
    pub clusters: Vec<SkinCluster>,
    /// One entry per skinned mesh node: the skinning method the file declared
    /// and its weight cap — display metadata for the Inspector.
    pub deformers: Vec<SkinDeformerInfo>,
}

/// One skin cluster: the binding of a bone to a mesh node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkinCluster {
    /// The bone, indexing [`ModelData::nodes`].
    pub bone: u32,
    /// The skinned mesh node this cluster deforms, indexing [`ModelData::nodes`].
    pub mesh_node: u32,
    /// Maps a *baked world-space* vertex of `mesh_node` into the bone's bind
    /// space: `geometry_to_bone × inverse(mesh.geometry_to_world)`. The palette
    /// entry for this cluster at any pose is `bone_world(pose) × this`, and the
    /// skinned position is the weighted sum of those applied to the baked vertex
    /// — exact for any pose, including the file's own default one.
    pub world_to_bone_bind: Mat4,
}

/// The skinning method an FBX skin deformer declares. The viewer always
/// evaluates linear blend skinning; the other variants are surfaced so the
/// Inspector can say what the file asked for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SkinningMethod {
    #[default]
    Linear,
    Rigid,
    DualQuaternion,
    BlendedDqLinear,
}

impl SkinningMethod {
    pub fn label(self) -> &'static str {
        match self {
            SkinningMethod::Linear => "Linear",
            SkinningMethod::Rigid => "Rigid",
            SkinningMethod::DualQuaternion => "Dual Quaternion (shown as linear)",
            SkinningMethod::BlendedDqLinear => "Blended DQ / linear (shown as linear)",
        }
    }
}

/// What one skinned mesh node's deformer declared — Inspector metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkinDeformerInfo {
    /// The skinned mesh node, indexing [`ModelData::nodes`].
    pub mesh_node: u32,
    pub method: SkinningMethod,
    /// The largest number of influences any vertex of this mesh carries, as the
    /// file reports it.
    pub max_weights_per_vertex: u32,
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
    /// every reader's ad-hoc bounds check. Verifies the row offsets are the right
    /// length, monotonic, and terminate at the influence count, that
    /// bones/weights/clusters are the same length, that every bone indexes a real
    /// node with a finite, non-negative weight, and that every cluster reference
    /// resolves to a cluster naming that same bone with a finite bind matrix.
    /// There is deliberately **no** upper bound on a weight: FBX does not require
    /// normalized weights, so rejecting `> 1.0` would refuse files that every other
    /// tool loads. (The corner map is validated by [`ModelData::validate_deform`],
    /// since blend shapes share it.)
    pub fn validate(&self, logical_count: usize, node_count: usize) -> Result<(), String> {
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
        if self.influence_cluster.len() != self.bones.len() {
            return Err(format!(
                "skin influence_cluster has {} entries but bones has {}",
                self.influence_cluster.len(),
                self.bones.len()
            ));
        }
        for (index, (&cluster, &bone)) in self.influence_cluster.iter().zip(&self.bones).enumerate()
        {
            let Some(entry) = self.clusters.get(cluster as usize) else {
                return Err(format!(
                    "skin influence {index} references cluster {cluster} of {}",
                    self.clusters.len()
                ));
            };
            if entry.bone != bone {
                return Err(format!(
                    "skin influence {index} names bone {bone} but its cluster {cluster} binds bone {}",
                    entry.bone
                ));
            }
        }
        for (index, cluster) in self.clusters.iter().enumerate() {
            if cluster.bone as usize >= node_count || cluster.mesh_node as usize >= node_count {
                return Err(format!(
                    "skin cluster {index} references node {} / {} of {node_count}",
                    cluster.bone, cluster.mesh_node
                ));
            }
            if !cluster.world_to_bone_bind.is_finite() {
                return Err(format!("skin cluster {index} has a non-finite bind matrix"));
            }
        }
        for (index, deformer) in self.deformers.iter().enumerate() {
            if deformer.mesh_node as usize >= node_count {
                return Err(format!(
                    "skin deformer {index} references node {} of {node_count}",
                    deformer.mesh_node
                ));
            }
        }
        Ok(())
    }
}

/// Blend-shape (morph target) data, stored **per logical source vertex** as a
/// compressed sparse-row table like [`SkinData`]: logical vertex `v`'s shape
/// offsets are `shape[offsets[v]..offsets[v+1]]` paired with `position[..]` /
/// `normal[..]`. Offsets are already rotated into the baked world orientation of
/// their mesh node (the importer applies the mesh's `geometry_to_world` linear
/// part), so adding `weight × offset` to a baked vertex *before* skinning is
/// exact — skinning is linear.
///
/// A channel is the artist-facing slider; it blends between its keyframes'
/// shapes by the ufbx in-between rule (see [`anim::channel_effective_weights`]).
/// In the common case a channel has exactly one keyframe at target weight 1.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MorphData {
    pub channels: Vec<MorphChannel>,
    pub shapes: Vec<MorphShape>,
    /// CSR row starts over the logical vertices, `logical_vertex_count + 1` long.
    pub offsets: Vec<u32>,
    /// Flat per-entry shape index (into [`MorphData::shapes`]), parallel to
    /// [`MorphData::position`] / [`MorphData::normal`].
    pub shape: Vec<u32>,
    /// Flat per-entry position offsets, baked-world oriented.
    pub position: Vec<Vec3>,
    /// Flat per-entry normal offsets (zero when the file declared none).
    pub normal: Vec<Vec3>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MorphChannel {
    pub name: String,
    /// The mesh node this channel deforms, indexing [`ModelData::nodes`].
    pub mesh_node: u32,
    /// The channel's weight at the file's default pose, in `0..=1`.
    pub rest_weight: f32,
    /// The channel's targets in ascending `target_weight` order.
    pub keyframes: Vec<MorphKeyframe>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MorphKeyframe {
    /// Index into [`MorphData::shapes`].
    pub shape: u32,
    /// The channel weight at which this shape applies at full strength.
    pub target_weight: f32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MorphShape {
    pub name: String,
}

impl MorphData {
    pub fn logical_vertex_count(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    /// The slice range into the flat entry arrays holding `logical`'s offsets.
    /// Empty for an out-of-range vertex.
    pub fn entry_range(&self, logical: usize) -> std::ops::Range<usize> {
        match (self.offsets.get(logical), self.offsets.get(logical + 1)) {
            (Some(&start), Some(&end)) if end >= start => start as usize..end as usize,
            _ => 0..0,
        }
    }

    /// The import-funnel guard, like [`SkinData::validate`].
    pub fn validate(&self, logical_count: usize, node_count: usize) -> Result<(), String> {
        if self.offsets.len() != logical_count + 1 {
            return Err(format!(
                "morph offsets has {} entries, expected {} (logical vertices + 1)",
                self.offsets.len(),
                logical_count + 1
            ));
        }
        if let Some(window) = self.offsets.windows(2).find(|window| window[0] > window[1]) {
            return Err(format!(
                "morph offsets are not monotonic: {} then {}",
                window[0], window[1]
            ));
        }
        let tail = self.offsets.last().copied().unwrap_or(0) as usize;
        if tail != self.shape.len() {
            return Err(format!(
                "morph offsets end at {tail} but there are {} entries",
                self.shape.len()
            ));
        }
        if self.position.len() != self.shape.len() || self.normal.len() != self.shape.len() {
            return Err(format!(
                "morph entry arrays disagree: {} shapes, {} positions, {} normals",
                self.shape.len(),
                self.position.len(),
                self.normal.len()
            ));
        }
        if let Some(&shape) = self
            .shape
            .iter()
            .find(|&&shape| shape as usize >= self.shapes.len())
        {
            return Err(format!(
                "morph entry references shape {shape} of {}",
                self.shapes.len()
            ));
        }
        if self
            .position
            .iter()
            .chain(&self.normal)
            .any(|offset| !offset.is_finite())
        {
            return Err("morph offset is not finite".to_owned());
        }
        for (index, channel) in self.channels.iter().enumerate() {
            if channel.mesh_node as usize >= node_count {
                return Err(format!(
                    "morph channel {index} references node {} of {node_count}",
                    channel.mesh_node
                ));
            }
            if !channel.rest_weight.is_finite() {
                return Err(format!(
                    "morph channel {index} has a non-finite rest weight"
                ));
            }
            for key in &channel.keyframes {
                if key.shape as usize >= self.shapes.len() {
                    return Err(format!(
                        "morph channel {index} references shape {} of {}",
                        key.shape,
                        self.shapes.len()
                    ));
                }
                if !key.target_weight.is_finite() {
                    return Err(format!(
                        "morph channel {index} has a non-finite target weight"
                    ));
                }
            }
        }
        Ok(())
    }
}

/// One keyframe of a baked animation channel: linearly (or, for rotations,
/// spherically) interpolated to the next.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Key<T> {
    /// Absolute time in seconds on the clip's own timeline.
    pub time: f64,
    pub value: T,
}

/// The baked transform animation of one node within a clip. A channel with no
/// keys is not animated by the clip and keeps the node's [`SceneNode::rest_local`]
/// value; a channel with keys holds its first/last value outside the key range.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodeTrack {
    /// Indexes [`ModelData::nodes`].
    pub node: u32,
    pub translation: Vec<Key<Vec3>>,
    pub rotation: Vec<Key<Quat>>,
    pub scale: Vec<Key<Vec3>>,
}

/// The baked weight animation of one blend-shape channel within a clip.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MorphTrack {
    /// Indexes [`MorphData::channels`].
    pub channel: u32,
    /// Channel weight in `0..=1` (the file's percent ÷ 100).
    pub keys: Vec<Key<f32>>,
}

/// One animation clip — an FBX animation stack, baked to keyframes at import.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnimationClip {
    pub name: String,
    /// The stack's playback range, in seconds. Frame numbering and the transport
    /// derive from this range and the file's frame rate, never from key counts:
    /// baked keys may extend past it (stepped keys, exporter padding) and are
    /// simply held there.
    pub time_begin: f64,
    pub time_end: f64,
    pub tracks: Vec<NodeTrack>,
    pub morph_tracks: Vec<MorphTrack>,
    /// The union of the posed mesh's bounds over every frame of the clip,
    /// measured once at import — what framing and the bounding-box overlay use
    /// while the clip is selected. `None` when the model has no geometry.
    pub bounds: Option<Bounds>,
}

impl AnimationClip {
    pub fn duration(&self) -> f64 {
        (self.time_end - self.time_begin).max(0.0)
    }

    /// The number of frames the transport steps through at `fps`: the range
    /// rounded to whole frames, plus the frame at `time_begin` itself.
    pub fn frame_count(&self, fps: f64) -> usize {
        let fps = frame_rate_or_default(fps);
        (self.duration() * fps).round().max(0.0) as usize + 1
    }

    /// The time of frame `frame` (0-based) at `fps`, clamped into the clip.
    pub fn frame_time(&self, frame: usize, fps: f64) -> f64 {
        let fps = frame_rate_or_default(fps);
        self.clamp_time(self.time_begin + frame as f64 / fps)
    }

    /// The frame `time` falls on at `fps` (nearest, 0-based, within the clip).
    pub fn frame_at(&self, time: f64, fps: f64) -> usize {
        let fps = frame_rate_or_default(fps);
        let frame = ((time - self.time_begin) * fps).round().max(0.0) as usize;
        frame.min(self.frame_count(fps) - 1)
    }

    /// The time `delta` whole frames from the frame `time` falls on, clamped
    /// into the clip — one step of the transport's frame buttons / `,` `.` keys.
    pub fn step_frame_time(&self, time: f64, delta: i64, fps: f64) -> f64 {
        let frame = self.frame_at(time, fps) as i64 + delta;
        let last = self.frame_count(fps) as i64 - 1;
        self.frame_time(frame.clamp(0, last) as usize, fps)
    }

    /// `time` clamped into the clip's range.
    pub fn clamp_time(&self, time: f64) -> f64 {
        time.clamp(self.time_begin, self.time_end.max(self.time_begin))
    }

    /// `time` wrapped into the clip's range (looping playback). A zero-length
    /// clip always reads as its start.
    pub fn wrap_time(&self, time: f64) -> f64 {
        let duration = self.duration();
        if duration <= 0.0 {
            return self.time_begin;
        }
        self.time_begin + (time - self.time_begin).rem_euclid(duration)
    }

    /// The import-funnel guard: every track names a real node / channel, and
    /// every key is finite with non-decreasing times.
    pub fn validate(&self, node_count: usize, channel_count: usize) -> Result<(), String> {
        if !self.time_begin.is_finite()
            || !self.time_end.is_finite()
            || self.time_end < self.time_begin
        {
            return Err(format!(
                "clip '{}' has an invalid time range {}..{}",
                self.name, self.time_begin, self.time_end
            ));
        }
        fn check_times<T>(keys: &[Key<T>], what: &str, clip: &str) -> Result<(), String> {
            if keys.iter().any(|key| !key.time.is_finite()) {
                return Err(format!("clip '{clip}' has a non-finite {what} key time"));
            }
            if keys.windows(2).any(|pair| pair[1].time < pair[0].time) {
                return Err(format!("clip '{clip}' has {what} keys out of order"));
            }
            Ok(())
        }
        for track in &self.tracks {
            if track.node as usize >= node_count {
                return Err(format!(
                    "clip '{}' animates node {} of {node_count}",
                    self.name, track.node
                ));
            }
            check_times(&track.translation, "translation", &self.name)?;
            check_times(&track.rotation, "rotation", &self.name)?;
            check_times(&track.scale, "scale", &self.name)?;
            if track.translation.iter().any(|key| !key.value.is_finite())
                || track.scale.iter().any(|key| !key.value.is_finite())
                || track.rotation.iter().any(|key| !key.value.is_finite())
            {
                return Err(format!(
                    "clip '{}' has a non-finite transform key",
                    self.name
                ));
            }
        }
        for track in &self.morph_tracks {
            if track.channel as usize >= channel_count {
                return Err(format!(
                    "clip '{}' animates morph channel {} of {channel_count}",
                    self.name, track.channel
                ));
            }
            check_times(&track.keys, "morph", &self.name)?;
            if track.keys.iter().any(|key| !key.value.is_finite()) {
                return Err(format!("clip '{}' has a non-finite morph key", self.name));
            }
        }
        Ok(())
    }
}

/// The frame rate assumed when a file declares none.
pub const DEFAULT_FRAME_RATE: f64 = 30.0;

/// `fps` when it is a usable rate, else [`DEFAULT_FRAME_RATE`].
pub fn frame_rate_or_default(fps: f64) -> f64 {
    if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        DEFAULT_FRAME_RATE
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
    /// The file's frame rate, or [`DEFAULT_FRAME_RATE`] when it declared none.
    pub fn frame_rate_or_default(&self) -> f64 {
        frame_rate_or_default(self.frame_rate)
    }

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

    /// The number of vertices an engine's GPU vertex buffer would hold for this
    /// mesh: unique attribute tuples per draw group, counted over the referenced
    /// vertices.
    ///
    /// Import expands every face corner into its own vertex (FBX indexes
    /// normals/UVs per corner, and the polygon-topology views need the corner-run
    /// layout), so `vertices.len()` describes this viewer's internal buffer, not
    /// the asset. What the asset actually *costs* is what any engine importer's
    /// lossless indexing produces: one vertex per distinct
    /// (position, normal, UVs, color) tuple, per (node, material) draw group —
    /// duplicates across groups stay separate, exactly as separate draws keep
    /// separate buffers. This is the figure the stats panel reports as the GPU
    /// vertex count, and it matches the Opt workspace's post-index baseline.
    ///
    /// Equality is bit-exact after folding `-0.0` to `+0.0` and every NaN to one
    /// pattern — the same canonical form the Opt weld uses, so the two counts
    /// can never disagree. The tangent is deliberately excluded: it is derived
    /// from position/normal/UV, so identical inputs carry identical tangents and
    /// including it would only let floating-point noise split a vertex.
    pub fn count_gpu_vertices(&self) -> usize {
        self.mesh_group_stats()
            .iter()
            .map(|group| group.gpu_vertex_count)
            .sum()
    }

    /// Measure every draw group in the mesh: one [`MeshGroupStats`] per
    /// (node, material) pair the triangles carry, in first-seen order.
    ///
    /// This is the one walk behind every count the stats overlay reports, whole
    /// model or scoped — [`count_gpu_vertices`] is its total. Run it once per
    /// model and sum the table with [`scope_stats`]: it is O(corners) with a hash
    /// per distinct vertex, far too heavy for the redraw path (invariant 6).
    ///
    /// Untagged models fold into a single group (node 0, material 0), matching
    /// the fallbacks [`material_draw_count`] and the renderer already take.
    ///
    /// [`count_gpu_vertices`]: ModelData::count_gpu_vertices
    /// [`scope_stats`]: ModelData::scope_stats
    /// [`material_draw_count`]: ModelData::material_draw_count
    pub fn mesh_group_stats(&self) -> Vec<MeshGroupStats> {
        use std::collections::{HashMap, HashSet};

        let triangle_count = self.indices.len() / 3;
        if triangle_count == 0 || self.vertices.is_empty() {
            return Vec::new();
        }
        let node_tags =
            (self.triangles.node.len() == triangle_count).then_some(self.triangles.node.as_slice());
        let material_tags = (self.triangles.material.len() == triangle_count)
            .then_some(self.triangles.material.as_slice());
        let face_tags = (self.triangles.to_face.len() == triangle_count)
            .then_some(self.triangles.to_face.as_slice());

        // Fold a component to the canonical bit pattern (`-0.0` -> `+0.0`, any
        // NaN -> the one `f32::NAN`), so equality is by value, not encoding.
        let canonical = |value: f32| -> [u8; 4] {
            let folded = if value.is_nan() {
                f32::NAN
            } else if value == 0.0 {
                0.0
            } else {
                value
            };
            folded.to_ne_bytes()
        };

        let mut groups: Vec<MeshGroupStats> = Vec::new();
        let mut group_of: HashMap<(u32, u32), usize> = HashMap::new();
        // A vertex index is keyed at most once per group; a group's set holds
        // the distinct attribute tuples among them.
        let mut seen: HashSet<(u32, u32, u32)> = HashSet::new();
        let mut unique: HashSet<(u32, u32, Vec<u8>)> = HashSet::new();
        // A source polygon's triangles all carry its own node and material, so a
        // face falls in exactly one group; this mask is what keeps a triangulated
        // n-gon from counting its polygon once per triangle.
        let mut counted_face = vec![false; self.faces.len()];

        for (triangle, corners) in self.indices.chunks_exact(3).enumerate() {
            let node = node_tags.map_or(0, |tags| tags[triangle]);
            let material = material_tags.map_or(0, |tags| tags[triangle]);
            let group_index = *group_of.entry((node, material)).or_insert_with(|| {
                groups.push(MeshGroupStats {
                    node,
                    material,
                    ..MeshGroupStats::default()
                });
                groups.len() - 1
            });
            let group = &mut groups[group_index];
            group.triangle_count += 1;
            match face_tags.and_then(|tags| counted_face.get_mut(tags[triangle] as usize)) {
                Some(counted) => {
                    if !*counted {
                        *counted = true;
                        group.polygon_count += 1;
                    }
                }
                // No topology to attribute the triangle to — a processed mesh is
                // pure triangles and carries none — so each triangle is its own
                // polygon, the same fallback every `faces` consumer takes.
                None => group.polygon_count += 1,
            }
            for &index in corners {
                let Some(vertex) = self.vertices.get(index as usize) else {
                    continue;
                };
                if !seen.insert((node, material, index)) {
                    continue;
                }
                let mut key = Vec::with_capacity((12 + self.uv_channels.len() * 2) * 4);
                for component in [vertex.position.x, vertex.position.y, vertex.position.z] {
                    key.extend_from_slice(&canonical(component));
                }
                for component in [vertex.normal.x, vertex.normal.y, vertex.normal.z] {
                    key.extend_from_slice(&canonical(component));
                }
                // Every UV set participates; a single-set model carries its UVs
                // only on the vertex itself.
                if self.uv_channels.is_empty() {
                    key.extend_from_slice(&canonical(vertex.uv.x));
                    key.extend_from_slice(&canonical(vertex.uv.y));
                } else {
                    for channel in &self.uv_channels {
                        let uv = channel.get(index as usize).copied().unwrap_or_default();
                        key.extend_from_slice(&canonical(uv.x));
                        key.extend_from_slice(&canonical(uv.y));
                    }
                }
                for component in [
                    vertex.vertex_color.x,
                    vertex.vertex_color.y,
                    vertex.vertex_color.z,
                    vertex.vertex_color.w,
                ] {
                    key.extend_from_slice(&canonical(component));
                }
                if unique.insert((node, material, key)) {
                    group.gpu_vertex_count += 1;
                }
            }
        }
        groups
    }

    /// A per-node inclusion mask for the subtree rooted at `root` (inclusive):
    /// a node is in it when walking its parent links reaches `root`.
    ///
    /// This is what "a node selection" means everywhere — the highlighted
    /// triangles, the selection bounding box, the scoped stats column — so the
    /// walk lives here rather than once per consumer. O(nodes x depth), and a
    /// guard bounds any malformed parent cycle.
    pub fn node_subtree_mask(&self, root: usize) -> Vec<bool> {
        let mut mask = vec![false; self.nodes.len()];
        if root >= self.nodes.len() {
            return mask;
        }
        let node_count = self.nodes.len();
        for (index, included) in mask.iter_mut().enumerate() {
            let mut current = Some(index);
            let mut guard = 0;
            while let Some(node) = current {
                if node == root {
                    *included = true;
                    break;
                }
                current = self.nodes.get(node).and_then(|node| node.parent);
                guard += 1;
                if guard > node_count {
                    break;
                }
            }
        }
        mask
    }

    /// Sum a [`mesh_group_stats`] table over the groups `scope` covers.
    ///
    /// The table is the expensive part and is built once per model; this is a
    /// walk over a handful of groups, so the stats overlay can re-scope it
    /// every time the selection or the hidden set changes.
    ///
    /// [`mesh_group_stats`]: ModelData::mesh_group_stats
    pub fn scope_stats(&self, groups: &[MeshGroupStats], scope: StatsScope<'_>) -> ScopeStats {
        let mut stats = ScopeStats::default();
        // One draw call per slot, so the count is small and a linear dedupe beats
        // standing a hash set up.
        let mut materials: Vec<u32> = Vec::new();

        for group in groups {
            let included = match scope {
                StatsScope::Nodes(mask) => mask.get(group.node as usize).copied().unwrap_or(false),
                StatsScope::Material(slot) => group.material == slot,
            };
            if !included {
                continue;
            }
            stats.polygon_count += group.polygon_count;
            stats.triangle_count += group.triangle_count;
            stats.gpu_vertex_count += group.gpu_vertex_count;
            if !materials.contains(&group.material) {
                materials.push(group.material);
            }
        }
        stats.draw_count = materials.len();

        // The DCC count is authored per mesh, so only a scope that *is* a set of
        // meshes has one. A model whose producer never recorded it reports none
        // at all rather than a zero that would read as "this selection holds no
        // vertices" (invariant 5).
        let tracked = self.nodes.iter().any(|node| node.source_vertex_count > 0);
        stats.vertex_count = match scope {
            StatsScope::Nodes(mask) if tracked => Some(
                self.nodes
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| mask.get(*index).copied().unwrap_or(false))
                    .map(|(_, node)| node.source_vertex_count)
                    .sum(),
            ),
            StatsScope::Nodes(_) | StatsScope::Material(_) => None,
        };
        stats
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
            // The whole cube is this one node, so its share is the model's own
            // `vertex_count` below.
            source_vertex_count: 24,
            transform: Mat4::IDENTITY,
            rest_local: LocalTransform::IDENTITY,
            kind: NodeKind::Mesh,
            bone: None,
        }],
        uv_set_names: vec!["UVMap".to_owned()],
        stats: ModelStats {
            polygon_count: 6,
            triangle_count: 12,
            vertex_count: 24,
            // Overwritten below by the measured count (each flat-shaded corner
            // is genuinely unique, so it stays 24).
            gpu_vertex_count: 0,
            uv_set_count: 1,
            material_count: 1,
            draw_count: 1,
            bone_count: 0,
            clip_count: 0,
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
    model.stats.gpu_vertex_count = model.count_gpu_vertices();
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

    /// Two mesh nodes, two materials, one triangle each — enough to tell the
    /// four draw groups apart and to check a scope sums only its own.
    fn two_node_model() -> ModelData {
        let corner = |x: f32| Vertex {
            position: Vec3::new(x, 0.0, 0.0),
            ..Vertex::default()
        };
        let node = |name: &str, source_vertex_count: usize, mesh_part: usize| SceneNode {
            name: name.to_owned(),
            parent: None,
            mesh_part: Some(mesh_part),
            source_vertex_count,
            transform: Mat4::IDENTITY,
            rest_local: LocalTransform::IDENTITY,
            kind: NodeKind::Mesh,
            bone: None,
        };
        ModelData {
            // Four triangles, every corner a distinct position, so each triangle
            // contributes three unique GPU vertices to its own group.
            vertices: (0..12).map(|i| corner(i as f32)).collect(),
            indices: (0..12).collect(),
            faces: (0..4)
                .map(|face| TopologyFace {
                    first_index: face * 3,
                    index_count: 3,
                })
                .collect(),
            triangles: TriangleData {
                to_face: vec![0, 1, 2, 3],
                material: vec![0, 1, 0, 1],
                node: vec![0, 0, 1, 1],
            },
            nodes: vec![node("a", 40, 0), node("b", 60, 1)],
            stats: ModelStats {
                vertex_count: 100,
                ..ModelStats::default()
            },
            ..ModelData::default()
        }
    }

    #[test]
    fn mesh_group_stats_split_the_model_into_draw_groups() {
        let model = two_node_model();
        let groups = model.mesh_group_stats();

        // One group per (node, material) pair the triangles carry.
        assert_eq!(groups.len(), 4);
        for group in &groups {
            assert_eq!(group.triangle_count, 1);
            assert_eq!(group.polygon_count, 1);
            assert_eq!(group.gpu_vertex_count, 3);
        }

        // The groups partition the mesh, so their shares add back up to the
        // whole-model figures the stats panel already reports.
        let total: usize = groups.iter().map(|group| group.gpu_vertex_count).sum();
        assert_eq!(total, model.count_gpu_vertices());
        assert_eq!(
            groups
                .iter()
                .map(|group| group.triangle_count)
                .sum::<usize>(),
            model.indices.len() / 3
        );
    }

    #[test]
    fn a_triangulated_polygon_is_counted_once_per_group() {
        // The demo cube is six quads, each triangulated into two triangles: the
        // Polys figure must stay 6, not become 12.
        let model = demo_cube_model();
        let groups = model.mesh_group_stats();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].polygon_count, 6);
        assert_eq!(groups[0].triangle_count, 12);
    }

    #[test]
    fn scope_stats_sum_only_the_scope() {
        let model = two_node_model();
        let groups = model.mesh_group_stats();

        // One node: its two draw groups, and its own authored vertex count.
        let first = model.scope_stats(&groups, StatsScope::Nodes(&[true, false]));
        assert_eq!(first.triangle_count, 2);
        assert_eq!(first.polygon_count, 2);
        assert_eq!(first.gpu_vertex_count, 6);
        assert_eq!(first.draw_count, 2);
        assert_eq!(first.vertex_count, Some(40));

        // Both nodes: the whole mesh, and the file's own vertex count back.
        let both = model.scope_stats(&groups, StatsScope::Nodes(&[true, true]));
        assert_eq!(both.triangle_count, 4);
        assert_eq!(both.gpu_vertex_count, model.count_gpu_vertices());
        assert_eq!(both.draw_count, model.material_draw_count());
        assert_eq!(both.vertex_count, Some(model.stats.vertex_count));

        // A material slot spans both nodes — one draw, and no authored vertex
        // count, since a material is not a set of meshes.
        let slot = model.scope_stats(&groups, StatsScope::Material(1));
        assert_eq!(slot.triangle_count, 2);
        assert_eq!(slot.draw_count, 1);
        assert_eq!(slot.vertex_count, None);

        // An empty mask measures nothing, rather than everything — a measured
        // zero, since the scope really is a (empty) set of meshes.
        let none = model.scope_stats(&groups, StatsScope::Nodes(&[false, false]));
        assert_eq!(none.triangle_count, 0);
        assert_eq!(none.gpu_vertex_count, 0);
        assert_eq!(none.draw_count, 0);
        assert_eq!(none.vertex_count, Some(0));
    }

    #[test]
    fn scope_stats_report_no_vertex_count_when_the_model_never_recorded_one() {
        // What a processed / procedurally-built mesh looks like: real geometry,
        // but no per-node authored count to apportion.
        let mut model = two_node_model();
        for node in &mut model.nodes {
            node.source_vertex_count = 0;
        }
        let groups = model.mesh_group_stats();
        let scope = model.scope_stats(&groups, StatsScope::Nodes(&[true, false]));
        assert_eq!(scope.triangle_count, 2);
        assert_eq!(scope.vertex_count, None);
    }

    #[test]
    fn node_subtree_mask_covers_descendants_only() {
        let mut model = demo_cube_model();
        let child = |parent: Option<usize>| SceneNode {
            name: "child".to_owned(),
            parent,
            mesh_part: None,
            source_vertex_count: 0,
            transform: Mat4::IDENTITY,
            rest_local: LocalTransform::IDENTITY,
            kind: NodeKind::Empty,
            bone: None,
        };
        // 0 is the cube; 1 hangs off it, 2 off 1, 3 is a separate root.
        model.nodes.push(child(Some(0)));
        model.nodes.push(child(Some(1)));
        model.nodes.push(child(None));

        assert_eq!(model.node_subtree_mask(0), [true, true, true, false]);
        assert_eq!(model.node_subtree_mask(1), [false, true, true, false]);
        assert_eq!(model.node_subtree_mask(3), [false, false, false, true]);
        // Out of range selects nothing rather than panicking.
        assert_eq!(model.node_subtree_mask(9), [false; 4]);
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

    // ── SkinData ────────────────────────────────────────────────────────────
    //
    // A 2-logical-vertex skin over a 6-corner (2-triangle) mesh, bound to nodes
    // 1 and 2: vertex 0 is split 0.75/0.25 between them, vertex 1 rides node 2
    // alone.
    fn sample_skin() -> SkinData {
        let cluster = |bone: u32| SkinCluster {
            bone,
            mesh_node: 0,
            world_to_bone_bind: Mat4::IDENTITY,
        };
        SkinData {
            offsets: vec![0, 2, 3],
            bones: vec![1, 2, 2],
            weights: vec![0.75, 0.25, 1.0],
            influence_cluster: vec![0, 1, 1],
            clusters: vec![cluster(1), cluster(2)],
            deformers: Vec::new(),
        }
    }

    /// The render mesh [`sample_skin`] projects onto: 6 corners over 2 logical
    /// vertices.
    fn sample_corner_map() -> Vec<u32> {
        vec![0, 0, 1, 1, 0, 1]
    }

    /// A model wrapping [`sample_skin`] so [`ModelData::validate_deform`] can be
    /// exercised end to end.
    fn skinned_model() -> ModelData {
        ModelData {
            vertices: vec![Vertex::default(); 6],
            nodes: (0..3).map(|_| SceneNode::default()).collect(),
            stats: ModelStats {
                vertex_count: 2,
                ..ModelStats::default()
            },
            corner_to_logical: sample_corner_map(),
            skin: Some(sample_skin()),
            ..ModelData::default()
        }
    }

    #[test]
    fn skin_validate_accepts_a_consistent_skin() {
        assert_eq!(sample_skin().validate(2, 3), Ok(()));
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
    fn validate_deform_accepts_a_consistent_skinned_model() {
        assert_eq!(skinned_model().validate_deform(), Ok(()));
    }

    #[test]
    fn validate_deform_catches_a_drifted_corner_map() {
        let mut model = skinned_model();
        model.corner_to_logical.pop();
        let error = model.validate_deform().unwrap_err();
        assert!(error.contains("corner_to_logical has 5"), "{error}");
    }

    #[test]
    fn validate_deform_catches_an_out_of_range_corner_map() {
        let mut model = skinned_model();
        model.corner_to_logical[3] = 9;
        let error = model.validate_deform().unwrap_err();
        assert!(error.contains("source vertex 9 of 2"), "{error}");
    }

    #[test]
    fn validate_deform_requires_the_corner_map_for_a_skin() {
        let mut model = skinned_model();
        model.corner_to_logical.clear();
        let error = model.validate_deform().unwrap_err();
        assert!(error.contains("corner_to_logical map"), "{error}");
    }

    #[test]
    fn skin_validate_catches_a_cluster_naming_another_bone() {
        let mut skin = sample_skin();
        skin.influence_cluster[0] = 1;
        let error = skin.validate(2, 3).unwrap_err();
        assert!(
            error.contains("names bone 1 but its cluster 1 binds bone 2"),
            "{error}"
        );
    }

    #[test]
    fn skin_validate_catches_an_out_of_range_cluster() {
        let mut skin = sample_skin();
        skin.influence_cluster[2] = 5;
        let error = skin.validate(2, 3).unwrap_err();
        assert!(error.contains("references cluster 5 of 2"), "{error}");
        let mut skin = sample_skin();
        skin.clusters[0].mesh_node = 3;
        let error = skin.validate(2, 3).unwrap_err();
        assert!(error.contains("cluster 0 references node"), "{error}");
    }

    #[test]
    fn skin_validate_catches_a_drifted_offsets_length() {
        let mut skin = sample_skin();
        skin.offsets.push(3);
        let error = skin.validate(2, 3).unwrap_err();
        assert!(error.contains("offsets has 4"), "{error}");
    }

    #[test]
    fn skin_validate_catches_non_monotonic_offsets() {
        let mut skin = sample_skin();
        skin.offsets = vec![0, 3, 2];
        let error = skin.validate(2, 3).unwrap_err();
        assert!(error.contains("not monotonic"), "{error}");
    }

    #[test]
    fn skin_validate_catches_offsets_that_miss_the_influences() {
        let mut skin = sample_skin();
        skin.offsets = vec![0, 2, 2];
        let error = skin.validate(2, 3).unwrap_err();
        assert!(error.contains("end at 2 but there are 3"), "{error}");
    }

    #[test]
    fn skin_validate_catches_mismatched_bones_and_weights() {
        let mut skin = sample_skin();
        skin.weights.pop();
        let error = skin.validate(2, 3).unwrap_err();
        assert!(
            error.contains("bones has 3 entries but weights has 2"),
            "{error}"
        );
    }

    #[test]
    fn skin_validate_catches_an_out_of_range_bone() {
        let mut skin = sample_skin();
        skin.bones[1] = 7;
        let error = skin.validate(2, 3).unwrap_err();
        assert!(error.contains("bone references node 7 of 3"), "{error}");
    }

    #[test]
    fn skin_validate_catches_a_negative_or_non_finite_weight() {
        for bad in [-0.25_f32, f32::NAN, f32::INFINITY] {
            let mut skin = sample_skin();
            skin.weights[0] = bad;
            let error = skin.validate(2, 3).unwrap_err();
            assert!(error.contains("negative or not finite"), "{bad}: {error}");
        }
    }

    #[test]
    fn skin_validate_accepts_unnormalized_weights() {
        // FBX does not require normalized weights; a weight above 1.0 is sloppy
        // but loadable, and refusing it would reject files other tools open.
        let mut skin = sample_skin();
        skin.weights[0] = 1.75;
        assert_eq!(skin.validate(2, 3), Ok(()));
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
            offsets: vec![0, 2],
            bones: vec![1, 2],
            weights: vec![1.5, 0.5],
            ..SkinData::default()
        };
        assert_eq!(unnormalized.summed_weight(0, &[1]), 1.5);
        assert_eq!(unnormalized.influence_fraction(0, &[1]), 0.75);
        assert_eq!(unnormalized.influence_fraction(0, &[1, 2]), 1.0);

        // A vertex with no influences at all reads as zero, not NaN.
        let empty = SkinData {
            offsets: vec![0, 0],
            ..SkinData::default()
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

    /// Every corner of the flat-shaded demo cube is genuinely unique (shared
    /// positions, but a different normal per face and different UVs per corner),
    /// so its GPU cost equals its corner count.
    #[test]
    fn gpu_vertex_count_of_the_demo_cube_is_every_corner() {
        let model = demo_cube_model();
        assert_eq!(model.count_gpu_vertices(), 24);
        assert_eq!(model.stats.gpu_vertex_count, 24);
    }

    /// Corner-split duplicates — identical in every attribute — collapse to one
    /// GPU vertex, and `-0.0` counts as `0.0` (an indexer compares values, not
    /// encodings).
    #[test]
    fn gpu_vertex_count_merges_bit_identical_corners() {
        let mut model = demo_cube_model();
        // Append exact copies of triangle 0's corners as three new vertices and
        // a triangle over them: the mesh grows, its GPU cost must not.
        let corners: Vec<Vertex> = model.indices[..3]
            .iter()
            .map(|&index| model.vertices[index as usize])
            .collect();
        let base = model.vertices.len() as u32;
        model.vertices.extend(corners);
        model.indices.extend_from_slice(&[base, base + 1, base + 2]);
        model.triangles.to_face.push(0);
        model.triangles.material.push(0);
        model.triangles.node.push(0);

        assert_eq!(model.count_gpu_vertices(), 24, "duplicates cost nothing");
    }

    /// A vertex shared by triangles of two materials is uploaded once per draw
    /// group, exactly as the Opt pipeline's per-(node, material) partition keeps
    /// it — the two counts must never disagree.
    #[test]
    fn gpu_vertex_count_keeps_material_boundaries_separate() {
        let mut model = demo_cube_model();
        assert_eq!(model.count_gpu_vertices(), 24);
        // A quad face is two triangles sharing two corners. Splitting the pair
        // across materials puts those shared corners on a draw-group boundary,
        // and each group uploads its own copy — exactly as the Opt pipeline's
        // per-(node, material) partition keeps them, so the counts can't drift.
        model.triangles.material[1] = 1;
        assert_eq!(
            model.count_gpu_vertices(),
            26,
            "the two shared corners are uploaded once per group"
        );
    }
}
