//! Pointer-to-Rust leaf helpers — the last of this crate's `unsafe` outside the
//! bridge call itself.
//!
//! Each one turns a raw pointer the bridge wrote into something safe: a checked
//! slice, an owned `String`, an error message. A null pointer with a non-zero
//! length is an error rather than a dereference, and a zero length is empty
//! rather than a null check.
//!
//! ## Why the accessors are here rather than at the call sites
//!
//! [`checked_slice`] is `unsafe` and takes a bare pointer, which means it cannot
//! promise anything about *where* that pointer came from or how long it stays
//! valid. It used to be a safe function returning `&'a [T]` for a lifetime the
//! caller chose freely — a signature that claims a guarantee the arguments do not
//! carry. The convention it rested on (the marshal walk holds
//! `&ReviewImportScene` throughout) was real, but nothing enforced it, and a
//! later safe caller could have asked for a longer borrow or passed a pointer
//! from anywhere.
//!
//! The bottom of the file turns each bridge array into a method on the struct
//! that owns it, so the returned slice borrows that struct and the compiler is
//! what keeps the scene alive for as long as anything reads it. That confines the
//! `unsafe` to this file — [`super::marshal_model`] and [`super::marshal_extras`]
//! keep `deny(unsafe_code)`, and now no raw pointer even reaches them — and it is
//! what lets [`checked_slice`] state its preconditions honestly.
//!
//! ## Why the accessors live on [`Bridged`] and not on the mirrors
//!
//! A borrow alone does not make an accessor sound. The mirror structs are plain
//! data with `pub(super)` fields, so any safe code in `ffi` could build one — or
//! set a count on one — and an accessor on the mirror itself would hand
//! `from_raw_parts` a pointer and a length nothing vouches for: undefined
//! behaviour without an `unsafe` anywhere in sight. The accessors therefore live
//! on [`Bridged`], which only `unsafe` code can construct and which hands its
//! contents out read-only, so the one promise every accessor rests on — "these
//! pointers and counts are the ones the bridge wrote" — is made once, at an
//! `unsafe` call that says so, and cannot be broken afterwards.

#![allow(
    unsafe_code,
    reason = "invariant 9: the pointer-to-slice leaves every safe accessor is built on"
)]

use std::ffi::CStr;
use std::ops::Deref;
use std::os::raw::c_char;
use std::ptr::NonNull;
use std::slice;

use crate::ImportError;

use super::raw_extras::*;
use super::raw_scene::*;

/// A bridge struct whose pointer + count pairs are vouched for: what the C side
/// filled (or, in a test, what the test pointed at its own arrays).
///
/// The contents are reachable only through [`Deref`] — read-only — so nothing
/// safe can change a count or a pointer once the value exists, and the only way
/// to make one is the `unsafe` [`Bridged::new`]. Every array accessor below is a
/// method on `Bridged<_>`, and element types that carry pointers of their own
/// (a name) are handed out as `Bridged<_>` too, by the accessor that read them.
#[repr(transparent)]
pub(super) struct Bridged<T>(T);

impl<T> Bridged<T> {
    /// Vouch for `value`.
    ///
    /// # Safety
    ///
    /// For as long as the returned value lives:
    ///
    /// * every non-null array pointer in `value` (at any depth an accessor reads)
    ///   points at at least as many contiguous, initialized, properly aligned
    ///   elements as the count the accessor pairs it with, in one allocation that
    ///   nothing writes to or frees;
    /// * every non-null name pointer is a NUL-terminated string that stays valid
    ///   and unchanged as long.
    ///
    /// A null pointer is always allowed: an accessor reports it as an error
    /// rather than reading it. An all-zero value satisfies this trivially.
    pub(super) unsafe fn new(value: T) -> Self {
        Self(value)
    }

    /// A raw pointer to the contents, for handing to C — the bridge's fill and
    /// free. Using it is `unsafe` like any other raw pointer, which is what keeps
    /// the contents unwritable from safe code.
    pub(super) fn as_mut_ptr(&mut self) -> *mut T {
        &raw mut self.0
    }
}

impl<T> Deref for Bridged<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

/// The bridge's geometry scene, vouched for.
pub(super) type BridgeScene = Bridged<ReviewImportScene>;
/// The bridge's source-property capture, vouched for.
pub(super) type BridgeExtras = Bridged<ReviewImportExtras>;

