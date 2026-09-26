//! Raw declarations for the export bridge (`src/export_bridge.c`).
//!
//! Kept beside [`crate::ffi`] as the crate's second and last `unsafe` surface.
//! The structs mirror `export_bridge.h` field for field; changing one without
//! the other is the single thing the compiler cannot catch here, so they are
//! written in the same order with the same names.
//!
//! Unlike the meshoptimizer bindings, this one goes through a hand-written C
//! bridge rather than calling the library directly: ufbx_write's API is handle-
//! and setter-based, so driving it from Rust would spread `unsafe` across a
//! hundred call sites and leak its scene lifetime into Rust. The bridge does the
//! whole write in one call with a single ownership story.

#![allow(
    unsafe_code,
    reason = "invariant 9: the raw export bridge declarations"
)]
#![cfg(has_ufbxw)]

use std::ffi::{c_char, c_int};

/// Must equal `RVO_EXPORT_ERROR_LENGTH` in `export_bridge.h`.
pub const ERROR_LENGTH: usize = 512;

#[repr(C)]
pub struct RvoExportProp {
    pub name: *const c_char,
    pub kind: u32,
    pub flags: u32,
    pub value_int: i64,
    pub value_real: [f64; 4],
    pub value_str: *const c_char,
    pub blob: *const u8,
    pub blob_length: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RvoExportPropRange {
    pub first: u32,
    pub count: u32,
}

#[repr(C)]
pub struct RvoExportNode {
    pub name: *const c_char,
    pub parent: i32,
    pub translation: [f64; 3],
    /// Quaternion, `(x, y, z, w)`.
    pub rotation: [f64; 4],
    pub scaling: [f64; 3],
    pub authored_transform: i32,
    pub props: RvoExportPropRange,
    pub attribute_kind: u32,
    pub attribute_name: *const c_char,
    pub attribute_props: RvoExportPropRange,
}

#[repr(C)]
pub struct RvoExportMaterialTexture {
    pub prop: *const c_char,
    pub texture: i32,
}

#[repr(C)]
pub struct RvoExportMaterial {
    pub name: *const c_char,
    pub shader: u32,
    pub shading_model: *const c_char,
    pub props: RvoExportPropRange,
    pub base_color: [f64; 3],
    pub emissive: [f64; 3],
    pub shininess_exponent: f64,
    pub reflection_factor: f64,
    pub textures: *const RvoExportMaterialTexture,
    pub texture_count: usize,
}

#[repr(C)]
pub struct RvoExportTextureLayer {
    pub texture: i32,
    pub blend_mode: i32,
    pub alpha: f64,
}

#[repr(C)]
pub struct RvoExportTexture {
    pub name: *const c_char,
    pub layered: i32,
    pub filename: *const c_char,
    pub relative_filename: *const c_char,
    pub content: *const u8,
    pub content_length: usize,
    pub video: i32,
    pub props: RvoExportPropRange,
    pub layers: *const RvoExportTextureLayer,
    pub layer_count: usize,
}

#[repr(C)]
pub struct RvoExportVideo {
    pub name: *const c_char,
    pub filename: *const c_char,
    pub relative_filename: *const c_char,
    pub content: *const u8,
    pub content_length: usize,
    pub props: RvoExportPropRange,
}

#[repr(C)]
pub struct RvoExportCluster {
    pub bone: i32,
    pub name: *const c_char,
    pub transform: [f64; 16],
    pub transform_link: [f64; 16],
    pub vertices: *const i32,
    pub weights: *const f64,
    pub weight_count: usize,
}

#[repr(C)]
pub struct RvoExportSkin {
    pub skinning_type: u32,
    pub clusters: *const RvoExportCluster,
    pub cluster_count: usize,
    pub dq_vertices: *const i32,
    pub dq_weights: *const f64,
    pub dq_count: usize,
    pub bind_pose: i32,
}

#[repr(C)]
pub struct RvoExportBlendShape {
    pub name: *const c_char,
    pub vertices: *const i32,
    pub offsets: *const f64,
    pub normals: *const f64,
    pub offset_count: usize,
    pub target_weight: f64,
}

#[repr(C)]
pub struct RvoExportBlendChannel {
    pub name: *const c_char,
    pub weight: f64,
    pub shapes: *const RvoExportBlendShape,
    pub shape_count: usize,
}

#[repr(C)]
pub struct RvoExportPoseNode {
    pub node: i32,
    pub matrix: [f64; 16],
}

#[repr(C)]
pub struct RvoExportPose {
    pub name: *const c_char,
    pub nodes: *const RvoExportPoseNode,
    pub node_count: usize,
}

#[repr(C)]
pub struct RvoExportKey {
    pub time: i64,
    pub value: f64,
    pub flags: u32,
    pub weight_left: f64,
    pub weight_right: f64,
    pub slope_left: f64,
    pub slope_right: f64,
}

#[repr(C)]
pub struct RvoExportCurve {
    pub keys: *const RvoExportKey,
    pub key_count: usize,
    pub pre_mode: u32,
    pub pre_repeat: i32,
    pub post_mode: u32,
    pub post_repeat: i32,
}

#[repr(C)]
pub struct RvoExportAnimProp {
    pub target_kind: u32,
    pub target: i32,
    pub target2: i32,
    pub prop_name: *const c_char,
    pub default_value: [f64; 3],
    pub curves: [*const RvoExportCurve; 3],
}

#[repr(C)]
pub struct RvoExportAnimLayer {
    pub name: *const c_char,
    pub stack: i32,
    pub weight: f64,
    pub props: RvoExportPropRange,
    pub anim_props: *const RvoExportAnimProp,
    pub anim_prop_count: usize,
}

#[repr(C)]
pub struct RvoExportAnimStack {
    pub name: *const c_char,
    pub props: RvoExportPropRange,
    pub time_begin: i64,
    pub time_end: i64,
}

#[repr(C)]
pub struct RvoExportDisplayLayer {
    pub name: *const c_char,
    pub props: RvoExportPropRange,
    pub nodes: *const i32,
    pub node_count: usize,
}

#[repr(C)]
pub struct RvoExportSelectionNode {
    pub node: i32,
    pub include_node: i32,
    pub vertices: *const i32,
    pub vertex_count: usize,
    pub edges: *const i32,
    pub edge_count: usize,
    pub faces: *const i32,
    pub face_count: usize,
}

#[repr(C)]
pub struct RvoExportSelectionSet {
    pub name: *const c_char,
    pub props: RvoExportPropRange,
    pub nodes: *const RvoExportSelectionNode,
    pub node_count: usize,
}

#[repr(C)]
pub struct RvoExportColorSet {
    pub name: *const c_char,
    pub values: *const f64,
}

#[repr(C)]
pub struct RvoExportMesh {
    pub name: *const c_char,
    pub node: i32,
    pub positions: *const f64,
    pub vertex_count: usize,
    pub indices: *const i32,
    pub index_count: usize,
    pub face_offsets: *const i32,
    pub face_count: usize,
    pub normals: *const f64,
    pub colors: *const f64,
    pub tangents: *const f64,
    pub uv_sets: *const *const f64,
    pub uv_set_names: *const *const c_char,
    pub uv_set_count: usize,
    pub material_slots: *const i32,
    pub material_slot_count: usize,
    pub face_materials: *const i32,
    pub color_set_name: *const c_char,
    pub color_sets: *const RvoExportColorSet,
    pub color_set_count: usize,
    pub face_smoothing: *const u8,
    pub face_hole: *const u8,
    pub face_group: *const i32,
    pub edges: *const i32,
    pub edge_count: usize,
    pub edge_smoothing: *const u8,
    pub edge_crease: *const f64,
    pub edge_visibility: *const u8,
    pub vertex_crease: *const f64,
    pub props: RvoExportPropRange,
    pub skins: *const RvoExportSkin,
    pub skin_count: usize,
    pub blend_channels: *const RvoExportBlendChannel,
    pub blend_channel_count: usize,
}

#[repr(C)]
pub struct RvoExportScene {
    /// Centimeters per scene unit (FBX `UnitScaleFactor`). See
    /// [`crate::export::UNIT_SCALE_CM`].
    pub unit_scale_cm: f64,
    pub props: *const RvoExportProp,
    pub prop_count: usize,
    pub axis_right: i32,
    pub axis_up: i32,
    pub axis_front: i32,
    pub time_mode: i32,
    pub frame_rate: f64,
    pub settings_props: RvoExportPropRange,
    pub scene_info_props: RvoExportPropRange,
    pub original_application_vendor: *const c_char,
    pub original_application_name: *const c_char,
    pub original_application_version: *const c_char,
    pub original_filename: *const c_char,
    pub application_name: *const c_char,
    pub application_version: *const c_char,
    pub nodes: *const RvoExportNode,
    pub node_count: usize,
    pub materials: *const RvoExportMaterial,
    pub material_count: usize,
    pub textures: *const RvoExportTexture,
    pub texture_count: usize,
    pub videos: *const RvoExportVideo,
    pub video_count: usize,
    pub meshes: *const RvoExportMesh,
    pub mesh_count: usize,
    pub poses: *const RvoExportPose,
    pub pose_count: usize,
    pub anim_stacks: *const RvoExportAnimStack,
    pub anim_stack_count: usize,
    pub anim_layers: *const RvoExportAnimLayer,
    pub anim_layer_count: usize,
    pub active_stack: i32,
    pub display_layers: *const RvoExportDisplayLayer,
    pub display_layer_count: usize,
    pub selection_sets: *const RvoExportSelectionSet,
    pub selection_set_count: usize,
}

unsafe extern "C" {
    /// Returns 0 on success; on failure writes a NUL-terminated message into
    /// `error` (which must have room for `error_length` bytes).
    pub fn review_export_fbx(
        scene: *const RvoExportScene,
        path: *const c_char,
        ascii: c_int,
        error: *mut c_char,
        error_length: usize,
    ) -> c_int;
}

// The test-only probe of the vendored writer's patches (`src/ufbxw_probe.c`);
// see `crate::probe`. Same error contract as `review_export_fbx`.
#[cfg(has_ufbxw_probe)]
unsafe extern "C" {
    pub fn review_ufbxw_patch_probe(
        path: *const c_char,
        ascii: c_int,
        error: *mut c_char,
        error_length: usize,
    ) -> c_int;
}
