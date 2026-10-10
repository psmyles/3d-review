//! Everything a source file carries that the viewer never draws with but a
//! faithful re-export has to write back: authored properties, node attributes,
//! materials' full property sets and texture connections, mesh topology layers,
//! extra skin layers, poses, display layers, selection sets, scene metadata,
//! and the authored animation curves.
//!
//! [`SourceExtras`] is produced by the import *after* the drawable
//! [`ModelData`](crate::ModelData) has been published, and delivered as its own
//! value: the model is immutable once it is on screen (the renderer keys its
//! buffers on it), and nothing in the viewport reads any of this. It is a
//! carry-only payload — the Opt workspace threads it through processing and the
//! exporter reads it; no UI consults it.
//!
//! Index spaces are the model's: `nodes` is parallel to `ModelData::nodes`,
//! `materials` to `ModelData::materials`, `meshes` to the mesh parts (a
//! `SceneNode::mesh_part`), corners are `ModelData::vertices` indices, logical
//! vertices the numbering `ModelData::corner_to_logical` maps into, faces
//! `ModelData::faces` indices. Every enum here is this crate's own mirror of
//! the FBX vocabulary — nothing from any importer leaks in (invariant 10) — and
//! each mirror keeps its numeric code so an unknown value survives the round
//! trip rather than being dropped.

use glam::{Mat4, Quat, Vec3, Vec4};

/// Defines an enum whose variants carry their FBX / ufbx code, with a lossless
/// `from_code` (a code this build does not name lands in `Unnamed(u32)`) and `code`.
macro_rules! mirror_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $code:expr),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant,)+
            /// A code this build does not name; carried through as-is.
            Unnamed(u32),
        }

        impl $name {
            pub fn from_code(code: u32) -> Self {
                match code {
                    $($code => Self::$variant,)+
                    other => Self::Unnamed(other),
                }
            }

            pub fn code(self) -> u32 {
                match self {
                    $(Self::$variant => $code,)+
                    Self::Unnamed(other) => other,
                }
            }
        }
    };
}

mirror_enum! {
    /// The declared type of an authored property (ufbx's `ufbx_prop_type`).
    PropType {
        Unknown = 0, Boolean = 1, Integer = 2, Number = 3, Vector = 4, Color = 5,
        ColorWithAlpha = 6, Text = 7, DateTime = 8, Translation = 9, Rotation = 10,
        Scaling = 11, Distance = 12, Compound = 13, Blob = 14, Reference = 15,
    }
}

mirror_enum! {
    RotationOrder { Xyz = 0, Xzy = 1, Yzx = 2, Yxz = 3, Zxy = 4, Zyx = 5, Spheric = 6 }
}

mirror_enum! {
    InheritMode { Normal = 0, IgnoreParentScale = 1, ComponentwiseScale = 2 }
}

mirror_enum! {
    /// Why a node exists that the source did not author.
    Synthetic { None = 0, ScaleHelper = 1, GeometryTransformHelper = 2, Root = 3 }
}

mirror_enum! {
    AttributeKind {
        None = 0, Mesh = 1, Bone = 2, Light = 3, Camera = 4, Empty = 5, LodGroup = 6, Other = 7,
    }
}

mirror_enum! {
    ShaderType {
        Unknown = 0, FbxLambert = 1, FbxPhong = 2, OslStandardSurface = 3,
        ArnoldStandardSurface = 4, MaxPhysicalMaterial = 5, MaxPbrMetalRough = 6,
        MaxPbrSpecGloss = 7, GltfMaterial = 8, OpenPbrMaterial = 9, ShaderFxGraph = 10,
        BlenderPhong = 11, WavefrontMtl = 12,
    }
}

mirror_enum! {
    TextureKind { File = 0, Layered = 1, Procedural = 2, Shader = 3 }
}

mirror_enum! {
    WrapMode { Repeat = 0, Clamp = 1 }
}

mirror_enum! {
    /// A layered texture's per-layer blend (ufbx's `ufbx_blend_mode`, which is
    /// the FBX `BlendModes` numbering).
    BlendMode {
        Translucent = 0, Additive = 1, Multiply = 2, Multiply2x = 3, Over = 4, Replace = 5,
        Dissolve = 6, Darken = 7, ColorBurn = 8, LinearBurn = 9, DarkerColor = 10,
        Lighten = 11, Screen = 12, ColorDodge = 13, LinearDodge = 14, LighterColor = 15,
        SoftLight = 16, HardLight = 17, VividLight = 18, LinearLight = 19, PinLight = 20,
        HardMix = 21, Difference = 22, Exclusion = 23, Subtract = 24, Divide = 25, Hue = 26,
        Saturation = 27, Color = 28, Luminosity = 29, Overlay = 30,
    }
}

