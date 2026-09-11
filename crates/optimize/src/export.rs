//! Writing a processed LOD chain out as FBX, through the vendored ufbx_write.
//!
//! ## Shape of the work
//!
//! [`export_fbx`] turns one or more [`ProcessedLod`]s into either a single file
//! holding the whole chain as suffixed sibling nodes, or one file per level. The
//! per-level geometry is regrouped back into meshes (one per source node), the
//! scene graph is either rebuilt or flattened, and the whole payload crosses
//! into C once per file as flat arrays — see `export_bridge.h`.
//!
//! ## What is written
//!
//! Everything the source authored that the optimization stack did not change.
//! With the source-property capture ([`SourceExtras`]) in hand, the export
//! writes the **whole** scene graph — every node, not only the mesh-bearing
//! ones — from each node's authored properties (local transform, rotation
//! order, pivots, pre/post rotations, geometric transform, inherit type,
//! visibility, user properties), its node attribute (bone, light, camera, null,
//! LOD group) with the attribute's own properties, every material with its
//! full property set on the shader model the file declared, the textures and
//! embedded videos those materials reference, the scene's settings and
//! metadata, and the authored tangent basis. Without the capture (an export in
//! the moments before it lands) the older computed path is used: the mesh
//! nodes' ancestor chains from their world transforms, and a Phong material
//! carrying the viewer's PBR figures.
//!
//! Materials are the ones the *source file* declared. Material edits made in
//! the viewer are previews of a look, not authored asset data, so writing them
//! into an exported mesh would quietly change the asset.

use std::collections::HashMap;
use std::ffi::{CString, c_char, c_int};
use std::path::{Path, PathBuf};

use glam::{Mat3, Mat4, Quat, Vec3};
use review_model::extras::{
    AttributeKind, CoordinateAxis, Prop, ShaderType, Synthetic, TextureKind,
};
use review_model::{ModelData, SourceExtras};

use crate::OptError;
use crate::process::{LevelCarry, NodePolygons, ProcessedLod};
use crate::stack::{ExportOptions, FbxFormat, HierarchyMode, LodPackaging};
use crate::submesh::NO_FACE;

/// A root node's parent index — `RVO_NO_PARENT` in `export_bridge.h`.
const NO_PARENT: i32 = -1;

/// `RVO_ATTRIB_*` in `export_bridge.h`.
const ATTRIB_NONE: u32 = 0;
const ATTRIB_BONE: u32 = 1;
const ATTRIB_LIGHT: u32 = 2;
const ATTRIB_CAMERA: u32 = 3;
const ATTRIB_NULL: u32 = 4;
const ATTRIB_LOD_GROUP: u32 = 5;

/// `RVO_SHADER_*` in `export_bridge.h`.
const SHADER_LAMBERT: u32 = 0;
const SHADER_PHONG: u32 = 1;
const SHADER_CUSTOM: u32 = 2;

/// FBX's `UnitScaleFactor` — centimeters per scene unit — for each unit a DCC
/// authors in. Mirrors `review-ui`'s `KNOWN_UNITS` table (its meters-per-unit
/// figures × 100), which is the same set import reads back off a file.
const KNOWN_UNIT_SCALES_CM: [f64; 6] = [100.0, 1.0, 0.1, 2.54, 30.48, 91.44];

/// `UnitScaleFactor` for a source that declared no unit (the demo cube, a file
/// missing the metadata): meters, which is what import normalizes to and so
/// what the geometry already is.
const DEFAULT_UNIT_SCALE_CM: f64 = 100.0;

/// The unit one export is written in — the *source file's* own, so a centimeter
/// asset comes back out of the tool as centimeters instead of being silently
/// re-authored in meters.
///
/// Import normalizes every file to meters, so the geometry reaching the writer
/// is metric whatever the file declared. FBX does not fix a unit, it *declares*
/// one, so honouring the source means two figures that have to agree: the
/// declared factor, and coordinates actually expressed in that unit.
#[derive(Debug, Clone, Copy)]
struct UnitScale {
    /// Exported units per meter — `100.0` for a centimeter file.
    ///
    /// Applied **once, at the top of the hierarchy**: `place_node` builds the
    /// chain against a root frame of `1/per_meter` rather than the identity, so
    /// the factor lands in the topmost emitted node's scale and the whole
    /// subtree rides it. Uniform scaling acts on an affine transform by
    /// conjugation, so scaling the root by `s` scales the assembled scene by
    /// `s` exactly — no scale node injected, and rotations and normals
    /// untouched.
    ///
    /// Applying it per-*local* value instead would double-count: import parks
    /// the unit normalization in the node transforms themselves, so a node's
    /// own inverse — which is what `build_mesh` puts the geometry through under
    /// [`HierarchyMode::Rebuild`] — already hands back the source file's unit.
    /// Only flat geometry, which stays in world meters, is scaled directly.
    ///
    /// With the source's authored properties in hand the question does not
    /// arise: every node's local transform is written as the file authored it,
    /// in the file's own unit, and the geometry comes back to that unit through
    /// the node's inverse exactly as above.
    per_meter: f32,
    /// Centimeters per exported unit, written as FBX's `UnitScaleFactor`.
    unit_scale_cm: f64,
}

impl UnitScale {
    /// `source_unit_meters` is [`review_model::ModelStats::source_unit_meters`]:
    /// meters per source unit as the file declared it, `0.0` for none.
    fn from_source(source_unit_meters: f32) -> Self {
        let declared = f64::from(source_unit_meters) * 100.0;
        let unit_scale_cm = if declared.is_finite() && declared > 0.0 {
            // Snap to the unit the DCC meant. The factor reaches us through an
            // f32, so a centimeter file's `0.01` becomes `0.99999998` here, and
            // writing that would leave the file declaring a unit no importer
            // names and every coordinate scaled by its reciprocal.
            KNOWN_UNIT_SCALES_CM
                .into_iter()
                .find(|known| (declared - known).abs() <= known * 0.001)
                .unwrap_or(declared)
        } else {
            DEFAULT_UNIT_SCALE_CM
        };
        Self {
            // Derived from the snapped factor rather than from the source figure
            // a second time: the declared unit and the coordinates must be exact
            // reciprocals, or the file says one thing and carries another.
            per_meter: (100.0 / unit_scale_cm) as f32,
            unit_scale_cm,
        }
    }
}

/// What an export actually produced, for the confirmation the user sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportReport {
    /// Every file written, in order.
    pub files: Vec<PathBuf>,
    pub mesh_count: usize,
    pub triangle_count: usize,
    /// Things worth saying about the result — a dropped skin, a node whose
    /// transform could not be inverted, properties that had not loaded.
    pub notes: Vec<String>,
}

/// True when this build has the vendored ufbx_write compiled in.
pub const fn available() -> bool {
    cfg!(has_ufbxw)
}

/// Write `lods` to `path` according to `options`.
///
/// `extras` is the source's property capture, when it has landed; without it
/// the export writes what the viewer shows and says so in the report.
///
/// `path` is the file the user chose; under [`LodPackaging::FilePerLod`] it
/// supplies the stem and each level gets a `_LOD<n>` suffix — unless there is
/// only one level, which is written to `path` itself.
pub fn export_fbx(
    lods: &[ProcessedLod],
    source: &ModelData,
    extras: Option<&SourceExtras>,
    path: &Path,
    options: &ExportOptions,
) -> Result<ExportReport, OptError> {
    let _z = crate::prof::zone!("Export FBX");

    if !available() {
        return Err(OptError::Unavailable);
    }
    if lods.is_empty() {
        return Err(OptError::EmptyMesh);
    }

    let mut report = ExportReport {
        files: Vec::new(),
        mesh_count: 0,
        triangle_count: 0,
        notes: Vec::new(),
    };
    if extras.is_none() {
        report.notes.push(
            "The source's properties had not finished loading: node transforms, materials and \
             scene settings were written from what the viewer shows, without textures or user \
             properties."
                .to_owned(),
        );
    }

    match options.packaging {
        LodPackaging::SingleFileSuffixed => {
            let scene = build_scene(lods, source, extras, options, &mut report)?;
            write_scene(&scene, path, options.format)?;
            report.files.push(path.to_path_buf());
        }
        LodPackaging::FilePerLod => {
            // A lone level is not a chain: there is nothing for a `_LOD0` suffix to
            // distinguish it from, and a stack that only reduces the mesh in place
            // is meant to stand in for the source asset — so it goes to the path
            // the user actually chose.
            let chain = lods.len() > 1;
            for lod in lods {
                // Each file holds one level, so its meshes keep their plain names
                // rather than a suffix the filename already carries.
                let scene = build_scene(
                    std::slice::from_ref(lod),
                    source,
                    extras,
                    options,
                    &mut report,
                )?;
                let level_path = if chain {
                    level_path(path, lod.level)
                } else {
                    path.to_path_buf()
                };
                write_scene(&scene, &level_path, options.format)?;
                report.files.push(level_path);
            }
        }
    }

    report.notes.dedup();
    Ok(report)
}

/// `asset.fbx` → `asset_LOD2.fbx`.
fn level_path(path: &Path, level: usize) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "mesh".to_owned());
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().into_owned())
        .unwrap_or_else(|| "fbx".to_owned());
    path.with_file_name(format!("{stem}_LOD{level}.{extension}"))
}

// ---------------------------------------------------------------------------
// Scene assembly
// ---------------------------------------------------------------------------

/// A range of [`SceneData::props`]: `(first, count)`.
#[derive(Debug, Clone, Copy, Default)]
struct PropRange {
    first: u32,
    count: u32,
}

/// One authored property, owned in C-ready form.
struct PropData {
    name: CString,
    kind: u32,
    flags: u32,
    value_int: i64,
    value_real: [f64; 4],
    value_str: CString,
    blob: Vec<u8>,
}

/// The owned Rust side of a scene payload. Every buffer the bridge borrows lives
/// here, so the whole thing must outlive the `review_export_fbx` call — which
/// it does, since [`write_scene`] takes it by reference.
struct ClusterData {
    bone: i32,
    name: CString,
    transform: [f64; 16],
    transform_link: [f64; 16],
    vertices: Vec<i32>,
    weights: Vec<f64>,
}

struct SkinExportData {
    skinning_type: u32,
    clusters: Vec<ClusterData>,
    dq_vertices: Vec<i32>,
    dq_weights: Vec<f64>,
    bind_pose: i32,
}

struct BlendShapeData {
    name: CString,
    vertices: Vec<i32>,
    offsets: Vec<f64>,
    normals: Vec<f64>,
    target_weight: f64,
}

struct BlendChannelData {
    name: CString,
    weight: f64,
    shapes: Vec<BlendShapeData>,
}

struct PoseData {
    name: CString,
    /// `(scene node, matrix)`.
    nodes: Vec<(i32, [f64; 16])>,
}

#[derive(Clone, Copy)]
struct KeyData {
    time: i64,
    value: f64,
    flags: u32,
    weight_left: f64,
    weight_right: f64,
    slope_left: f64,
    slope_right: f64,
}

#[derive(Clone)]
struct CurveData {
    keys: Vec<KeyData>,
    pre_mode: u32,
    pre_repeat: i32,
    post_mode: u32,
    post_repeat: i32,
}

struct AnimPropData {
    target_kind: u32,
    target: i32,
    target2: i32,
    prop_name: CString,
    default: [f64; 3],
    curves: [Option<CurveData>; 3],
}

struct AnimLayerData {
    name: CString,
    stack: i32,
    weight: f64,
    props: PropRange,
    anim_props: Vec<AnimPropData>,
}

struct AnimStackData {
    name: CString,
    props: PropRange,
    time_begin: i64,
    time_end: i64,
}

struct DisplayLayerData {
    name: CString,
    props: PropRange,
    nodes: Vec<i32>,
}

struct SelectionNodeData {
    node: i32,
    include_node: bool,
    vertices: Vec<i32>,
    edges: Vec<i32>,
    faces: Vec<i32>,
}

struct SelectionSetData {
    name: CString,
    props: PropRange,
    nodes: Vec<SelectionNodeData>,
}

struct SceneData {
    /// The unit every coordinate below is expressed in, and the factor the file
    /// declares so a reader recovers it.
    unit: UnitScale,
    /// Every authored property any element references, by range.
    props: Vec<PropData>,
    settings: SettingsData,
    nodes: Vec<NodeData>,
    materials: Vec<MaterialData>,
    textures: Vec<TextureData>,
    videos: Vec<VideoData>,
    meshes: Vec<MeshData>,
    poses: Vec<PoseData>,
    /// Capture texture index → scene texture index, or -1.
    texture_map: Vec<i32>,
    anim_stacks: Vec<AnimStackData>,
    anim_layers: Vec<AnimLayerData>,
    active_stack: i32,
    display_layers: Vec<DisplayLayerData>,
    selection_sets: Vec<SelectionSetData>,
    /// Source node → the suffixed per-level copies made of it.
    level_copies: HashMap<usize, Vec<i32>>,
}

struct SettingsData {
    /// `ufbxw_coordinate_axis` codes, or -1 for the writer's default.
    axes: [i32; 3],
    time_mode: i32,
    frame_rate: f64,
    settings_props: PropRange,
    scene_info_props: PropRange,
    original_application: [CString; 3],
    original_filename: CString,
    application_name: CString,
    application_version: CString,
}

struct NodeData {
    name: CString,
    parent: i32,
    translation: [f64; 3],
    rotation: [f64; 4],
    scaling: [f64; 3],
    authored_transform: bool,
    props: PropRange,
    attribute_kind: u32,
    attribute_name: CString,
    attribute_props: PropRange,
}

struct MaterialData {
    name: CString,
    shader: u32,
    shading_model: CString,
    props: PropRange,
    base_color: [f64; 3],
    emissive: [f64; 3],
    shininess_exponent: f64,
    reflection_factor: f64,
    /// `(material property, texture index)`.
    textures: Vec<(CString, i32)>,
}

struct TextureData {
    name: CString,
    layered: bool,
    filename: CString,
    relative_filename: CString,
    content: Vec<u8>,
    video: i32,
    props: PropRange,
    /// `(texture index, blend mode, alpha)`.
    layers: Vec<(i32, i32, f64)>,
}

struct VideoData {
    name: CString,
    filename: CString,
    relative_filename: CString,
    content: Vec<u8>,
    props: PropRange,
}