pub(super) fn read_error_message(error: &ReviewImportError) -> String {
    // SAFETY: `error.message` is a fixed 256-byte array the bridge always writes
    // as a NUL-terminated string (it is zero-initialized at `[0; 256]` before the
    // call), so `from_ptr` reads a valid C string bounded by the array.
    unsafe {
        CStr::from_ptr(error.message.as_ptr())
            .to_str()
            .ok()
            .filter(|message| !message.is_empty())
            .unwrap_or("unknown FBX import error")
            .to_owned()
    }
}

/// Read a bridge-owned C string, or `None` for a null pointer.
///
/// # Safety
///
/// `value` must be null, or point at a NUL-terminated C string that is valid for
/// the duration of the call. Every caller is an accessor below, reading a pointer
/// out of a `&self` the bridge filled.
pub(super) unsafe fn read_optional_c_string(value: *const c_char) -> Option<String> {
    if value.is_null() {
        return None;
    }

    // SAFETY: `value` is non-null (checked above), and the caller guarantees it
    // points at a live NUL-terminated C string — which for every caller here is
    // one the bridge allocated and holds until its scene or capture is freed.
    let text = unsafe { CStr::from_ptr(value) };
    // Lossy: an FBX name in some other encoding still has to reach the
    // Outliner as *something* — dropping it would leave a blank row with no
    // way to tell it from an unnamed node.
    Some(String::from_utf8_lossy(text.to_bytes()).into_owned())
}

/// A bridge array as a slice, or a `LoadFailed` naming the field when the bridge
/// handed back a null pointer for a non-empty one.
///
/// A zero length is the empty slice whatever the pointer is, so an absent array
/// never needs a null check of its own.
///
/// # Safety
///
/// When `len > 0`, `ptr` must point at `len` contiguous, initialized, properly
/// aligned `T` values in a single allocation that stays valid — and unwritten by
/// anything else — for the whole of `'a`. Nothing in the signature establishes
/// that, which is why the only callers are the accessors below: each reads a
/// pointer out of a `&'a self` the bridge filled, so `'a` is the borrow of the
/// struct that owns the allocation and the scene cannot be freed while the slice
/// lives.
pub(super) unsafe fn checked_slice<'a, T>(
    ptr: *const T,
    len: usize,
    field_name: &str,
) -> Result<&'a [T], ImportError> {
    if len == 0 {
        return Ok(&[]);
    }

    let Some(ptr) = NonNull::new(ptr as *mut T) else {
        return Err(ImportError::LoadFailed(format!(
            "FBX bridge returned a null pointer for non-empty {field_name}"
        )));
    };

    // SAFETY: `ptr` is non-null (just checked) and `len > 0`, so the slice is
    // non-empty; the caller guarantees the rest — `len` contiguous, initialized,
    // aligned `T` in one allocation live for `'a`.
    Ok(unsafe { slice::from_raw_parts(ptr.as_ptr(), len) })
}

// ---------------------------------------------------------------------------
// Safe accessors: one per bridge array, borrowing the struct that owns it
// ---------------------------------------------------------------------------

/// Generate a `&self -> Result<&[T], ImportError>` accessor per bridge array, on
/// [`Bridged`]`<$owner>`.
///
/// Each entry is `method: Element = (pointer_field, length_field)`, with an
/// optional `* n` for an array whose length is a multiple of its count (an edge
/// list is two vertex indices per edge). Several arrays deliberately share one
/// length field — the per-edge and per-face layers, and the three parallel skin
/// influence streams — which is the bridge's own contract and the reason the
/// length is named rather than derived. An element type that carries pointers of
/// its own is named as `Bridged<Mirror>`: the slice is handed out as vouched-for
/// elements, so their name accessors are sound for the same reason.
macro_rules! slice_accessors {
    (
        $owner:ty {
            $( $name:ident : $elem:ty = ($ptr:ident, $len:ident $( * $mul:literal )? ) ; )*
        }
    ) => {
        impl Bridged<$owner> {
            $(
                #[doc = concat!(
                    "The bridge's `", stringify!($ptr), "` array, borrowed from this struct."
                )]
                pub(super) fn $name(&self) -> Result<&[$elem], ImportError> {
                    let len = self.0.$len $( .saturating_mul($mul) )?;
                    // SAFETY: `Bridged::new`'s contract is that this non-null
                    // pointer addresses `len` contiguous, initialized, aligned
                    // values in one allocation that stays put for as long as
                    // `self` lives — what the bridge fills, and frees only when
                    // the scene or capture is. The returned slice borrows `self`.
                    // A `Bridged<E>` element is `repr(transparent)` over `E`, so
                    // the cast changes the type and not the layout.
                    unsafe { checked_slice(self.0.$ptr.cast::<$elem>().cast_const(), len, stringify!($ptr)) }
                }
            )*
        }
    };
}

