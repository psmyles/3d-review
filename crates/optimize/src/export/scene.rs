//! The payload: one `#[repr(C)]`-adjacent owned struct per thing the bridge
//! writes, plus the [`SceneData`] accumulator every builder appends to.
//!
//! Nothing here has behavior beyond bookkeeping. The builders in the sibling
//! modules each fill one part of a `SceneData`, and [`super::write`] hands the
//! whole of it to C as flat arrays.

use std::collections::HashMap;
use std::ffi::CString;

use review_model::extras::Prop;

use super::*;

/// A root node's parent index — `RVO_NO_PARENT` in `export_bridge.h`.
pub(crate) const NO_PARENT: i32 = -1;

/// `RVO_ATTRIB_*` in `export_bridge.h`.
pub(crate) const ATTRIB_NONE: u32 = 0;

pub(crate) const ATTRIB_BONE: u32 = 1;

pub(crate) const ATTRIB_LIGHT: u32 = 2;

pub(crate) const ATTRIB_CAMERA: u32 = 3;

pub(crate) const ATTRIB_NULL: u32 = 4;

pub(crate) const ATTRIB_LOD_GROUP: u32 = 5;

/// `RVO_SHADER_*` in `export_bridge.h`.
pub(crate) const SHADER_LAMBERT: u32 = 0;

pub(crate) const SHADER_PHONG: u32 = 1;

pub(crate) const SHADER_CUSTOM: u32 = 2;

// ---------------------------------------------------------------------------
// Scene assembly
// ---------------------------------------------------------------------------

/// A range of [`SceneData::props`]: `(first, count)`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PropRange {
    pub(crate) first: u32,
    pub(crate) count: u32,
}

/// One authored property, owned in C-ready form.
pub(crate) struct PropData {
    pub(crate) name: CString,
    pub(crate) kind: u32,
    pub(crate) flags: u32,
    pub(crate) value_int: i64,
    pub(crate) value_real: [f64; 4],
    pub(crate) value_str: CString,
    pub(crate) blob: Vec<u8>,
}

/// The owned Rust side of a scene payload. Every buffer the bridge borrows lives
/// here, so the whole thing must outlive the `review_export_fbx` call — which
/// it does, since [`write_scene`] takes it by reference.
#[cfg_attr(test, derive(Default))]
pub(crate) struct ClusterData {
    pub(crate) bone: i32,
    pub(crate) name: CString,
    pub(crate) transform: [f64; 16],
    pub(crate) transform_link: [f64; 16],
    pub(crate) vertices: Vec<i32>,
    pub(crate) weights: Vec<f64>,
}

#[cfg_attr(test, derive(Default))]
pub(crate) struct SkinExportData {
    pub(crate) skinning_type: u32,
    pub(crate) clusters: Vec<ClusterData>,
    pub(crate) dq_vertices: Vec<i32>,
    pub(crate) dq_weights: Vec<f64>,
    pub(crate) bind_pose: i32,
}

#[cfg_attr(test, derive(Default))]
pub(crate) struct BlendShapeData {
    pub(crate) name: CString,
    pub(crate) vertices: Vec<i32>,
    pub(crate) offsets: Vec<f64>,
    pub(crate) normals: Vec<f64>,
    pub(crate) target_weight: f64,
}

#[cfg_attr(test, derive(Default))]
pub(crate) struct BlendChannelData {
    pub(crate) name: CString,
    pub(crate) weight: f64,
    pub(crate) shapes: Vec<BlendShapeData>,
}

pub(crate) struct PoseData {
    pub(crate) name: CString,
    /// `(scene node, matrix)`.
    pub(crate) nodes: Vec<(i32, [f64; 16])>,
}

#[derive(Clone, Copy)]
pub(crate) struct KeyData {
    pub(crate) time: i64,
    pub(crate) value: f64,
    pub(crate) flags: u32,
    pub(crate) weight_left: f64,
    pub(crate) weight_right: f64,
    pub(crate) slope_left: f64,
    pub(crate) slope_right: f64,
}

#[derive(Clone)]
pub(crate) struct CurveData {
    pub(crate) keys: Vec<KeyData>,
    pub(crate) pre_mode: u32,
    pub(crate) pre_repeat: i32,
    pub(crate) post_mode: u32,
    pub(crate) post_repeat: i32,
}

pub(crate) struct AnimPropData {
    pub(crate) target_kind: u32,
    pub(crate) target: i32,
    pub(crate) target2: i32,
    pub(crate) prop_name: CString,
    pub(crate) default: [f64; 3],
    pub(crate) curves: [Option<CurveData>; 3],
}