struct MeshData {
    name: CString,
    node: i32,
    positions: Vec<f64>,
    /// The polygon-vertex stream, cut by `face_offsets`.
    indices: Vec<i32>,
    face_offsets: Vec<i32>,
    /// How many triangles the faces amount to, for the report.
    triangle_count: usize,
    vertex_count: usize,
    normals: Vec<f64>,
    colors: Vec<f64>,
    /// `4 * vertex_count` (xyz + handedness), empty when the source authored
    /// no tangent layer.
    tangents: Vec<f64>,
    uv_sets: Vec<Vec<f64>>,
    uv_set_names: Vec<CString>,
    material_slots: Vec<i32>,
    /// Per face; empty for a single-material mesh.
    face_materials: Vec<i32>,
    color_set_name: Option<CString>,
    /// Sets beyond the first: name and `4 * vertex_count` values.
    color_sets: Vec<(CString, Vec<f64>)>,
    /// Per local vertex, the level vertex it came from (Rust-side only).
    level_vertices: Vec<u32>,
    /// Per local vertex, a source corner it came from (`u32::MAX` unknown).
    source_corners: Vec<u32>,
    /// Per exported face / edge, the source face / part edge (`u32::MAX` for
    /// one the stack made).
    face_sources: Vec<u32>,
    edge_sources: Vec<u32>,
    /// Per exported blend channel, the model's channel index.
    channel_sources: Vec<u32>,
    source_node: Option<usize>,
    level: usize,
    skins: Vec<SkinExportData>,
    blend_channels: Vec<BlendChannelData>,
    /// Per face, empty when the source carried no such layer.
    face_smoothing: Vec<u8>,
    face_hole: Vec<u8>,
    face_group: Vec<i32>,
    /// Edges as corner positions, and their layers (empty when absent).
    edges: Vec<i32>,
    edge_smoothing: Vec<u8>,
    edge_crease: Vec<f64>,
    edge_visibility: Vec<u8>,
    /// Per vertex, empty when absent.
    vertex_crease: Vec<f64>,
    props: PropRange,
}

impl SceneData {
    fn new(unit: UnitScale) -> Self {
        Self {
            unit,
            props: Vec::new(),
            settings: SettingsData {
                axes: [-1; 3],
                time_mode: -1,
                frame_rate: 0.0,
                settings_props: PropRange::default(),
                scene_info_props: PropRange::default(),
                original_application: [CString::default(), CString::default(), CString::default()],
                original_filename: CString::default(),
                application_name: c_string("3D Review", "3D Review"),
                application_version: c_string(env!("CARGO_PKG_VERSION"), ""),
            },
            nodes: Vec::new(),
            materials: Vec::new(),
            textures: Vec::new(),
            videos: Vec::new(),
            meshes: Vec::new(),
            poses: Vec::new(),
            texture_map: Vec::new(),
            anim_stacks: Vec::new(),
            anim_layers: Vec::new(),
            active_stack: -1,
            display_layers: Vec::new(),
            selection_sets: Vec::new(),
            level_copies: HashMap::new(),
        }
    }

    /// Append `props` to the table and return their range. A property whose
    /// name cannot be a C string is dropped rather than misnamed.
    fn push_props(&mut self, props: &[Prop]) -> PropRange {
        let first = self.props.len() as u32;
        for prop in props {
            let Ok(name) = CString::new(prop.name.replace('\0', "")) else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            self.props.push(PropData {
                name,
                kind: prop.kind.code(),
                flags: prop.flags.0,
                value_int: prop.value_int,
                value_real: prop.value_real,
                value_str: c_string_or_empty(&prop.value_str),
                blob: prop.value_blob.clone(),
            });
        }
        PropRange {
            first,
            count: self.props.len() as u32 - first,
        }
    }

    fn mesh_count_into(&self, report: &mut ExportReport) {
        report.mesh_count += self.meshes.len();
    }
}

/// Build the payload for one output file.
fn build_scene(
    lods: &[ProcessedLod],
    source: &ModelData,
    extras: Option<&SourceExtras>,
    options: &ExportOptions,
    report: &mut ExportReport,
) -> Result<SceneData, OptError> {
    let mut scene = SceneData::new(UnitScale::from_source(source.stats.source_unit_meters));

    if let Some(extras) = extras {
        build_settings(&mut scene, extras);
        let texture_map = build_textures(&mut scene, extras, report);
        build_materials_from_extras(&mut scene, source, extras, &texture_map);
        scene.texture_map = texture_map;
    } else {
        build_materials_from_viewer(&mut scene, source);
    }

    // Whether more than one level shares this file decides whether meshes need a
    // `_LOD<n>` suffix to stay distinguishable.
    let suffix_levels = lods.len() > 1;

    // Source node index → scene node index for every node already emitted into
    // this file, so meshes sharing ancestors share one chain (see `place_node`).
    let mut placed = HashMap::new();

    // With the capture, the whole authored graph goes out first — every node,
    // mesh-bearing or not — so lights, cameras, empties and bones survive.
    if let Some(extras) = extras
        && options.hierarchy == HierarchyMode::Rebuild
    {
        place_graph(&mut scene, &mut placed, source, extras);
        build_poses(&mut scene, &mut placed, source, extras, report);
    }
    let deforms = source.skin.is_some() || source.morph.is_some();
    if deforms && options.hierarchy == HierarchyMode::FlatBaked {
        report.notes.push(
            "Skinning and blend shapes were not written: a flat hierarchy bakes the geometry \
             into world space, which has no bones to bind to."
                .to_owned(),
        );
    }

    for lod in lods {
        let groups = group_by_node(&lod.model);
        for group in groups {
            let node_index = match (extras, options.hierarchy) {
                (Some(extras), HierarchyMode::Rebuild) => {
                    place_level_node(&mut scene, &placed, source, extras, &group, lod.level)
                }
                _ => {
                    let (node_index, notes) = place_node(
                        &mut scene,
                        &mut placed,
                        source,
                        &group,
                        options.hierarchy,
                        lod.level,
                    );
                    report.notes.extend(notes);
                    node_index
                }
            };

            let (mut mesh, notes) = build_mesh(
                &lod.model,
                &lod.carry,
                &group,
                node_index,
                suffix_levels.then_some(lod.level),
                options.hierarchy,
                source,
                extras,
                scene.unit,
            );
            if let Some(part) = group
                .source_node
                .zip(extras)
                .and_then(|(index, extras)| extras.mesh_of_node(index as u32))
            {
                mesh.props = scene.push_props(&part.props);
            }
            let mesh_name = mesh.name.to_string_lossy().into_owned();
            if notes.polygons_lost {
                report.notes.push(format!(
                    "LOD {}: '{mesh_name}' was written as triangles — the stack rebuilt its geometry.",
                    lod.level
                ));
            } else if notes.triangles_rebuilt > 0 {
                report.notes.push(format!(
                    "LOD {}: {} triangle(s) of '{mesh_name}' were written as triangles — the stack \
                     rebuilt them.",
                    lod.level, notes.triangles_rebuilt
                ));
            }
            if options.hierarchy == HierarchyMode::Rebuild {
                build_deform(
                    &mut scene,
                    &mut mesh,
                    &mut placed,
                    lod,
                    &group,
                    source,
                    extras,
                    report,
                );
            }
            report.triangle_count += mesh.triangle_count;
            scene.meshes.push(mesh);
        }
    }

    if let Some(extras) = extras
        && options.hierarchy == HierarchyMode::Rebuild
    {
        build_display_layers(&mut scene, &placed, extras);
        build_selection_sets(&mut scene, &placed, source, extras, report);
        build_animation(&mut scene, &placed, extras, report);
    } else if extras.is_some_and(|extras| !extras.animations.is_empty()) {
        report.notes.push(
            "Animation was not written: a flat hierarchy has no nodes for the curves to drive."
                .to_owned(),
        );
    }

    scene.mesh_count_into(report);
    if scene.meshes.is_empty() {
        return Err(OptError::EmptyMesh);
    }
    Ok(scene)
}

/// The `ufbxw_time_mode` code for a frame rate, when it is one of the standard
/// modes; `None` means custom.
fn time_mode_for(fps: f64) -> Option<i32> {
    const MODES: [(f64, i32); 12] = [
        (120.0, 1),
        (100.0, 2),
        (60.0, 3),
        (50.0, 4),
        (48.0, 5),
        (30.0, 6),
        (25.0, 10),
        (24.0, 11),
        (1000.0, 12),
        (96.0, 15),
        (72.0, 16),
        (59.94, 17),
    ];
    MODES
        .into_iter()
        .find(|(rate, _)| (fps - rate).abs() < 1e-3)
        .map(|(_, mode)| mode)
}

fn axis_code(axis: CoordinateAxis) -> i32 {
    match axis {
        CoordinateAxis::Unknown | CoordinateAxis::Unnamed(_) => -1,
        other => other.code() as i32,
    }
}

/// Scene settings and metadata from the capture.
fn build_settings(scene: &mut SceneData, extras: &SourceExtras) {
    let source = &extras.scene;
    let axes = [
        axis_code(source.axes[0]),
        axis_code(source.axes[1]),
        axis_code(source.axes[2]),
    ];
    // Only a complete, known triple is declared; a partial one would describe
    // a frame no reader could resolve.
    scene.settings.axes = if axes.iter().all(|&axis| axis >= 0) {
        axes
    } else {
        [-1; 3]
    };
    // The source's own mode when it was a standard one; otherwise its rate as
    // a custom mode, so the clips play at the speed they were authored.
    let fps = source.frames_per_second;
    scene.settings.time_mode = match source.time_mode {
        review_model::extras::TimeMode::Custom | review_model::extras::TimeMode::Default => {
            time_mode_for(fps).unwrap_or(14)
        }
        other => other.code() as i32,
    };
    scene.settings.frame_rate = fps;
    scene.settings.settings_props = scene.push_props(&source.settings_props);
    scene.settings.scene_info_props = scene.push_props(&source.scene_props);
    scene.settings.original_application = [
        c_string_or_empty(&source.original_application.vendor),
        c_string_or_empty(&source.original_application.name),
        c_string_or_empty(&source.original_application.version),
    ];
    scene.settings.original_filename = c_string_or_empty(&source.original_file_path);
}

/// Emit every texture and video of the capture. Returns the capture's texture
/// index → scene texture index map (`-1` for one that could not be written).
///
/// Videos go out as they are. Textures are reordered so that every layer of a
/// layered texture precedes it — the bridge assembles a layered texture from
/// textures that already exist — which means a plain file texture always comes
/// first and a layered one after its members.
fn build_textures(
    scene: &mut SceneData,
    extras: &SourceExtras,
    report: &mut ExportReport,
) -> Vec<i32> {
    for video in &extras.videos {
        let props = scene.push_props(&video.props);
        scene.videos.push(VideoData {
            name: c_string_or_empty(&video.name),
            filename: c_string_or_empty(&video.absolute_filename),
            relative_filename: c_string_or_empty(&video.relative_filename),
            content: video.content.clone(),
            props,
        });
    }

    let mut map = vec![-1i32; extras.textures.len()];
    // Two passes: file textures, then layered ones whose layers are all placed.
    for (index, texture) in extras.textures.iter().enumerate() {
        if texture.kind != TextureKind::Layered {
            map[index] = push_texture(scene, texture, &map, false);
        }
    }
    let mut dropped = 0usize;
    for (index, texture) in extras.textures.iter().enumerate() {
        if texture.kind == TextureKind::Layered {
            if texture
                .layers
                .iter()
                .any(|layer| map.get(layer.texture as usize).copied().unwrap_or(-1) < 0)
            {
                // A layered texture nested inside another: not something the
                // exporter reassembles, and a texture no material can reach is
                // not worth a partial copy.
                dropped += 1;
                continue;
            }
            map[index] = push_texture(scene, texture, &map, true);
        }
    }
    if dropped > 0 {
        report.notes.push(format!(
            "{dropped} layered texture(s) nested inside another layered texture were not written."
        ));
    }
    map
}

fn push_texture(
    scene: &mut SceneData,
    texture: &review_model::extras::TextureExtras,
    map: &[i32],
    layered: bool,
) -> i32 {
    let props = scene.push_props(&texture.props);
    let video = texture
        .video
        .filter(|&video| (video as usize) < scene.videos.len())
        .map_or(-1, |video| video as i32);
    // Embedded bytes ride the video when there is one (a texture's content is
    // its video's), else the texture itself.
    let content = if video >= 0 {
        Vec::new()
    } else {
        texture.content.clone()
    };
    scene.textures.push(TextureData {
        name: c_string_or_empty(&texture.name),
        layered,
        filename: c_string_or_empty(&texture.absolute_filename),
        relative_filename: c_string_or_empty(&texture.relative_filename),
        content,
        video,
        props,
        layers: texture
            .layers
            .iter()
            .map(|layer| {
                (
                    map[layer.texture as usize],
                    layer.blend_mode.code() as i32,
                    layer.alpha,
                )
            })
            .collect(),
    });
    (scene.textures.len() - 1) as i32
}

/// Materials from the capture: the shader model the file declared, every
/// authored property, and the texture connections.
fn build_materials_from_extras(
    scene: &mut SceneData,
    source: &ModelData,
    extras: &SourceExtras,
    texture_map: &[i32],
) {
    for (index, material) in source.materials.iter().enumerate() {
        let name = c_string(&material.name, &format!("Material{index}"));
        let Some(authored) = extras.materials.get(index) else {
            scene.materials.push(viewer_material(name, material));
            continue;
        };
        let shader = match authored.shader_type {
            ShaderType::FbxLambert => SHADER_LAMBERT,
            ShaderType::FbxPhong => SHADER_PHONG,
            // A material ufbx could not classify still declared *some* model;
            // written as that custom name with its properties, so a reader that
            // knows the model recovers it. One that declared none is a Phong.
            ShaderType::Unknown if authored.shading_model.is_empty() => SHADER_PHONG,
            _ => SHADER_CUSTOM,
        };
        let props = scene.push_props(&authored.props);
        let textures = authored
            .textures
            .iter()
            .filter_map(|texture| {
                let index = *texture_map.get(texture.texture as usize)?;
                (index >= 0).then(|| (c_string_or_empty(&texture.material_prop), index))
            })
            .filter(|(prop, _)| !prop.is_empty())
            .collect();
        scene.materials.push(MaterialData {
            name,
            shader,
            shading_model: c_string_or_empty(&authored.shading_model),
            props,
            base_color: [0.0; 3],
            emissive: [0.0; 3],
            shininess_exponent: 0.0,
            reflection_factor: 0.0,
            textures,
        });
    }
}

/// Materials from what the viewer shows, for an export before the capture
/// landed: a Phong carrying the PBR figures on its classic slots.
fn build_materials_from_viewer(scene: &mut SceneData, source: &ModelData) {
    for (index, material) in source.materials.iter().enumerate() {
        let name = c_string(&material.name, &format!("Material{index}"));
        scene.materials.push(viewer_material(name, material));
    }
}

