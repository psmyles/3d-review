use std::path::Path;

use review_model::ModelData;
use thiserror::Error;

mod prof;
mod tracy_alloc;

pub use tracy_alloc::TracyAllocator;

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("FBX import is unavailable until third_party/ufbx contains ufbx.c and ufbx.h")]
    UfbxUnavailable,
    #[error("unsupported file extension: {0}")]
    UnsupportedExtension(String),
    #[error("failed to load model: {0}")]
    LoadFailed(String),
}

/// Whether the process was asked to start maximized — e.g. launched from a
/// shortcut whose **Run** field is set to *Maximized*. Windows passes that hint
/// through `STARTUPINFO.wShowWindow`, but winit creates its window without
/// consulting it, so `app` queries this to set the initial window state.
/// Always `false` off Windows. (Lives here because invariant 9 funnels every
/// `unsafe`/FFI through `crates/import`.)
#[cfg(windows)]
pub fn startup_show_maximized() -> bool {
    use windows_sys::Win32::System::Threading::{
        GetStartupInfoW, STARTF_USESHOWWINDOW, STARTUPINFOW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWMAXIMIZED;

    // SAFETY: `GetStartupInfoW` fills the entire caller-provided `STARTUPINFOW`
    // and cannot fail; we hand it a zeroed, correctly-sized struct and only read
    // the scalar `dwFlags`/`wShowWindow` fields it sets.
    let info = unsafe {
        let mut info: STARTUPINFOW = std::mem::zeroed();
        GetStartupInfoW(&mut info);
        info
    };

    info.dwFlags & STARTF_USESHOWWINDOW != 0 && i32::from(info.wShowWindow) == SW_SHOWMAXIMIZED
}

/// Off Windows there is no `STARTUPINFO` launch hint; never request maximize.
#[cfg(not(windows))]
pub fn startup_show_maximized() -> bool {
    false
}

pub fn load_model(path: impl AsRef<Path>) -> Result<ModelData, ImportError> {
    let path = path.as_ref();
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if extension.eq_ignore_ascii_case("fbx") => load_fbx(path),
        Some(extension) => Err(ImportError::UnsupportedExtension(extension.to_owned())),
        None => Err(ImportError::UnsupportedExtension("<none>".to_owned())),
    }
}

pub fn load_fbx(_path: &Path) -> Result<ModelData, ImportError> {
    #[cfg(has_ufbx)]
    {
        ffi::load_fbx(_path)
    }

    #[cfg(not(has_ufbx))]
    {
        Err(ImportError::UfbxUnavailable)
    }
}

#[cfg(has_ufbx)]
mod ffi {
    use std::{
        ffi::{CStr, CString},
        mem::MaybeUninit,
        os::raw::{c_char, c_int},
        path::Path,
        ptr::NonNull,
        slice,
    };

    use glam::{Mat4, Vec2, Vec3, Vec4};
    use review_model::{
        BoneInfo, MaterialImportDefaults, ModelData, ModelStats, NodeKind, SceneNode, SkinData,
        TopologyFace, TriangleData, Vertex,
    };

    use crate::ImportError;

    #[repr(C)]
    struct ReviewImportVertex {
        position: [f32; 3],
        normal: [f32; 3],
        uv: [f32; 2],
        tangent: [f32; 4],
        vertex_color: [f32; 4],
    }

    #[repr(C)]
    struct ReviewImportFace {
        first_index: u32,
        index_count: u32,
    }

    #[repr(C)]
    struct ReviewImportMaterial {
        name: *mut c_char,
        base_color: [f32; 3],
        smoothness: f32,
        metallic: f32,
        emissive: [f32; 3],
    }

    #[repr(C)]
    struct ReviewImportNode {
        name: *mut c_char,
        parent: i32,
        mesh_part_index: i32,
        /// This node's mesh's own logical (DCC) vertex count, 0 for a node with
        /// no mesh. Summing it over the mesh-bearing nodes reproduces
        /// `source_vertex_count`.
        source_vertex_count: u32,
        transform: [f32; 16],
        /// A `review_import_node_kind` code; see [`node_kind_from_code`].
        kind: u32,
        bone_radius: f32,
        bone_relative_length: f32,
    }

    #[repr(C)]
    struct ReviewImportScene {
        vertices: *mut ReviewImportVertex,
        vertex_count: usize,
        indices: *mut u32,
        index_count: usize,
        faces: *mut ReviewImportFace,
        face_count: usize,
        tri_to_face: *mut u32,
        tri_to_face_count: usize,
        materials: *mut ReviewImportMaterial,
        material_count: usize,
        uv_set_count: u32,
        uvs: *mut f32,
        uv_value_count: usize,
        /// Source DCC logical vertex count (invariant 5's Verts stat);
        /// `vertex_count` above is the per-corner expanded array length.
        source_vertex_count: usize,
        source_unit_meters: f32,
        uv_set_names: *mut *mut c_char,
        uv_set_name_count: usize,
        nodes: *mut ReviewImportNode,
        node_count: usize,
        tri_material: *mut u32,
        tri_material_count: usize,
        tri_node: *mut u32,
        tri_node_count: usize,
        /// Per expanded corner, the logical source vertex it came from — the
        /// index the CSR skin rows below are keyed by.
        corner_source_vertex: *mut u32,
        corner_source_vertex_count: usize,
        /// CSR skin weights over the logical vertices; all null / zero for an
        /// unskinned scene.
        skin_offsets: *mut u32,
        skin_offset_count: usize,
        skin_bones: *mut u32,
        skin_weights: *mut f32,
        skin_influence_count: usize,
    }

