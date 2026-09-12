//! The scene graph: nodes, what kind of thing each one is, and the rest pose
//! transform a clip overrides.

use glam::{Mat4, Quat, Vec3};

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