fn viewer_material(name: CString, material: &review_model::MaterialImportDefaults) -> MaterialData {
    // Import reads a Phong's shininess as `roughness = 1 − 0.1·√exponent`, so
    // the exponent that reads back as this smoothness is `(10·smoothness)²`.
    let shininess_exponent = f64::from(10.0 * material.smoothness).powi(2);
    MaterialData {
        name,
        shader: SHADER_PHONG,
        shading_model: CString::default(),
        props: PropRange::default(),
        base_color: [
            f64::from(material.base_color.x),
            f64::from(material.base_color.y),
            f64::from(material.base_color.z),
        ],
        emissive: [
            f64::from(material.emissive.x),
            f64::from(material.emissive.y),
            f64::from(material.emissive.z),
        ],
        shininess_exponent,
        reflection_factor: f64::from(material.metallic),
        textures: Vec::new(),
    }
}

/// One mesh's worth of a processed level: the triangles owned by a single source
/// node, and the compacted vertex set they reference.
struct NodeGroup {
    /// Index into `ModelData::nodes`, or `None` when the model carries no node
    /// tags (everything then becomes one mesh).
    source_node: Option<usize>,
    /// Triangle indices into the level's index buffer.
    triangles: Vec<usize>,
}

/// Split a processed level into per-source-node groups, in first-seen order.
fn group_by_node(model: &ModelData) -> Vec<NodeGroup> {
    let triangle_count = model.indices.len() / 3;
    if triangle_count == 0 {
        return Vec::new();
    }
    if model.triangles.node.len() != triangle_count {
        return vec![NodeGroup {
            source_node: None,
            triangles: (0..triangle_count).collect(),
        }];
    }

    let mut order: Vec<u32> = Vec::new();
    let mut groups: Vec<NodeGroup> = Vec::new();
    for triangle in 0..triangle_count {
        let node = model.triangles.node[triangle];
        let slot = match order.iter().position(|&seen| seen == node) {
            Some(slot) => slot,
            None => {
                order.push(node);
                groups.push(NodeGroup {
                    source_node: Some(node as usize),
                    triangles: Vec::new(),
                });
                groups.len() - 1
            }
        };
        groups[slot].triangles.push(triangle);
    }
    groups
}

/// The `RVO_ATTRIB_*` kind for a captured attribute.
fn attribute_code(kind: AttributeKind) -> u32 {
    match kind {
        AttributeKind::Bone => ATTRIB_BONE,
        AttributeKind::Light => ATTRIB_LIGHT,
        AttributeKind::Camera => ATTRIB_CAMERA,
        AttributeKind::Empty => ATTRIB_NULL,
        AttributeKind::LodGroup => ATTRIB_LOD_GROUP,
        // A mesh attribute is the geometry itself; anything else has no writer.
        _ => ATTRIB_NONE,
    }
}

/// Whether a source node is one the importer made up rather than the file
/// authored: the synthetic file root and the scale helpers ufbx inserts for
/// non-standard inherit modes. Neither is written; the authored `InheritType`
/// lets a reader re-derive the helper, and the file has its own root.
fn is_synthetic(source: &ModelData, extras: &SourceExtras, index: usize) -> bool {
    match extras.nodes.get(index).map(|node| node.synthetic) {
        Some(Synthetic::Root | Synthetic::ScaleHelper) => true,
        Some(_) => false,
        // No capture entry (never, given `validate`) — fall back to the
        // structural test the computed path uses.
        None => source.nodes.get(index).is_some_and(|node| {
            node.parent.is_none() && node.name.is_empty() && node.mesh_part.is_none()
        }),
    }
}

/// The nearest authored ancestor of `index`, skipping synthetic nodes.
fn authored_parent(source: &ModelData, extras: &SourceExtras, index: usize) -> Option<usize> {
    let mut cursor = source.nodes.get(index)?.parent;
    let mut steps = 0;
    while let Some(parent) = cursor {
        if !is_synthetic(source, extras, parent) {
            return Some(parent);
        }
        cursor = source.nodes.get(parent)?.parent;
        steps += 1;
        if steps > source.nodes.len() {
            return None;
        }
    }
    None
}

/// Emit one authored node (after its authored parent) with the properties the
/// file gave it, returning its scene index. `placed` memoizes the walk so
/// shared ancestors are emitted once, in parent-first order.
fn emit_authored_node(
    scene: &mut SceneData,
    placed: &mut HashMap<usize, i32>,
    source: &ModelData,
    extras: &SourceExtras,
    index: usize,
    depth: usize,
) -> Option<i32> {
    if let Some(&existing) = placed.get(&index) {
        return Some(existing);
    }
    // A parent cycle would recurse forever; the chain can never legitimately be
    // deeper than the node table.
    if depth > source.nodes.len() {
        return None;
    }
    let node = source.nodes.get(index)?;
    let authored = extras.nodes.get(index)?;
    let parent = match authored_parent(source, extras, index) {
        Some(parent) => emit_authored_node(scene, placed, source, extras, parent, depth + 1)
            .unwrap_or(NO_PARENT),
        None => NO_PARENT,
    };
    let data = authored_node_data(scene, node, authored, parent, &node.name);
    scene.nodes.push(data);
    let placed_index = (scene.nodes.len() - 1) as i32;
    placed.insert(index, placed_index);
    Some(placed_index)
}

/// The node payload for `node` written from its authored properties.
fn authored_node_data(
    scene: &mut SceneData,
    node: &review_model::SceneNode,
    authored: &review_model::extras::NodeExtras,
    parent: i32,
    name: &str,
) -> NodeData {
    let props = scene.push_props(&authored.props);
    let (attribute_kind, attribute_name, attribute_props) = match &authored.attribute {
        Some(attribute) => (
            attribute_code(attribute.kind),
            c_string_or_empty(&attribute.name),
            scene.push_props(&attribute.props),
        ),
        None => (ATTRIB_NONE, CString::default(), PropRange::default()),
    };
    // The computed transform is kept beside the authored properties only as
    // the fallback the bridge never reads when `authored_transform` is set.
    let local = node.rest_local;
    NodeData {
        name: c_string(name, "Node"),
        parent,
        translation: [
            f64::from(local.translation.x),
            f64::from(local.translation.y),
            f64::from(local.translation.z),
        ],
        rotation: [
            f64::from(local.rotation.x),
            f64::from(local.rotation.y),
            f64::from(local.rotation.z),
            f64::from(local.rotation.w),
        ],
        scaling: [
            f64::from(local.scale.x),
            f64::from(local.scale.y),
            f64::from(local.scale.z),
        ],
        authored_transform: true,
        props,
        attribute_kind,
        attribute_name,
        attribute_props,
    }
}

/// Emit the whole authored scene graph, parent-first, into `placed`.
fn place_graph(
    scene: &mut SceneData,
    placed: &mut HashMap<usize, i32>,
    source: &ModelData,
    extras: &SourceExtras,
) {
    for index in 0..source.nodes.len() {
        if is_synthetic(source, extras, index) {
            continue;
        }
        emit_authored_node(scene, placed, source, extras, index, 0);
    }
}

/// The scene node a level's mesh attaches to, with the whole graph already
/// placed: level 0 uses the source node itself; a later level gets a suffixed
/// sibling copy — same authored properties and attribute — beside it.
fn place_level_node(
    scene: &mut SceneData,
    placed: &HashMap<usize, i32>,
    source: &ModelData,
    extras: &SourceExtras,
    group: &NodeGroup,
    level: usize,
) -> i32 {
    let Some(index) = group.source_node else {
        // No node tags at all: one root holds the whole level.
        scene.nodes.push(NodeData {
            name: c_string(&suffixed(&source.name, level), "Mesh"),
            parent: NO_PARENT,
            translation: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scaling: [1.0; 3],
            authored_transform: false,
            props: PropRange::default(),
            attribute_kind: ATTRIB_NONE,
            attribute_name: CString::default(),
            attribute_props: PropRange::default(),
        });
        return (scene.nodes.len() - 1) as i32;
    };
    let base = placed.get(&index).copied().unwrap_or(NO_PARENT);
    if level == 0 || base == NO_PARENT {
        return base;
    }
    let (Some(node), Some(authored)) = (source.nodes.get(index), extras.nodes.get(index)) else {
        return base;
    };
    let parent = scene.nodes[base as usize].parent;
    let data = authored_node_data(scene, node, authored, parent, &suffixed(&node.name, level));
    scene.nodes.push(data);
    let copy = (scene.nodes.len() - 1) as i32;
    scene.level_copies.entry(index).or_default().push(copy);
    copy
}

/// Create (or reuse) the scene node this group's mesh attaches to, returning its
/// index plus any notes raised while working out its transform. The computed
/// path, used without the source-property capture.
///
/// Under [`HierarchyMode::Rebuild`] the source node's ancestors are emitted too,
/// each with the local transform implied by the world transforms import
/// recorded. `placed` remembers every node already emitted into this scene, so
/// meshes with common ancestors — sibling LODs under one group node, or one
/// node's levels in a suffixed chain — hang off a single shared chain rather
/// than each duplicating it from the root. Only the mesh-bearing leaf carries
/// the level suffix, which puts a chain's levels beside each other as siblings —
/// the layout game FBX files use for their own LOD chains. The importer's
/// synthetic file root (nameless, meshless, parentless) is not re-emitted at
/// all: the written file has its own root, so an explicit copy would wrap every
/// re-import in one extra level.
///
/// Under [`HierarchyMode::FlatBaked`] a single identity root node is created per
/// mesh instead.
fn place_node(
    scene: &mut SceneData,
    placed: &mut HashMap<usize, i32>,
    source: &ModelData,
    group: &NodeGroup,
    hierarchy: HierarchyMode,
    level: usize,
) -> (i32, Vec<String>) {
    let mut notes = Vec::new();

    let plain = |name: CString, parent: i32, local: Mat4| -> NodeData {
        let (scale, rotation, translation) = local.to_scale_rotation_translation();
        let rotation = if rotation.is_finite() {
            rotation
        } else {
            Quat::IDENTITY
        };
        let scale = if scale.is_finite() { scale } else { Vec3::ONE };
        NodeData {
            name,
            parent,
            translation: [
                f64::from(translation.x),
                f64::from(translation.y),
                f64::from(translation.z),
            ],
            rotation: [
                f64::from(rotation.x),
                f64::from(rotation.y),
                f64::from(rotation.z),
                f64::from(rotation.w),
            ],
            scaling: [f64::from(scale.x), f64::from(scale.y), f64::from(scale.z)],
            authored_transform: false,
            props: PropRange::default(),
            attribute_kind: ATTRIB_NONE,
            attribute_name: CString::default(),
            attribute_props: PropRange::default(),
        }
    };

    let Some(source_index) = group
        .source_node
        .filter(|_| hierarchy == HierarchyMode::Rebuild)
    else {
        // Flat: one identity root per mesh, holding world-space geometry.
        let name = group
            .source_node
            .and_then(|index| source.nodes.get(index))
            .map(|node| node.name.clone())
            .unwrap_or_else(|| source.name.clone());
        scene.nodes.push(plain(
            c_string(&suffixed(&name, level), "Mesh"),
            NO_PARENT,
            Mat4::IDENTITY,
        ));
        return ((scene.nodes.len() - 1) as i32, notes);
    };

    // Emit the ancestor chain root-first, so every parent exists (and precedes
    // its child in the array) before the child is created — which is exactly the
    // ordering the bridge validates.
    let mut chain = Vec::new();
    let mut cursor = Some(source_index);
    while let Some(index) = cursor {
        chain.push(index);
        cursor = source.nodes.get(index).and_then(|node| node.parent);
        // A cycle in the imported hierarchy would loop forever; the chain can
        // never legitimately be longer than the node table.
        if chain.len() > source.nodes.len() {
            notes.push("A node's parent chain looped; that branch was exported flat.".to_owned());
            chain.truncate(1);
            break;
        }
    }
    chain.reverse();

    let mut parent = NO_PARENT;
    // The frame the chain hangs off is the *written file's* root, whose unit is
    // the source's — not the meter import normalized everything to. Starting
    // the walk at `1/per_meter` instead of the identity is what puts the unit
    // conversion into the topmost emitted node's scale, once, for the whole
    // subtree. Everything below it then computes `parent⁻¹ × child` in the
    // source's own unit, because that is the unit import's node transforms are
    // already expressed in (it parks the normalization at the synthetic root,
    // which the skip below folds in here).
    let mut parent_world = Mat4::from_scale(Vec3::splat(1.0 / scene.unit.per_meter));
    for &index in &chain {
        let Some(node) = source.nodes.get(index) else {
            continue;
        };
        let leaf = index == source_index;
        // The synthetic file root — skipped, per above. (Never the leaf, so the
        // mesh always has a real node to attach to.) `parent_world` is left
        // alone so its transform — import parks the unit normalization there —
        // folds into its children's locals instead of vanishing.
        if !leaf && node.parent.is_none() && node.name.is_empty() && node.mesh_part.is_none() {
            continue;
        }
        // Reuse a node an earlier group already emitted. A suffixed leaf is a
        // per-level variant of its source node, so it is never shared.
        let suffix = leaf && level != 0;
        if !suffix && let Some(&existing) = placed.get(&index) {
            parent = existing;
            parent_world = node.transform;
            continue;
        }
        // Local = inverse(parent world) * world. A non-invertible parent (a zero
        // scale axis) leaves the child at the parent's origin rather than
        // producing NaNs.
        let inverse = parent_world.inverse();
        let local = if inverse.is_finite() {
            inverse * node.transform
        } else {
            notes.push(format!(
                "'{}' has a non-invertible transform; its children were exported \
                 relative to it without it.",
                node.name
            ));
            node.transform
        };

        let name = if suffix {
            suffixed(&node.name, level)
        } else {
            node.name.clone()
        };
        scene
            .nodes
            .push(plain(c_string(&name, "Node"), parent, local));
        parent = (scene.nodes.len() - 1) as i32;
        if !suffix {
            placed.insert(index, parent);
        }
        parent_world = node.transform;
    }

    (parent, notes)
}

/// `"Body"` at level 0, `"Body_LOD2"` above it. Level 0 keeps the plain name so
/// a chain-less export round-trips with the source's own names.
fn suffixed(name: &str, level: usize) -> String {
    if level == 0 {
        name.to_owned()
    } else {
        format!("{name}_LOD{level}")
    }
}