pub(crate) struct AnimLayerData {
    pub(crate) name: CString,
    pub(crate) stack: i32,
    pub(crate) weight: f64,
    pub(crate) props: PropRange,
    pub(crate) anim_props: Vec<AnimPropData>,
}

pub(crate) struct AnimStackData {
    pub(crate) name: CString,
    pub(crate) props: PropRange,
    pub(crate) time_begin: i64,
    pub(crate) time_end: i64,
}

pub(crate) struct DisplayLayerData {
    pub(crate) name: CString,
    pub(crate) props: PropRange,
    pub(crate) nodes: Vec<i32>,
}

pub(crate) struct SelectionNodeData {
    pub(crate) node: i32,
    pub(crate) include_node: bool,
    pub(crate) vertices: Vec<i32>,
    pub(crate) edges: Vec<i32>,
    pub(crate) faces: Vec<i32>,
}

pub(crate) struct SelectionSetData {
    pub(crate) name: CString,
    pub(crate) props: PropRange,
    pub(crate) nodes: Vec<SelectionNodeData>,
}

pub(crate) struct SceneData {
    /// The unit every coordinate below is expressed in, and the factor the file
    /// declares so a reader recovers it.
    pub(crate) unit: UnitScale,
    /// Every authored property any element references, by range.
    pub(crate) props: Vec<PropData>,
    pub(crate) settings: SettingsData,
    pub(crate) nodes: Vec<NodeData>,
    pub(crate) materials: Vec<MaterialData>,
    pub(crate) textures: Vec<TextureData>,
    pub(crate) videos: Vec<VideoData>,
    pub(crate) meshes: Vec<MeshData>,
    pub(crate) poses: Vec<PoseData>,
    /// Capture texture index → scene texture index, or -1.
    pub(crate) texture_map: Vec<i32>,
    pub(crate) anim_stacks: Vec<AnimStackData>,
    pub(crate) anim_layers: Vec<AnimLayerData>,
    pub(crate) active_stack: i32,
    pub(crate) display_layers: Vec<DisplayLayerData>,
    pub(crate) selection_sets: Vec<SelectionSetData>,
    /// Source node → the suffixed per-level copies made of it.
    pub(crate) level_copies: HashMap<usize, Vec<i32>>,
    /// A user string property written over what the source authored (the
    /// viewer's review comments): see [`NodeStrings`].
    pub(crate) node_strings: Option<NodeStrings>,
}

/// A user string property the export writes over what the source authored —
/// how data the viewer keeps on a node (its review comments) reaches an export.
/// The property named `name` is dropped from every exported node, then written
/// with `values[source node]` on that source node's own node; the `_LOD<n>`
/// copies of it never carry it, so a chain doesn't repeat it once per level.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeStrings {
    pub name: String,
    /// Source node index → the property's text.
    pub values: HashMap<usize, String>,
}

impl SceneData {
    pub(crate) fn new(unit: UnitScale) -> Self {
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
            node_strings: None,
        }
    }

    /// [`Self::push_props`] for a node's authored properties, applying
    /// [`Self::node_strings`]: the property it names is dropped, and written
    /// afresh on the source node itself (not on a `copy` made for a later level).
    pub(crate) fn push_node_props(
        &mut self,
        props: &[Prop],
        source_index: usize,
        copy: bool,
    ) -> PropRange {
        let Some(strings) = self.node_strings.take() else {
            return self.push_props(props);
        };
        let kept: Vec<Prop> = props
            .iter()
            .filter(|prop| prop.name != strings.name)
            .cloned()
            .collect();
        let mut range = self.push_props(&kept);
        if !copy
            && let Some(value) = strings.values.get(&source_index)
            && let Ok(name) = CString::new(strings.name.replace('\0', ""))
        {
            self.props.push(PropData {
                name,
                kind: review_model::extras::PropType::Text.code(),
                flags: review_model::extras::PropFlags::USER_DEFINED
                    | review_model::extras::PropFlags::VALUE_STR,
                value_int: 0,
                value_real: [0.0; 4],
                value_str: c_string_or_empty(value),
                blob: Vec::new(),
            });
            range.count += 1;
        }
        self.node_strings = Some(strings);
        range
    }

    /// Append `props` to the table and return their range. A property whose
    /// name cannot be a C string is dropped rather than misnamed.
    pub(crate) fn push_props(&mut self, props: &[Prop]) -> PropRange {
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

    pub(crate) fn mesh_count_into(&self, report: &mut ExportReport) {
        report.mesh_count += self.meshes.len();
    }
}