/// Generate a `&self -> Option<String>` accessor per bridge name pointer, on
/// [`Bridged`]`<$owner>`.
macro_rules! name_accessors {
    ( $( $owner:ty => $field:ident ; )* ) => {
        $(
            impl Bridged<$owner> {
                #[doc = concat!("The bridge's `", stringify!($field), "` string, if it wrote one.")]
                pub(super) fn $field(&self) -> Option<String> {
                    // SAFETY: `Bridged::new`'s contract: the field is null or a
                    // NUL-terminated string that stays valid as long as `self`,
                    // which this call borrows.
                    unsafe { read_optional_c_string(self.0.$field) }
                }
            }
        )*
    };
}

slice_accessors!(ReviewImportScene {
    vertices: ReviewImportVertex = (vertices, vertex_count);
    indices: u32 = (indices, index_count);
    faces: ReviewImportFace = (faces, face_count);
    tri_to_face: u32 = (tri_to_face, tri_to_face_count);
    tri_material: u32 = (tri_material, tri_material_count);
    tri_node: u32 = (tri_node, tri_node_count);
    nodes: Bridged<ReviewImportNode> = (nodes, node_count);
    materials: Bridged<ReviewImportMaterial> = (materials, material_count);
    uvs: f32 = (uvs, uv_value_count);
    uv_set_names: *mut c_char = (uv_set_names, uv_set_name_count);
    corner_source_vertex: u32 = (corner_source_vertex, corner_source_vertex_count);
    skin_offsets: u32 = (skin_offsets, skin_offset_count);
    // The three influence streams are parallel, so they share one count.
    skin_bones: u32 = (skin_bones, skin_influence_count);
    skin_weights: f32 = (skin_weights, skin_influence_count);
    skin_influence_cluster: u32 = (skin_influence_cluster, skin_influence_count);
    skin_clusters: Bridged<ReviewImportSkinCluster> = (skin_clusters, skin_cluster_count);
    skin_deformers: ReviewImportSkinDeformer = (skin_deformers, skin_deformer_count);
    morph_channels: Bridged<ReviewImportMorphChannel> = (morph_channels, morph_channel_count);
    morph_keyframes: ReviewImportMorphKeyframe = (morph_keyframes, morph_keyframe_count);
    morph_shapes: Bridged<ReviewImportMorphShape> = (morph_shapes, morph_shape_count);
    morph_entries: ReviewImportMorphEntry = (morph_entries, morph_entry_count);
    anim_stacks: Bridged<ReviewImportAnimStack> = (anim_stacks, anim_stack_count);
    anim_node_tracks: ReviewImportNodeTrack = (anim_node_tracks, anim_node_track_count);
    anim_vec3_keys: ReviewImportVec3Key = (anim_vec3_keys, anim_vec3_key_count);
    anim_quat_keys: ReviewImportQuatKey = (anim_quat_keys, anim_quat_key_count);
    anim_morph_tracks: ReviewImportMorphTrack = (anim_morph_tracks, anim_morph_track_count);
    anim_scalar_keys: ReviewImportScalarKey = (anim_scalar_keys, anim_scalar_key_count);
});

name_accessors! {
    ReviewImportNode => name;
    ReviewImportMaterial => name;
    ReviewImportSkinCluster => name;
    ReviewImportMorphChannel => name;
    ReviewImportMorphShape => name;
    ReviewImportAnimStack => name;
}

impl Bridged<ReviewImportScene> {
    /// One entry of the `uv_set_names` pointer table.
    ///
    /// The table itself is an array of `char*`, so its elements are pointers
    /// rather than values — the one array whose accessor cannot hand back the
    /// strings directly.
    pub(super) fn uv_set_name(&self, index: usize) -> Option<String> {
        let names = self.uv_set_names().ok()?;
        // SAFETY: `Bridged::new`'s contract covers the table's entries: each is
        // null or a string the bridge owns for the life of the scene, and
        // `&self` borrows the scene.
        unsafe { read_optional_c_string(*names.get(index)?) }
    }
}