/// Build one mesh's arrays from `group`'s triangles: the vertices they reach,
/// and the polygon-vertex stream cut into the source faces the level's carry
/// preserved — a triangle no face survived for goes out as a triangle.
#[allow(clippy::too_many_arguments)]
fn build_mesh(
    model: &ModelData,
    carry: &LevelCarry,
    group: &NodeGroup,
    node: i32,
    suffix_level: Option<usize>,
    hierarchy: HierarchyMode,
    source: &ModelData,
    extras: Option<&SourceExtras>,
    unit: UnitScale,
) -> (MeshData, MeshNotes) {
    let source_node = group.source_node.and_then(|index| source.nodes.get(index));
    let part = group
        .source_node
        .zip(extras)
        .and_then(|(index, extras)| extras.mesh_of_node(index as u32));

    // Geometry is world-baked; under `Rebuild` it has to move back into the
    // owning node's local space, or it would be transformed twice on import.
    // With the capture that space is the *geometry* space — the node's
    // geometric transform (written back as its `Geometric*` properties) sits
    // between the two.
    let node_world = (hierarchy == HierarchyMode::Rebuild)
        .then(|| {
            let node = source_node?;
            let geometry_to_node = group
                .source_node
                .zip(extras)
                .and_then(|(index, extras)| extras.nodes.get(index))
                .map_or(Mat4::IDENTITY, |authored| authored.geometry_to_node);
            let world = node.transform * geometry_to_node;
            world.inverse().is_finite().then_some(world)
        })
        .flatten();
    let to_local = node_world.map(|world| world.inverse());
    // A normal rides the *inverse transpose* of the map its positions take, and
    // that map is `to_local` — so the normal's matrix is the transpose of the
    // node's own world transform, not of its inverse. The two agree for a
    // uniform scale, which is why only a rotated node shows the difference: it
    // rotates every normal the wrong way round.
    let normal_to_local = node_world.map(|world| Mat3::from_mat4(world).transpose());
    // Tangents are directions along the surface, so they take the same map as
    // the positions (without the translation), not the normal's.
    let direction_to_local = to_local.map(Mat3::from_mat4);

    let channel_count = model.uv_channels.len();
    let has_uv = channel_count > 0 || !model.vertices.is_empty();
    let write_tangents = part.is_some_and(|part| part.tangents_authored);
    let color_set_count = carry.color_channels.len();
    let has_crease = !carry.vertex_crease.is_empty();

    let mut local_of_global: Vec<i32> = vec![-1; model.vertices.len()];
    let mut positions: Vec<f64> = Vec::new();
    let mut normals: Vec<f64> = Vec::new();
    let mut colors: Vec<f64> = Vec::new();
    let mut tangents: Vec<f64> = Vec::new();
    let mut uv_sets: Vec<Vec<f64>> = vec![Vec::new(); channel_count.max(usize::from(has_uv))];
    let mut color_values: Vec<Vec<f64>> = vec![Vec::new(); color_set_count];
    let mut vertex_crease: Vec<f64> = Vec::new();

    // An out-of-range index means a malformed mesh, and the whole triangle has
    // to go: emitting the corners that are in range would leave an index buffer
    // that is no longer a multiple of three and shift every later triangle by
    // one. Every loop below walks `group.triangles`, so every one consults this.
    let whole_triangle = |triangle: usize| {
        model.indices[triangle * 3..triangle * 3 + 3]
            .iter()
            .all(|&index| (index as usize) < model.vertices.len())
    };

    // The local vertex for a level vertex, emitted on first use.
    let mut local_vertex = |global: usize| -> i32 {
        let slot = local_of_global[global];
        if slot >= 0 {
            return slot;
        }
        let vertex = model.vertices[global];
        let slot = (positions.len() / 3) as i32;
        local_of_global[global] = slot;

        // Under `Rebuild` the node's own inverse hands back the source file's
        // unit already — import parks the unit normalization in the node
        // transforms, and the chain reproduces it at the top of the exported
        // hierarchy. Flat geometry keeps world meters and is the one place the
        // factor is applied directly (see `UnitScale::per_meter`).
        let position = match to_local {
            Some(inverse) => inverse.transform_point3(vertex.position),
            None => unit.per_meter * vertex.position,
        };
        positions.extend_from_slice(&[
            f64::from(position.x),
            f64::from(position.y),
            f64::from(position.z),
        ]);

        // Directions, so the unit factor never touches them; they do need
        // renormalizing after a non-uniform scale.
        let normal = match normal_to_local {
            Some(matrix) => (matrix * vertex.normal).normalize_or(vertex.normal),
            None => vertex.normal,
        };
        normals.extend_from_slice(&[
            f64::from(normal.x),
            f64::from(normal.y),
            f64::from(normal.z),
        ]);

        colors.extend_from_slice(&[
            f64::from(vertex.vertex_color.x),
            f64::from(vertex.vertex_color.y),
            f64::from(vertex.vertex_color.z),
            f64::from(vertex.vertex_color.w),
        ]);
        for (channel, values) in color_values.iter_mut().enumerate() {
            let color = carry.color_channels[channel]
                .get(global)
                .copied()
                .unwrap_or(glam::Vec4::ONE);
            values.extend_from_slice(&[
                f64::from(color.x),
                f64::from(color.y),
                f64::from(color.z),
                f64::from(color.w),
            ]);
        }
        if has_crease {
            vertex_crease.push(f64::from(
                carry.vertex_crease.get(global).copied().unwrap_or(0.0),
            ));
        }

        if write_tangents {
            let tangent = vertex.tangent.truncate();
            let tangent = match direction_to_local {
                Some(matrix) => (matrix * tangent).normalize_or(tangent),
                None => tangent,
            };
            tangents.extend_from_slice(&[
                f64::from(tangent.x),
                f64::from(tangent.y),
                f64::from(tangent.z),
                f64::from(vertex.tangent.w),
            ]);
        }

        if channel_count == 0 {
            if let Some(set) = uv_sets.first_mut() {
                set.extend_from_slice(&[f64::from(vertex.uv.x), f64::from(vertex.uv.y)]);
            }
        } else {
            for (channel, set) in uv_sets.iter_mut().enumerate() {
                let uv = model
                    .uv_channels
                    .get(channel)
                    .and_then(|uvs| uvs.get(global))
                    .copied()
                    .unwrap_or_default();
                set.extend_from_slice(&[f64::from(uv.x), f64::from(uv.y)]);
            }
        }
        slot
    };

    // Which carried face each level triangle belongs to, for this node's pieces.
    let pieces: Vec<&NodePolygons> = carry
        .polygons
        .iter()
        .filter(|piece| group.source_node == Some(piece.node as usize))
        .collect();
    let mut face_of_triangle: HashMap<usize, (usize, usize)> = HashMap::new();
    for (piece_index, piece) in pieces.iter().enumerate() {
        for (offset, &face) in piece.triangle_face.iter().enumerate() {
            if face != NO_FACE {
                face_of_triangle.insert(
                    piece.triangle_first as usize + offset,
                    (piece_index, face as usize),
                );
            }
        }
    }
    let any_face_smoothing = pieces.iter().any(|piece| !piece.face_smoothing.is_empty());
    let any_face_hole = pieces.iter().any(|piece| !piece.face_hole.is_empty());
    let any_face_group = pieces.iter().any(|piece| !piece.face_group.is_empty());
    let any_edge_smoothing = pieces.iter().any(|piece| !piece.edge_smoothing.is_empty());
    let any_edge_crease = pieces.iter().any(|piece| !piece.edge_crease.is_empty());
    let any_edge_visibility = pieces.iter().any(|piece| !piece.edge_visibility.is_empty());

    // Per-face material, expressed as indices into this mesh's own slot list —
    // which is the order the bridge connects them to the node in.
    let triangle_count = model.indices.len() / 3;
    let has_material = model.triangles.material.len() == triangle_count;
    let mut material_slots: Vec<i32> = Vec::new();
    let slot_of = |global: u32, material_slots: &mut Vec<i32>| -> i32 {
        // The no-material sentinel maps to slot 0 of an empty list, which the
        // bridge writes as "no material".
        if global == u32::MAX || global as usize >= source.materials.len() {
            return 0;
        }
        match material_slots
            .iter()
            .position(|&seen| seen == global as i32)
        {
            Some(slot) => slot as i32,
            None => {
                material_slots.push(global as i32);
                (material_slots.len() - 1) as i32
            }
        }
    };

    let mut indices: Vec<i32> = Vec::with_capacity(group.triangles.len() * 3);
    let mut face_offsets: Vec<i32> = vec![0];
    let mut face_sources: Vec<u32> = Vec::new();
    let mut face_materials: Vec<i32> = Vec::new();
    let mut face_smoothing: Vec<u8> = Vec::new();
    let mut face_hole: Vec<u8> = Vec::new();
    let mut face_group: Vec<i32> = Vec::new();
    // (local a, local b) of every polygon edge -> the corner it starts at.
    let mut corner_of_edge: HashMap<(i32, i32), i32> = HashMap::new();
    let mut emitted_faces: std::collections::HashSet<(usize, usize)> =
        std::collections::HashSet::new();
    let mut triangles_as_triangles = 0usize;
    let mut written_triangles = 0usize;

    let mut push_face = |corners: &[i32],
                         material: i32,
                         layers: (bool, bool, i32),
                         indices: &mut Vec<i32>,
                         face_offsets: &mut Vec<i32>| {
        let first = indices.len();
        indices.extend_from_slice(corners);
        face_offsets.push(indices.len() as i32);
        face_materials.push(material);
        if any_face_smoothing {
            face_smoothing.push(u8::from(layers.0));
        }
        if any_face_hole {
            face_hole.push(u8::from(layers.1));
        }
        if any_face_group {
            face_group.push(layers.2);
        }
        for (offset, &corner) in corners.iter().enumerate() {
            let next = corners[(offset + 1) % corners.len()];
            corner_of_edge
                .entry((corner, next))
                .or_insert((first + offset) as i32);
        }
        written_triangles += corners.len().saturating_sub(2);
    };

    for &triangle in &group.triangles {
        if !whole_triangle(triangle) {
            continue;
        }
        let material = if has_material {
            slot_of(model.triangles.material[triangle], &mut material_slots)
        } else {
            0
        };
        match face_of_triangle.get(&triangle) {
            Some(&(piece_index, face)) => {
                if !emitted_faces.insert((piece_index, face)) {
                    continue;
                }
                let piece = pieces[piece_index];
                // Every corner of a carried face is a vertex some triangle of
                // this group reaches, so all resolve.
                let corners: Vec<i32> = piece
                    .face(face)
                    .iter()
                    .map(|&global| local_vertex(global as usize))
                    .collect();
                let layers = (
                    piece.face_smoothing.get(face).copied().unwrap_or(false),
                    piece.face_hole.get(face).copied().unwrap_or(false),
                    piece.face_group.get(face).copied().unwrap_or(0) as i32,
                );
                push_face(&corners, material, layers, &mut indices, &mut face_offsets);
                face_sources.push(piece.source_face.get(face).copied().unwrap_or(u32::MAX));
            }
            None => {
                let corners: Vec<i32> = (0..3)
                    .map(|corner| local_vertex(model.indices[triangle * 3 + corner] as usize))
                    .collect();
                push_face(
                    &corners,
                    material,
                    (false, false, 0),
                    &mut indices,
                    &mut face_offsets,
                );
                face_sources.push(u32::MAX);
                if !pieces.is_empty() {
                    triangles_as_triangles += 1;
                }
            }
        }
    }
    if material_slots.len() <= 1 {
        // The bridge only reads per-face assignment for a multi-material mesh.
        face_materials.clear();
    }

    // Edges, as the corner they start at in the stream just built. One a face
    // no longer has cannot be named and is dropped.
    let mut edges: Vec<i32> = Vec::new();
    let mut edge_sources: Vec<u32> = Vec::new();
    let mut edge_smoothing: Vec<u8> = Vec::new();
    let mut edge_crease: Vec<f64> = Vec::new();
    let mut edge_visibility: Vec<u8> = Vec::new();
    for piece in &pieces {
        for (index, edge) in piece.edges.iter().enumerate() {
            let a = local_of_global[edge[0] as usize];
            let b = local_of_global[edge[1] as usize];
            if a < 0 || b < 0 {
                continue;
            }
            let Some(&corner) = corner_of_edge
                .get(&(a, b))
                .or_else(|| corner_of_edge.get(&(b, a)))
            else {
                continue;
            };
            edges.push(corner);
            edge_sources.push(piece.source_edge.get(index).copied().unwrap_or(u32::MAX));
            if any_edge_smoothing {
                edge_smoothing.push(u8::from(
                    piece.edge_smoothing.get(index).copied().unwrap_or(false),
                ));
            }
            if any_edge_crease {
                edge_crease.push(f64::from(
                    piece.edge_crease.get(index).copied().unwrap_or(0.0),
                ));
            }
            if any_edge_visibility {
                edge_visibility.push(u8::from(
                    piece.edge_visibility.get(index).copied().unwrap_or(true),
                ));
            }
        }
    }

    // The geometry element keeps its own authored name when the capture has
    // it; a node and its mesh are named separately in FBX.
    let name = part
        .filter(|part| !part.name.is_empty())
        .map(|part| part.name.clone())
        .or_else(|| source_node.map(|node| node.name.clone()))
        .unwrap_or_else(|| model.name.clone());
    let name = match suffix_level {
        Some(level) => suffixed(&name, level),
        None => name,
    };

    let uv_set_names: Vec<CString> = (0..uv_sets.len())
        .map(|channel| {
            let label = source
                .uv_set_names
                .get(channel)
                .filter(|name| !name.is_empty())
                .cloned()
                .unwrap_or_else(|| format!("UVMap{channel}"));
            c_string(&label, "UVMap")
        })
        .collect();
    let color_set_name = part
        .and_then(|part| part.color_sets.first())
        .filter(|set| !set.name.is_empty())
        .map(|set| c_string(&set.name, "Col"));
    let color_sets: Vec<(CString, Vec<f64>)> = color_values
        .into_iter()
        .enumerate()
        .map(|(channel, values)| {
            let label = part
                .and_then(|part| part.color_sets.get(channel + 1))
                .map(|set| set.name.clone())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| format!("Col{}", channel + 1));
            (c_string(&label, "Col"), values)
        })
        .collect();

    let notes = MeshNotes {
        triangles_rebuilt: triangles_as_triangles,
        polygons_lost: pieces.is_empty() && part.is_some() && extras.is_some(),
    };
    let vertex_count = positions.len() / 3;
    let mut level_vertices = vec![0u32; vertex_count];
    for (level_vertex, &local) in local_of_global.iter().enumerate() {
        if local >= 0 {
            level_vertices[local as usize] = level_vertex as u32;
        }
    }
    let source_corners: Vec<u32> = level_vertices
        .iter()
        .map(|&level_vertex| {
            carry
                .source_corner
                .get(level_vertex as usize)
                .copied()
                .unwrap_or(u32::MAX)
        })
        .collect();
    let mesh = MeshData {
        name: c_string(&name, "Mesh"),
        node,
        positions,
        indices,
        face_offsets,
        triangle_count: written_triangles,
        vertex_count,
        normals,
        colors,
        tangents,
        uv_sets,
        uv_set_names,
        material_slots,
        face_materials,
        color_set_name,
        color_sets,
        face_smoothing,
        face_hole,
        face_group,
        edges,
        edge_smoothing,
        edge_crease,
        edge_visibility,
        vertex_crease,
        props: PropRange::default(),
        level_vertices,
        source_corners,
        face_sources,
        edge_sources,
        channel_sources: Vec::new(),
        source_node: group.source_node,
        level: suffix_level.unwrap_or(0),
        skins: Vec::new(),
        blend_channels: Vec::new(),
    };
    (mesh, notes)
}