mirror_enum! {
    LightType { Point = 0, Directional = 1, Spot = 2, Area = 3, Volume = 4 }
}

mirror_enum! {
    LightDecay { None = 0, Linear = 1, Quadratic = 2, Cubic = 3 }
}

mirror_enum! {
    LightAreaShape { Rectangle = 0, Sphere = 1 }
}

mirror_enum! {
    ProjectionMode { Perspective = 0, Orthographic = 1 }
}

mirror_enum! {
    AspectMode {
        WindowSize = 0, FixedRatio = 1, FixedResolution = 2, FixedWidth = 3, FixedHeight = 4,
    }
}

mirror_enum! {
    ApertureMode { HorizontalAndVertical = 0, Horizontal = 1, Vertical = 2, FocalLength = 3 }
}

mirror_enum! {
    GateFit { None = 0, Vertical = 1, Horizontal = 2, Fill = 3, Overscan = 4, Stretch = 5 }
}

mirror_enum! {
    ApertureFormat {
        Custom = 0, Theatrical16mm = 1, Super16mm = 2, Academy35mm = 3, TvProjection35mm = 4,
        FullAperture35mm = 5, Projection185_35mm = 6, Anamorphic35mm = 7, Projection70mm = 8,
        VistaVision = 9, DynaVision = 10, Imax = 11,
    }
}

mirror_enum! {
    LodDisplay { UseLod = 0, Show = 1, Hide = 2 }
}

mirror_enum! {
    SubdivisionDisplayMode { Disabled = 0, Hull = 1, HullAndSmooth = 2, Smooth = 3 }
}

mirror_enum! {
    SubdivisionBoundary {
        Default = 0, Legacy = 1, SharpCorners = 2, SharpNone = 3, SharpBoundary = 4,
        SharpInterior = 5,
    }
}

mirror_enum! {
    TimeMode {
        Default = 0, Fps120 = 1, Fps100 = 2, Fps60 = 3, Fps50 = 4, Fps48 = 5, Fps30 = 6,
        Fps30Drop = 7, NtscDropFrame = 8, NtscFullFrame = 9, Pal = 10, Fps24 = 11,
        Fps1000 = 12, FilmFullFrame = 13, Custom = 14, Fps96 = 15, Fps72 = 16, Fps59_94 = 17,
    }
}

mirror_enum! {
    TimeProtocol { Smpte = 0, FrameCount = 1, Default = 2 }
}

mirror_enum! {
    SnapMode { None = 0, Snap = 1, Play = 2, SnapAndPlay = 3 }
}

mirror_enum! {
    Interpolation { ConstantPrev = 0, ConstantNext = 1, Linear = 2, Cubic = 3 }
}

mirror_enum! {
    ExtrapolationMode { Constant = 0, Repeat = 1, Mirror = 2, Slope = 3, RepeatRelative = 4 }
}

mirror_enum! {
    CoordinateAxis {
        PositiveX = 0, NegativeX = 1, PositiveY = 2, NegativeY = 3, PositiveZ = 4,
        NegativeZ = 5, Unknown = 6,
    }
}

/// The flags an authored property carried (ufbx's `ufbx_prop_flags`, kept as
/// the raw bits so lock / mute bits round-trip too).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct PropFlags(pub u32);

impl PropFlags {
    pub const ANIMATABLE: u32 = 0x1;
    pub const USER_DEFINED: u32 = 0x2;
    pub const HIDDEN: u32 = 0x4;
    pub const ANIMATED: u32 = 0x2000;
    pub const NO_VALUE: u32 = 0x10000;
    pub const VALUE_REAL: u32 = 0x100000;
    pub const VALUE_VEC2: u32 = 0x200000;
    pub const VALUE_VEC3: u32 = 0x400000;
    pub const VALUE_VEC4: u32 = 0x800000;
    pub const VALUE_INT: u32 = 0x1000000;
    pub const VALUE_STR: u32 = 0x2000000;
    pub const VALUE_BLOB: u32 = 0x4000000;

    pub fn animatable(self) -> bool {
        self.0 & Self::ANIMATABLE != 0
    }

    pub fn user_defined(self) -> bool {
        self.0 & Self::USER_DEFINED != 0
    }

    pub fn hidden(self) -> bool {
        self.0 & Self::HIDDEN != 0
    }

    pub fn animated(self) -> bool {
        self.0 & Self::ANIMATED != 0
    }

