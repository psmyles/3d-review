//! `#[repr(C)]` mirrors of the source-property capture `ufbx_extras.h` declares.
//!
//! Field for field, in declaration order. The capture mirrors ufbx's whole
//! element vocabulary, which is why this is the longest of the two mirror files;
//! [`super::marshal_extras`] turns it into `review_model::SourceExtras`.

use std::os::raw::c_char;

// ---- The source-property capture (`ufbx_extras.h`), field for field. ----

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct XStr {
    pub(super) offset: u32,
    pub(super) length: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct XBytes {
    pub(super) offset: usize,
    pub(super) length: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct XPropRange {
    pub(super) first: u32,
    pub(super) count: u32,
}

#[repr(C)]
pub(super) struct XProp {
    pub(super) name: XStr,
    pub(super) kind: u32,
    pub(super) flags: u32,
    pub(super) value_int: i64,
    pub(super) value_real: [f64; 4],
    pub(super) value_str: XStr,
    pub(super) value_blob: XBytes,
}

#[repr(C)]
pub(super) struct XNode {
    pub(super) props: XPropRange,
    pub(super) rotation_order: u32,
    pub(super) inherit_mode: u32,
    pub(super) original_inherit_mode: u32,
    pub(super) geometry_to_node: [f64; 16],
    pub(super) synthetic: u32,
    pub(super) visible: u32,
    pub(super) attribute_kind: u32,
    pub(super) attribute_index: i32,
    pub(super) attribute_name: XStr,
    pub(super) attribute_props: XPropRange,
}

#[repr(C)]
pub(super) struct XLight {
    pub(super) color: [f64; 3],
    pub(super) intensity: f64,
    pub(super) local_direction: [f64; 3],
    pub(super) kind: u32,
    pub(super) decay: u32,
    pub(super) area_shape: u32,
    pub(super) inner_angle: f64,
    pub(super) outer_angle: f64,
    pub(super) cast_light: u32,
    pub(super) cast_shadows: u32,
}

#[repr(C)]
pub(super) struct XCamera {
    pub(super) projection_mode: u32,
    pub(super) resolution_is_pixels: u32,
    pub(super) resolution: [f64; 2],
    pub(super) field_of_view_deg: [f64; 2],
    pub(super) orthographic_extent: f64,
    pub(super) aspect_ratio: f64,
    pub(super) near_plane: f64,
    pub(super) far_plane: f64,
    pub(super) aspect_mode: u32,
    pub(super) aperture_mode: u32,
    pub(super) gate_fit: u32,
    pub(super) aperture_format: u32,
    pub(super) focal_length_mm: f64,
    pub(super) film_size_inch: [f64; 2],
    pub(super) aperture_size_inch: [f64; 2],
    pub(super) squeeze_ratio: f64,
}

#[repr(C)]
pub(super) struct XLodGroup {
    pub(super) relative_distances: u32,
    pub(super) ignore_parent_transform: u32,
    pub(super) use_distance_limit: u32,
    pub(super) distance_limit_min: f64,
    pub(super) distance_limit_max: f64,
    pub(super) level_first: u32,
    pub(super) level_count: u32,
}

#[repr(C)]
pub(super) struct XLodLevel {
    pub(super) distance: f64,
    pub(super) display: u32,
}

#[repr(C)]
pub(super) struct XMaterial {
    pub(super) props: XPropRange,
    pub(super) shader_type: u32,
    pub(super) shading_model: XStr,
    pub(super) texture_first: u32,
    pub(super) texture_count: u32,
}

#[repr(C)]
pub(super) struct XMaterialTexture {
    pub(super) material_prop: XStr,
    pub(super) shader_prop: XStr,
    pub(super) texture: u32,
}

#[repr(C)]
pub(super) struct XTexture {
    pub(super) name: XStr,
    pub(super) kind: u32,
    pub(super) filename: XStr,
    pub(super) absolute_filename: XStr,
    pub(super) relative_filename: XStr,
    pub(super) uv_set: XStr,
    pub(super) wrap_u: u32,
    pub(super) wrap_v: u32,
    pub(super) has_uv_transform: u32,
    pub(super) uv_translation: [f64; 3],
    pub(super) uv_rotation: [f64; 4],
    pub(super) uv_scale: [f64; 3],
    pub(super) content: XBytes,
    pub(super) video: i32,
    pub(super) layer_first: u32,
    pub(super) layer_count: u32,
    pub(super) props: XPropRange,
}

#[repr(C)]
pub(super) struct XTextureLayer {
    pub(super) texture: u32,
    pub(super) blend_mode: u32,
    pub(super) alpha: f64,
}

#[repr(C)]
pub(super) struct XVideo {
    pub(super) name: XStr,
    pub(super) filename: XStr,
    pub(super) absolute_filename: XStr,
    pub(super) relative_filename: XStr,
    pub(super) content: XBytes,
    pub(super) props: XPropRange,
}

#[repr(C)]
pub(super) struct XColorSet {
    pub(super) name: XStr,
    pub(super) index: u32,
    pub(super) value_first: u32,
}

#[repr(C)]
pub(super) struct XUvSet {
    pub(super) name: XStr,
    pub(super) index: u32,
    pub(super) has_values: u32,
}

#[repr(C)]
pub(super) struct XUnusedVertex {
    pub(super) logical: u32,
    pub(super) position: [f32; 3],
}

#[repr(C)]
pub(super) struct XFaceGroup {
    pub(super) id: i32,
    pub(super) name: XStr,
}

#[repr(C)]
pub(super) struct XExtraSkin {
    pub(super) method: u32,
    pub(super) max_weights_per_vertex: u32,
    pub(super) cluster_first: u32,
    pub(super) cluster_count: u32,
    pub(super) offset_first: u32,
    pub(super) influence_first: u32,
    pub(super) influence_count: u32,
}

#[repr(C)]
pub(super) struct XExtraCluster {
    pub(super) bone: u32,
    pub(super) name: XStr,
    pub(super) mesh_node_to_bone: [f64; 16],
    pub(super) bind_to_world: [f64; 16],
}

#[repr(C)]
pub(super) struct XExtraInfluence {
    pub(super) cluster: u32,
    pub(super) weight: f32,
}

#[repr(C)]
pub(super) struct XDqWeight {
    pub(super) logical_vertex: u32,
    pub(super) weight: f64,
}

#[repr(C)]
pub(super) struct XMesh {
    pub(super) node: u32,
    pub(super) name: XStr,
    pub(super) props: XPropRange,
    pub(super) corner_first: u32,
    pub(super) corner_count: u32,
    pub(super) logical_first: u32,
    pub(super) logical_count: u32,
    pub(super) face_first: u32,
    pub(super) face_count: u32,
    pub(super) tangents_authored: u32,
    pub(super) reversed_winding: u32,
    pub(super) color_set_first: u32,
    pub(super) color_set_count: u32,
    pub(super) edge_first: u32,
    pub(super) edge_count: u32,
    pub(super) face_group_first: u32,
    pub(super) face_group_count: u32,
    pub(super) has_face_smoothing: u32,
    pub(super) has_face_hole: u32,
    pub(super) has_face_group: u32,
    pub(super) has_edge_smoothing: u32,
    pub(super) has_edge_crease: u32,
    pub(super) has_edge_visibility: u32,
    pub(super) has_vertex_crease: u32,
    pub(super) subdivision_preview_levels: u32,
    pub(super) subdivision_render_levels: u32,
    pub(super) subdivision_display_mode: u32,
    pub(super) subdivision_boundary: u32,
    pub(super) subdivision_uv_boundary: u32,
    pub(super) extra_skin_first: u32,
    pub(super) extra_skin_count: u32,
    pub(super) dq_first: u32,
    pub(super) dq_count: u32,
    pub(super) normals_authored: u32,
    pub(super) uv_set_first: u32,
    pub(super) uv_set_count: u32,
    pub(super) unused_vertex_first: u32,
    pub(super) unused_vertex_count: u32,
}

#[repr(C)]
pub(super) struct XPose {
    pub(super) name: XStr,
    pub(super) is_bind_pose: u32,
    pub(super) entry_first: u32,
    pub(super) entry_count: u32,
    pub(super) props: XPropRange,
}

#[repr(C)]
pub(super) struct XPoseEntry {
    pub(super) node: u32,
    pub(super) bone_to_world: [f64; 16],
}

#[repr(C)]
pub(super) struct XDisplayLayer {
    pub(super) name: XStr,
    pub(super) visible: u32,
    pub(super) frozen: u32,
    pub(super) ui_color: [f64; 3],
    pub(super) node_first: u32,
    pub(super) node_count: u32,
    pub(super) props: XPropRange,
}

#[repr(C)]
pub(super) struct XSelectionSet {
    pub(super) name: XStr,
    pub(super) props: XPropRange,
    pub(super) node_first: u32,
    pub(super) node_count: u32,
}

#[repr(C)]
pub(super) struct XSelectionNode {
    pub(super) node: i32,
    pub(super) include_node: u32,
    pub(super) vertex_first: u32,
    pub(super) vertex_count: u32,
    pub(super) edge_first: u32,
    pub(super) edge_count: u32,
    pub(super) face_first: u32,
    pub(super) face_count: u32,
}

#[repr(C)]
pub(super) struct XAnimStack {
    pub(super) name: XStr,
    pub(super) props: XPropRange,
    pub(super) clip: i32,
    pub(super) time_begin: f64,
    pub(super) time_end: f64,
    pub(super) layer_first: u32,
    pub(super) layer_count: u32,
}

#[repr(C)]
pub(super) struct XAnimLayer {
    pub(super) name: XStr,
    pub(super) weight: f64,
    pub(super) weight_is_animated: u32,
    pub(super) blended: u32,
    pub(super) additive: u32,
    pub(super) compose_rotation: u32,
    pub(super) compose_scale: u32,
    pub(super) props: XPropRange,
    pub(super) anim_prop_first: u32,
    pub(super) anim_prop_count: u32,
}

#[repr(C)]
pub(super) struct XAnimProp {
    pub(super) target_kind: u32,
    pub(super) target: u32,
    pub(super) element_type: u32,
    pub(super) element_name: XStr,
    pub(super) prop_name: XStr,
    pub(super) default_value: [f64; 3],
    pub(super) curves: [i32; 3],
}

#[repr(C)]
pub(super) struct XAnimCurve {
    pub(super) key_first: u32,
    pub(super) key_count: u32,
    pub(super) pre_mode: u32,
    pub(super) pre_repeat: i32,
    pub(super) post_mode: u32,
    pub(super) post_repeat: i32,
}

#[repr(C)]
pub(super) struct XAnimKey {
    pub(super) time: f64,
    pub(super) value: f64,
    pub(super) interpolation: u32,
    pub(super) left_dx: f32,
    pub(super) left_dy: f32,
    pub(super) right_dx: f32,
    pub(super) right_dy: f32,
}

#[repr(C)]
pub(super) struct XScene {
    pub(super) creator: XStr,
    pub(super) filename: XStr,
    pub(super) original_file_path: XStr,
    pub(super) version: u32,
    pub(super) ascii: u32,
    pub(super) original_vendor: XStr,
    pub(super) original_name: XStr,
    pub(super) original_version: XStr,
    pub(super) latest_vendor: XStr,
    pub(super) latest_name: XStr,
    pub(super) latest_version: XStr,
    pub(super) scene_props: XPropRange,
    pub(super) settings_props: XPropRange,
    pub(super) axis_right: u32,
    pub(super) axis_up: u32,
    pub(super) axis_front: u32,
    pub(super) original_axis_up: u32,
    pub(super) unit_meters: f64,
    pub(super) original_unit_meters: f64,
    pub(super) frames_per_second: f64,
    pub(super) ambient_color: [f64; 3],
    pub(super) default_camera: XStr,
    pub(super) time_mode: u32,
    pub(super) time_protocol: u32,
    pub(super) snap_mode: u32,
}

#[repr(C)]
pub(super) struct ReviewImportExtras {
    pub(super) strings: *mut c_char,
    pub(super) string_count: usize,
    pub(super) string_capacity: usize,
    pub(super) bytes: *mut u8,
    pub(super) byte_count: usize,
    pub(super) byte_capacity: usize,
    pub(super) props: *mut XProp,
    pub(super) prop_count: usize,
    pub(super) scene: XScene,
    pub(super) nodes: *mut XNode,
    pub(super) node_count: usize,
    pub(super) lights: *mut XLight,
    pub(super) light_count: usize,
    pub(super) cameras: *mut XCamera,
    pub(super) camera_count: usize,
    pub(super) lod_groups: *mut XLodGroup,
    pub(super) lod_group_count: usize,
    pub(super) lod_levels: *mut XLodLevel,
    pub(super) lod_level_count: usize,
    pub(super) materials: *mut XMaterial,
    pub(super) material_count: usize,
    pub(super) material_textures: *mut XMaterialTexture,
    pub(super) material_texture_count: usize,
    pub(super) textures: *mut XTexture,
    pub(super) texture_count: usize,
    pub(super) texture_layers: *mut XTextureLayer,
    pub(super) texture_layer_count: usize,
    pub(super) videos: *mut XVideo,
    pub(super) video_count: usize,
    pub(super) meshes: *mut XMesh,
    pub(super) mesh_count: usize,
    pub(super) color_sets: *mut XColorSet,
    pub(super) color_set_count: usize,
    pub(super) color_values: *mut f64,
    pub(super) color_value_count: usize,
    pub(super) edges: *mut u32,
    pub(super) edge_smoothing: *mut u8,
    pub(super) edge_crease: *mut f64,
    pub(super) edge_visibility: *mut u8,
    pub(super) edge_count: usize,
    pub(super) face_smoothing: *mut u8,
    pub(super) face_hole: *mut u8,
    pub(super) face_group: *mut u32,
    pub(super) face_count: usize,
    pub(super) vertex_crease: *mut f64,
    pub(super) vertex_crease_count: usize,
    pub(super) face_groups: *mut XFaceGroup,
    pub(super) face_group_count: usize,
    pub(super) extra_skins: *mut XExtraSkin,
    pub(super) extra_skin_count: usize,
    pub(super) extra_clusters: *mut XExtraCluster,
    pub(super) extra_cluster_count: usize,
    pub(super) extra_skin_offsets: *mut u32,
    pub(super) extra_skin_offset_count: usize,
    pub(super) extra_influences: *mut XExtraInfluence,
    pub(super) extra_influence_count: usize,
    pub(super) dq_weights: *mut XDqWeight,
    pub(super) dq_weight_count: usize,
    pub(super) poses: *mut XPose,
    pub(super) pose_count: usize,
    pub(super) pose_entries: *mut XPoseEntry,
    pub(super) pose_entry_count: usize,
    pub(super) display_layers: *mut XDisplayLayer,
    pub(super) display_layer_count: usize,
    pub(super) layer_nodes: *mut u32,
    pub(super) layer_node_count: usize,
    pub(super) selection_sets: *mut XSelectionSet,
    pub(super) selection_set_count: usize,
    pub(super) selection_nodes: *mut XSelectionNode,
    pub(super) selection_node_count: usize,
    pub(super) selection_indices: *mut u32,
    pub(super) selection_index_count: usize,
    pub(super) anim_stacks: *mut XAnimStack,
    pub(super) anim_stack_count: usize,
    pub(super) stack_layers: *mut u32,
    pub(super) stack_layer_count: usize,
    pub(super) anim_layers: *mut XAnimLayer,
    pub(super) anim_layer_count: usize,
    pub(super) anim_props: *mut XAnimProp,
    pub(super) anim_prop_count: usize,
    pub(super) anim_curves: *mut XAnimCurve,
    pub(super) anim_curve_count: usize,
    pub(super) anim_keys: *mut XAnimKey,
    pub(super) anim_key_count: usize,

    pub(super) uv_sets: *mut XUvSet,
    pub(super) uv_set_count: usize,
    pub(super) unused_vertices: *mut XUnusedVertex,
    pub(super) unused_vertex_count: usize,
}