/// What a mesh had to give up, for the report.
struct MeshNotes {
    /// Triangles written as triangles inside a mesh that otherwise kept its
    /// polygons.
    triangles_rebuilt: usize,
    /// The whole mesh went out as triangles although the source had polygons.
    polygons_lost: bool,
}

// ---------------------------------------------------------------------------
// Skins, blend shapes, poses
// ---------------------------------------------------------------------------

/// One shape's sparse offsets over a mesh: `(vertices, xyz offsets, xyz normal deltas)`.
#[derive(Clone, Default)]
struct ShapeOffsets(Vec<i32>, Vec<f64>, Vec<f64>);

/// `ufbxw_skinning_type` for a source skinning method.
fn skinning_type_code(method: review_model::SkinningMethod) -> u32 {
    match method {
        review_model::SkinningMethod::Rigid => 0,
        review_model::SkinningMethod::Linear => 1,
        review_model::SkinningMethod::DualQuaternion => 2,
        review_model::SkinningMethod::BlendedDqLinear => 3,
    }
}

/// A world matrix in meters, expressed in the file's unit: ufbx reads the
/// file's `TransformLink` / pose matrices and pre-multiplies its root scale,
/// so writing them back means scaling the whole 3×4 affine part by the units
/// per meter.
fn world_matrix_in_file_units(matrix: Mat4, per_meter: f32) -> [f64; 16] {
    let mut out = matrix.as_dmat4().to_cols_array();
    for column in 0..4 {
        for row in 0..3 {
            out[column * 4 + row] *= f64::from(per_meter);
        }
    }
    out
}

/// The scene node a source node was placed at, placing its ancestor chain on
/// the computed path if nothing has yet (bones are rarely mesh ancestors).
fn placed_or_place(
    scene: &mut SceneData,
    placed: &mut HashMap<usize, i32>,
    source: &ModelData,
    node: usize,
    extras: Option<&SourceExtras>,
) -> Option<i32> {
    if let Some(&existing) = placed.get(&node) {
        return Some(existing);
    }
    match extras {
        Some(extras) => emit_authored_node(scene, placed, source, extras, node, 0),
        None => {
            let group = NodeGroup {
                source_node: Some(node),
                triangles: Vec::new(),
            };
            let (index, _) = place_node(scene, placed, source, &group, HierarchyMode::Rebuild, 0);
            (index >= 0).then_some(index)
        }
    }
}

/// The authored bind poses, placed. Only the file's *bind* poses have a writer
/// (a rest pose is a different element the vendored writer does not emit).
fn build_poses(
    scene: &mut SceneData,
    placed: &mut HashMap<usize, i32>,
    source: &ModelData,
    extras: &SourceExtras,
    report: &mut ExportReport,
) {
    let mut skipped = 0usize;
    for pose in &extras.poses {
        if !pose.is_bind_pose {
            skipped += 1;
            continue;
        }
        let per_meter = scene.unit.per_meter;
        let mut nodes = Vec::with_capacity(pose.entries.len());
        for entry in &pose.entries {
            let Some(node) =
                placed_or_place(scene, placed, source, entry.node as usize, Some(extras))
            else {
                continue;
            };
            nodes.push((
                node,
                world_matrix_in_file_units(entry.bone_to_world, per_meter),
            ));
        }
        scene.poses.push(PoseData {
            name: c_string_or_empty(&pose.name),
            nodes,
        });
    }
    if skipped > 0 {
        report.notes.push(format!(
            "{skipped} non-bind pose(s) were not written (only bind poses are)."
        ));
    }
}