    #[repr(C)]
    struct ReviewImportError {
        message: [c_char; 256],
    }

    unsafe extern "C" {
        fn review_import_load_fbx(
            path: *const c_char,
            out_scene: *mut ReviewImportScene,
            out_error: *mut ReviewImportError,
        ) -> c_int;

        fn review_import_free_scene(scene: *mut ReviewImportScene);
    }

    pub(super) fn load_fbx(path: &Path) -> Result<ModelData, ImportError> {
        let _z = crate::prof::zone!("Load FBX");
        let path_string = path.to_string_lossy();
        let c_path = CString::new(path_string.as_bytes())
            .map_err(|_| ImportError::LoadFailed("path contains embedded NUL byte".to_owned()))?;
        let mut scene = MaybeUninit::<ReviewImportScene>::zeroed();
        let mut error = ReviewImportError { message: [0; 256] };

        let loaded = {
            // The ufbx C parse + the bridge's two-pass extraction (the bulk of a
            // load), measured as one GPU-free CPU zone.
            let _z = crate::prof::zone!("ufbx Parse");
            // SAFETY: `c_path` is a valid NUL-terminated C string that outlives the
            // call; `error` is a live stack value; `scene` is a zeroed,
            // correctly-sized `ReviewImportScene` the bridge fully writes on
            // success (return != 0) or frees + re-zeroes itself on failure (its
            // `cleanup:` path calls `review_import_free_scene`, so no C buffers
            // leak and no free is needed here on the error return below). All
            // three pointers are non-null and valid for the duration of the call.
            unsafe { review_import_load_fbx(c_path.as_ptr(), scene.as_mut_ptr(), &mut error) }
        };

        if loaded == 0 {
            return Err(ImportError::LoadFailed(read_error_message(&error)));
        }

        // SAFETY: `loaded != 0` means the bridge fully initialized `scene`, so the
        // `MaybeUninit` now holds a valid `ReviewImportScene`.
        let mut scene = unsafe { scene.assume_init() };
        let model = {
            // Walk the flat bridge arrays into our `ModelData` (slices, bounds, BVH).
            let _z = crate::prof::zone!("Build ModelData");
            model_from_bridge_scene(path, &scene)
        };
        // SAFETY: `scene` is the bridge-allocated scene we own; this frees its C-side
        // buffers exactly once, on both the success and error paths of the extraction
        // above (we still return `model` afterwards). The bridge tolerates the zeroed
        // fields a partial parse may leave. No further access to `scene` follows.
        unsafe {
            review_import_free_scene(&mut scene);
        }
        model
    }

    /// The bridge's flat geometry arrays, marshaled into owned Rust buffers.
    struct MarshaledGeometry {
        vertices: Vec<Vertex>,
        indices: Vec<u32>,
        faces: Vec<TopologyFace>,
        triangles: TriangleData,
    }

    /// Marshal the bridge's flat geometry arrays (vertices / indices / original
    /// faces / per-triangle metadata) into owned Rust buffers.
    fn marshal_geometry(scene: &ReviewImportScene) -> Result<MarshaledGeometry, ImportError> {
        let vertices = checked_slice(scene.vertices, scene.vertex_count, "vertices")?
            .iter()
            .map(|vertex| Vertex {
                position: Vec3::from_array(vertex.position),
                normal: Vec3::from_array(vertex.normal),
                uv: Vec2::from_array(vertex.uv),
                tangent: Vec4::from_array(vertex.tangent),
                vertex_color: Vec4::from_array(vertex.vertex_color),
            })
            .collect::<Vec<_>>();
        let indices = checked_slice(scene.indices, scene.index_count, "indices")?.to_vec();
        let faces = checked_slice(scene.faces, scene.face_count, "faces")?
            .iter()
            .map(|face| TopologyFace {
                first_index: face.first_index,
                index_count: face.index_count,
            })
            .collect::<Vec<_>>();
        let triangles = TriangleData {
            to_face: checked_slice(scene.tri_to_face, scene.tri_to_face_count, "tri_to_face")?
                .to_vec(),
            material: checked_slice(scene.tri_material, scene.tri_material_count, "tri_material")?
                .to_vec(),
            node: checked_slice(scene.tri_node, scene.tri_node_count, "tri_node")?.to_vec(),
        };
        Ok(MarshaledGeometry {
            vertices,
            indices,
            faces,
            triangles,
        })
    }