pub(crate) struct SettingsData {
    /// `ufbxw_coordinate_axis` codes, or -1 for the writer's default.
    pub(crate) axes: [i32; 3],
    pub(crate) time_mode: i32,
    pub(crate) frame_rate: f64,
    pub(crate) settings_props: PropRange,
    pub(crate) scene_info_props: PropRange,
    pub(crate) original_application: [CString; 3],
    pub(crate) original_filename: CString,
    pub(crate) application_name: CString,
    pub(crate) application_version: CString,
}

pub(crate) struct NodeData {
    pub(crate) name: CString,
    pub(crate) parent: i32,
    pub(crate) translation: [f64; 3],
    pub(crate) rotation: [f64; 4],
    pub(crate) scaling: [f64; 3],
    pub(crate) authored_transform: bool,
    pub(crate) props: PropRange,
    pub(crate) attribute_kind: u32,
    pub(crate) attribute_name: CString,
    pub(crate) attribute_props: PropRange,
}

pub(crate) struct MaterialData {
    pub(crate) name: CString,
    pub(crate) shader: u32,
    pub(crate) shading_model: CString,
    pub(crate) props: PropRange,
    pub(crate) base_color: [f64; 3],
    pub(crate) emissive: [f64; 3],
    pub(crate) shininess_exponent: f64,
    pub(crate) reflection_factor: f64,
    /// `(material property, texture index)`.
    pub(crate) textures: Vec<(CString, i32)>,
}

pub(crate) struct TextureData {
    pub(crate) name: CString,
    pub(crate) layered: bool,
    pub(crate) filename: CString,
    pub(crate) relative_filename: CString,
    pub(crate) content: Vec<u8>,
    pub(crate) video: i32,
    pub(crate) props: PropRange,
    /// `(texture index, blend mode, alpha)`.
    pub(crate) layers: Vec<(i32, i32, f64)>,
}

pub(crate) struct VideoData {
    pub(crate) name: CString,
    pub(crate) filename: CString,
    pub(crate) relative_filename: CString,
    pub(crate) content: Vec<u8>,
    pub(crate) props: PropRange,
}

#[cfg_attr(test, derive(Default))]
pub(crate) struct MeshData {
    pub(crate) name: CString,
    pub(crate) node: i32,
    pub(crate) positions: Vec<f64>,
    /// The polygon-vertex stream, cut by `face_offsets`.
    pub(crate) indices: Vec<i32>,
    pub(crate) face_offsets: Vec<i32>,
    /// How many triangles the faces amount to, for the report.
    pub(crate) triangle_count: usize,
    pub(crate) vertex_count: usize,
    pub(crate) normals: Vec<f64>,
    pub(crate) colors: Vec<f64>,
    /// `4 * vertex_count` (xyz + handedness), empty when the source authored
    /// no tangent layer.
    pub(crate) tangents: Vec<f64>,
    pub(crate) uv_sets: Vec<Vec<f64>>,
    pub(crate) uv_set_names: Vec<CString>,
    pub(crate) material_slots: Vec<i32>,
    /// Per face; empty for a single-material mesh.
    pub(crate) face_materials: Vec<i32>,
    pub(crate) color_set_name: Option<CString>,
    /// Sets beyond the first: name and `4 * vertex_count` values.
    pub(crate) color_sets: Vec<(CString, Vec<f64>)>,
    /// Per local vertex, the level vertex it came from (Rust-side only).
    pub(crate) level_vertices: Vec<u32>,
    /// Per local vertex, a source corner it came from (`u32::MAX` unknown).
    pub(crate) source_corners: Vec<u32>,
    /// Per exported face / edge, the source face / part edge (`u32::MAX` for
    /// one the stack made).
    pub(crate) face_sources: Vec<u32>,
    pub(crate) edge_sources: Vec<u32>,
    /// Per exported blend channel, the model's channel index.
    pub(crate) channel_sources: Vec<u32>,
    pub(crate) source_node: Option<usize>,
    pub(crate) level: usize,
    pub(crate) skins: Vec<SkinExportData>,
    pub(crate) blend_channels: Vec<BlendChannelData>,
    /// Per face, empty when the source carried no such layer.
    pub(crate) face_smoothing: Vec<u8>,
    pub(crate) face_hole: Vec<u8>,
    pub(crate) face_group: Vec<i32>,
    /// Edges as corner positions, and their layers (empty when absent).
    pub(crate) edges: Vec<i32>,
    pub(crate) edge_smoothing: Vec<u8>,
    pub(crate) edge_crease: Vec<f64>,
    pub(crate) edge_visibility: Vec<u8>,
    /// Per vertex, empty when absent.
    pub(crate) vertex_crease: Vec<f64>,
    pub(crate) props: PropRange,
}