/// The skins and blend shapes of one exported mesh, from the level's own deform
/// tables (whose logical vertices are the level's vertices) and the carry.
#[allow(clippy::too_many_arguments)]
fn build_deform(
    scene: &mut SceneData,
    mesh: &mut MeshData,
    placed: &mut HashMap<usize, i32>,
    lod: &ProcessedLod,
    group: &NodeGroup,
    source: &ModelData,
    extras: Option<&SourceExtras>,
    report: &mut ExportReport,
) {
    let Some(node_index) = group.source_node else {
        return;
    };
    let level = &lod.model;
    let has_skin = level.skin.is_some()
        || lod
            .carry
            .extra_skins
            .iter()
            .any(|layer| layer.node as usize == node_index);
    let has_morph = level.morph.as_ref().is_some_and(|morph| {
        morph
            .channels
            .iter()
            .any(|channel| channel.mesh_node as usize == node_index)
    });
    if !has_skin && !has_morph {
        return;
    }
    let Some(node) = source.nodes.get(node_index) else {
        return;
    };
    let per_meter = scene.unit.per_meter;
    // Level vertex → this mesh's local vertex.
    let mut local_of_level: HashMap<u32, i32> = HashMap::with_capacity(mesh.level_vertices.len());
    for (local, &level_vertex) in mesh.level_vertices.iter().enumerate() {
        local_of_level.insert(level_vertex, local as i32);
    }
    // Deltas move back into geometry space like the positions did.
    let geometry_to_node = extras
        .and_then(|extras| extras.nodes.get(node_index))
        .map_or(Mat4::IDENTITY, |authored| authored.geometry_to_node);
    let world = node.transform * geometry_to_node;
    let to_local = world.inverse();
    let linear = Mat3::from_mat4(world);
    let determinant = linear.determinant().abs();

    // ---- Skins
    if let Some(skin) = &level.skin {
        let mut clusters: Vec<ClusterData> = Vec::new();
        let mut cluster_slot: HashMap<u32, usize> = HashMap::new();
        let mut unplaced = 0usize;
        for (index, cluster) in skin.clusters.iter().enumerate() {
            if cluster.mesh_node as usize != node_index {
                continue;
            }
            let Some(bone) = placed_or_place(scene, placed, source, cluster.bone as usize, extras)
            else {
                unplaced += 1;
                continue;
            };
            cluster_slot.insert(index as u32, clusters.len());
            clusters.push(ClusterData {
                bone,
                name: c_string_or_empty(&cluster.name),
                // `Transform` is authored in the file's units and read verbatim;
                // `TransformLink` is a world matrix ufbx normalized to meters.
                transform: cluster.mesh_node_to_bone.as_dmat4().to_cols_array(),
                transform_link: world_matrix_in_file_units(cluster.bind_to_world, per_meter),
                vertices: Vec::new(),
                weights: Vec::new(),
            });
        }
        if unplaced > 0 {
            report.notes.push(format!(
                "{unplaced} skin cluster(s) of '{}' bind to nodes that could not be written.",
                node.name
            ));
        }
        if !clusters.is_empty() {
            for (local, &level_vertex) in mesh.level_vertices.iter().enumerate() {
                let range = skin.influence_range(level_vertex as usize);
                for (&cluster, &weight) in skin.influence_cluster[range.clone()]
                    .iter()
                    .zip(&skin.weights[range])
                {
                    if let Some(&slot) = cluster_slot.get(&cluster) {
                        clusters[slot].vertices.push(local as i32);
                        clusters[slot].weights.push(f64::from(weight));
                    }
                }
            }
            let method = skin
                .deformers
                .iter()
                .find(|deformer| deformer.mesh_node as usize == node_index)
                .map_or(review_model::SkinningMethod::Linear, |deformer| {
                    deformer.method
                });
            let mut dq_vertices = Vec::new();
            let mut dq_weights = Vec::new();
            for &(level_vertex, weight) in &lod.carry.dq_weights {
                if let Some(&local) = local_of_level.get(&level_vertex) {
                    dq_vertices.push(local);
                    dq_weights.push(f64::from(weight));
                }
            }
            let bind_pose = scene
                .poses
                .iter()
                .position(|pose| {
                    pose.nodes
                        .iter()
                        .any(|(placed_node, _)| *placed_node == mesh.node)
                })
                .map_or(-1, |index| index as i32);
            mesh.skins.push(SkinExportData {
                skinning_type: skinning_type_code(method),
                clusters,
                dq_vertices,
                dq_weights,
                bind_pose,
            });
        }
    }
    // Further skin layers, from the carry and the capture's cluster tables.
    if let Some(part) = extras.and_then(|extras| extras.mesh_of_node(node_index as u32)) {
        for layer in lod
            .carry
            .extra_skins
            .iter()
            .filter(|layer| layer.node as usize == node_index)
        {
            let Some(authored) = part.extra_skins.get(layer.layer) else {
                continue;
            };
            let mut clusters: Vec<ClusterData> = Vec::new();
            let mut slot_of: Vec<Option<usize>> = Vec::with_capacity(authored.clusters.len());
            for cluster in &authored.clusters {
                let placed_bone =
                    placed_or_place(scene, placed, source, cluster.bone as usize, extras);
                slot_of.push(placed_bone.map(|bone| {
                    clusters.push(ClusterData {
                        bone,
                        name: c_string_or_empty(&cluster.name),
                        transform: cluster.mesh_node_to_bone.as_dmat4().to_cols_array(),
                        transform_link: world_matrix_in_file_units(
                            cluster.bind_to_world,
                            per_meter,
                        ),
                        vertices: Vec::new(),
                        weights: Vec::new(),
                    });
                    clusters.len() - 1
                }));
            }
            for &(level_vertex, cluster, weight) in &layer.influences {
                let (Some(&local), Some(Some(slot))) = (
                    local_of_level.get(&level_vertex),
                    slot_of.get(cluster as usize),
                ) else {
                    continue;
                };
                clusters[*slot].vertices.push(local);
                clusters[*slot].weights.push(f64::from(weight));
            }
            mesh.skins.push(SkinExportData {
                skinning_type: skinning_type_code(authored.method),
                clusters,
                dq_vertices: Vec::new(),
                dq_weights: Vec::new(),
                bind_pose: -1,
            });
        }
    }

    // ---- Blend shapes
    if let Some(morph) = &level.morph {
        // Every shape's offsets over this mesh, gathered in one walk.
        let mut by_shape: HashMap<u32, ShapeOffsets> = HashMap::new();
        for (local, &level_vertex) in mesh.level_vertices.iter().enumerate() {
            let range = match (
                morph.offsets.get(level_vertex as usize),
                morph.offsets.get(level_vertex as usize + 1),
            ) {
                (Some(&start), Some(&end)) if end >= start => start as usize..end as usize,
                _ => 0..0,
            };
            for index in range {
                let entry = by_shape.entry(morph.shape[index]).or_default();
                let position = to_local.transform_vector3(morph.position[index]);
                // The import rotated normal deltas by the cofactor matrix
                // (|det|·L⁻ᵀ); undoing it is Lᵀ / |det|, and the result stays
                // unnormalized as the file authored it.
                let normal = if determinant > 1e-12 {
                    linear.transpose() * morph.normal[index] / determinant
                } else {
                    morph.normal[index]
                };
                entry.0.push(local as i32);
                entry.1.extend_from_slice(&[
                    f64::from(position.x),
                    f64::from(position.y),
                    f64::from(position.z),
                ]);
                entry.2.extend_from_slice(&[
                    f64::from(normal.x),
                    f64::from(normal.y),
                    f64::from(normal.z),
                ]);
            }
        }
        for (channel_index, channel) in morph
            .channels
            .iter()
            .enumerate()
            .filter(|(_, channel)| channel.mesh_node as usize == node_index)
        {
            mesh.channel_sources.push(channel_index as u32);
            let shapes = channel
                .keyframes
                .iter()
                .map(|key| {
                    let ShapeOffsets(vertices, offsets, normals) =
                        by_shape.get(&key.shape).cloned().unwrap_or_default();
                    let has_normals = normals.iter().any(|&value| value != 0.0);
                    BlendShapeData {
                        name: c_string_or_empty(
                            morph
                                .shapes
                                .get(key.shape as usize)
                                .map_or("", |shape| shape.name.as_str()),
                        ),
                        vertices,
                        offsets,
                        normals: if has_normals { normals } else { Vec::new() },
                        target_weight: f64::from(key.target_weight) * 100.0,
                    }
                })
                .collect();
            mesh.blend_channels.push(BlendChannelData {
                name: c_string_or_empty(&channel.name),
                weight: f64::from(channel.rest_weight) * 100.0,
                shapes,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Animation, display layers, selection sets
// ---------------------------------------------------------------------------

/// FBX ktime units per second (`UFBXW_KTIME_SECOND`).
const KTIME_SECOND: f64 = 46_186_158_000.0;

/// `ufbxw_keyframe_flags` bits.
const KEY_CONSTANT: u32 = 0x1;
const KEY_CONSTANT_NEXT: u32 = 0x2;
const KEY_LINEAR: u32 = 0x4;
const KEY_CUBIC: u32 = 0x8;
const KEY_TANGENT_USER: u32 = 0x100;
const KEY_TANGENT_BROKEN: u32 = 0x200;
const KEY_WEIGHTED_LEFT: u32 = 0x1000;
const KEY_WEIGHTED_RIGHT: u32 = 0x2000;

/// `RVO_TARGET_*`.
const TARGET_NODE: u32 = 0;
const TARGET_NODE_ATTRIBUTE: u32 = 1;
const TARGET_MATERIAL: u32 = 2;
const TARGET_TEXTURE: u32 = 3;
const TARGET_VIDEO: u32 = 4;
const TARGET_BLEND_CHANNEL: u32 = 5;
const TARGET_DISPLAY_LAYER: u32 = 6;
const TARGET_ANIM_LAYER: u32 = 7;

fn ktime(seconds: f64) -> i64 {
    (seconds * KTIME_SECOND).round() as i64
}

/// One authored curve in the writer's terms. The reader expanded each key's
/// tangents to `(dx, dy)` handles as `dx = weight × interval`, `dy = dx ×
/// slope`; this is the exact inverse, per key against its neighbours.
fn curve_data(curve: &review_model::extras::Curve) -> CurveData {
    let keys = &curve.keys;
    let mut out = Vec::with_capacity(keys.len());
    for (index, key) in keys.iter().enumerate() {
        let mut flags = match key.interpolation {
            review_model::extras::Interpolation::ConstantPrev => KEY_CONSTANT,
            review_model::extras::Interpolation::ConstantNext => KEY_CONSTANT_NEXT,
            review_model::extras::Interpolation::Linear => KEY_LINEAR,
            review_model::extras::Interpolation::Cubic
            | review_model::extras::Interpolation::Unnamed(_) => KEY_CUBIC,
        };
        let mut weight_left = 1.0 / 3.0;
        let mut weight_right = 1.0 / 3.0;
        let mut slope_left = 0.0;
        let mut slope_right = 0.0;
        if flags == KEY_CUBIC {
            flags |= KEY_TANGENT_USER | KEY_TANGENT_BROKEN | KEY_WEIGHTED_LEFT | KEY_WEIGHTED_RIGHT;
            let (dx_left, dy_left) = (f64::from(key.left.0), f64::from(key.left.1));
            let (dx_right, dy_right) = (f64::from(key.right.0), f64::from(key.right.1));
            if index > 0 {
                let interval = key.time - keys[index - 1].time;
                if interval > 0.0 && dx_left > 0.0 {
                    weight_left = dx_left / interval;
                    slope_left = dy_left / dx_left;
                }
            }
            if index + 1 < keys.len() {
                let interval = keys[index + 1].time - key.time;
                if interval > 0.0 && dx_right > 0.0 {
                    weight_right = dx_right / interval;
                    slope_right = dy_right / dx_right;
                }
            }
        }
        out.push(KeyData {
            time: ktime(key.time),
            value: key.value,
            flags,
            weight_left,
            weight_right,
            slope_left,
            slope_right,
        });
    }
    CurveData {
        keys: out,
        pre_mode: curve.pre.mode.code(),
        pre_repeat: curve.pre.repeat_count,
        post_mode: curve.post.mode.code(),
        post_repeat: curve.post.repeat_count,
    }
}

/// The authored animation: every stack, every layer under the first stack
/// that lists it, and every animated property resolved onto the elements
/// written above.
fn build_animation(
    scene: &mut SceneData,
    placed: &HashMap<usize, i32>,
    extras: &SourceExtras,
    report: &mut ExportReport,
) {
    if extras.animations.is_empty() {
        return;
    }
    for stack in &extras.animations {
        let props = scene.push_props(&stack.props);
        scene.anim_stacks.push(AnimStackData {
            name: c_string_or_empty(&stack.name),
            props,
            time_begin: ktime(stack.time_begin),
            time_end: ktime(stack.time_end),
        });
    }
    scene.active_stack = 0;

    // Capture layer index → exported layer index (a layer no stack lists has
    // no place in the file).
    let mut layer_map: Vec<i32> = vec![-1; extras.anim_layers.len()];
    let mut orphaned = 0usize;
    for (index, _) in extras.anim_layers.iter().enumerate() {
        let stack = extras
            .animations
            .iter()
            .position(|stack| stack.layers.iter().any(|&layer| layer as usize == index));
        match stack {
            Some(stack) => {
                layer_map[index] = scene.anim_layers.len() as i32;
                scene.anim_layers.push(AnimLayerData {
                    name: CString::default(),
                    stack: stack as i32,
                    weight: 0.0,
                    props: PropRange::default(),
                    anim_props: Vec::new(),
                });
            }
            None => orphaned += 1,
        }
    }
    if orphaned > 0 {
        report.notes.push(format!(
            "{orphaned} animation layer(s) belonged to no stack and were not written."
        ));
    }

    let mut unmapped = 0usize;
    for (index, layer) in extras.anim_layers.iter().enumerate() {
        let exported = layer_map[index];
        if exported < 0 {
            continue;
        }
        let props = scene.push_props(&layer.props);
        let mut anim_props = Vec::new();
        for anim in &layer.anim {
            let curves = [
                anim.curves[0].as_ref().map(curve_data),
                anim.curves[1].as_ref().map(curve_data),
                anim.curves[2].as_ref().map(curve_data),
            ];
            // A curve node without curves still carries its defaults, and a
            // reader bakes a track for its node — it is written as authored.
            let default = [
                f64::from(anim.default.x),
                f64::from(anim.default.y),
                f64::from(anim.default.z),
            ];
            let mut targets: Vec<(u32, i32, i32)> = Vec::new();
            match &anim.target {
                review_model::extras::ElementRef::Node(node)
                | review_model::extras::ElementRef::NodeAttribute(node) => {
                    let kind = if matches!(anim.target, review_model::extras::ElementRef::Node(_)) {
                        TARGET_NODE
                    } else {
                        TARGET_NODE_ATTRIBUTE
                    };
                    if let Some(&base) = placed.get(&(*node as usize)) {
                        targets.push((kind, base, 0));
                    }
                    // A later level's copy of the node animates the same way.
                    if let Some(copies) = scene.level_copies.get(&(*node as usize)) {
                        targets.extend(copies.iter().map(|&copy| (kind, copy, 0)));
                    }
                }
                review_model::extras::ElementRef::Material(material) => {
                    if (*material as usize) < scene.materials.len() {
                        targets.push((TARGET_MATERIAL, *material as i32, 0));
                    }
                }
                review_model::extras::ElementRef::Texture(texture) => {
                    if let Some(&mapped) = scene.texture_map.get(*texture as usize)
                        && mapped >= 0
                    {
                        targets.push((TARGET_TEXTURE, mapped, 0));
                    }
                }
                review_model::extras::ElementRef::Video(video) => {
                    if (*video as usize) < scene.videos.len() {
                        targets.push((TARGET_VIDEO, *video as i32, 0));
                    }
                }
                review_model::extras::ElementRef::BlendChannel(channel) => {
                    for (mesh_index, mesh) in scene.meshes.iter().enumerate() {
                        if let Some(slot) = mesh.channel_sources.iter().position(|&c| c == *channel)
                        {
                            targets.push((TARGET_BLEND_CHANNEL, mesh_index as i32, slot as i32));
                        }
                    }
                }
                review_model::extras::ElementRef::DisplayLayer(layer) => {
                    if (*layer as usize) < scene.display_layers.len() {
                        targets.push((TARGET_DISPLAY_LAYER, *layer as i32, 0));
                    }
                }
                review_model::extras::ElementRef::AnimLayer(layer) => {
                    if let Some(&mapped) = layer_map.get(*layer as usize)
                        && mapped >= 0
                    {
                        targets.push((TARGET_ANIM_LAYER, mapped, 0));
                    }
                }
                review_model::extras::ElementRef::Unmapped { .. } => {}
            }
            if targets.is_empty() {
                unmapped += 1;
                continue;
            }
            for (kind, target, target2) in targets {
                anim_props.push(AnimPropData {
                    target_kind: kind,
                    target,
                    target2,
                    prop_name: c_string_or_empty(&anim.prop_name),
                    default,
                    curves: curves.clone(),
                });
            }
        }
        let data = &mut scene.anim_layers[exported as usize];
        data.name = c_string_or_empty(&layer.name);
        data.weight = layer.weight;
        data.props = props;
        data.anim_props = anim_props;
    }
    if unmapped > 0 {
        report.notes.push(format!(
            "{unmapped} animated propert{} target elements this export has no counterpart for and \
             were not written.",
            if unmapped == 1 { "y" } else { "ies" }
        ));
    }
}

/// Display layers over the placed nodes.
fn build_display_layers(
    scene: &mut SceneData,
    placed: &HashMap<usize, i32>,
    extras: &SourceExtras,
) {
    for layer in &extras.display_layers {
        let props = scene.push_props(&layer.props);
        scene.display_layers.push(DisplayLayerData {
            name: c_string_or_empty(&layer.name),
            props,
            nodes: layer
                .nodes
                .iter()
                .filter_map(|node| placed.get(&(*node as usize)).copied())
                .collect(),
        });
    }
}

/// Selection sets: node membership always; vertex / edge / face members
/// mapped onto the exported level-0 mesh where its polygons survived.
fn build_selection_sets(
    scene: &mut SceneData,
    placed: &HashMap<usize, i32>,
    source: &ModelData,
    extras: &SourceExtras,
    report: &mut ExportReport,
) {
    let mut components_dropped = 0usize;
    for set in &extras.selection_sets {
        let props = scene.push_props(&set.props);
        let mut nodes = Vec::new();
        for entry in &set.nodes {
            let Some(node_index) = entry.node else {
                continue;
            };
            let Some(&node) = placed.get(&(node_index as usize)) else {
                continue;
            };
            let has_components =
                !entry.vertices.is_empty() || !entry.edges.is_empty() || !entry.faces.is_empty();
            let mesh = scene
                .meshes
                .iter()
                .find(|mesh| mesh.level == 0 && mesh.source_node == Some(node_index as usize));
            let (vertices, edges, faces) = match (mesh, has_components) {
                (Some(mesh), true) => {
                    let wanted_vertices: std::collections::HashSet<u32> =
                        entry.vertices.iter().copied().collect();
                    let wanted_edges: std::collections::HashSet<u32> =
                        entry.edges.iter().copied().collect();
                    let wanted_faces: std::collections::HashSet<u32> =
                        entry.faces.iter().copied().collect();
                    let vertices: Vec<i32> = mesh
                        .source_corners
                        .iter()
                        .enumerate()
                        .filter(|&(_, &corner)| {
                            corner != u32::MAX
                                && source
                                    .corner_to_logical
                                    .get(corner as usize)
                                    .is_some_and(|logical| wanted_vertices.contains(logical))
                        })
                        .map(|(local, _)| local as i32)
                        .collect();
                    let edges: Vec<i32> = mesh
                        .edge_sources
                        .iter()
                        .enumerate()
                        .filter(|&(_, &edge)| edge != u32::MAX && wanted_edges.contains(&edge))
                        .map(|(index, _)| index as i32)
                        .collect();
                    let faces: Vec<i32> = mesh
                        .face_sources
                        .iter()
                        .enumerate()
                        .filter(|&(_, &face)| face != u32::MAX && wanted_faces.contains(&face))
                        .map(|(index, _)| index as i32)
                        .collect();
                    if vertices.len() < entry.vertices.len()
                        || edges.len() < entry.edges.len()
                        || faces.len() < entry.faces.len()
                    {
                        components_dropped += 1;
                    }
                    (vertices, edges, faces)
                }
                (None, true) => {
                    components_dropped += 1;
                    (Vec::new(), Vec::new(), Vec::new())
                }
                _ => (Vec::new(), Vec::new(), Vec::new()),
            };
            nodes.push(SelectionNodeData {
                node,
                include_node: entry.include_node,
                vertices,
                edges,
                faces,
            });
        }
        scene.selection_sets.push(SelectionSetData {
            name: c_string_or_empty(&set.name),
            props,
            nodes,
        });
    }
    if components_dropped > 0 {
        report.notes.push(format!(
            "{components_dropped} selection set entr{} lost some vertex / edge / face members: the \
             stack rebuilt or removed them.",
            if components_dropped == 1 { "y" } else { "ies" }
        ));
    }
}

/// A C string from `text`, falling back to `fallback` when the text is empty or
/// contains an interior NUL (which a name read from a file legitimately might).
fn c_string(text: &str, fallback: &str) -> CString {
    if text.is_empty() {
        return CString::new(fallback).unwrap_or_default();
    }
    CString::new(text.replace('\0', ""))
        .unwrap_or_else(|_| CString::new(fallback).unwrap_or_default())
}

/// A C string from `text`, empty when it is empty.
fn c_string_or_empty(text: &str) -> CString {
    CString::new(text.replace('\0', "")).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// The FFI call
// ---------------------------------------------------------------------------

/// Hand one assembled scene to the bridge.
#[cfg(has_ufbxw)]
fn write_scene(scene: &SceneData, path: &Path, format: FbxFormat) -> Result<(), OptError> {
    use crate::export_ffi::{
        RvoExportAnimLayer, RvoExportAnimProp, RvoExportAnimStack, RvoExportBlendChannel,
        RvoExportBlendShape, RvoExportCluster, RvoExportColorSet, RvoExportCurve,
        RvoExportDisplayLayer, RvoExportKey, RvoExportMaterial, RvoExportMaterialTexture,
        RvoExportMesh, RvoExportNode, RvoExportPose, RvoExportPoseNode, RvoExportProp,
        RvoExportPropRange, RvoExportScene, RvoExportSelectionNode, RvoExportSelectionSet,
        RvoExportSkin, RvoExportTexture, RvoExportTextureLayer, RvoExportVideo,
    };

    let path_string = path.to_string_lossy().into_owned();
    let c_path = CString::new(path_string)
        .map_err(|_| OptError::Export("the output path contains a NUL byte".to_owned()))?;

    let range = |range: PropRange| RvoExportPropRange {
        first: range.first,
        count: range.count,
    };
    let bytes = |bytes: &[u8]| {
        if bytes.is_empty() {
            std::ptr::null()
        } else {
            bytes.as_ptr()
        }
    };

    // The `repr(C)` mirrors borrow every buffer in `scene`, which outlives this
    // function; the bridge in turn only borrows them for the duration of the
    // call, so nothing here escapes.
    let props: Vec<RvoExportProp> = scene
        .props
        .iter()
        .map(|prop| RvoExportProp {
            name: prop.name.as_ptr(),
            kind: prop.kind,
            flags: prop.flags,
            value_int: prop.value_int,
            value_real: prop.value_real,
            value_str: prop.value_str.as_ptr(),
            blob: bytes(&prop.blob),
            blob_length: prop.blob.len(),
        })
        .collect();
    let nodes: Vec<RvoExportNode> = scene
        .nodes
        .iter()
        .map(|node| RvoExportNode {
            name: node.name.as_ptr(),
            parent: node.parent,
            translation: node.translation,
            rotation: node.rotation,
            scaling: node.scaling,
            authored_transform: i32::from(node.authored_transform),
            props: range(node.props),
            attribute_kind: node.attribute_kind,
            attribute_name: node.attribute_name.as_ptr(),
            attribute_props: range(node.attribute_props),
        })
        .collect();
    let material_textures: Vec<Vec<RvoExportMaterialTexture>> = scene
        .materials
        .iter()
        .map(|material| {
            material
                .textures
                .iter()
                .map(|(prop, texture)| RvoExportMaterialTexture {
                    prop: prop.as_ptr(),
                    texture: *texture,
                })
                .collect()
        })
        .collect();
    let materials: Vec<RvoExportMaterial> = scene
        .materials
        .iter()
        .zip(&material_textures)
        .map(|(material, textures)| RvoExportMaterial {
            name: material.name.as_ptr(),
            shader: material.shader,
            shading_model: material.shading_model.as_ptr(),
            props: range(material.props),
            base_color: material.base_color,
            emissive: material.emissive,
            shininess_exponent: material.shininess_exponent,
            reflection_factor: material.reflection_factor,
            textures: if textures.is_empty() {
                std::ptr::null()
            } else {
                textures.as_ptr()
            },
            texture_count: textures.len(),
        })
        .collect();
    let texture_layers: Vec<Vec<RvoExportTextureLayer>> = scene
        .textures
        .iter()
        .map(|texture| {
            texture
                .layers
                .iter()
                .map(|&(texture, blend_mode, alpha)| RvoExportTextureLayer {
                    texture,
                    blend_mode,
                    alpha,
                })
                .collect()
        })
        .collect();
    let textures: Vec<RvoExportTexture> = scene
        .textures
        .iter()
        .zip(&texture_layers)
        .map(|(texture, layers)| RvoExportTexture {
            name: texture.name.as_ptr(),
            layered: i32::from(texture.layered),
            filename: texture.filename.as_ptr(),
            relative_filename: texture.relative_filename.as_ptr(),
            content: bytes(&texture.content),
            content_length: texture.content.len(),
            video: texture.video,
            props: range(texture.props),
            layers: if layers.is_empty() {
                std::ptr::null()
            } else {
                layers.as_ptr()
            },
            layer_count: layers.len(),
        })
        .collect();
    let videos: Vec<RvoExportVideo> = scene
        .videos
        .iter()
        .map(|video| RvoExportVideo {
            name: video.name.as_ptr(),
            filename: video.filename.as_ptr(),
            relative_filename: video.relative_filename.as_ptr(),
            content: bytes(&video.content),
            content_length: video.content.len(),
            props: range(video.props),
        })
        .collect();

    // Pointer tables for the per-mesh UV and color sets, kept alive alongside
    // the mesh mirrors below.
    let uv_pointers: Vec<(Vec<*const f64>, Vec<*const c_char>, Vec<RvoExportColorSet>)> = scene
        .meshes
        .iter()
        .map(|mesh| {
            (
                mesh.uv_sets.iter().map(|set| set.as_ptr()).collect(),
                mesh.uv_set_names.iter().map(|name| name.as_ptr()).collect(),
                mesh.color_sets
                    .iter()
                    .map(|(name, values)| RvoExportColorSet {
                        name: name.as_ptr(),
                        values: values.as_ptr(),
                    })
                    .collect(),
            )
        })
        .collect();
    let opt_u8 = |values: &[u8]| {
        if values.is_empty() {
            std::ptr::null()
        } else {
            values.as_ptr()
        }
    };
    let opt_i32 = |values: &[i32]| {
        if values.is_empty() {
            std::ptr::null()
        } else {
            values.as_ptr()
        }
    };

    // The deform mirrors, owned beside the meshes so every pointer stays live.
    let cluster_mirrors: Vec<Vec<Vec<RvoExportCluster>>> = scene
        .meshes
        .iter()
        .map(|mesh| {
            mesh.skins
                .iter()
                .map(|skin| {
                    skin.clusters
                        .iter()
                        .map(|cluster| RvoExportCluster {
                            bone: cluster.bone,
                            name: cluster.name.as_ptr(),
                            transform: cluster.transform,
                            transform_link: cluster.transform_link,
                            vertices: opt_i32(&cluster.vertices),
                            weights: optional(&cluster.weights),
                            weight_count: cluster.weights.len(),
                        })
                        .collect()
                })
                .collect()
        })
        .collect();
    let skin_mirrors: Vec<Vec<RvoExportSkin>> = scene
        .meshes
        .iter()
        .zip(&cluster_mirrors)
        .map(|(mesh, clusters)| {
            mesh.skins
                .iter()
                .zip(clusters)
                .map(|(skin, clusters)| RvoExportSkin {
                    skinning_type: skin.skinning_type,
                    clusters: if clusters.is_empty() {
                        std::ptr::null()
                    } else {
                        clusters.as_ptr()
                    },
                    cluster_count: clusters.len(),
                    dq_vertices: opt_i32(&skin.dq_vertices),
                    dq_weights: optional(&skin.dq_weights),
                    dq_count: skin.dq_weights.len(),
                    bind_pose: skin.bind_pose,
                })
                .collect()
        })
        .collect();
    let shape_mirrors: Vec<Vec<Vec<RvoExportBlendShape>>> = scene
        .meshes
        .iter()
        .map(|mesh| {
            mesh.blend_channels
                .iter()
                .map(|channel| {
                    channel
                        .shapes
                        .iter()
                        .map(|shape| RvoExportBlendShape {
                            name: shape.name.as_ptr(),
                            vertices: opt_i32(&shape.vertices),
                            offsets: optional(&shape.offsets),
                            normals: optional(&shape.normals),
                            offset_count: shape.vertices.len(),
                            target_weight: shape.target_weight,
                        })
                        .collect()
                })
                .collect()
        })
        .collect();
    let channel_mirrors: Vec<Vec<RvoExportBlendChannel>> = scene
        .meshes
        .iter()
        .zip(&shape_mirrors)
        .map(|(mesh, shapes)| {
            mesh.blend_channels
                .iter()
                .zip(shapes)
                .map(|(channel, shapes)| RvoExportBlendChannel {
                    name: channel.name.as_ptr(),
                    weight: channel.weight,
                    shapes: if shapes.is_empty() {
                        std::ptr::null()
                    } else {
                        shapes.as_ptr()
                    },
                    shape_count: shapes.len(),
                })
                .collect()
        })
        .collect();
    let pose_nodes: Vec<Vec<RvoExportPoseNode>> = scene
        .poses
        .iter()
        .map(|pose| {
            pose.nodes
                .iter()
                .map(|&(node, matrix)| RvoExportPoseNode { node, matrix })
                .collect()
        })
        .collect();
    let poses: Vec<RvoExportPose> = scene
        .poses
        .iter()
        .zip(&pose_nodes)
        .map(|(pose, nodes)| RvoExportPose {
            name: pose.name.as_ptr(),
            nodes: if nodes.is_empty() {
                std::ptr::null()
            } else {
                nodes.as_ptr()
            },
            node_count: nodes.len(),
        })
        .collect();

    let meshes: Vec<RvoExportMesh> = scene
        .meshes
        .iter()
        .zip(&uv_pointers)
        .zip(skin_mirrors.iter().zip(&channel_mirrors))
        .map(
            |((mesh, (sets, names, color_sets)), (skins, channels))| RvoExportMesh {
                name: mesh.name.as_ptr(),
                node: mesh.node,
                positions: mesh.positions.as_ptr(),
                vertex_count: mesh.vertex_count,
                indices: mesh.indices.as_ptr(),
                index_count: mesh.indices.len(),
                face_offsets: mesh.face_offsets.as_ptr(),
                face_count: mesh.face_offsets.len().saturating_sub(1),
                normals: optional(&mesh.normals),
                colors: optional(&mesh.colors),
                tangents: optional(&mesh.tangents),
                uv_sets: if sets.is_empty() {
                    std::ptr::null()
                } else {
                    sets.as_ptr()
                },
                uv_set_names: if names.is_empty() {
                    std::ptr::null()
                } else {
                    names.as_ptr()
                },
                uv_set_count: sets.len(),
                material_slots: if mesh.material_slots.is_empty() {
                    std::ptr::null()
                } else {
                    mesh.material_slots.as_ptr()
                },
                material_slot_count: mesh.material_slots.len(),
                face_materials: if mesh.face_materials.is_empty() {
                    std::ptr::null()
                } else {
                    mesh.face_materials.as_ptr()
                },
                color_set_name: mesh
                    .color_set_name
                    .as_ref()
                    .map_or(std::ptr::null(), |name| name.as_ptr()),
                color_sets: if color_sets.is_empty() {
                    std::ptr::null()
                } else {
                    color_sets.as_ptr()
                },
                color_set_count: color_sets.len(),
                face_smoothing: opt_u8(&mesh.face_smoothing),
                face_hole: opt_u8(&mesh.face_hole),
                face_group: opt_i32(&mesh.face_group),
                edges: opt_i32(&mesh.edges),
                edge_count: mesh.edges.len(),
                edge_smoothing: opt_u8(&mesh.edge_smoothing),
                edge_crease: optional(&mesh.edge_crease),
                edge_visibility: opt_u8(&mesh.edge_visibility),
                vertex_crease: optional(&mesh.vertex_crease),
                props: range(mesh.props),
                skins: if skins.is_empty() {
                    std::ptr::null()
                } else {
                    skins.as_ptr()
                },
                skin_count: skins.len(),
                blend_channels: if channels.is_empty() {
                    std::ptr::null()
                } else {
                    channels.as_ptr()
                },
                blend_channel_count: channels.len(),
            },
        )
        .collect();

    // Animation mirrors: keys, then curves, then the properties pointing at
    // them, then the layers — each level owning the storage the next borrows.
    let key_mirrors: Vec<Vec<[Vec<RvoExportKey>; 3]>> = scene
        .anim_layers
        .iter()
        .map(|layer| {
            layer
                .anim_props
                .iter()
                .map(|prop| {
                    let keys = |curve: &Option<CurveData>| -> Vec<RvoExportKey> {
                        curve
                            .as_ref()
                            .map(|curve| {
                                curve
                                    .keys
                                    .iter()
                                    .map(|key| RvoExportKey {
                                        time: key.time,
                                        value: key.value,
                                        flags: key.flags,
                                        weight_left: key.weight_left,
                                        weight_right: key.weight_right,
                                        slope_left: key.slope_left,
                                        slope_right: key.slope_right,
                                    })
                                    .collect()
                            })
                            .unwrap_or_default()
                    };
                    [
                        keys(&prop.curves[0]),
                        keys(&prop.curves[1]),
                        keys(&prop.curves[2]),
                    ]
                })
                .collect()
        })
        .collect();
    let curve_mirrors: Vec<Vec<[Option<RvoExportCurve>; 3]>> = scene
        .anim_layers
        .iter()
        .zip(&key_mirrors)
        .map(|(layer, keys)| {
            layer
                .anim_props
                .iter()
                .zip(keys)
                .map(|(prop, keys)| {
                    let curve = |index: usize| -> Option<RvoExportCurve> {
                        prop.curves[index].as_ref().map(|curve| RvoExportCurve {
                            keys: if keys[index].is_empty() {
                                std::ptr::null()
                            } else {
                                keys[index].as_ptr()
                            },
                            key_count: keys[index].len(),
                            pre_mode: curve.pre_mode,
                            pre_repeat: curve.pre_repeat,
                            post_mode: curve.post_mode,
                            post_repeat: curve.post_repeat,
                        })
                    };
                    [curve(0), curve(1), curve(2)]
                })
                .collect()
        })
        .collect();
    let anim_prop_mirrors: Vec<Vec<RvoExportAnimProp>> = scene
        .anim_layers
        .iter()
        .zip(&curve_mirrors)
        .map(|(layer, curves)| {
            layer
                .anim_props
                .iter()
                .zip(curves)
                .map(|(prop, curves)| RvoExportAnimProp {
                    target_kind: prop.target_kind,
                    target: prop.target,
                    target2: prop.target2,
                    prop_name: prop.prop_name.as_ptr(),
                    default_value: prop.default,
                    curves: [
                        curves[0]
                            .as_ref()
                            .map_or(std::ptr::null(), |curve| curve as *const _),
                        curves[1]
                            .as_ref()
                            .map_or(std::ptr::null(), |curve| curve as *const _),
                        curves[2]
                            .as_ref()
                            .map_or(std::ptr::null(), |curve| curve as *const _),
                    ],
                })
                .collect()
        })
        .collect();
    let anim_layers: Vec<RvoExportAnimLayer> = scene
        .anim_layers
        .iter()
        .zip(&anim_prop_mirrors)
        .map(|(layer, props)| RvoExportAnimLayer {
            name: layer.name.as_ptr(),
            stack: layer.stack,
            weight: layer.weight,
            props: range(layer.props),
            anim_props: if props.is_empty() {
                std::ptr::null()
            } else {
                props.as_ptr()
            },
            anim_prop_count: props.len(),
        })
        .collect();
    let anim_stacks: Vec<RvoExportAnimStack> = scene
        .anim_stacks
        .iter()
        .map(|stack| RvoExportAnimStack {
            name: stack.name.as_ptr(),
            props: range(stack.props),
            time_begin: stack.time_begin,
            time_end: stack.time_end,
        })
        .collect();
    let display_layers: Vec<RvoExportDisplayLayer> = scene
        .display_layers
        .iter()
        .map(|layer| RvoExportDisplayLayer {
            name: layer.name.as_ptr(),
            props: range(layer.props),
            nodes: opt_i32(&layer.nodes),
            node_count: layer.nodes.len(),
        })
        .collect();
    let selection_node_mirrors: Vec<Vec<RvoExportSelectionNode>> = scene
        .selection_sets
        .iter()
        .map(|set| {
            set.nodes
                .iter()
                .map(|node| RvoExportSelectionNode {
                    node: node.node,
                    include_node: i32::from(node.include_node),
                    vertices: opt_i32(&node.vertices),
                    vertex_count: node.vertices.len(),
                    edges: opt_i32(&node.edges),
                    edge_count: node.edges.len(),
                    faces: opt_i32(&node.faces),
                    face_count: node.faces.len(),
                })
                .collect()
        })
        .collect();
    let selection_sets: Vec<RvoExportSelectionSet> = scene
        .selection_sets
        .iter()
        .zip(&selection_node_mirrors)
        .map(|(set, nodes)| RvoExportSelectionSet {
            name: set.name.as_ptr(),
            props: range(set.props),
            nodes: if nodes.is_empty() {
                std::ptr::null()
            } else {
                nodes.as_ptr()
            },
            node_count: nodes.len(),
        })
        .collect();

    let settings = &scene.settings;
    let payload = RvoExportScene {
        unit_scale_cm: scene.unit.unit_scale_cm,
        props: if props.is_empty() {
            std::ptr::null()
        } else {
            props.as_ptr()
        },
        prop_count: props.len(),
        axis_right: settings.axes[0],
        axis_up: settings.axes[1],
        axis_front: settings.axes[2],
        time_mode: settings.time_mode,
        frame_rate: settings.frame_rate,
        settings_props: range(settings.settings_props),
        scene_info_props: range(settings.scene_info_props),
        original_application_vendor: settings.original_application[0].as_ptr(),
        original_application_name: settings.original_application[1].as_ptr(),
        original_application_version: settings.original_application[2].as_ptr(),
        original_filename: settings.original_filename.as_ptr(),
        application_name: settings.application_name.as_ptr(),
        application_version: settings.application_version.as_ptr(),
        nodes: nodes.as_ptr(),
        node_count: nodes.len(),
        materials: if materials.is_empty() {
            std::ptr::null()
        } else {
            materials.as_ptr()
        },
        material_count: materials.len(),
        textures: if textures.is_empty() {
            std::ptr::null()
        } else {
            textures.as_ptr()
        },
        texture_count: textures.len(),
        videos: if videos.is_empty() {
            std::ptr::null()
        } else {
            videos.as_ptr()
        },
        video_count: videos.len(),
        meshes: meshes.as_ptr(),
        mesh_count: meshes.len(),
        poses: if poses.is_empty() {
            std::ptr::null()
        } else {
            poses.as_ptr()
        },
        pose_count: poses.len(),
        anim_stacks: if anim_stacks.is_empty() {
            std::ptr::null()
        } else {
            anim_stacks.as_ptr()
        },
        anim_stack_count: anim_stacks.len(),
        anim_layers: if anim_layers.is_empty() {
            std::ptr::null()
        } else {
            anim_layers.as_ptr()
        },
        anim_layer_count: anim_layers.len(),
        active_stack: scene.active_stack,
        display_layers: if display_layers.is_empty() {
            std::ptr::null()
        } else {
            display_layers.as_ptr()
        },
        display_layer_count: display_layers.len(),
        selection_sets: if selection_sets.is_empty() {
            std::ptr::null()
        } else {
            selection_sets.as_ptr()
        },
        selection_set_count: selection_sets.len(),
    };

    // Element type inferred as `c_char` from the call below, whose signedness is
    // the platform's rather than a fixed `i8`.
    let mut error = [0; crate::export_ffi::ERROR_LENGTH];
    // SAFETY: `payload` and every array it points at are live for this call and
    // sized exactly by the counts beside them; `c_path` is a valid NUL-terminated
    // string; `error` is a live buffer of exactly `ERROR_LENGTH` bytes, which is
    // what the length argument declares. The C side validates the payload's
    // internal consistency (index ranges, parent ordering, property ranges)
    // before using it, and frees everything it allocates on every path.
    let status = unsafe {
        crate::export_ffi::review_export_fbx(
            &raw const payload,
            c_path.as_ptr(),
            c_int::from(format == FbxFormat::Ascii),
            error.as_mut_ptr(),
            error.len(),
        )
    };

    if status != 0 {
        return Err(OptError::Export(error_message(&error)));
    }
    Ok(())
}

#[cfg(not(has_ufbxw))]
fn write_scene(_scene: &SceneData, _path: &Path, _format: FbxFormat) -> Result<(), OptError> {
    Err(OptError::Unavailable)
}

/// A null pointer for an empty buffer — the bridge reads "attribute absent".
#[cfg(has_ufbxw)]
fn optional(values: &[f64]) -> *const f64 {
    if values.is_empty() {
        std::ptr::null()
    } else {
        values.as_ptr()
    }
}

/// Read the bridge's NUL-terminated message out of its fixed buffer.
#[cfg(has_ufbxw)]
fn error_message(buffer: &[c_char]) -> String {
    let bytes: Vec<u8> = buffer
        .iter()
        .take_while(|&&byte| byte != 0)
        .map(|&byte| byte as u8)
        .collect();
    let message = String::from_utf8_lossy(&bytes).into_owned();
    if message.is_empty() {
        "the FBX writer failed without reporting a reason".to_owned()
    } else {
        message
    }
}

/// Unit coverage for the pure helpers. The FFI write path is covered end to end
/// by `tests/export_round_trip.rs`, which reads every written file back.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_path_suffixes_the_stem_and_keeps_the_extension() {
        assert_eq!(
            level_path(Path::new("out/asset.fbx"), 2),
            PathBuf::from("out/asset_LOD2.fbx")
        );
    }

    #[test]
    fn level_path_supplies_a_name_and_an_extension_when_the_path_has_neither() {
        assert_eq!(
            level_path(Path::new("asset"), 1),
            PathBuf::from("asset_LOD1.fbx")
        );
        assert_eq!(level_path(Path::new(""), 0), PathBuf::from("mesh_LOD0.fbx"));
    }

    #[test]
    fn unit_scale_follows_the_source_and_snaps_to_the_unit_it_meant() {
        // A centimeter file's 0.01 arrives through an f32, so the naive
        // `x * 100.0` is 0.99999998 — a factor no importer names, and one whose
        // reciprocal drifts every coordinate written with it.
        let centimeters = UnitScale::from_source(0.01);
        assert_eq!(centimeters.unit_scale_cm, 1.0);
        assert_eq!(centimeters.per_meter, 100.0);

        let inches = UnitScale::from_source(0.0254);
        assert_eq!(inches.unit_scale_cm, 2.54);

        let meters = UnitScale::from_source(1.0);
        assert_eq!(meters.unit_scale_cm, 100.0);
        assert_eq!(meters.per_meter, 1.0);

        // An unrecognised unit is still the file's own unit and is written as
        // declared; only a missing one falls back to the meters the geometry
        // already is.
        assert_eq!(UnitScale::from_source(0.5).unit_scale_cm, 50.0);
        for missing in [0.0, -1.0, f32::NAN] {
            let fallback = UnitScale::from_source(missing);
            assert_eq!(fallback.unit_scale_cm, DEFAULT_UNIT_SCALE_CM);
            assert_eq!(fallback.per_meter, 1.0);
        }
    }

    #[test]
    fn time_mode_matches_standard_rates_and_leaves_the_rest_custom() {
        assert_eq!(time_mode_for(30.0), Some(6));
        assert_eq!(time_mode_for(24.0), Some(11));
        assert_eq!(time_mode_for(59.94), Some(17));
        assert_eq!(time_mode_for(31.5), None);
    }

    #[test]
    fn c_string_falls_back_for_empty_text() {
        assert_eq!(c_string("", "Mesh").to_bytes(), b"Mesh");
    }

    #[test]
    fn c_string_keeps_a_name_carrying_an_interior_nul() {
        assert_eq!(c_string("Bo\0dy", "Mesh").to_bytes(), b"Body");
    }

    #[cfg(has_ufbxw)]
    #[test]
    fn error_message_reads_the_bridge_text_and_explains_its_absence() {
        let mut buffer = [0; crate::export_ffi::ERROR_LENGTH];
        for (slot, &byte) in buffer.iter_mut().zip(b"could not write the file") {
            *slot = byte as c_char;
        }
        assert_eq!(error_message(&buffer), "could not write the file");
        assert_eq!(
            error_message(&[0; 8]),
            "the FBX writer failed without reporting a reason"
        );
    }

    /// Two nodes each claiming the other as parent — what the ancestor walk has
    /// to survive, since it follows `parent` links to the root.
    fn cyclic_model() -> ModelData {
        let node = |name: &str, parent: usize| review_model::SceneNode {
            name: name.to_owned(),
            parent: Some(parent),
            mesh_part: Some(0),
            source_vertex_count: 0,
            transform: Mat4::IDENTITY,
            rest_local: Default::default(),
            kind: review_model::NodeKind::Mesh,
            bone: None,
        };
        ModelData {
            name: "Looped".to_owned(),
            nodes: vec![node("A", 1), node("B", 0)],
            ..ModelData::default()
        }
    }

    fn empty_scene() -> SceneData {
        SceneData::new(UnitScale::from_source(0.0))
    }

    #[test]
    fn place_node_cuts_a_parent_cycle_and_says_so() {
        let source = cyclic_model();
        let mut scene = empty_scene();
        let group = NodeGroup {
            source_node: Some(0),
            triangles: Vec::new(),
        };

        let (node, notes) = place_node(
            &mut scene,
            &mut HashMap::new(),
            &source,
            &group,
            HierarchyMode::Rebuild,
            0,
        );

        assert_eq!(node, 0, "the mesh still attaches to a real node");
        assert_eq!(
            scene.nodes.len(),
            1,
            "the loop emits one node, not an endless chain"
        );
        assert!(
            notes.iter().any(|note| note.contains("looped")),
            "the user is told the branch was flattened: {notes:?}"
        );
    }

    /// A source hierarchy shaped like a game FBX carrying its LOD chain: the
    /// importer's synthetic file root, a group node, and two sibling mesh
    /// leaves under it.
    fn lod_sibling_model() -> ModelData {
        let node =
            |name: &str, parent: Option<usize>, mesh: Option<usize>| review_model::SceneNode {
                name: name.to_owned(),
                parent,
                mesh_part: mesh,
                source_vertex_count: 0,
                transform: Mat4::IDENTITY,
                rest_local: Default::default(),
                kind: if mesh.is_some() {
                    review_model::NodeKind::Mesh
                } else {
                    review_model::NodeKind::Empty
                },
                bone: None,
            };
        ModelData {
            name: "Crate".to_owned(),
            nodes: vec![
                node("", None, None),
                node("SM_Crate", Some(0), None),
                node("SM_Crate_LOD0", Some(1), Some(0)),
                node("SM_Crate_LOD1", Some(1), Some(1)),
            ],
            ..ModelData::default()
        }
    }

    fn placed_names(scene: &SceneData) -> Vec<&str> {
        scene
            .nodes
            .iter()
            .map(|node| node.name.to_str().expect("names are UTF-8"))
            .collect()
    }

    /// Sibling meshes under one group come out under a single shared ancestor
    /// chain — not one copy of it per mesh — and the importer's synthetic file
    /// root is not re-emitted as a wrapper node.
    #[test]
    fn place_node_shares_ancestors_and_drops_the_synthetic_file_root() {
        let source = lod_sibling_model();
        let mut scene = empty_scene();
        let mut placed = HashMap::new();

        for leaf in [2, 3] {
            let group = NodeGroup {
                source_node: Some(leaf),
                triangles: Vec::new(),
            };
            let (node, _) = place_node(
                &mut scene,
                &mut placed,
                &source,
                &group,
                HierarchyMode::Rebuild,
                0,
            );
            assert!(node >= 0, "the mesh attaches to a real node");
        }

        assert_eq!(
            placed_names(&scene),
            ["SM_Crate", "SM_Crate_LOD0", "SM_Crate_LOD1"],
            "one shared chain, no file-root wrapper"
        );
        assert_eq!(scene.nodes[0].parent, NO_PARENT);
        assert_eq!(scene.nodes[1].parent, 0, "first leaf under the group");
        assert_eq!(scene.nodes[2].parent, 0, "second leaf beside the first");
    }

    /// A suffixed chain (several levels in one file) puts each level's leaf as
    /// a sibling under the same unsuffixed ancestors, mirroring how game FBX
    /// files lay out their own LOD chains.
    #[test]
    fn place_node_puts_suffixed_levels_as_siblings_under_one_chain() {
        let source = lod_sibling_model();
        let mut scene = empty_scene();
        let mut placed = HashMap::new();
        let group = NodeGroup {
            source_node: Some(2),
            triangles: Vec::new(),
        };

        for level in 0..2 {
            place_node(
                &mut scene,
                &mut placed,
                &source,
                &group,
                HierarchyMode::Rebuild,
                level,
            );
        }

        assert_eq!(
            placed_names(&scene),
            ["SM_Crate", "SM_Crate_LOD0", "SM_Crate_LOD0_LOD1"],
            "only the leaf is suffixed; the ancestors are shared"
        );
        assert_eq!(scene.nodes[1].parent, 0);
        assert_eq!(scene.nodes[2].parent, 0, "the level-1 leaf is a sibling");
    }

    /// The authored graph: every node except the synthetic root goes out, a
    /// scale helper is skipped with its children re-parented past it, and a
    /// later level's mesh node is a suffixed sibling with the same attribute.
    #[test]
    fn place_graph_writes_every_authored_node_and_skips_the_helpers() {
        use review_model::extras::{
            Application, AttributeExtras, InheritMode, NodeExtras, RotationOrder, SceneExtras,
            SnapMode, TimeMode, TimeProtocol,
        };

        let mut source = lod_sibling_model();
        // A scale helper between the group and its second leaf, and a light.
        source.nodes.push(review_model::SceneNode {
            name: "helper".to_owned(),
            parent: Some(1),
            ..Default::default()
        });
        source.nodes[3].parent = Some(4);
        source.nodes.push(review_model::SceneNode {
            name: "Key".to_owned(),
            parent: Some(1),
            kind: review_model::NodeKind::Light,
            ..Default::default()
        });
        let node_extras = |synthetic, attribute: Option<AttributeKind>| NodeExtras {
            props: Vec::new(),
            rotation_order: RotationOrder::Xyz,
            inherit_mode: InheritMode::Normal,
            original_inherit_mode: InheritMode::Normal,
            geometry_to_node: Mat4::IDENTITY,
            synthetic,
            visible: true,
            attribute: attribute.map(|kind| AttributeExtras {
                kind,
                name: String::new(),
                props: Vec::new(),
                light: None,
                camera: None,
                lod_group: None,
            }),
        };
        let extras = SourceExtras {
            scene: SceneExtras {
                creator: String::new(),
                filename: String::new(),
                original_file_path: String::new(),
                version: 7700,
                ascii: false,
                original_application: Application::default(),
                latest_application: Application::default(),
                scene_props: Vec::new(),
                settings_props: Vec::new(),
                axes: [CoordinateAxis::Unknown; 3],
                original_axis_up: CoordinateAxis::Unknown,
                unit_meters: 1.0,
                original_unit_meters: 1.0,
                frames_per_second: 30.0,
                ambient_color: Vec3::ZERO,
                default_camera: String::new(),
                time_mode: TimeMode::Fps30,
                time_protocol: TimeProtocol::Default,
                snap_mode: SnapMode::None,
            },
            nodes: vec![
                node_extras(Synthetic::Root, None),
                node_extras(Synthetic::None, None),
                node_extras(Synthetic::None, Some(AttributeKind::Mesh)),
                node_extras(Synthetic::None, Some(AttributeKind::Mesh)),
                node_extras(Synthetic::ScaleHelper, None),
                node_extras(Synthetic::None, Some(AttributeKind::Light)),
            ],
            materials: Vec::new(),
            textures: Vec::new(),
            videos: Vec::new(),
            meshes: Vec::new(),
            poses: Vec::new(),
            display_layers: Vec::new(),
            selection_sets: Vec::new(),
            anim_layers: Vec::new(),
            animations: Vec::new(),
        };

        let mut scene = empty_scene();
        let mut placed = HashMap::new();
        place_graph(&mut scene, &mut placed, &source, &extras);

        assert_eq!(
            placed_names(&scene),
            ["SM_Crate", "SM_Crate_LOD0", "SM_Crate_LOD1", "Key"],
            "no root, no helper, and the light is there"
        );
        assert_eq!(scene.nodes[2].parent, 0, "re-parented past the helper");
        assert_eq!(scene.nodes[3].attribute_kind, ATTRIB_LIGHT);
        assert!(scene.nodes.iter().all(|node| node.authored_transform));

        let group = NodeGroup {
            source_node: Some(2),
            triangles: Vec::new(),
        };
        assert_eq!(
            place_level_node(&mut scene, &placed, &source, &extras, &group, 0),
            1
        );
        let copy = place_level_node(&mut scene, &placed, &source, &extras, &group, 1);
        assert_eq!(copy, 4);
        assert_eq!(scene.nodes[4].parent, 0, "the level copy is a sibling");
        assert_eq!(scene.nodes[4].name.to_str().unwrap(), "SM_Crate_LOD0_LOD1");
    }
}