    pub fn has_value(self) -> bool {
        self.0 & Self::NO_VALUE == 0
    }

    /// How many real components the file gave the property (0..=4).
    pub fn real_count(self) -> usize {
        if self.0 & Self::VALUE_VEC4 != 0 {
            4
        } else if self.0 & Self::VALUE_VEC3 != 0 {
            3
        } else if self.0 & Self::VALUE_VEC2 != 0 {
            2
        } else if self.0 & Self::VALUE_REAL != 0 {
            1
        } else {
            0
        }
    }

    pub fn has_int(self) -> bool {
        self.0 & Self::VALUE_INT != 0
    }

    pub fn has_string(self) -> bool {
        self.0 & Self::VALUE_STR != 0
    }

    pub fn has_blob(self) -> bool {
        self.0 & Self::VALUE_BLOB != 0
    }
}

/// One authored property, as the file wrote it. Every value field the file
/// carried is kept (a `Distance` has both a number and a unit string; a user
/// enum an int and its option string); [`PropFlags`] says which are present.
#[derive(Debug, Clone, PartialEq)]
pub struct Prop {
    pub name: String,
    pub kind: PropType,
    pub flags: PropFlags,
    pub value_int: i64,
    pub value_real: [f64; 4],
    pub value_str: String,
    pub value_blob: Vec<u8>,
}

impl Prop {
    pub fn user_defined(&self) -> bool {
        self.flags.user_defined()
    }
}

/// A light's authored parameters (ufbx's normalized reading of the FBX
/// `FbxLight` attribute; `intensity` is the file's `Intensity` ÷ 100).
#[derive(Debug, Clone, PartialEq)]
pub struct LightExtras {
    pub color: Vec3,
    pub intensity: f64,
    pub local_direction: Vec3,
    pub kind: LightType,
    pub decay: LightDecay,
    pub area_shape: LightAreaShape,
    pub inner_angle: f64,
    pub outer_angle: f64,
    pub cast_light: bool,
    pub cast_shadows: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CameraExtras {
    pub projection_mode: ProjectionMode,
    pub resolution_is_pixels: bool,
    pub resolution: [f64; 2],
    pub field_of_view_deg: [f64; 2],
    pub orthographic_extent: f64,
    pub aspect_ratio: f64,
    pub near_plane: f64,
    pub far_plane: f64,
    pub aspect_mode: AspectMode,
    pub aperture_mode: ApertureMode,
    pub gate_fit: GateFit,
    pub aperture_format: ApertureFormat,
    pub focal_length_mm: f64,
    pub film_size_inch: [f64; 2],
    pub aperture_size_inch: [f64; 2],
    pub squeeze_ratio: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LodLevel {
    /// The distance at which this level starts (0 for the first).
    pub distance: f64,
    pub display: LodDisplay,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LodGroupExtras {
    pub relative_distances: bool,
    pub ignore_parent_transform: bool,
    pub use_distance_limit: bool,
    pub distance_limit_min: f64,
    pub distance_limit_max: f64,
    /// One per child, in child order.
    pub levels: Vec<LodLevel>,
}

/// The node attribute a node carries, with its own authored properties. The
/// typed payloads are `Some` for their kind only.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeExtras {
    pub kind: AttributeKind,
    pub name: String,
    pub props: Vec<Prop>,
    pub light: Option<LightExtras>,
    pub camera: Option<CameraExtras>,
    pub lod_group: Option<LodGroupExtras>,
}

/// Per node, parallel to `ModelData::nodes`.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeExtras {
    /// The node's explicit properties: `Lcl Translation` / `Rotation` /
    /// `Scaling`, `RotationOrder`, `PreRotation`, the pivots and offsets,
    /// `GeometricTranslation` / `Rotation` / `Scaling`, `InheritType`,
    /// `Visibility`, user properties — in file units.
    pub props: Vec<Prop>,
    pub rotation_order: RotationOrder,
    pub inherit_mode: InheritMode,
    /// The inherit mode the file declared, before the importer's helper-node
    /// handling made every node a plain parent × local product.
    pub original_inherit_mode: InheritMode,
    /// The geometric transform: geometry space → this node's space.
    pub geometry_to_node: Mat4,
    pub synthetic: Synthetic,
    pub visible: bool,
    pub attribute: Option<AttributeExtras>,
}

/// One texture connection of a material: `material_prop` is the FBX property
/// the texture feeds (`DiffuseColor`, ...), `shader_prop` the shader-side name
/// when the material has a shader binding.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialTexture {
    pub material_prop: String,
    pub shader_prop: String,
    /// Indexes [`SourceExtras::textures`].
    pub texture: u32,
}