    /// The bridge's `review_import_node_kind` codes. An unrecognized code is
    /// [`NodeKind::Other`] rather than an error, so the C side can grow a new
    /// attribute type without breaking an older Rust build.
    fn node_kind_from_code(code: u32) -> NodeKind {
        match code {
            1 => NodeKind::Mesh,
            2 => NodeKind::Bone,
            3 => NodeKind::Light,
            4 => NodeKind::Camera,
            5 => NodeKind::Empty,
            _ => NodeKind::Other,
        }
    }

    /// Marshal the bridge's scene-graph node table for the Outliner.
    fn marshal_nodes(scene: &ReviewImportScene) -> Result<Vec<SceneNode>, ImportError> {
        Ok(checked_slice(scene.nodes, scene.node_count, "nodes")?
            .iter()
            .map(|node| {
                let kind = node_kind_from_code(node.kind);
                SceneNode {
                    name: read_optional_c_string(node.name).unwrap_or_default(),
                    parent: (node.parent >= 0).then_some(node.parent as usize),
                    mesh_part: (node.mesh_part_index >= 0).then_some(node.mesh_part_index as usize),
                    source_vertex_count: node.source_vertex_count as usize,
                    transform: Mat4::from_cols_array(&node.transform),
                    kind,
                    bone: (kind == NodeKind::Bone).then_some(BoneInfo {
                        radius: node.bone_radius,
                        relative_length: node.bone_relative_length,
                    }),
                }
            })
            .collect())
    }

    /// Marshal the bridge's CSR skin table. Returns `None` for an unskinned
    /// scene (the bridge allocates nothing and reports zero influences), so the
    /// common non-skeletal model carries no skin payload at all.
    fn marshal_skin(scene: &ReviewImportScene) -> Result<Option<SkinData>, ImportError> {
        if scene.skin_influence_count == 0 {
            return Ok(None);
        }
        Ok(Some(SkinData {
            corner_to_logical: checked_slice(
                scene.corner_source_vertex,
                scene.corner_source_vertex_count,
                "corner_source_vertex",
            )?
            .to_vec(),
            offsets: checked_slice(scene.skin_offsets, scene.skin_offset_count, "skin_offsets")?
                .to_vec(),
            bones: checked_slice(scene.skin_bones, scene.skin_influence_count, "skin_bones")?
                .to_vec(),
            weights: checked_slice(
                scene.skin_weights,
                scene.skin_influence_count,
                "skin_weights",
            )?
            .to_vec(),
        }))
    }

    /// Marshal the bridge's material table into the editable table's import
    /// defaults.
    fn marshal_materials(
        scene: &ReviewImportScene,
    ) -> Result<Vec<MaterialImportDefaults>, ImportError> {
        Ok(
            checked_slice(scene.materials, scene.material_count, "materials")?
                .iter()
                .map(|material| MaterialImportDefaults {
                    name: read_optional_c_string(material.name)
                        .unwrap_or_else(|| "Default".to_owned()),
                    base_color: Vec3::from_array(material.base_color),
                    smoothness: material.smoothness,
                    metallic: material.metallic,
                    emissive: Vec3::from_array(material.emissive),
                })
                .collect(),
        )
    }

    fn model_from_bridge_scene(
        path: &Path,
        scene: &ReviewImportScene,
    ) -> Result<ModelData, ImportError> {
        let MarshaledGeometry {
            vertices,
            indices,
            faces,
            triangles,
        } = marshal_geometry(scene)?;
        let nodes = marshal_nodes(scene)?;
        let skin = marshal_skin(scene)?;
        let bone_count = nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Bone)
            .count();
        let uv_channels = build_uv_channels(scene)?;
        let uv_set_names =
            checked_slice(scene.uv_set_names, scene.uv_set_name_count, "uv_set_names")?
                .iter()
                .map(|&name| read_optional_c_string(name).unwrap_or_default())
                .collect::<Vec<_>>();
        let materials = marshal_materials(scene)?;

        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("Imported Model")
            .to_owned();

        let mut model = ModelData {
            name,
            vertices,
            indices,
            faces,
            triangles,
            nodes,
            uv_channels,
            uv_set_names,
            bounds: None,
            stats: ModelStats {
                polygon_count: scene.face_count,
                triangle_count: scene.index_count / 3,
                // The source DCC's logical vertex count, not the per-corner
                // expanded render count (invariant 5: faithful stats).
                vertex_count: scene.source_vertex_count,
                // Measured below once the whole model is assembled, like Draws.
                gpu_vertex_count: 0,
                uv_set_count: scene.uv_set_count as usize,
                material_count: scene.material_count,
                // Overwritten below from `material_draw_count` so the Draws stat
                // matches the renderer's per-material grouping (invariant 5).
                draw_count: 0,
                bone_count,
                source_unit_meters: scene.source_unit_meters,
            },
            materials,
            skin,
        };

        // Every index must address a real vertex. The bridge derives each one by
        // subtracting a face's first index from a triangulation result, an
        // unsigned subtraction that would wrap into a huge index rather than
        // fail if either ever drifted — so it is checked here, at the funnel,
        // before the renderer or the optimizer reads the buffer.
        if let Some(&index) = model
            .indices
            .iter()
            .find(|&&index| index as usize >= model.vertices.len())
        {
            return Err(ImportError::LoadFailed(format!(
                "FBX bridge returned index {index} for a mesh of {} vertices",
                model.vertices.len()
            )));
        }

