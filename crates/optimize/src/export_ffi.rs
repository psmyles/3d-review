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
#![cfg(has_ufbxw)]

use std::ffi::{c_char, c_int};

/// Must equal `RVO_EXPORT_ERROR_LENGTH` in `export_bridge.h`.
pub const ERROR_LENGTH: usize = 512;

#[repr(C)]
pub struct RvoExportNode {
    pub name: *const c_char,
    pub parent: i32,
    pub translation: [f64; 3],
    /// Quaternion, `(x, y, z, w)`.
    pub rotation: [f64; 4],
    pub scaling: [f64; 3],
}

#[repr(C)]
pub struct RvoExportMaterial {
    pub name: *const c_char,
    pub base_color: [f64; 3],
    pub emissive: [f64; 3],
}

#[repr(C)]
pub struct RvoExportMesh {
    pub name: *const c_char,
    pub node: i32,
    pub positions: *const f64,
    pub vertex_count: usize,
    pub indices: *const i32,
    pub triangle_count: usize,
    pub normals: *const f64,
    pub colors: *const f64,
    pub uv_sets: *const *const f64,
    pub uv_set_names: *const *const c_char,
    pub uv_set_count: usize,
    pub material_slots: *const i32,
    pub material_slot_count: usize,
    pub face_materials: *const i32,
}

#[repr(C)]
pub struct RvoExportScene {
    /// Centimeters per scene unit (FBX `UnitScaleFactor`). See
    /// [`crate::export::UNIT_SCALE_CM`].
    pub unit_scale_cm: f64,
    pub nodes: *const RvoExportNode,
    pub node_count: usize,
    pub materials: *const RvoExportMaterial,
    pub material_count: usize,
    pub meshes: *const RvoExportMesh,
    pub mesh_count: usize,
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