/// Per material, parallel to `ModelData::materials`.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialExtras {
    pub shader_type: ShaderType,
    pub shading_model: String,
    /// Every authored material property.
    pub props: Vec<Prop>,
    pub textures: Vec<MaterialTexture>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextureLayer {
    /// Indexes [`SourceExtras::textures`].
    pub texture: u32,
    pub blend_mode: BlendMode,
    pub alpha: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextureExtras {
    pub name: String,
    pub kind: TextureKind,
    /// The file names as authored: `filename` is ufbx's resolved best guess,
    /// `absolute_filename` / `relative_filename` the file's own two strings.
    pub filename: String,
    pub absolute_filename: String,
    pub relative_filename: String,
    pub uv_set: String,
    pub wrap_u: WrapMode,
    pub wrap_v: WrapMode,
    /// The authored UV transform, `Some` when the file declared one.
    pub uv_transform: Option<(Vec3, Quat, Vec3)>,
    /// Embedded image bytes, when the texture (or its video) carried them.
    pub content: Vec<u8>,
    /// Indexes [`SourceExtras::videos`].
    pub video: Option<u32>,
    /// Layers of a [`TextureKind::Layered`] texture, bottom first.
    pub layers: Vec<TextureLayer>,
    pub props: Vec<Prop>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoExtras {
    pub name: String,
    pub filename: String,
    pub absolute_filename: String,
    pub relative_filename: String,
    pub content: Vec<u8>,
    pub props: Vec<Prop>,
}

/// One vertex-color set of a mesh part. Set 0's values are the geometry's own
/// `Vertex::vertex_color`, so only its name is here; later sets carry their
/// values per corner of the part.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorSetExtras {
    pub name: String,
    pub index: u32,
    /// Per corner of the part (`corner_count` long), empty for set 0.
    pub values: Vec<Vec4>,
}

/// One UV set a mesh part carries.
#[derive(Debug, Clone, PartialEq)]
pub struct UvSetExtras {
    pub name: String,
    pub index: u32,
    /// Whether the layer holds any values (a declared but empty layer does not).
    pub has_values: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FaceGroup {
    pub id: i32,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SubdivisionExtras {
    pub preview_levels: u32,
    pub render_levels: u32,
    pub display_mode: SubdivisionDisplayMode,
    pub boundary: SubdivisionBoundary,
    pub uv_boundary: SubdivisionBoundary,
}

/// A cluster of a skin layer beyond the first (the primary skin's clusters are
/// on `SkinData::clusters`).
#[derive(Debug, Clone, PartialEq)]
pub struct ExtraCluster {
    /// Indexes `ModelData::nodes`.
    pub bone: u32,
    pub name: String,
    /// The authored `Transform`: mesh node → bone.
    pub mesh_node_to_bone: Mat4,
    /// The authored `TransformLink`: bone → world at bind, in meters.
    pub bind_to_world: Mat4,
}

/// A skin deformer beyond the mesh's first — a layered skin. Weights are a CSR
/// over the part's logical vertices.
#[derive(Debug, Clone, PartialEq)]
pub struct SkinLayerExtras {
    pub method: crate::SkinningMethod,
    pub max_weights_per_vertex: u32,
    pub clusters: Vec<ExtraCluster>,
    /// `logical_count + 1` row starts into `influences`.
    pub offsets: Vec<u32>,
    /// `(cluster index, weight)` per influence.
    pub influences: Vec<(u32, f32)>,
}

/// Per mesh part, in `SceneNode::mesh_part` order.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshExtras {
    /// The node carrying this part, indexing `ModelData::nodes`.
    pub node: u32,
    /// The geometry element's own name (a node and its mesh are named separately).
    pub name: String,
    pub props: Vec<Prop>,
    /// This part's range of `ModelData::vertices`.
    pub corner_first: u32,
    pub corner_count: u32,
    /// This part's range of logical vertices.
    pub logical_first: u32,
    pub logical_count: u32,
    /// This part's range of `ModelData::faces`.
    pub face_first: u32,
    pub face_count: u32,
    /// Whether the file carried a tangent layer (otherwise the model's tangents
    /// were synthesized at import and are not written back).
    pub tangents_authored: bool,
    /// Whether the file carried normals (otherwise they were generated at
    /// import). Read by the audit; the exporter writes the model's normals
    /// either way.
    pub normals_authored: bool,
    /// The UV sets this part carries, in the file's order. The geometry matches
    /// channels by *position*, so this is the only record of which sets each
    /// part really has and what they are called.
    pub uv_sets: Vec<UvSetExtras>,
    /// Control points no face references, as `(global logical vertex, world
    /// position)`. They produce no render corner, so nothing else records
    /// where they are.
    pub unused_vertices: Vec<(u32, Vec3)>,
    pub reversed_winding: bool,
    pub color_sets: Vec<ColorSetExtras>,
    /// Each edge as two corner indices into `ModelData::vertices`
    /// (`u32::MAX` for an endpoint the file left unresolved).
    pub edges: Vec<[u32; 2]>,
    /// Per edge, present only when the file carried the layer.
    pub edge_smoothing: Vec<bool>,
    pub edge_crease: Vec<f32>,
    pub edge_visibility: Vec<bool>,
    /// Per face of the part, present only when the file carried the layer.
    pub face_smoothing: Vec<bool>,
    pub face_hole: Vec<bool>,
    pub face_group: Vec<u32>,
    pub face_groups: Vec<FaceGroup>,
    /// Per logical vertex of the part, present only when the file carried it.
    pub vertex_crease: Vec<f32>,
    pub subdivision: SubdivisionExtras,
    pub extra_skins: Vec<SkinLayerExtras>,
    /// The primary skin's dual-quaternion blend weights: `(global logical
    /// vertex, weight)`.
    pub dq_weights: Vec<(u32, f32)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PoseEntry {
    /// Indexes `ModelData::nodes`.
    pub node: u32,
    /// The authored bone → world matrix, in meters.
    pub bone_to_world: Mat4,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PoseExtras {
    pub name: String,
    pub is_bind_pose: bool,
    pub entries: Vec<PoseEntry>,
    pub props: Vec<Prop>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DisplayLayerExtras {
    pub name: String,
    pub visible: bool,
    pub frozen: bool,
    pub ui_color: Vec3,
    /// Members, indexing `ModelData::nodes`.
    pub nodes: Vec<u32>,
    pub props: Vec<Prop>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectionNodeExtras {
    /// The selected node, indexing `ModelData::nodes`; `None` when the file's
    /// target was missing.
    pub node: Option<u32>,
    pub include_node: bool,
    /// Global logical vertices.
    pub vertices: Vec<u32>,
    /// Indices into the target part's [`MeshExtras::edges`].
    pub edges: Vec<u32>,
    /// Global faces (`ModelData::faces`).
    pub faces: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectionSetExtras {
    pub name: String,
    pub props: Vec<Prop>,
    pub nodes: Vec<SelectionNodeExtras>,
}

/// What an animated property belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElementRef {
    /// Indexes `ModelData::nodes`.
    Node(u32),
    /// The attribute of the node at this index (a light's `Intensity`, a
    /// camera's `FocalLength`, ...).
    NodeAttribute(u32),
    /// Indexes `ModelData::materials`.
    Material(u32),
    /// Indexes [`SourceExtras::textures`].
    Texture(u32),
    /// Indexes [`SourceExtras::videos`].
    Video(u32),
    /// Indexes `MorphData::channels`.
    BlendChannel(u32),
    /// Indexes [`SourceExtras::display_layers`].
    DisplayLayer(u32),
    /// Indexes [`SourceExtras::anim_layers`].
    AnimLayer(u32),
    /// An element this model has no counterpart for; the exporter reports it.
    Unmapped {
        /// ufbx's element type code.
        element_type: u32,
        name: String,
    },
}

/// One authored keyframe. Tangents are ufbx's `(dx, dy)` — the handle offset
/// in seconds and value units — for the cubic case.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawKey {
    pub time: f64,
    pub value: f64,
    pub interpolation: Interpolation,
    pub left: (f32, f32),
    pub right: (f32, f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extrapolation {
    pub mode: ExtrapolationMode,
    pub repeat_count: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    pub keys: Vec<RawKey>,
    pub pre: Extrapolation,
    pub post: Extrapolation,
}

/// One animated property within a layer: up to three component curves
/// (`d|X`, `d|Y`, `d|Z`; a scalar uses the first).
#[derive(Debug, Clone, PartialEq)]
pub struct AnimPropCurves {
    pub target: ElementRef,
    pub prop_name: String,
    pub default: Vec3,
    pub curves: [Option<Curve>; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayerCurves {
    pub name: String,
    pub weight: f64,
    pub weight_is_animated: bool,
    pub blended: bool,
    pub additive: bool,
    pub compose_rotation: bool,
    pub compose_scale: bool,
    pub props: Vec<Prop>,
    pub anim: Vec<AnimPropCurves>,
}

/// One animation stack as authored. Its layers are shared with every stack
/// that references them, hence the indirection.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipCurves {
    pub name: String,
    pub props: Vec<Prop>,
    /// The baked clip this stack became (`ModelData::animations`), `None`
    /// when the bake failed.
    pub clip: Option<u32>,
    pub time_begin: f64,
    pub time_end: f64,
    /// Indexes [`SourceExtras::anim_layers`].
    pub layers: Vec<u32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Application {
    pub vendor: String,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneExtras {
    pub creator: String,
    /// The file name ufbx recorded, and the `SourceDocument`'s original path.
    pub filename: String,
    pub original_file_path: String,
    /// The source's FBX version (7700 = FBX 2020).
    pub version: u32,
    pub ascii: bool,
    pub original_application: Application,
    pub latest_application: Application,
    /// `SceneInfo` properties beyond the application block.
    pub scene_props: Vec<Prop>,
    /// `GlobalSettings` properties (`TimeSpanStart` / `Stop`, custom frame
    /// rate, ...).
    pub settings_props: Vec<Prop>,
    pub axes: [CoordinateAxis; 3],
    pub original_axis_up: CoordinateAxis,
    pub unit_meters: f64,
    pub original_unit_meters: f64,
    pub frames_per_second: f64,
    pub ambient_color: Vec3,
    pub default_camera: String,
    pub time_mode: TimeMode,
    pub time_protocol: TimeProtocol,
    pub snap_mode: SnapMode,
}

/// The whole source-property capture. See the module docs for what it is and
/// how it relates to the model it accompanies.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceExtras {
    pub scene: SceneExtras,
    pub nodes: Vec<NodeExtras>,
    pub materials: Vec<MaterialExtras>,
    pub textures: Vec<TextureExtras>,
    pub videos: Vec<VideoExtras>,
    pub meshes: Vec<MeshExtras>,
    pub poses: Vec<PoseExtras>,
    pub display_layers: Vec<DisplayLayerExtras>,
    pub selection_sets: Vec<SelectionSetExtras>,
    pub anim_layers: Vec<LayerCurves>,
    pub animations: Vec<ClipCurves>,
}

impl SourceExtras {
    /// Whether any mesh part authored its tangents (the model's tangents are
    /// otherwise synthesized and not written back).
    pub fn tangents_authored(&self) -> bool {
        self.meshes.iter().any(|mesh| mesh.tangents_authored)
    }

    /// The extras of the part carried by `node`, if it has one.
    pub fn mesh_of_node(&self, node: u32) -> Option<&MeshExtras> {
        self.meshes.iter().find(|mesh| mesh.node == node)
    }

    /// The funnel guard: every index in range, every parallel array the length
    /// its owner declares. `counts` are the model's, so a capture that drifted
    /// from the geometry it describes is rejected here, once.
    pub fn validate(&self, counts: &ExtrasCounts) -> Result<(), String> {
        let texture_count = self.textures.len();
        let video_count = self.videos.len();

        if self.nodes.len() != counts.nodes {
            return Err(format!(
                "extras carry {} nodes, the model {}",
                self.nodes.len(),
                counts.nodes
            ));
        }
        if self.materials.len() != counts.materials {
            return Err(format!(
                "extras carry {} materials, the model {}",
                self.materials.len(),
                counts.materials
            ));
        }
        if self.meshes.len() != counts.mesh_parts {
            return Err(format!(
                "extras carry {} mesh parts, the model {}",
                self.meshes.len(),
                counts.mesh_parts
            ));
        }

        for (index, node) in self.nodes.iter().enumerate() {
            if let Some(attribute) = &node.attribute {
                let typed = match attribute.kind {
                    AttributeKind::Light => attribute.light.is_some(),
                    AttributeKind::Camera => attribute.camera.is_some(),
                    AttributeKind::LodGroup => attribute.lod_group.is_some(),
                    _ => true,
                };
                if !typed {
                    return Err(format!(
                        "node {index} has a {:?} attribute without its parameters",
                        attribute.kind
                    ));
                }
            }
        }

        for (index, material) in self.materials.iter().enumerate() {
            for texture in &material.textures {
                if texture.texture as usize >= texture_count {
                    return Err(format!(
                        "material {index} references texture {} of {texture_count}",
                        texture.texture
                    ));
                }
            }
        }
        for (index, texture) in self.textures.iter().enumerate() {
            if let Some(video) = texture.video
                && video as usize >= video_count
            {
                return Err(format!(
                    "texture {index} references video {video} of {video_count}"
                ));
            }
            for layer in &texture.layers {
                if layer.texture as usize >= texture_count {
                    return Err(format!(
                        "texture {index} layers texture {} of {texture_count}",
                        layer.texture
                    ));
                }
            }
        }

        let mut corner_cursor = 0u32;
        let mut logical_cursor = 0u32;
        let mut face_cursor = 0u32;
        for (index, mesh) in self.meshes.iter().enumerate() {
            if mesh.node as usize >= counts.nodes {
                return Err(format!(
                    "mesh part {index} names node {} of {}",
                    mesh.node, counts.nodes
                ));
            }
            if mesh.corner_first != corner_cursor
                || mesh.logical_first != logical_cursor
                || mesh.face_first != face_cursor
            {
                return Err(format!(
                    "mesh part {index} does not follow the previous part's ranges"
                ));
            }
            corner_cursor = corner_cursor.saturating_add(mesh.corner_count);
            logical_cursor = logical_cursor.saturating_add(mesh.logical_count);
            face_cursor = face_cursor.saturating_add(mesh.face_count);
            let corners = mesh.corner_count as usize;
            let faces = mesh.face_count as usize;
            let logical = mesh.logical_count as usize;
            for set in &mesh.color_sets {
                if !set.values.is_empty() && set.values.len() != corners {
                    return Err(format!(
                        "mesh part {index} color set {:?} has {} values for {corners} corners",
                        set.name,
                        set.values.len()
                    ));
                }
            }
            let edges = mesh.edges.len();
            for (layer, len) in [
                ("edge smoothing", mesh.edge_smoothing.len()),
                ("edge crease", mesh.edge_crease.len()),
                ("edge visibility", mesh.edge_visibility.len()),
            ] {
                if len != 0 && len != edges {
                    return Err(format!(
                        "mesh part {index} {layer} has {len} entries for {edges} edges"
                    ));
                }
            }
            for edge in &mesh.edges {
                for &corner in edge {
                    if corner != u32::MAX && (corner as usize) >= counts.corners {
                        return Err(format!(
                            "mesh part {index} edge references corner {corner} of {}",
                            counts.corners
                        ));
                    }
                }
            }
            for (layer, len) in [
                ("face smoothing", mesh.face_smoothing.len()),
                ("face hole", mesh.face_hole.len()),
                ("face group", mesh.face_group.len()),
            ] {
                if len != 0 && len != faces {
                    return Err(format!(
                        "mesh part {index} {layer} has {len} entries for {faces} faces"
                    ));
                }
            }
            if !mesh.vertex_crease.is_empty() && mesh.vertex_crease.len() != logical {
                return Err(format!(
                    "mesh part {index} vertex crease has {} entries for {logical} vertices",
                    mesh.vertex_crease.len()
                ));
            }
            for (skin_index, skin) in mesh.extra_skins.iter().enumerate() {
                if skin.offsets.len() != logical + 1 {
                    return Err(format!(
                        "mesh part {index} skin layer {skin_index} has {} row starts for {logical} vertices",
                        skin.offsets.len()
                    ));
                }
                if skin.offsets.windows(2).any(|pair| pair[0] > pair[1])
                    || skin.offsets.last().copied().unwrap_or(0) as usize != skin.influences.len()
                {
                    return Err(format!(
                        "mesh part {index} skin layer {skin_index} rows are not a CSR"
                    ));
                }
                for &(cluster, weight) in &skin.influences {
                    if cluster as usize >= skin.clusters.len()
                        || !weight.is_finite()
                        || weight < 0.0
                    {
                        return Err(format!(
                            "mesh part {index} skin layer {skin_index} has a bad influence"
                        ));
                    }
                }
                for cluster in &skin.clusters {
                    if cluster.bone as usize >= counts.nodes {
                        return Err(format!(
                            "mesh part {index} skin layer {skin_index} names bone {} of {}",
                            cluster.bone, counts.nodes
                        ));
                    }
                }
            }
            for &(vertex, weight) in &mesh.dq_weights {
                if vertex as usize >= counts.logical || !weight.is_finite() {
                    return Err(format!(
                        "mesh part {index} has a bad dual-quaternion weight"
                    ));
                }
            }
        }
        if corner_cursor as usize != counts.corners
            || logical_cursor as usize != counts.logical
            || face_cursor as usize != counts.faces
        {
            return Err(format!(
                "mesh parts cover {corner_cursor} corners / {logical_cursor} vertices / {face_cursor} faces, the model has {} / {} / {}",
                counts.corners, counts.logical, counts.faces
            ));
        }

        for (index, pose) in self.poses.iter().enumerate() {
            for entry in &pose.entries {
                if entry.node as usize >= counts.nodes {
                    return Err(format!(
                        "pose {index} names node {} of {}",
                        entry.node, counts.nodes
                    ));
                }
            }
        }
        for (index, layer) in self.display_layers.iter().enumerate() {
            for &node in &layer.nodes {
                if node as usize >= counts.nodes {
                    return Err(format!(
                        "display layer {index} names node {node} of {}",
                        counts.nodes
                    ));
                }
            }
        }
        for (index, set) in self.selection_sets.iter().enumerate() {
            for entry in &set.nodes {
                if let Some(node) = entry.node
                    && node as usize >= counts.nodes
                {
                    return Err(format!(
                        "selection set {index} names node {node} of {}",
                        counts.nodes
                    ));
                }
                if entry.vertices.iter().any(|&v| v as usize >= counts.logical)
                    || entry.faces.iter().any(|&f| f as usize >= counts.faces)
                {
                    return Err(format!(
                        "selection set {index} references geometry the model lacks"
                    ));
                }
            }
        }

        let layer_count = self.anim_layers.len();
        for (index, layer) in self.anim_layers.iter().enumerate() {
            for anim in &layer.anim {
                let in_range = match &anim.target {
                    ElementRef::Node(node) | ElementRef::NodeAttribute(node) => {
                        (*node as usize) < counts.nodes
                    }
                    ElementRef::Material(material) => (*material as usize) < counts.materials,
                    ElementRef::Texture(texture) => (*texture as usize) < texture_count,
                    ElementRef::Video(video) => (*video as usize) < video_count,
                    ElementRef::BlendChannel(channel) => {
                        (*channel as usize) < counts.morph_channels
                    }
                    ElementRef::DisplayLayer(layer) => {
                        (*layer as usize) < self.display_layers.len()
                    }
                    ElementRef::AnimLayer(layer) => (*layer as usize) < layer_count,
                    ElementRef::Unmapped { .. } => true,
                };
                if !in_range {
                    return Err(format!(
                        "animation layer {index} animates {:?} {:?}, which the model lacks",
                        anim.target, anim.prop_name
                    ));
                }
                for curve in anim.curves.iter().flatten() {
                    if curve
                        .keys
                        .iter()
                        .any(|key| !key.time.is_finite() || !key.value.is_finite())
                    {
                        return Err(format!(
                            "animation layer {index} curve for {:?} has a non-finite key",
                            anim.prop_name
                        ));
                    }
                }
            }
        }
        for (index, clip) in self.animations.iter().enumerate() {
            if let Some(clip_index) = clip.clip
                && clip_index as usize >= counts.clips
            {
                return Err(format!(
                    "stack {index} names clip {clip_index} of {}",
                    counts.clips
                ));
            }
            if clip
                .layers
                .iter()
                .any(|&layer| layer as usize >= layer_count)
            {
                return Err(format!(
                    "stack {index} names an animation layer the capture lacks"
                ));
            }
        }
        Ok(())
    }
}

/// The model-side counts [`SourceExtras::validate`] checks against.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExtrasCounts {
    pub nodes: usize,
    pub materials: usize,
    pub mesh_parts: usize,
    pub corners: usize,
    pub logical: usize,
    pub faces: usize,
    pub morph_channels: usize,
    pub clips: usize,
}

impl ExtrasCounts {
    pub fn of(model: &crate::ModelData) -> Self {
        Self {
            nodes: model.nodes.len(),
            materials: model.materials.len(),
            mesh_parts: model
                .nodes
                .iter()
                .filter(|node| node.mesh_part.is_some())
                .count(),
            corners: model.vertices.len(),
            logical: model.stats.vertex_count,
            faces: model.faces.len(),
            morph_channels: model.morph.as_ref().map_or(0, |morph| morph.channels.len()),
            clips: model.animations.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrors_keep_unknown_codes() {
        assert_eq!(PropType::from_code(3), PropType::Number);
        assert_eq!(PropType::from_code(99), PropType::Unnamed(99));
        assert_eq!(PropType::Unnamed(99).code(), 99);
        assert_eq!(BlendMode::from_code(30), BlendMode::Overlay);
        assert_eq!(BlendMode::Overlay.code(), 30);
    }

    #[test]
    fn prop_flags_decode_value_kinds() {
        let flags =
            PropFlags(PropFlags::USER_DEFINED | PropFlags::VALUE_VEC3 | PropFlags::VALUE_STR);
        assert!(flags.user_defined());
        assert!(!flags.animatable());
        assert_eq!(flags.real_count(), 3);
        assert!(flags.has_string());
        assert!(!flags.has_int());
        assert!(flags.has_value());
    }
}