        // Lockstep + range guard at the one import funnel (invariant 7), *before*
        // anything consumes the per-triangle arrays: each array must be empty or
        // exactly `triangle_count` long, and every entry must index a real
        // face / material / node. Catches a bridge marshaling drift here, once,
        // rather than in every renderer-side reader.
        model
            .triangles
            .validate(
                model.stats.triangle_count,
                model.faces.len(),
                model.materials.len(),
                model.nodes.len(),
            )
            .map_err(ImportError::LoadFailed)?;

        // Same funnel guard for the skin CSR: the corner map must cover exactly
        // the render mesh, the rows must be monotonic and terminate at the
        // influence count, and every influence must name a real node with a
        // finite non-negative weight. A drifted bridge fill is caught here, once.
        if let Some(skin) = &model.skin {
            skin.validate(
                model.vertices.len(),
                model.stats.vertex_count,
                model.nodes.len(),
            )
            .map_err(ImportError::LoadFailed)?;
        }

        model.recompute_bounds();
        // The FBX may carry UVs but no tangent layer (common for Maya exports); the
        // bridge then leaves a zero tangent per vertex. Synthesize a real tangent
        // basis from the UVs + normals so normal maps shade correctly — done once
        // here at the single import funnel (invariant 7).
        if model.has_degenerate_tangents() {
            model.generate_tangents();
        }
        // The renderer groups triangles into one draw per distinct material slot;
        // report that count so Draws is the real draw-call count.
        model.stats.draw_count = model.material_draw_count();
        // The asset's real GPU vertex cost (unique vertices per draw group) —
        // the corner-expanded buffer built above is this viewer's internal
        // layout and is deliberately not a reported stat.
        model.stats.gpu_vertex_count = model.count_gpu_vertices();

