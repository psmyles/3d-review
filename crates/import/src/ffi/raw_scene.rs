//! `#[repr(C)]` mirrors of the geometry and rig structs `ufbx_bridge.h` declares.
//!
//! Field for field, in declaration order. Nothing here has behavior: these are
//! the shapes the bridge writes and [`super::marshal_model`] reads.

use std::ffi::c_void;
use std::os::raw::c_char;

#[repr(C)]
pub(super) struct ReviewImportVertex {
    pub(super) position: [f32; 3],
    pub(super) normal: [f32; 3],
    pub(super) uv: [f32; 2],
    pub(super) tangent: [f32; 4],
    pub(super) vertex_color: [f32; 4],
}

#[repr(C)]
pub(super) struct ReviewImportFace {
    pub(super) first_index: u32,
    pub(super) index_count: u32,
}

#[repr(C)]
pub(super) struct ReviewImportMaterial {
    pub(super) name: *mut c_char,
    pub(super) base_color: [f32; 3],
    pub(super) smoothness: f32,
    pub(super) metallic: f32,
    pub(super) emissive: [f32; 3],
    /// The bridge's own back-reference to the ufbx material; opaque here.
    pub(super) _source: *const c_void,
}

#[repr(C)]
pub(super) struct ReviewImportNode {
    pub(super) name: *mut c_char,
    pub(super) parent: i32,
    pub(super) mesh_part_index: i32,
    /// This node's mesh's own logical (DCC) vertex count, 0 for a node with
    /// no mesh. Summing it over the mesh-bearing nodes reproduces
    /// `source_vertex_count`.
    pub(super) source_vertex_count: u32,
    pub(super) transform: [f32; 16],
    /// A `review_import_node_kind` code; see [`node_kind_from_code`].
    pub(super) kind: u32,
    pub(super) bone_radius: f32,
    pub(super) bone_relative_length: f32,
    /// The rest local transform: translation, rotation (xyzw), scale.
    pub(super) local_translation: [f32; 3],
    pub(super) local_rotation: [f32; 4],
    pub(super) local_scale: [f32; 3],
}

#[repr(C)]
pub(super) struct ReviewImportSkinCluster {
    pub(super) bone: u32,
    pub(super) mesh_node: u32,
    pub(super) world_to_bone_bind: [f32; 16],
    pub(super) mesh_node_to_bone: [f64; 16],
    pub(super) bind_to_world: [f64; 16],
    pub(super) name: *mut c_char,
}

#[repr(C)]
pub(super) struct ReviewImportSkinDeformer {
    pub(super) mesh_node: u32,
    /// `ufbx_skinning_method` code; see [`skinning_method_from_code`].
    pub(super) method: u32,
    pub(super) max_weights_per_vertex: u32,
}

#[repr(C)]
pub(super) struct ReviewImportMorphChannel {
    pub(super) name: *mut c_char,
    pub(super) mesh_node: u32,
    pub(super) rest_weight: f32,
    pub(super) keyframe_first: u32,
    pub(super) keyframe_count: u32,
}

#[repr(C)]
pub(super) struct ReviewImportMorphKeyframe {
    pub(super) shape: u32,
    pub(super) target_weight: f32,
}

#[repr(C)]
pub(super) struct ReviewImportMorphShape {
    pub(super) name: *mut c_char,
}

#[repr(C)]
pub(super) struct ReviewImportMorphEntry {
    pub(super) logical_vertex: u32,
    pub(super) shape: u32,
    pub(super) position: [f32; 3],
    pub(super) normal: [f32; 3],
}

