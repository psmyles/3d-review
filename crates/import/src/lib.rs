use std::path::Path;

use review_model::ModelData;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("FBX import is unavailable until third_party/ufbx contains ufbx.c and ufbx.h")]
    UfbxUnavailable,
    #[error("unsupported file extension: {0}")]
    UnsupportedExtension(String),
    #[error("failed to load model: {0}")]
    LoadFailed(String),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LoadOptions {
    pub triangulate: bool,
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

pub fn load_model(path: impl AsRef<Path>, options: LoadOptions) -> Result<ModelData, ImportError> {
    let path = path.as_ref();
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if extension.eq_ignore_ascii_case("fbx") => load_fbx(path, options),
        Some(extension) => Err(ImportError::UnsupportedExtension(extension.to_owned())),
        None => Err(ImportError::UnsupportedExtension("<none>".to_owned())),
    }
}

pub fn load_fbx(_path: &Path, _options: LoadOptions) -> Result<ModelData, ImportError> {
    #[cfg(has_ufbx)]
    {
        ffi::load_fbx(_path, _options)
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
        MaterialInfo, ModelData, ModelStats, ModelWarning, SceneNode, TopologyFace, Vertex,
    };

    use crate::{ImportError, LoadOptions};

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
        draw_count: u32,
        base_color: [f32; 3],
        smoothness: f32,
        metallic: f32,
        emissive: [f32; 3],
    }

    #[repr(C)]
    struct ReviewImportWarning {
        message: *mut c_char,
    }

    #[repr(C)]
    struct ReviewImportNode {
        name: *mut c_char,
        parent: i32,
        mesh_part_index: i32,
        transform: [f32; 16],
    }

    #[repr(C)]
    struct ReviewImportScene {
        name: *mut c_char,
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
        warnings: *mut ReviewImportWarning,
        warning_count: usize,
        uv_set_count: u32,
        draw_count: u32,
        uvs: *mut f32,
        uv_value_count: usize,
        source_unit_meters: f32,
        uv_set_names: *mut *mut c_char,
        uv_set_name_count: usize,
        nodes: *mut ReviewImportNode,
        node_count: usize,
        tri_material: *mut u32,
        tri_material_count: usize,
    }

    #[repr(C)]
    struct ReviewImportOptions {
        triangulate: bool,
    }

    #[repr(C)]
    struct ReviewImportError {
        message: [c_char; 256],
    }

    unsafe extern "C" {
        fn review_import_load_fbx(
            path: *const c_char,
            options: *const ReviewImportOptions,
            out_scene: *mut ReviewImportScene,
            out_error: *mut ReviewImportError,
        ) -> c_int;

        fn review_import_free_scene(scene: *mut ReviewImportScene);
    }

    pub(super) fn load_fbx(path: &Path, options: LoadOptions) -> Result<ModelData, ImportError> {
        let path_string = path.to_string_lossy();
        let c_path = CString::new(path_string.as_bytes())
            .map_err(|_| ImportError::LoadFailed("path contains embedded NUL byte".to_owned()))?;
        let bridge_options = ReviewImportOptions {
            triangulate: options.triangulate,
        };
        let mut scene = MaybeUninit::<ReviewImportScene>::zeroed();
        let mut error = ReviewImportError { message: [0; 256] };

        let loaded = unsafe {
            review_import_load_fbx(
                c_path.as_ptr(),
                &bridge_options,
                scene.as_mut_ptr(),
                &mut error,
            )
        };

        if loaded == 0 {
            return Err(ImportError::LoadFailed(read_error_message(&error)));
        }

        let mut scene = unsafe { scene.assume_init() };
        let model = model_from_bridge_scene(path, &scene);
        unsafe {
            review_import_free_scene(&mut scene);
        }
        model
    }

    fn model_from_bridge_scene(
        path: &Path,
        scene: &ReviewImportScene,
    ) -> Result<ModelData, ImportError> {
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
        let tri_to_face =
            checked_slice(scene.tri_to_face, scene.tri_to_face_count, "tri_to_face")?.to_vec();
        let tri_material =
            checked_slice(scene.tri_material, scene.tri_material_count, "tri_material")?.to_vec();
        let nodes = checked_slice(scene.nodes, scene.node_count, "nodes")?
            .iter()
            .map(|node| SceneNode {
                name: read_optional_c_string(node.name).unwrap_or_default(),
                parent: (node.parent >= 0).then_some(node.parent as usize),
                mesh_part: (node.mesh_part_index >= 0).then_some(node.mesh_part_index as usize),
                transform: Mat4::from_cols_array(&node.transform),
            })
            .collect::<Vec<_>>();
        let uv_channels = build_uv_channels(scene)?;
        let uv_set_names =
            checked_slice(scene.uv_set_names, scene.uv_set_name_count, "uv_set_names")?
                .iter()
                .map(|&name| read_optional_c_string(name).unwrap_or_default())
                .collect::<Vec<_>>();
        let materials = checked_slice(scene.materials, scene.material_count, "materials")?
            .iter()
            .map(|material| MaterialInfo {
                name: read_optional_c_string(material.name).unwrap_or_else(|| "Default".to_owned()),
                draw_count: material.draw_count as usize,
                base_color: Vec3::from_array(material.base_color),
                smoothness: material.smoothness,
                metallic: material.metallic,
                emissive: Vec3::from_array(material.emissive),
            })
            .collect::<Vec<_>>();
        let warnings = checked_slice(scene.warnings, scene.warning_count, "warnings")?
            .iter()
            .filter_map(|warning| {
                read_optional_c_string(warning.message).map(|message| ModelWarning { message })
            })
            .collect::<Vec<_>>();

        let name = read_optional_c_string(scene.name).unwrap_or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("Imported Model")
                .to_owned()
        });

        let mut model = ModelData {
            name,
            vertices,
            indices,
            faces,
            tri_to_face,
            tri_material,
            nodes,
            uv_channels,
            uv_set_names,
            bounds: None,
            stats: ModelStats {
                polygon_count: scene.face_count,
                triangle_count: scene.index_count / 3,
                vertex_count: scene.vertex_count,
                uv_set_count: scene.uv_set_count as usize,
                material_count: scene.material_count,
                // Overwritten below from `material_draw_count` so the Draws stat
                // matches the renderer's per-material grouping (invariant 5).
                draw_count: 0,
                source_unit_meters: scene.source_unit_meters,
            },
            materials,
            warnings,
        };
        model.recompute_bounds();
        // The renderer groups triangles into one draw per distinct material slot;
        // report that count rather than the C bridge's per-node tally so Draws is
        // the real draw-call count.
        model.stats.draw_count = model.material_draw_count();

        if model.vertices.is_empty() || model.indices.is_empty() {
            return Err(ImportError::LoadFailed(
                "loaded FBX did not contain triangulated mesh data".to_owned(),
            ));
        }

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

        let vertex_count = scene.vertex_count;
        let expected = channel_count * vertex_count * 2;
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
                        let base = (channel * vertex_count + vertex) * 2;
                        Vec2::new(values[base], values[base + 1])
                    })
                    .collect()
            })
            .collect();

        Ok(channels)
    }

    fn read_error_message(error: &ReviewImportError) -> String {
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

        unsafe { CStr::from_ptr(value).to_str().ok().map(str::to_owned) }
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

        Ok(unsafe { slice::from_raw_parts(ptr.as_ptr(), len) })
    }
}

#[cfg(all(test, has_ufbx))]
mod tests {
    use std::path::PathBuf;

    use super::{LoadOptions, load_model};

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/test_models")
            .join(name)
    }

    /// Phase 0 plumbing: a loaded FBX must carry the scene-graph hierarchy and a
    /// per-triangle material slot parallel to the triangle list.
    #[test]
    fn import_carries_nodes_and_per_triangle_material() {
        let model = load_model(fixture("meter_cube.fbx"), LoadOptions::default())
            .expect("meter_cube.fbx should import");

        assert!(
            !model.nodes.is_empty(),
            "imported scene-graph hierarchy must be non-empty"
        );
        assert_eq!(
            model.tri_material.len(),
            model.stats.triangle_count,
            "tri_material must hold exactly one entry per triangle"
        );
        assert_eq!(
            model.tri_material.len(),
            model.tri_to_face.len(),
            "tri_material must run parallel to tri_to_face"
        );

        // Every recorded slot is either a valid material index or the
        // no-material sentinel.
        for &slot in &model.tri_material {
            assert!(
                slot == u32::MAX || (slot as usize) < model.materials.len(),
                "tri_material slot {slot} out of range"
            );
        }
    }
}