        Ok(model)
    }

    /// Builds the per-channel UV table for multi-set models. Single-set models
    /// (and meshes the bridge left without a `uvs` allocation) return an empty
    /// vec, in which case the renderer falls back to [`Vertex::uv`].
    fn build_uv_channels(scene: &ReviewImportScene) -> Result<Vec<Vec<Vec2>>, ImportError> {
        let channel_count = scene.uv_set_count as usize;
        if channel_count <= 1 || scene.uvs.is_null() {
            return Ok(Vec::new());
        }

        // Checked: both counts are C-provided, so a wrapped multiply here would
        // defeat the length guard below and turn the indexing loop into a panic.
        let vertex_count = scene.vertex_count;
        let expected = channel_count
            .checked_mul(vertex_count)
            .and_then(|count| count.checked_mul(2))
            .ok_or_else(|| {
                ImportError::LoadFailed(format!(
                    "FBX bridge UV table overflows: {channel_count} channels x {vertex_count} vertices"
                ))
            })?;
        if scene.uv_value_count < expected {
            return Err(ImportError::LoadFailed(format!(
                "FBX bridge returned {} UV values, expected {expected}",
                scene.uv_value_count
            )));
        }

        let values = checked_slice(scene.uvs, scene.uv_value_count, "uvs")?;
        let channels = (0..channel_count)
            .map(|channel| {
                (0..vertex_count)
                    .map(|vertex| {
                        // In bounds: `base + 1 <= expected - 1 < values.len()`,
                        // with `expected` overflow-checked above.
                        let base = (channel * vertex_count + vertex) * 2;
                        Vec2::new(values[base], values[base + 1])
                    })
                    .collect()
            })
            .collect();

        Ok(channels)
    }

    fn read_error_message(error: &ReviewImportError) -> String {
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

    fn read_optional_c_string(value: *const c_char) -> Option<String> {
        if value.is_null() {
            return None;
        }

        // SAFETY: `value` is non-null (checked above) and points at a bridge-owned,
        // NUL-terminated C string that lives until `review_import_free_scene`.
        let text = unsafe { CStr::from_ptr(value) };
        // Lossy: an FBX name in some other encoding still has to reach the
        // Outliner as *something* — dropping it would leave a blank row with no
        // way to tell it from an unnamed node.
        Some(String::from_utf8_lossy(text.to_bytes()).into_owned())
    }

    fn checked_slice<'a, T>(
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

        // SAFETY: `ptr` is non-null (just checked) and the bridge guarantees it
        // points at `len` contiguous, properly aligned `T` values that stay valid
        // for the borrow `'a` (the caller holds `&ReviewImportScene` for the whole
        // walk). `len > 0` here, so the slice is non-empty and within one allocation.
        Ok(unsafe { slice::from_raw_parts(ptr.as_ptr(), len) })
    }

    #[cfg(test)]
    mod ffi_tests {
        use super::*;

        #[test]
        fn checked_slice_returns_empty_for_zero_len() {
            // Len 0 is the empty case regardless of the pointer (even null).
            assert!(
                checked_slice::<u32>(std::ptr::null(), 0, "verts")
                    .unwrap()
                    .is_empty()
            );
        }

        #[test]
        fn checked_slice_reads_a_valid_pointer() {
            let data = [1u32, 2, 3, 4];
            let slice = checked_slice(data.as_ptr(), data.len(), "verts").unwrap();
            assert_eq!(slice, &data[..]);
        }

        #[test]
        fn checked_slice_rejects_null_for_nonempty() {
            let err = checked_slice::<u32>(std::ptr::null(), 3, "indices").unwrap_err();
            assert!(
                matches!(err, ImportError::LoadFailed(message) if message.contains("indices")),
                "a null pointer for non-empty data must be a LoadFailed naming the field"
            );
        }

        #[test]
        fn read_optional_c_string_is_none_for_null() {
            assert_eq!(read_optional_c_string(std::ptr::null()), None);
        }

        #[test]
        fn read_optional_c_string_reads_a_c_string() {
            let text = CString::new("mesh_01").unwrap();
            assert_eq!(
                read_optional_c_string(text.as_ptr()),
                Some("mesh_01".to_owned())
            );
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

        /// A zeroed bridge scene (every pointer null, every count 0) with just the
        /// UV fields set — the shape `build_uv_channels` consumes.
        // SAFETY (of the test helper): `ReviewImportScene` is a plain `#[repr(C)]`
        // struct of pointers + integers, for which all-zero bytes is a valid value.
        fn uv_scene(uv_set_count: u32, vertex_count: usize, uvs: &[f32]) -> ReviewImportScene {
            let mut scene: ReviewImportScene = unsafe { std::mem::zeroed() };
            scene.uv_set_count = uv_set_count;
            scene.vertex_count = vertex_count;
            scene.uvs = uvs.as_ptr() as *mut f32;
            scene.uv_value_count = uvs.len();
            scene
        }

        #[test]
        fn build_uv_channels_reads_channel_major_values() {
            // 2 channels × 2 vertices × (u, v): channel-major layout.
            let uvs = [0.0, 0.1, 0.2, 0.3, 1.0, 1.1, 1.2, 1.3];
            let scene = uv_scene(2, 2, &uvs);
            let channels = build_uv_channels(&scene).expect("valid UV table");
            assert_eq!(channels.len(), 2);
            assert_eq!(channels[0], vec![Vec2::new(0.0, 0.1), Vec2::new(0.2, 0.3)]);
            assert_eq!(channels[1], vec![Vec2::new(1.0, 1.1), Vec2::new(1.2, 1.3)]);
        }

        #[test]
        fn build_uv_channels_rejects_a_short_buffer() {
            // 2 channels × 2 vertices needs 8 values; give 6.
            let uvs = [0.0; 6];
            let scene = uv_scene(2, 2, &uvs);
            assert!(matches!(
                build_uv_channels(&scene),
                Err(ImportError::LoadFailed(_))
            ));
        }

        /// A zeroed scene whose CSR skin arrays point at Rust-owned data — the
        /// same trick `uv_scene` uses, so `marshal_skin` can be exercised without
        /// an FBX file.
        fn skin_scene(
            corner_to_logical: &[u32],
            offsets: &[u32],
            bones: &[u32],
            weights: &[f32],
        ) -> ReviewImportScene {
            let mut scene: ReviewImportScene = unsafe { std::mem::zeroed() };
            scene.vertex_count = corner_to_logical.len();
            scene.corner_source_vertex = corner_to_logical.as_ptr() as *mut u32;
            scene.corner_source_vertex_count = corner_to_logical.len();
            scene.skin_offsets = offsets.as_ptr() as *mut u32;
            scene.skin_offset_count = offsets.len();
            scene.skin_bones = bones.as_ptr() as *mut u32;
            scene.skin_weights = weights.as_ptr() as *mut f32;
            scene.skin_influence_count = bones.len();
            scene
        }

        #[test]
        fn marshal_skin_is_none_for_an_unskinned_scene() {
            let scene: ReviewImportScene = unsafe { std::mem::zeroed() };
            assert!(
                marshal_skin(&scene)
                    .expect("no skin is not an error")
                    .is_none()
            );
        }

        #[test]
        fn marshal_skin_copies_the_csr_table() {
            let skin = marshal_skin(&skin_scene(
                &[0, 0, 1],
                &[0, 2, 3],
                &[1, 2, 2],
                &[0.75, 0.25, 1.0],
            ))
            .expect("valid skin")
            .expect("a skinned scene marshals to Some");
            assert_eq!(skin.corner_to_logical, vec![0, 0, 1]);
            assert_eq!(skin.offsets, vec![0, 2, 3]);
            assert_eq!(skin.bones, vec![1, 2, 2]);
            assert_eq!(skin.weights, vec![0.75, 0.25, 1.0]);
            assert_eq!(skin.influence_range(0), 0..2);
        }

        #[test]
        fn marshal_skin_rejects_a_null_array_with_a_nonzero_count() {
            // A count without its pointer must be a clean error naming the field,
            // not a `from_raw_parts` on null.
            let mut scene = skin_scene(&[0], &[0, 1], &[1], &[1.0]);
            scene.skin_bones = std::ptr::null_mut();
            assert!(matches!(
                marshal_skin(&scene),
                Err(ImportError::LoadFailed(message)) if message.contains("skin_bones")
            ));
        }

        #[test]
        fn node_kind_maps_every_bridge_code_and_falls_back() {
            assert_eq!(node_kind_from_code(1), NodeKind::Mesh);
            assert_eq!(node_kind_from_code(2), NodeKind::Bone);
            assert_eq!(node_kind_from_code(3), NodeKind::Light);
            assert_eq!(node_kind_from_code(4), NodeKind::Camera);
            assert_eq!(node_kind_from_code(5), NodeKind::Empty);
            assert_eq!(node_kind_from_code(0), NodeKind::Other);
            // An unknown code from a newer bridge must degrade, not panic.
            assert_eq!(node_kind_from_code(999), NodeKind::Other);
        }

        /// A zeroed scene carrying only geometry — the shape
        /// `model_from_bridge_scene`'s range guard reads.
        fn geometry_scene(vertices: &[ReviewImportVertex], indices: &[u32]) -> ReviewImportScene {
            let mut scene: ReviewImportScene = unsafe { std::mem::zeroed() };
            scene.vertices = vertices.as_ptr() as *mut ReviewImportVertex;
            scene.vertex_count = vertices.len();
            scene.indices = indices.as_ptr() as *mut u32;
            scene.index_count = indices.len();
            scene.source_vertex_count = vertices.len();
            scene.source_unit_meters = 1.0;
            scene
        }

        fn blank_vertices(count: usize) -> Vec<ReviewImportVertex> {
            (0..count)
                .map(|_| ReviewImportVertex {
                    position: [0.0; 3],
                    normal: [0.0; 3],
                    uv: [0.0; 2],
                    tangent: [0.0; 4],
                    vertex_color: [0.0; 4],
                })
                .collect()
        }

        #[test]
        fn an_index_addressing_no_vertex_fails_the_load() {
            let vertices = blank_vertices(3);
            let scene = geometry_scene(&vertices, &[0, 1, 3]);
            assert!(matches!(
                model_from_bridge_scene(Path::new("mesh.fbx"), &scene),
                Err(ImportError::LoadFailed(message))
                    if message.contains("index 3") && message.contains("3 vertices")
            ));
        }

        #[test]
        fn indices_within_the_vertex_buffer_load() {
            let vertices = blank_vertices(3);
            let scene = geometry_scene(&vertices, &[0, 1, 2]);
            let model =
                model_from_bridge_scene(Path::new("mesh.fbx"), &scene).expect("a whole triangle");
            assert_eq!(model.indices, vec![0, 1, 2]);
            assert_eq!(model.name, "mesh");
        }

        #[test]
        fn build_uv_channels_rejects_overflowing_counts() {
            // Malicious/corrupt counts whose product wraps `usize` must be a clean
            // error, not a wrapped guard followed by an index panic.
            let uvs = [0.0; 4];
            let scene = uv_scene(2, usize::MAX / 2 + 1, &uvs);
            assert!(matches!(
                build_uv_channels(&scene),
                Err(ImportError::LoadFailed(_))
            ));
        }
    }
}