slice_accessors!(ReviewImportExtras {
    bytes: u8 = (bytes, byte_count);
    props: XProp = (props, prop_count);
    nodes: XNode = (nodes, node_count);
    lights: XLight = (lights, light_count);
    cameras: XCamera = (cameras, camera_count);
    lod_groups: XLodGroup = (lod_groups, lod_group_count);
    lod_levels: XLodLevel = (lod_levels, lod_level_count);
    materials: XMaterial = (materials, material_count);
    material_textures: XMaterialTexture = (material_textures, material_texture_count);
    textures: XTexture = (textures, texture_count);
    texture_layers: XTextureLayer = (texture_layers, texture_layer_count);
    videos: XVideo = (videos, video_count);
    meshes: XMesh = (meshes, mesh_count);
    color_sets: XColorSet = (color_sets, color_set_count);
    color_values: f64 = (color_values, color_value_count);
    // Two vertex indices per edge; the three per-edge layers below are parallel
    // to the edge list, so they take the count itself.
    edges: u32 = (edges, edge_count * 2);
    edge_smoothing: u8 = (edge_smoothing, edge_count);
    edge_crease: f64 = (edge_crease, edge_count);
    edge_visibility: u8 = (edge_visibility, edge_count);
    face_smoothing: u8 = (face_smoothing, face_count);
    face_hole: u8 = (face_hole, face_count);
    face_group: u32 = (face_group, face_count);
    vertex_crease: f64 = (vertex_crease, vertex_crease_count);
    face_groups: XFaceGroup = (face_groups, face_group_count);
    extra_skins: XExtraSkin = (extra_skins, extra_skin_count);
    extra_clusters: XExtraCluster = (extra_clusters, extra_cluster_count);
    extra_skin_offsets: u32 = (extra_skin_offsets, extra_skin_offset_count);
    extra_influences: XExtraInfluence = (extra_influences, extra_influence_count);
    dq_weights: XDqWeight = (dq_weights, dq_weight_count);
    uv_sets: XUvSet = (uv_sets, uv_set_count);
    unused_vertices: XUnusedVertex = (unused_vertices, unused_vertex_count);
    poses: XPose = (poses, pose_count);
    pose_entries: XPoseEntry = (pose_entries, pose_entry_count);
    display_layers: XDisplayLayer = (display_layers, display_layer_count);
    layer_nodes: u32 = (layer_nodes, layer_node_count);
    selection_sets: XSelectionSet = (selection_sets, selection_set_count);
    selection_nodes: XSelectionNode = (selection_nodes, selection_node_count);
    selection_indices: u32 = (selection_indices, selection_index_count);
    anim_stacks: XAnimStack = (anim_stacks, anim_stack_count);
    stack_layers: u32 = (stack_layers, stack_layer_count);
    anim_layers: XAnimLayer = (anim_layers, anim_layer_count);
    anim_props: XAnimProp = (anim_props, anim_prop_count);
    anim_curves: XAnimCurve = (anim_curves, anim_curve_count);
    anim_keys: XAnimKey = (anim_keys, anim_key_count);
});