#[repr(C)]
pub(super) struct ReviewImportAnimStack {
    pub(super) name: *mut c_char,
    pub(super) time_begin: f64,
    pub(super) time_end: f64,
    pub(super) node_track_first: u32,
    pub(super) node_track_count: u32,
    pub(super) morph_track_first: u32,
    pub(super) morph_track_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct ReviewImportNodeTrack {
    pub(super) node: u32,
    pub(super) translation_first: u32,
    pub(super) translation_count: u32,
    pub(super) rotation_first: u32,
    pub(super) rotation_count: u32,
    pub(super) scale_first: u32,
    pub(super) scale_count: u32,
}

#[repr(C)]
pub(super) struct ReviewImportVec3Key {
    pub(super) time: f64,
    pub(super) value: [f32; 3],
}

#[repr(C)]
pub(super) struct ReviewImportQuatKey {
    pub(super) time: f64,
    pub(super) value: [f32; 4],
}

#[repr(C)]
pub(super) struct ReviewImportMorphTrack {
    pub(super) channel: u32,
    pub(super) first: u32,
    pub(super) count: u32,
}

#[repr(C)]
pub(super) struct ReviewImportScalarKey {
    pub(super) time: f64,
    pub(super) value: f32,
}

#[repr(C)]
pub(super) struct ReviewImportScene {
    pub(super) vertices: *mut ReviewImportVertex,
    pub(super) vertex_count: usize,
    pub(super) indices: *mut u32,
    pub(super) index_count: usize,
    pub(super) faces: *mut ReviewImportFace,
    pub(super) face_count: usize,
    pub(super) tri_to_face: *mut u32,
    pub(super) tri_to_face_count: usize,
    pub(super) materials: *mut ReviewImportMaterial,
    pub(super) material_count: usize,
    pub(super) uv_set_count: u32,
    pub(super) uvs: *mut f32,
    pub(super) uv_value_count: usize,
    /// Source DCC logical vertex count (invariant 5's Verts stat);
    /// `vertex_count` above is the per-corner expanded array length.
    pub(super) source_vertex_count: usize,
    pub(super) source_unit_meters: f32,
    pub(super) uv_set_names: *mut *mut c_char,
    pub(super) uv_set_name_count: usize,
    pub(super) nodes: *mut ReviewImportNode,
    pub(super) node_count: usize,
    pub(super) tri_material: *mut u32,
    pub(super) tri_material_count: usize,
    pub(super) tri_node: *mut u32,
    pub(super) tri_node_count: usize,
    /// Per expanded corner, the logical source vertex it came from — the
    /// index the CSR skin rows below are keyed by.
    pub(super) corner_source_vertex: *mut u32,
    pub(super) corner_source_vertex_count: usize,
    /// CSR skin weights over the logical vertices; all null / zero for an
    /// unskinned scene.
    pub(super) skin_offsets: *mut u32,
    pub(super) skin_offset_count: usize,
    pub(super) skin_bones: *mut u32,
    pub(super) skin_weights: *mut f32,
    pub(super) skin_influence_count: usize,
    /// Per influence (parallel to `skin_bones`), its `skin_clusters` index.
    pub(super) skin_influence_cluster: *mut u32,
    pub(super) skin_clusters: *mut ReviewImportSkinCluster,
    pub(super) skin_cluster_count: usize,
    pub(super) skin_deformers: *mut ReviewImportSkinDeformer,
    pub(super) skin_deformer_count: usize,
    /// Blend shapes; all null / zero when no mesh carries any.
    pub(super) morph_channels: *mut ReviewImportMorphChannel,
    pub(super) morph_channel_count: usize,
    pub(super) morph_keyframes: *mut ReviewImportMorphKeyframe,
    pub(super) morph_keyframe_count: usize,
    pub(super) morph_shapes: *mut ReviewImportMorphShape,
    pub(super) morph_shape_count: usize,
    pub(super) morph_entries: *mut ReviewImportMorphEntry,
    pub(super) morph_entry_count: usize,
    /// Animation clips; all null / zero for a file without animation.
    pub(super) anim_stacks: *mut ReviewImportAnimStack,
    pub(super) anim_stack_count: usize,
    pub(super) anim_node_tracks: *mut ReviewImportNodeTrack,
    pub(super) anim_node_track_count: usize,
    pub(super) anim_vec3_keys: *mut ReviewImportVec3Key,
    pub(super) anim_vec3_key_count: usize,
    pub(super) anim_quat_keys: *mut ReviewImportQuatKey,
    pub(super) anim_quat_key_count: usize,
    pub(super) anim_morph_tracks: *mut ReviewImportMorphTrack,
    pub(super) anim_morph_track_count: usize,
    pub(super) anim_scalar_keys: *mut ReviewImportScalarKey,
    pub(super) anim_scalar_key_count: usize,
    pub(super) frames_per_second: f64,
}

#[repr(C)]
pub(super) struct ReviewImportError {
    pub(super) message: [c_char; 256],
}