#[cfg(all(test, has_ufbx))]
mod tests {
    use std::path::PathBuf;

    use super::{ImportError, load_model};

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/test_models")
            .join(name)
    }

    /// Write `bytes` to a temp file with an `.fbx` extension and return its path.
    ///
    /// The directory is cargo's own, not the system temp dir: a fixed name under
    /// `%TEMP%` is shared with every other checkout, so two concurrent runs
    /// delete each other's fixture mid-test. `CARGO_TARGET_TMPDIR` covers only
    /// integration tests; a unit test gets the build script's `OUT_DIR`, which is
    /// just as private to this target directory.
    fn temp_fbx(name: &str, bytes: &[u8]) -> PathBuf {
        let directory = option_env!("CARGO_TARGET_TMPDIR").unwrap_or(env!("OUT_DIR"));
        let path = PathBuf::from(directory).join(format!("review-import-test-{name}.fbx"));
        std::fs::write(&path, bytes).expect("write temp fixture");
        path
    }

    /// Malformed input must come back as a clean [`ImportError::LoadFailed`] —
    /// never a crash — through the whole FFI funnel.
    #[test]
    fn malformed_fbx_is_a_clean_error() {
        let garbage = temp_fbx(
            "garbage",
            b"this is definitely not an FBX file \xff\xfe\x00",
        );
        let result = load_model(&garbage);
        let _ = std::fs::remove_file(&garbage);
        assert!(matches!(result, Err(ImportError::LoadFailed(_))));
    }

    /// An empty file is the degenerate malformed case.
    #[test]
    fn empty_fbx_is_a_clean_error() {
        let empty = temp_fbx("empty", b"");
        let result = load_model(&empty);
        let _ = std::fs::remove_file(&empty);
        assert!(matches!(result, Err(ImportError::LoadFailed(_))));
    }

    /// A truncated-but-real header: the FBX binary magic followed by nothing.
    /// ufbx must reject it without the bridge publishing partial geometry.
    #[test]
    fn truncated_fbx_is_a_clean_error() {
        let truncated = temp_fbx("truncated", b"Kaydara FBX Binary  \x00\x1a\x00");
        let result = load_model(&truncated);
        let _ = std::fs::remove_file(&truncated);
        assert!(matches!(result, Err(ImportError::LoadFailed(_))));
    }

    /// Phase 0 plumbing: a loaded FBX must carry the scene-graph hierarchy and a
    /// per-triangle material slot parallel to the triangle list.
    #[test]
    fn import_carries_nodes_and_per_triangle_material() {
        let model = load_model(fixture("meter_cube.fbx")).expect("meter_cube.fbx should import");

        assert!(
            !model.nodes.is_empty(),
            "imported scene-graph hierarchy must be non-empty"
        );
        assert_eq!(
            model.triangles.material.len(),
            model.stats.triangle_count,
            "tri_material must hold exactly one entry per triangle"
        );
        assert_eq!(
            model.triangles.material.len(),
            model.triangles.to_face.len(),
            "tri_material must run parallel to tri_to_face"
        );

        // Every recorded slot is either a valid material index or the
        // no-material sentinel.
        for &slot in &model.triangles.material {
            assert!(
                slot == u32::MAX || (slot as usize) < model.materials.len(),
                "tri_material slot {slot} out of range"
            );
        }

        // Per-triangle node index runs parallel to the triangle list and points
        // at a real scene-graph node (Phase 2: drives per-node selection / solo).
        assert_eq!(
            model.triangles.node.len(),
            model.stats.triangle_count,
            "tri_node must hold exactly one entry per triangle"
        );
        for &node in &model.triangles.node {
            assert!(
                (node as usize) < model.nodes.len(),
                "tri_node index {node} out of range"
            );
        }

        // Round-trip marshaling: the loaded model is internally consistent.
        let triangle_count = model.stats.triangle_count;
        assert!(
            model
                .triangles
                .validate(
                    triangle_count,
                    model.faces.len(),
                    model.materials.len(),
                    model.nodes.len(),
                )
                .is_ok(),
            "per-triangle arrays must stay in lockstep"
        );
        assert_eq!(
            model.indices.len(),
            triangle_count * 3,
            "index count must be three per triangle"
        );
        assert_eq!(
            model.triangles.to_face.len(),
            triangle_count,
            "tri_to_face must hold one entry per triangle"
        );
        for &face in &model.triangles.to_face {
            assert!(
                (face as usize) < model.faces.len(),
                "tri_to_face index {face} out of range"
            );
        }
        assert!(
            model.bounds.is_some(),
            "a non-empty imported mesh must compute bounds"
        );
        // Multi-set UV tables, when present, carry one full per-vertex channel each.
        if !model.uv_channels.is_empty() {
            assert_eq!(
                model.uv_channels.len(),
                model.stats.uv_set_count,
                "uv_channels must hold one entry per UV set"
            );
            for channel in &model.uv_channels {
                assert_eq!(
                    channel.len(),
                    model.vertices.len(),
                    "each UV channel must cover every vertex"
                );
            }
        }
    }

    /// The skeletal fixture must come through with a classified node table and a
    /// consistent skin CSR. This is the end-to-end guard on the corner -> logical
    /// mapping: if it drifted, the per-vertex influence sums below would be wrong
    /// (or `SkinData::validate` at the funnel would already have failed the load).
    #[test]
    fn import_carries_skeleton_and_skin() {
        let path = fixture("SK_Player_01.fbx");
        if !path.exists() {
            eprintln!("skipping: {} is not present", path.display());
            return;
        }
        let model = load_model(&path).expect("the skeletal fixture must import");

        // ── Bones ───────────────────────────────────────────────────────────
        let bones: Vec<usize> = model
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.kind == review_model::NodeKind::Bone)
            .map(|(index, _)| index)
            .collect();
        assert!(
            !bones.is_empty(),
            "a skeletal mesh must import at least one bone node"
        );
        assert_eq!(
            model.stats.bone_count,
            bones.len(),
            "the Bones stat must be the measured bone-node count"
        );
        for &bone in &bones {
            assert!(
                model.nodes[bone].bone.is_some(),
                "a bone node must carry its display params"
            );
        }
        {
            // The overlay sizes its leaf/root joint markers from these, so it
            // matters whether the file actually declared them.
            let params: Vec<(f32, f32)> = bones
                .iter()
                .filter_map(|&bone| model.nodes[bone].bone)
                .map(|bone| (bone.radius, bone.relative_length))
                .collect();
            let with_radius = params.iter().filter(|(radius, _)| *radius > 0.0).count();
            let with_length = params.iter().filter(|(_, length)| *length > 0.0).count();
            println!(
                "bone display params: {with_radius}/{} carry a radius,                  {with_length}/{} a relative length; first = {:?}",
                params.len(),
                params.len(),
                params.first()
            );
        }
        assert!(
            model
                .nodes
                .iter()
                .any(|node| node.kind == review_model::NodeKind::Mesh),
            "the skeletal fixture also carries geometry"
        );

        // ── Skin ────────────────────────────────────────────────────────────
        let skin = model
            .skin
            .as_ref()
            .expect("a skinned mesh must import skin data");
        // The funnel already ran `validate`; re-assert the shape here so a failure
        // reports as this test rather than as a generic load error.
        assert_eq!(
            skin.validate(
                model.vertices.len(),
                model.stats.vertex_count,
                model.nodes.len()
            ),
            Ok(())
        );
        assert_eq!(skin.corner_to_logical.len(), model.vertices.len());
        assert_eq!(skin.offsets.len(), model.stats.vertex_count + 1);

        // Every influence must name a node the Outliner can actually show.
        for &bone in &skin.bones {
            assert!(
                (bone as usize) < model.nodes.len(),
                "skin influence references node {bone} of {}",
                model.nodes.len()
            );
        }

        // Per-vertex weight sums: a sane rig normalizes to ~1.0. This is the
        // assertion that would catch a broken corner -> logical mapping, since a
        // mis-mapped vertex reads another vertex's (or no) influences.
        let mut skinned_vertices = 0usize;
        let mut sum_of_sums = 0.0_f64;
        let mut worst = 0.0_f32;
        for logical in 0..skin.logical_vertex_count() {
            let range = skin.influence_range(logical);
            if range.is_empty() {
                continue;
            }
            let total: f32 = skin.weights[range].iter().sum();
            skinned_vertices += 1;
            sum_of_sums += f64::from(total);
            worst = worst.max((total - 1.0).abs());
        }
        assert!(skinned_vertices > 0, "no vertex carried any influence");

        let mean_influences = skin.influence_count() as f64 / skinned_vertices as f64;
        let mean_sum = sum_of_sums / skinned_vertices as f64;
        println!(
            "SK_Player_01: {} bones, {} nodes, {} logical verts ({} skinned),              {} influences, {mean_influences:.2} influences/vertex,              mean weight sum {mean_sum:.4} (worst deviation {worst:.4})",
            model.stats.bone_count,
            model.nodes.len(),
            skin.logical_vertex_count(),
            skinned_vertices,
            skin.influence_count(),
        );

        assert!(
            (0.99..=1.01).contains(&mean_sum),
            "mean per-vertex weight sum {mean_sum} is not ~1.0 — the corner -> logical              mapping or the cluster walk is likely wrong"
        );
        assert!(
            mean_influences > 1.0,
            "a real rig blends more than one bone per vertex on average, got {mean_influences}"
        );

        // Per-bone influence counts must *vary* — a finger should move far fewer
        // vertices than a spine. A flat count across bones would mean the lookup
        // is ignoring which bone was asked about.
        {
            let count_for = |name: &str| -> Option<(String, usize)> {
                let node = model.nodes.iter().position(|n| {
                    n.name.contains(name) && n.kind == review_model::NodeKind::Bone
                })?;
                let key = [node as u32];
                let count = (0..skin.logical_vertex_count())
                    .filter(|&logical| {
                        skin.bones[skin.influence_range(logical)]
                            .iter()
                            .any(|bone| key.binary_search(bone).is_ok())
                    })
                    .count();
                Some((model.nodes[node].name.clone(), count))
            };
            let samples: Vec<(String, usize)> = ["Spine1", "Head", "Pinky3_L", "Hand_L"]
                .iter()
                .filter_map(|name| count_for(name))
                .collect();
            println!("per-bone influenced vertices: {samples:?}");
            let counts: Vec<usize> = samples.iter().map(|(_, count)| *count).collect();
            assert!(
                counts.iter().any(|&count| count > 0),
                "no sampled bone influenced anything"
            );
            assert!(
                counts.iter().min() != counts.iter().max(),
                "every sampled bone influenced the same number of vertices - the                  per-bone lookup is not actually discriminating: {samples:?}"
            );
        }

        // The corner map must land inside the logical range for every render vertex.
        for &logical in &skin.corner_to_logical {
            assert!(
                (logical as usize) < skin.logical_vertex_count(),
                "corner maps to logical vertex {logical} of {}",
                skin.logical_vertex_count()
            );
        }
    }

    /// The negative control: a plain mesh must import with no bones and no skin
    /// payload at all, so the skeletal UI stays hidden for ordinary models.
    #[test]
    fn unskinned_import_carries_no_skeleton() {
        let path = fixture("meter_cube.fbx");
        if !path.exists() {
            eprintln!("skipping: {} is not present", path.display());
            return;
        }
        let model = load_model(&path).expect("the cube fixture must import");
        assert_eq!(model.stats.bone_count, 0);
        assert!(model.skin.is_none(), "an unskinned mesh must carry no skin");
        assert!(
            model
                .nodes
                .iter()
                .all(|node| node.kind != review_model::NodeKind::Bone),
            "an unskinned mesh must classify no node as a bone"
        );
    }
}