impl Bridged<ReviewImportExtras> {
    /// The capture's string pool as bytes.
    ///
    /// The pool is one `char*` block that every captured name indexes into by
    /// offset and length, rather than an array of pointers — so the accessor
    /// reads it as bytes, which is the only one of these that has to change the
    /// pointer's type.
    pub(super) fn strings(&self) -> Result<&[u8], ImportError> {
        // SAFETY: `Bridged::new`'s contract: `string_count` bytes the bridge
        // owns until the capture is freed; `u8` has looser alignment than
        // `c_char` and identical size, and the slice borrows `self`.
        unsafe { checked_slice(self.0.strings.cast::<u8>(), self.0.string_count, "strings") }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;

    use super::*;

    #[test]
    fn checked_slice_returns_empty_for_zero_len() {
        // Len 0 is the empty case regardless of the pointer (even null).
        // SAFETY: len 0 imposes no requirement on the pointer.
        let empty = unsafe { checked_slice::<u32>(std::ptr::null(), 0, "verts") };
        assert!(empty.unwrap().is_empty());
    }

    #[test]
    fn checked_slice_reads_a_valid_pointer() {
        let data = [1u32, 2, 3, 4];
        // SAFETY: `data` is a live array of exactly `len` initialized, aligned
        // `u32`, outliving the borrow.
        let slice = unsafe { checked_slice(data.as_ptr(), data.len(), "verts") }.unwrap();
        assert_eq!(slice, &data[..]);
    }

    #[test]
    fn checked_slice_rejects_null_for_nonempty() {
        // SAFETY: a null pointer is rejected before it is read.
        let err = unsafe { checked_slice::<u32>(std::ptr::null(), 3, "indices") }.unwrap_err();
        assert!(
            matches!(err, ImportError::LoadFailed(message) if message.contains("indices")),
            "a null pointer for non-empty data must be a LoadFailed naming the field"
        );
    }

    /// The accessor path, which is what the marshal modules actually call: a
    /// zeroed scene is what a partial parse leaves, and a non-zero count beside a
    /// null pointer has to be an error rather than a dereference.
    #[test]
    fn an_accessor_on_a_null_array_names_the_field() {
        // SAFETY: `ReviewImportScene` is all raw pointers and integers, for which
        // the all-zero bit pattern is valid — the same value `load_fbx` hands the
        // bridge — and an all-zero scene points at nothing, so vouching for it
        // promises nothing.
        let empty = unsafe { Bridged::new(std::mem::zeroed::<ReviewImportScene>()) };
        assert!(
            empty.vertices().unwrap().is_empty(),
            "a zero count is the empty slice, not a null-pointer error"
        );

        // SAFETY: as above.
        let mut scene: ReviewImportScene = unsafe { std::mem::zeroed() };
        scene.vertex_count = 4;
        // SAFETY: the only array pointer is null, which an accessor refuses to
        // read; `Bridged::new` allows that.
        let scene = unsafe { Bridged::new(scene) };
        assert!(
            matches!(
                scene.vertices(),
                Err(ImportError::LoadFailed(message)) if message.contains("vertices")
            ),
            "a count without an array must fail naming the field"
        );
    }

    /// The per-edge layers share `edge_count` and the edge list itself is two
    /// indices per edge — the one place an accessor's length is not the field
    /// beside it.
    #[test]
    fn the_edge_accessors_agree_on_one_count() {
        let edges = [0u32, 1, 1, 2, 2, 0];
        let smoothing = [1u8, 0, 1];
        // SAFETY: as above.
        let mut extras: ReviewImportExtras = unsafe { std::mem::zeroed() };
        extras.edge_count = 3;
        extras.edges = edges.as_ptr().cast_mut();
        extras.edge_smoothing = smoothing.as_ptr().cast_mut();
        // SAFETY: `edges` holds `2 * edge_count` and `smoothing` `edge_count`
        // values, both outliving the wrapper; every other pointer is null.
        let extras = unsafe { Bridged::new(extras) };

        assert_eq!(extras.edges().unwrap().len(), 6, "two indices per edge");
        assert_eq!(extras.edge_smoothing().unwrap().len(), 3, "one per edge");
    }

    #[test]
    fn read_optional_c_string_is_none_for_null() {
        // SAFETY: null is the documented "no string" case.
        assert_eq!(unsafe { read_optional_c_string(std::ptr::null()) }, None);
    }

    #[test]
    fn read_optional_c_string_reads_a_c_string() {
        let text = CString::new("mesh_01").unwrap();
        // SAFETY: `text` is a live NUL-terminated C string outliving the call.
        let read = unsafe { read_optional_c_string(text.as_ptr()) };
        assert_eq!(read, Some("mesh_01".to_owned()));
    }

    #[test]
    fn read_error_message_falls_back_when_empty() {
        let error = ReviewImportError { message: [0; 256] };
        assert_eq!(read_error_message(&error), "unknown FBX import error");
    }

    #[test]
    fn read_error_message_reads_the_bridge_text() {
        let mut error = ReviewImportError { message: [0; 256] };
        for (slot, &byte) in error.message.iter_mut().zip(b"bad fbx") {
            *slot = byte as c_char;
        }
        assert_eq!(read_error_message(&error), "bad fbx");
    }
}
