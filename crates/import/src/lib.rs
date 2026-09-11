use std::path::Path;

use review_model::{AnimContext, ModelData, SourceExtras};
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

/// The stage an import is in, as reported to [`load_model_with_progress`].
///
/// These are the four measured phases of a load, in order; the numbers beside
/// them are a 2.8M-triangle, 120 MB FBX, which is the shape that made a progress
/// indicator worth having at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportStage {
    /// ufbx parsing the file (~1.6 s). The one stage with a real denominator:
    /// `done`/`total` are bytes, straight from ufbx's own progress callback.
    Reading,
    /// Marshaling the bridge's flat arrays into `ModelData`, validating them, and
    /// the rest-pose bounds and tangents the first frame needs (~0.3 s). No
    /// denominator. The model is drawable at the end of this stage.
    Building,
    /// Marshaling the source-property capture ([`SourceExtras`]) — authored
    /// properties, textures, topology layers, curves. No denominator, and after
    /// the model is on screen — see [`PendingExtras`].
    Extras,
    /// Measuring each clip's motion envelope (~0.6 s); `done`/`total` count clips.
    /// Runs *after* the model is on screen — see [`measure_clip_bounds`].
    Measuring,
    /// The per-draw-group stats table (~2.6 s). No denominator, and also after the
    /// model is on screen — see [`ModelData::mesh_group_stats`].
    Finishing,
}

impl ImportStage {
    /// The user-facing verb for this stage, as it appears in the loading card.
    pub fn label(self) -> &'static str {
        match self {
            Self::Reading => "Reading",
            Self::Building => "Building mesh",
            Self::Extras => "Reading source properties",
            Self::Measuring => "Measuring animation",
            Self::Finishing => "Measuring stats",
        }
    }
}

/// One progress report from an import. `total` is 0 when the stage has no
/// denominator, in which case only the stage itself is meaningful.
#[derive(Debug, Clone, Copy)]
pub struct ImportProgress {
    pub stage: ImportStage,
    pub done: u64,
    pub total: u64,
}

impl ImportProgress {
    /// A stage with no denominator, or one that hasn't started counting.
    pub fn stage(stage: ImportStage) -> Self {
        Self {
            stage,
            done: 0,
            total: 0,
        }
    }

    /// How far through this stage the import is, `None` when it can't be known.
    pub fn fraction(self) -> Option<f32> {
        (self.total > 0).then(|| (self.done as f32 / self.total as f32).clamp(0.0, 1.0))
    }
}

/// What [`load_model_with_progress`] reports through. Called on the importing
/// thread — from inside the ufbx parse for [`ImportStage::Reading`] — so it must
/// be cheap and must not block: `app` throttles and forwards to the event loop.
pub type ProgressSink<'a> = &'a dyn Fn(ImportProgress);

/// Import `path` completely, reporting nothing: the drawable model plus every
/// measurement [`load_model_with_progress`] leaves for the caller. What a test or
/// a batch tool wants; the viewer takes the staged path instead, so it can put
/// the mesh on screen before the measuring is done.
pub fn load_model(path: impl AsRef<Path>) -> Result<ModelData, ImportError> {
    let mut model = load_model_with_progress(path, &|_| {})?;
    model.stats.gpu_vertex_count = model.count_gpu_vertices();
    Ok(model)
}

/// [`load_model`] plus the source-property capture: the complete import, for
/// tests, batch callers and anything that will re-export. `None` extras means
/// the capture was unavailable, never that the file had nothing to capture.
pub fn load_model_full(
    path: impl AsRef<Path>,
) -> Result<(ModelData, Option<SourceExtras>), ImportError> {
    let StagedImport { mut model, extras } = load_model_staged(path, &|_| {})?;
    model.stats.gpu_vertex_count = model.count_gpu_vertices();
    let extras = extras.marshal(&model)?;
    Ok((model, extras))
}

/// A drawable model plus the source-property capture still waiting to be
/// marshaled. See [`load_model_staged`].
pub struct StagedImport {
    pub model: ModelData,
    pub extras: PendingExtras,
}

/// The source-property capture as the C bridge left it: taken in the same
/// extraction call as the geometry (invariant 7), but marshaled into
/// [`SourceExtras`] only when [`PendingExtras::marshal`] is called — which
/// `app` does *after* it has published the model, so the viewport shows the
/// mesh first and the properties stream in behind it. The C buffers are freed
/// when this is marshaled or dropped, on every path.
pub struct PendingExtras {
    #[cfg(has_ufbx)]
    handle: Option<ffi::ExtrasHandle>,
}

impl PendingExtras {
    /// Nothing captured (a build without vendored ufbx, or a caller that
    /// declined the capture).
    pub fn none() -> Self {
        Self {
            #[cfg(has_ufbx)]
            handle: None,
        }
    }

    /// Marshal the capture against the model it was taken with. `Ok(None)` when
    /// nothing was captured; `Err` when the capture does not describe `model`
    /// (a drift the funnel guard [`SourceExtras::validate`] caught), in which
    /// case nothing is published.
    pub fn marshal(self, model: &ModelData) -> Result<Option<SourceExtras>, ImportError> {
        #[cfg(has_ufbx)]
        {
            let _z = crate::prof::zone!("Marshal Extras");
            match self.handle {
                Some(handle) => handle.marshal(model).map(Some),
                None => Ok(None),
            }
        }
        #[cfg(not(has_ufbx))]
        {
            let _ = model;
            Ok(None)
        }
    }
}

impl std::fmt::Debug for PendingExtras {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PendingExtras")
    }
}

/// [`load_model_with_progress`] that also keeps the source-property capture,
/// for the caller to marshal once the model is on screen. Same stages, same
/// stopping point for the model itself.
pub fn load_model_staged(
    path: impl AsRef<Path>,
    progress: ProgressSink<'_>,
) -> Result<StagedImport, ImportError> {
    let path = path.as_ref();
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if extension.eq_ignore_ascii_case("fbx") => load_fbx_staged(path, progress),
        Some(extension) => Err(ImportError::UnsupportedExtension(extension.to_owned())),
        None => Err(ImportError::UnsupportedExtension("<none>".to_owned())),
    }
}

/// Import `path` as far as *drawable*, reporting each stage to `progress` as it
/// goes: geometry, materials, the scene graph, the rest-pose bounds the camera
/// frames on, and tangents.
///
/// This is the one import funnel (invariant 7), and it stops where the viewport
/// stops caring. Two measurements are deliberately **not** made here, because
/// nothing on screen needs them and together they are the majority of a large
/// load — a 2.8M-triangle scene reaches this point in 1.8 s and takes another
/// 3.2 s to measure:
///
/// * each clip's motion envelope ([`measure_clip_bounds`]), which only framing
///   and the bounding box on a *selected* clip use;
/// * the per-draw-group table ([`ModelData::mesh_group_stats`]) behind the stats
///   card's GPU Verts row and its scoped columns — hence `stats.gpu_vertex_count`
///   is left `0`, which the card reads as "not measured yet" and simply omits.
///
/// The caller makes them when it wants them; [`load_model`] makes them straight
/// away, `app` makes them on the import worker after the model is on screen and
/// folds each into the UI as it lands.
pub fn load_model_with_progress(
    path: impl AsRef<Path>,
    progress: ProgressSink<'_>,
) -> Result<ModelData, ImportError> {
    let path = path.as_ref();
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if extension.eq_ignore_ascii_case("fbx") => load_fbx(path, progress),
        Some(extension) => Err(ImportError::UnsupportedExtension(extension.to_owned())),
        None => Err(ImportError::UnsupportedExtension("<none>".to_owned())),
    }
}

pub fn load_fbx(_path: &Path, _progress: ProgressSink<'_>) -> Result<ModelData, ImportError> {
    #[cfg(has_ufbx)]
    {
        ffi::load_fbx(_path, _progress, false).map(|staged| staged.model)
    }

    #[cfg(not(has_ufbx))]
    {
        Err(ImportError::UfbxUnavailable)
    }
}

fn load_fbx_staged(_path: &Path, _progress: ProgressSink<'_>) -> Result<StagedImport, ImportError> {
    #[cfg(has_ufbx)]
    {
        ffi::load_fbx(_path, _progress, true)
    }

    #[cfg(not(has_ufbx))]
    {
        Err(ImportError::UfbxUnavailable)
    }
}

/// Each clip's motion envelope — the union of the posed mesh's bounds over every
/// frame — parallel to [`ModelData::animations`]. `None` for a clip that moves
/// nothing measurable.
///
/// Split out of the import funnel so it can run *after* the model is drawn (see
/// [`load_model_with_progress`]); it lives here rather than in the caller so the
/// two things a caller could get wrong — using the file's own frame rate, and
/// building the [`AnimContext`] once rather than per clip — are decided once.
pub fn measure_clip_bounds(model: &ModelData) -> Vec<Option<review_model::Bounds>> {
    if model.animations.is_empty() {
        return Vec::new();
    }
    let ctx = AnimContext::new(model);
    let fps = model.frame_rate_or_default();
    model
        .animations
        .iter()
        .map(|clip| review_model::anim::clip_bounds(model, &ctx, clip, fps))
        .collect()
}

#[cfg(has_ufbx)]
mod ffi {
    use std::{
        ffi::{CStr, CString, c_void},
        mem::MaybeUninit,
        os::raw::{c_char, c_int},
        path::Path,
        ptr::NonNull,
        slice,
    };

    use glam::{DMat4, Mat4, Quat, Vec2, Vec3, Vec4};
    use review_model::{
        AnimContext, AnimationClip, BoneInfo, ExtrasCounts, Key, LocalTransform,
        MaterialImportDefaults, ModelData, ModelStats, MorphChannel, MorphData, MorphKeyframe,
        MorphShape, MorphTrack, NodeKind, NodeTrack, SceneNode, SkinCluster, SkinData,
        SkinDeformerInfo, SkinningMethod, SourceExtras, TopologyFace, TriangleData, Vertex, anim,
        extras,
    };

    use crate::{ImportError, PendingExtras, StagedImport};

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
        /// The bridge's own back-reference to the ufbx material; opaque here.
        _source: *const c_void,
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
        /// The rest local transform: translation, rotation (xyzw), scale.
        local_translation: [f32; 3],
        local_rotation: [f32; 4],
        local_scale: [f32; 3],
    }

    #[repr(C)]
    struct ReviewImportSkinCluster {
        bone: u32,
        mesh_node: u32,
        world_to_bone_bind: [f32; 16],
        mesh_node_to_bone: [f64; 16],
        bind_to_world: [f64; 16],
        name: *mut c_char,
    }

    #[repr(C)]
    struct ReviewImportSkinDeformer {
        mesh_node: u32,
        /// `ufbx_skinning_method` code; see [`skinning_method_from_code`].
        method: u32,
        max_weights_per_vertex: u32,
    }

    #[repr(C)]
    struct ReviewImportMorphChannel {
        name: *mut c_char,
        mesh_node: u32,
        rest_weight: f32,
        keyframe_first: u32,
        keyframe_count: u32,
    }

    #[repr(C)]
    struct ReviewImportMorphKeyframe {
        shape: u32,
        target_weight: f32,
    }

    #[repr(C)]
    struct ReviewImportMorphShape {
        name: *mut c_char,
    }

    #[repr(C)]
    struct ReviewImportMorphEntry {
        logical_vertex: u32,
        shape: u32,
        position: [f32; 3],
        normal: [f32; 3],
    }

    #[repr(C)]
    struct ReviewImportAnimStack {
        name: *mut c_char,
        time_begin: f64,
        time_end: f64,
        node_track_first: u32,
        node_track_count: u32,
        morph_track_first: u32,
        morph_track_count: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ReviewImportNodeTrack {
        node: u32,
        translation_first: u32,
        translation_count: u32,
        rotation_first: u32,
        rotation_count: u32,
        scale_first: u32,
        scale_count: u32,
    }

    #[repr(C)]
    struct ReviewImportVec3Key {
        time: f64,
        value: [f32; 3],
    }

    #[repr(C)]
    struct ReviewImportQuatKey {
        time: f64,
        value: [f32; 4],
    }

    #[repr(C)]
    struct ReviewImportMorphTrack {
        channel: u32,
        first: u32,
        count: u32,
    }

    #[repr(C)]
    struct ReviewImportScalarKey {
        time: f64,
        value: f32,
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
        /// Per influence (parallel to `skin_bones`), its `skin_clusters` index.
        skin_influence_cluster: *mut u32,
        skin_clusters: *mut ReviewImportSkinCluster,
        skin_cluster_count: usize,
        skin_deformers: *mut ReviewImportSkinDeformer,
        skin_deformer_count: usize,
        /// Blend shapes; all null / zero when no mesh carries any.
        morph_channels: *mut ReviewImportMorphChannel,
        morph_channel_count: usize,
        morph_keyframes: *mut ReviewImportMorphKeyframe,
        morph_keyframe_count: usize,
        morph_shapes: *mut ReviewImportMorphShape,
        morph_shape_count: usize,
        morph_entries: *mut ReviewImportMorphEntry,
        morph_entry_count: usize,
        /// Animation clips; all null / zero for a file without animation.
        anim_stacks: *mut ReviewImportAnimStack,
        anim_stack_count: usize,
        anim_node_tracks: *mut ReviewImportNodeTrack,
        anim_node_track_count: usize,
        anim_vec3_keys: *mut ReviewImportVec3Key,
        anim_vec3_key_count: usize,
        anim_quat_keys: *mut ReviewImportQuatKey,
        anim_quat_key_count: usize,
        anim_morph_tracks: *mut ReviewImportMorphTrack,
        anim_morph_track_count: usize,
        anim_scalar_keys: *mut ReviewImportScalarKey,
        anim_scalar_key_count: usize,
        frames_per_second: f64,
    }

    #[repr(C)]
    struct ReviewImportError {
        message: [c_char; 256],
    }

    // ---- The source-property capture (`ufbx_extras.h`), field for field. ----

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct XStr {
        offset: u32,
        length: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct XBytes {
        offset: usize,
        length: usize,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct XPropRange {
        first: u32,
        count: u32,
    }

    #[repr(C)]
    struct XProp {
        name: XStr,
        kind: u32,
        flags: u32,
        value_int: i64,
        value_real: [f64; 4],
        value_str: XStr,
        value_blob: XBytes,
    }

    #[repr(C)]
    struct XNode {
        props: XPropRange,
        rotation_order: u32,
        inherit_mode: u32,
        original_inherit_mode: u32,
        geometry_to_node: [f64; 16],
        synthetic: u32,
        visible: u32,
        attribute_kind: u32,
        attribute_index: i32,
        attribute_name: XStr,
        attribute_props: XPropRange,
    }

    #[repr(C)]
    struct XLight {
        color: [f64; 3],
        intensity: f64,
        local_direction: [f64; 3],
        kind: u32,
        decay: u32,
        area_shape: u32,
        inner_angle: f64,
        outer_angle: f64,
        cast_light: u32,
        cast_shadows: u32,
    }

    #[repr(C)]
    struct XCamera {
        projection_mode: u32,
        resolution_is_pixels: u32,
        resolution: [f64; 2],
        field_of_view_deg: [f64; 2],
        orthographic_extent: f64,
        aspect_ratio: f64,
        near_plane: f64,
        far_plane: f64,
        aspect_mode: u32,
        aperture_mode: u32,
        gate_fit: u32,
        aperture_format: u32,
        focal_length_mm: f64,
        film_size_inch: [f64; 2],
        aperture_size_inch: [f64; 2],
        squeeze_ratio: f64,
    }

    #[repr(C)]
    struct XLodGroup {
        relative_distances: u32,
        ignore_parent_transform: u32,
        use_distance_limit: u32,
        distance_limit_min: f64,
        distance_limit_max: f64,
        level_first: u32,
        level_count: u32,
    }

    #[repr(C)]
    struct XLodLevel {
        distance: f64,
        display: u32,
    }

    #[repr(C)]
    struct XMaterial {
        props: XPropRange,
        shader_type: u32,
        shading_model: XStr,
        texture_first: u32,
        texture_count: u32,
    }

    #[repr(C)]
    struct XMaterialTexture {
        material_prop: XStr,
        shader_prop: XStr,
        texture: u32,
    }

    #[repr(C)]
    struct XTexture {
        name: XStr,
        kind: u32,
        filename: XStr,
        absolute_filename: XStr,
        relative_filename: XStr,
        uv_set: XStr,
        wrap_u: u32,
        wrap_v: u32,
        has_uv_transform: u32,
        uv_translation: [f64; 3],
        uv_rotation: [f64; 4],
        uv_scale: [f64; 3],
        content: XBytes,
        video: i32,
        layer_first: u32,
        layer_count: u32,
        props: XPropRange,
    }

    #[repr(C)]
    struct XTextureLayer {
        texture: u32,
        blend_mode: u32,
        alpha: f64,
    }

    #[repr(C)]
    struct XVideo {
        name: XStr,
        filename: XStr,
        absolute_filename: XStr,
        relative_filename: XStr,
        content: XBytes,
        props: XPropRange,
    }

    #[repr(C)]
    struct XColorSet {
        name: XStr,
        index: u32,
        value_first: u32,
    }

    #[repr(C)]
    struct XFaceGroup {
        id: i32,
        name: XStr,
    }

    #[repr(C)]
    struct XExtraSkin {
        method: u32,
        max_weights_per_vertex: u32,
        cluster_first: u32,
        cluster_count: u32,
        offset_first: u32,
        influence_first: u32,
        influence_count: u32,
    }

    #[repr(C)]
    struct XExtraCluster {
        bone: u32,
        name: XStr,
        mesh_node_to_bone: [f64; 16],
        bind_to_world: [f64; 16],
    }

    #[repr(C)]
    struct XExtraInfluence {
        cluster: u32,
        weight: f32,
    }

    #[repr(C)]
    struct XDqWeight {
        logical_vertex: u32,
        weight: f64,
    }

    #[repr(C)]
    struct XMesh {
        node: u32,
        name: XStr,
        props: XPropRange,
        corner_first: u32,
        corner_count: u32,
        logical_first: u32,
        logical_count: u32,
        face_first: u32,
        face_count: u32,
        tangents_authored: u32,
        reversed_winding: u32,
        color_set_first: u32,
        color_set_count: u32,
        edge_first: u32,
        edge_count: u32,
        face_group_first: u32,
        face_group_count: u32,
        has_face_smoothing: u32,
        has_face_hole: u32,
        has_face_group: u32,
        has_edge_smoothing: u32,
        has_edge_crease: u32,
        has_edge_visibility: u32,
        has_vertex_crease: u32,
        subdivision_preview_levels: u32,
        subdivision_render_levels: u32,
        subdivision_display_mode: u32,
        subdivision_boundary: u32,
        subdivision_uv_boundary: u32,
        extra_skin_first: u32,
        extra_skin_count: u32,
        dq_first: u32,
        dq_count: u32,
    }

    #[repr(C)]
    struct XPose {
        name: XStr,
        is_bind_pose: u32,
        entry_first: u32,
        entry_count: u32,
        props: XPropRange,
    }

    #[repr(C)]
    struct XPoseEntry {
        node: u32,
        bone_to_world: [f64; 16],
    }

    #[repr(C)]
    struct XDisplayLayer {
        name: XStr,
        visible: u32,
        frozen: u32,
        ui_color: [f64; 3],
        node_first: u32,
        node_count: u32,
        props: XPropRange,
    }

    #[repr(C)]
    struct XSelectionSet {
        name: XStr,
        props: XPropRange,
        node_first: u32,
        node_count: u32,
    }

    #[repr(C)]
    struct XSelectionNode {
        node: i32,
        include_node: u32,
        vertex_first: u32,
        vertex_count: u32,
        edge_first: u32,
        edge_count: u32,
        face_first: u32,
        face_count: u32,
    }

    #[repr(C)]
    struct XAnimStack {
        name: XStr,
        props: XPropRange,
        clip: i32,
        time_begin: f64,
        time_end: f64,
        layer_first: u32,
        layer_count: u32,
    }

    #[repr(C)]
    struct XAnimLayer {
        name: XStr,
        weight: f64,
        weight_is_animated: u32,
        blended: u32,
        additive: u32,
        compose_rotation: u32,
        compose_scale: u32,
        props: XPropRange,
        anim_prop_first: u32,
        anim_prop_count: u32,
    }

    #[repr(C)]
    struct XAnimProp {
        target_kind: u32,
        target: u32,
        element_type: u32,
        element_name: XStr,
        prop_name: XStr,
        default_value: [f64; 3],
        curves: [i32; 3],
    }

    #[repr(C)]
    struct XAnimCurve {
        key_first: u32,
        key_count: u32,
        pre_mode: u32,
        pre_repeat: i32,
        post_mode: u32,
        post_repeat: i32,
    }

    #[repr(C)]
    struct XAnimKey {
        time: f64,
        value: f64,
        interpolation: u32,
        left_dx: f32,
        left_dy: f32,
        right_dx: f32,
        right_dy: f32,
    }

    #[repr(C)]
    struct XScene {
        creator: XStr,
        filename: XStr,
        original_file_path: XStr,
        version: u32,
        ascii: u32,
        original_vendor: XStr,
        original_name: XStr,
        original_version: XStr,
        latest_vendor: XStr,
        latest_name: XStr,
        latest_version: XStr,
        scene_props: XPropRange,
        settings_props: XPropRange,
        axis_right: u32,
        axis_up: u32,
        axis_front: u32,
        original_axis_up: u32,
        unit_meters: f64,
        original_unit_meters: f64,
        frames_per_second: f64,
        ambient_color: [f64; 3],
        default_camera: XStr,
        time_mode: u32,
        time_protocol: u32,
        snap_mode: u32,
    }

    #[repr(C)]
    struct ReviewImportExtras {
        strings: *mut c_char,
        string_count: usize,
        string_capacity: usize,
        bytes: *mut u8,
        byte_count: usize,
        byte_capacity: usize,
        props: *mut XProp,
        prop_count: usize,
        scene: XScene,
        nodes: *mut XNode,
        node_count: usize,
        lights: *mut XLight,
        light_count: usize,
        cameras: *mut XCamera,
        camera_count: usize,
        lod_groups: *mut XLodGroup,
        lod_group_count: usize,
        lod_levels: *mut XLodLevel,
        lod_level_count: usize,
        materials: *mut XMaterial,
        material_count: usize,
        material_textures: *mut XMaterialTexture,
        material_texture_count: usize,
        textures: *mut XTexture,
        texture_count: usize,
        texture_layers: *mut XTextureLayer,
        texture_layer_count: usize,
        videos: *mut XVideo,
        video_count: usize,
        meshes: *mut XMesh,
        mesh_count: usize,
        color_sets: *mut XColorSet,
        color_set_count: usize,
        color_values: *mut f64,
        color_value_count: usize,
        edges: *mut u32,
        edge_smoothing: *mut u8,
        edge_crease: *mut f64,
        edge_visibility: *mut u8,
        edge_count: usize,
        face_smoothing: *mut u8,
        face_hole: *mut u8,
        face_group: *mut u32,
        face_count: usize,
        vertex_crease: *mut f64,
        vertex_crease_count: usize,
        face_groups: *mut XFaceGroup,
        face_group_count: usize,
        extra_skins: *mut XExtraSkin,
        extra_skin_count: usize,
        extra_clusters: *mut XExtraCluster,
        extra_cluster_count: usize,
        extra_skin_offsets: *mut u32,
        extra_skin_offset_count: usize,
        extra_influences: *mut XExtraInfluence,
        extra_influence_count: usize,
        dq_weights: *mut XDqWeight,
        dq_weight_count: usize,
        poses: *mut XPose,
        pose_count: usize,
        pose_entries: *mut XPoseEntry,
        pose_entry_count: usize,
        display_layers: *mut XDisplayLayer,
        display_layer_count: usize,
        layer_nodes: *mut u32,
        layer_node_count: usize,
        selection_sets: *mut XSelectionSet,
        selection_set_count: usize,
        selection_nodes: *mut XSelectionNode,
        selection_node_count: usize,
        selection_indices: *mut u32,
        selection_index_count: usize,
        anim_stacks: *mut XAnimStack,
        anim_stack_count: usize,
        stack_layers: *mut u32,
        stack_layer_count: usize,
        anim_layers: *mut XAnimLayer,
        anim_layer_count: usize,
        anim_props: *mut XAnimProp,
        anim_prop_count: usize,
        anim_curves: *mut XAnimCurve,
        anim_curve_count: usize,
        anim_keys: *mut XAnimKey,
        anim_key_count: usize,
    }

    /// The C-side capture, owned until marshaled. It holds only heap buffers
    /// the bridge allocated for this call — nothing borrowed from ufbx, whose
    /// scene is already freed — so moving it to another thread is sound.
    pub(crate) struct ExtrasHandle {
        raw: Box<ReviewImportExtras>,
    }

    // SAFETY: the handle exclusively owns C heap allocations that no other
    // thread references (the bridge hands them over and never touches them
    // again); the pointers inside are plain data until `review_import_free_extras`.
    unsafe impl Send for ExtrasHandle {}

    impl Drop for ExtrasHandle {
        fn drop(&mut self) {
            // SAFETY: `raw` was filled by `review_import_load_fbx` and is freed
            // exactly once, here; the bridge's free tolerates a zeroed struct.
            unsafe {
                review_import_free_extras(&mut *self.raw);
            }
        }
    }

    /// The bridge's progress hook: `user` is a `*const ProgressSink`.
    type ReviewImportProgressFn = unsafe extern "C" fn(*mut c_void, u64, u64);

    unsafe extern "C" {
        fn review_import_load_fbx(
            path: *const c_char,
            out_scene: *mut ReviewImportScene,
            out_extras: *mut ReviewImportExtras,
            out_error: *mut ReviewImportError,
            progress: Option<ReviewImportProgressFn>,
            progress_user: *mut c_void,
        ) -> c_int;

        fn review_import_free_scene(scene: *mut ReviewImportScene);

        fn review_import_free_extras(extras: *mut ReviewImportExtras);
    }

    /// The trampoline the bridge calls from inside the ufbx parse. `user` is the
    /// `&ProgressSink` handed to [`load_fbx`], which outlives the whole call.
    ///
    /// A panic here would unwind through C, so the sink is called inside
    /// `catch_unwind` and a panicking one is simply ignored — a broken progress
    /// indicator must not take the import down with it.
    unsafe extern "C" fn report_read_progress(user: *mut c_void, done: u64, total: u64) {
        let Some(sink) = NonNull::new(user.cast::<crate::ProgressSink<'_>>()) else {
            return;
        };
        // SAFETY: `user` is the `&ProgressSink` `load_fbx` passed to the bridge,
        // which borrows it for no longer than the `review_import_load_fbx` call
        // this callback is made from; the pointer is therefore live and aligned,
        // and nothing else aliases it mutably.
        let sink = unsafe { sink.as_ref() };
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            sink(crate::ImportProgress {
                stage: crate::ImportStage::Reading,
                done,
                total,
            });
        }));
    }

    pub(super) fn load_fbx(
        path: &Path,
        progress: crate::ProgressSink<'_>,
        capture_extras: bool,
    ) -> Result<StagedImport, ImportError> {
        let _z = crate::prof::zone!("Load FBX");
        let path_string = path.to_string_lossy();
        let c_path = CString::new(path_string.as_bytes())
            .map_err(|_| ImportError::LoadFailed("path contains embedded NUL byte".to_owned()))?;
        let mut scene = MaybeUninit::<ReviewImportScene>::zeroed();
        let mut error = ReviewImportError { message: [0; 256] };
        // Boxed so the handle that outlives this call never moves the struct
        // the bridge wrote pointers into. Zeroed is a valid "nothing captured"
        // value the free tolerates.
        // SAFETY: `ReviewImportExtras` is all raw pointers and integers, for
        // which the all-zero bit pattern is a valid value.
        let mut extras: Option<Box<ReviewImportExtras>> =
            capture_extras.then(|| Box::new(unsafe { std::mem::zeroed() }));

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
            // `progress_user` is a pointer to this stack borrow of the caller's
            // sink, which the bridge only dereferences from within this call.
            // `extras` is either null (no capture) or a live boxed struct the
            // bridge fills and this call's handle then owns.
            let mut sink = progress;
            let extras_ptr = extras
                .as_deref_mut()
                .map_or(std::ptr::null_mut(), |extras| {
                    extras as *mut ReviewImportExtras
                });
            unsafe {
                review_import_load_fbx(
                    c_path.as_ptr(),
                    scene.as_mut_ptr(),
                    extras_ptr,
                    &mut error,
                    Some(report_read_progress),
                    (&raw mut sink).cast::<c_void>(),
                )
            }
        };

        if loaded == 0 {
            // The bridge freed both outputs on its failure path; the zeroed box
            // is dropped here without a free.
            return Err(ImportError::LoadFailed(read_error_message(&error)));
        }
        // From here the capture is owned by a handle that frees it on drop.
        let extras = extras.map(|raw| ExtrasHandle { raw });

        // SAFETY: `loaded != 0` means the bridge fully initialized `scene`, so the
        // `MaybeUninit` now holds a valid `ReviewImportScene`.
        let mut scene = unsafe { scene.assume_init() };
        let model = {
            // Walk the flat bridge arrays into our `ModelData` (slices, bounds, BVH).
            let _z = crate::prof::zone!("Build ModelData");
            model_from_bridge_scene(path, &scene, progress)
        };
        // SAFETY: `scene` is the bridge-allocated scene we own; this frees its C-side
        // buffers exactly once, on both the success and error paths of the extraction
        // above (we still return `model` afterwards). The bridge tolerates the zeroed
        // fields a partial parse may leave. No further access to `scene` follows.
        unsafe {
            review_import_free_scene(&mut scene);
        }
        Ok(StagedImport {
            model: model?,
            extras: PendingExtras { handle: extras },
        })
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
                    rest_local: LocalTransform {
                        translation: Vec3::from_array(node.local_translation),
                        rotation: Quat::from_array(node.local_rotation).normalize(),
                        scale: Vec3::from_array(node.local_scale),
                    },
                    kind,
                    bone: (kind == NodeKind::Bone).then_some(BoneInfo {
                        radius: node.bone_radius,
                        relative_length: node.bone_relative_length,
                    }),
                }
            })
            .collect())
    }

    /// The bridge's `review_import_skin_deformer::method` codes (ufbx's
    /// `ufbx_skinning_method`). Unknown codes read as linear, which is how the
    /// viewer evaluates every skin anyway.
    fn skinning_method_from_code(code: u32) -> SkinningMethod {
        match code {
            1 => SkinningMethod::Rigid,
            2 => SkinningMethod::DualQuaternion,
            3 => SkinningMethod::BlendedDqLinear,
            _ => SkinningMethod::Linear,
        }
    }

    /// The per-corner logical-vertex map, marshaled for every model (skinned or
    /// not) since blend shapes need it as well as skin weights.
    fn marshal_corner_map(scene: &ReviewImportScene) -> Result<Vec<u32>, ImportError> {
        Ok(checked_slice(
            scene.corner_source_vertex,
            scene.corner_source_vertex_count,
            "corner_source_vertex",
        )?
        .to_vec())
    }

    /// Marshal the bridge's CSR skin table + cluster table. Returns `None` for
    /// an unskinned scene (the bridge allocates nothing and reports zero
    /// influences), so the common non-skeletal model carries no skin payload at
    /// all.
    fn marshal_skin(scene: &ReviewImportScene) -> Result<Option<SkinData>, ImportError> {
        if scene.skin_influence_count == 0 {
            return Ok(None);
        }
        let clusters = checked_slice(
            scene.skin_clusters,
            scene.skin_cluster_count,
            "skin_clusters",
        )?
        .iter()
        .map(|cluster| SkinCluster {
            bone: cluster.bone,
            mesh_node: cluster.mesh_node,
            world_to_bone_bind: Mat4::from_cols_array(&cluster.world_to_bone_bind),
            mesh_node_to_bone: DMat4::from_cols_array(&cluster.mesh_node_to_bone).as_mat4(),
            bind_to_world: DMat4::from_cols_array(&cluster.bind_to_world).as_mat4(),
            name: read_optional_c_string(cluster.name).unwrap_or_default(),
        })
        .collect();
        let deformers = checked_slice(
            scene.skin_deformers,
            scene.skin_deformer_count,
            "skin_deformers",
        )?
        .iter()
        .map(|deformer| SkinDeformerInfo {
            mesh_node: deformer.mesh_node,
            method: skinning_method_from_code(deformer.method),
            max_weights_per_vertex: deformer.max_weights_per_vertex,
        })
        .collect();
        Ok(Some(SkinData {
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
            influence_cluster: checked_slice(
                scene.skin_influence_cluster,
                scene.skin_influence_count,
                "skin_influence_cluster",
            )?
            .to_vec(),
            clusters,
            deformers,
        }))
    }

    /// Marshal the bridge's blend shapes: the sparse per-shape offsets are
    /// sorted into the per-logical-vertex CSR the shader and the CPU reference
    /// walk. `None` when no mesh carries a channel.
    fn marshal_morph(
        scene: &ReviewImportScene,
        logical_count: usize,
    ) -> Result<Option<MorphData>, ImportError> {
        if scene.morph_channel_count == 0 {
            return Ok(None);
        }
        let keyframes = checked_slice(
            scene.morph_keyframes,
            scene.morph_keyframe_count,
            "morph_keyframes",
        )?;
        let channels = checked_slice(
            scene.morph_channels,
            scene.morph_channel_count,
            "morph_channels",
        )?
        .iter()
        .map(|channel| {
            let first = channel.keyframe_first as usize;
            let end = first.saturating_add(channel.keyframe_count as usize);
            let range = keyframes.get(first..end).ok_or_else(|| {
                ImportError::LoadFailed(format!(
                    "FBX bridge morph channel keyframes {first}..{end} exceed {}",
                    keyframes.len()
                ))
            })?;
            Ok(MorphChannel {
                name: read_optional_c_string(channel.name).unwrap_or_default(),
                mesh_node: channel.mesh_node,
                rest_weight: channel.rest_weight,
                keyframes: range
                    .iter()
                    .map(|key| MorphKeyframe {
                        shape: key.shape,
                        target_weight: key.target_weight,
                    })
                    .collect(),
            })
        })
        .collect::<Result<Vec<_>, ImportError>>()?;
        let shapes = checked_slice(scene.morph_shapes, scene.morph_shape_count, "morph_shapes")?
            .iter()
            .map(|shape| MorphShape {
                name: read_optional_c_string(shape.name).unwrap_or_default(),
            })
            .collect();
        let entries = checked_slice(
            scene.morph_entries,
            scene.morph_entry_count,
            "morph_entries",
        )?;

        // Counting sort into CSR rows by logical vertex, preserving the bridge's
        // emit order within a row. An entry naming a vertex past the logical
        // count is a bridge drift and fails the load here.
        let mut offsets = vec![0u32; logical_count + 1];
        for entry in entries {
            let logical = entry.logical_vertex as usize;
            if logical >= logical_count {
                return Err(ImportError::LoadFailed(format!(
                    "FBX bridge morph entry references source vertex {logical} of {logical_count}"
                )));
            }
            offsets[logical + 1] += 1;
        }
        for index in 1..offsets.len() {
            offsets[index] += offsets[index - 1];
        }
        let mut cursor = offsets.clone();
        let mut shape = vec![0u32; entries.len()];
        let mut position = vec![Vec3::ZERO; entries.len()];
        let mut normal = vec![Vec3::ZERO; entries.len()];
        for entry in entries {
            let slot = &mut cursor[entry.logical_vertex as usize];
            let index = *slot as usize;
            *slot += 1;
            shape[index] = entry.shape;
            position[index] = Vec3::from_array(entry.position);
            normal[index] = Vec3::from_array(entry.normal);
        }

        Ok(Some(MorphData {
            channels,
            shapes,
            offsets,
            shape,
            position,
            normal,
        }))
    }

    /// Marshal the bridge's baked animation stacks into clips.
    fn marshal_animations(scene: &ReviewImportScene) -> Result<Vec<AnimationClip>, ImportError> {
        if scene.anim_stack_count == 0 {
            return Ok(Vec::new());
        }
        let node_tracks = checked_slice(
            scene.anim_node_tracks,
            scene.anim_node_track_count,
            "anim_node_tracks",
        )?;
        let vec3_keys = checked_slice(
            scene.anim_vec3_keys,
            scene.anim_vec3_key_count,
            "anim_vec3_keys",
        )?;
        let quat_keys = checked_slice(
            scene.anim_quat_keys,
            scene.anim_quat_key_count,
            "anim_quat_keys",
        )?;
        let morph_tracks = checked_slice(
            scene.anim_morph_tracks,
            scene.anim_morph_track_count,
            "anim_morph_tracks",
        )?;
        let scalar_keys = checked_slice(
            scene.anim_scalar_keys,
            scene.anim_scalar_key_count,
            "anim_scalar_keys",
        )?;

        fn range<'a, T>(
            items: &'a [T],
            first: u32,
            count: u32,
            what: &str,
        ) -> Result<&'a [T], ImportError> {
            let first = first as usize;
            let end = first.saturating_add(count as usize);
            items.get(first..end).ok_or_else(|| {
                ImportError::LoadFailed(format!(
                    "FBX bridge {what} range {first}..{end} exceeds {}",
                    items.len()
                ))
            })
        }
        let vec3 = |first: u32, count: u32| -> Result<Vec<Key<Vec3>>, ImportError> {
            Ok(range(vec3_keys, first, count, "vec3 key")?
                .iter()
                .map(|key| Key {
                    time: key.time,
                    value: Vec3::from_array(key.value),
                })
                .collect())
        };

        checked_slice(scene.anim_stacks, scene.anim_stack_count, "anim_stacks")?
            .iter()
            .map(|stack| {
                let tracks = range(
                    node_tracks,
                    stack.node_track_first,
                    stack.node_track_count,
                    "node track",
                )?
                .iter()
                .map(|track| {
                    Ok(NodeTrack {
                        node: track.node,
                        translation: vec3(track.translation_first, track.translation_count)?,
                        rotation: range(
                            quat_keys,
                            track.rotation_first,
                            track.rotation_count,
                            "quat key",
                        )?
                        .iter()
                        .map(|key| Key {
                            time: key.time,
                            value: Quat::from_array(key.value),
                        })
                        .collect(),
                        scale: vec3(track.scale_first, track.scale_count)?,
                    })
                })
                .collect::<Result<Vec<_>, ImportError>>()?;
                let morph_tracks = range(
                    morph_tracks,
                    stack.morph_track_first,
                    stack.morph_track_count,
                    "morph track",
                )?
                .iter()
                .map(|track| {
                    Ok(MorphTrack {
                        channel: track.channel,
                        keys: range(scalar_keys, track.first, track.count, "scalar key")?
                            .iter()
                            .map(|key| Key {
                                time: key.time,
                                value: key.value,
                            })
                            .collect(),
                    })
                })
                .collect::<Result<Vec<_>, ImportError>>()?;
                Ok(AnimationClip {
                    name: read_optional_c_string(stack.name).unwrap_or_default(),
                    time_begin: stack.time_begin,
                    time_end: stack.time_end,
                    tracks,
                    morph_tracks,
                })
            })
            .collect()
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
        progress: crate::ProgressSink<'_>,
    ) -> Result<ModelData, ImportError> {
        progress(crate::ImportProgress::stage(crate::ImportStage::Building));
        let MarshaledGeometry {
            vertices,
            indices,
            faces,
            triangles,
        } = marshal_geometry(scene)?;
        let nodes = marshal_nodes(scene)?;
        let corner_to_logical = marshal_corner_map(scene)?;
        let skin = marshal_skin(scene)?;
        let morph = marshal_morph(scene, scene.source_vertex_count)?;
        let animations = marshal_animations(scene)?;
        let bone_count = nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Bone)
            .count();
        let clip_count = animations.len();
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
                clip_count,
                source_unit_meters: scene.source_unit_meters,
            },
            materials,
            corner_to_logical,
            skin,
            morph,
            animations,
            frame_rate: scene.frames_per_second,
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

        // Same funnel guard for everything the deform path reads: the corner map
        // must cover exactly the render mesh, the skin / morph CSR rows must be
        // monotonic and terminate at their entry counts, every influence must
        // name a real node with a finite non-negative weight (and a cluster
        // binding that same bone), and every clip must animate real nodes /
        // channels with ordered finite keys. A drifted bridge fill is caught
        // here, once.
        model.validate_deform().map_err(ImportError::LoadFailed)?;

        // Bounds describe what is on screen. A skinned or morphed model rests in
        // the file's default pose, which the GPU skins into — so its bounds are
        // measured through the same deformation, not from the bind-pose buffer.
        // (Each clip's own motion envelope is *not* measured here: nothing is on
        // screen that needs it until a clip is selected, and on a large scene it
        // costs more than everything above it — see `measure_clip_bounds`.)
        if model.needs_deform() {
            let _z = crate::prof::zone!("Rest Bounds");
            let ctx = AnimContext::new(&model);
            model.bounds = anim::rest_bounds(&model, &ctx);
        } else {
            model.recompute_bounds();
        }
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
        // `stats.gpu_vertex_count` stays 0 — "not measured yet". It is an
        // O(corners) hash walk over the whole mesh, by far the longest item in a
        // large import, and it is a *displayed number*, not something the viewport
        // draws with; the caller measures it once the model is up.

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

    // ---- Marshaling the capture into `SourceExtras` ----

    impl ExtrasHandle {
        pub(crate) fn marshal(self, model: &ModelData) -> Result<SourceExtras, ImportError> {
            let extras = marshal_extras(&self.raw)?;
            extras
                .validate(&ExtrasCounts::of(model))
                .map_err(|message| {
                    ImportError::LoadFailed(format!("source properties: {message}"))
                })?;
            Ok(extras)
        }
    }

    /// Borrowed views over the capture's arenas and tables, with the range
    /// checks every reference goes through.
    struct ExtrasView<'a> {
        strings: &'a [u8],
        bytes: &'a [u8],
        props: &'a [XProp],
    }

    impl ExtrasView<'_> {
        fn str(&self, s: XStr) -> String {
            let first = s.offset as usize;
            let end = first.saturating_add(s.length as usize);
            self.strings
                .get(first..end)
                .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
                .unwrap_or_default()
        }

        fn bytes(&self, b: XBytes) -> Vec<u8> {
            let end = b.offset.saturating_add(b.length);
            self.bytes
                .get(b.offset..end)
                .map(<[u8]>::to_vec)
                .unwrap_or_default()
        }

        fn props(&self, range: XPropRange) -> Result<Vec<extras::Prop>, ImportError> {
            slice_range(self.props, range.first, range.count, "props")?
                .iter()
                .map(|prop| {
                    Ok(extras::Prop {
                        name: self.str(prop.name),
                        kind: extras::PropType::from_code(prop.kind),
                        flags: extras::PropFlags(prop.flags),
                        value_int: prop.value_int,
                        value_real: prop.value_real,
                        value_str: self.str(prop.value_str),
                        value_blob: self.bytes(prop.value_blob),
                    })
                })
                .collect()
        }
    }

    fn slice_range<'a, T>(
        items: &'a [T],
        first: u32,
        count: u32,
        what: &str,
    ) -> Result<&'a [T], ImportError> {
        let first = first as usize;
        let end = first.saturating_add(count as usize);
        items.get(first..end).ok_or_else(|| {
            ImportError::LoadFailed(format!(
                "source properties: {what} range {first}..{end} exceeds {}",
                items.len()
            ))
        })
    }

    fn dmat(m: &[f64; 16]) -> Mat4 {
        DMat4::from_cols_array(m).as_mat4()
    }

    fn v3(v: [f64; 3]) -> Vec3 {
        Vec3::new(v[0] as f32, v[1] as f32, v[2] as f32)
    }

    fn marshal_extras(raw: &ReviewImportExtras) -> Result<SourceExtras, ImportError> {
        let view = ExtrasView {
            strings: checked_slice(raw.strings.cast::<u8>(), raw.string_count, "strings")?,
            bytes: checked_slice(raw.bytes, raw.byte_count, "bytes")?,
            props: checked_slice(raw.props, raw.prop_count, "props")?,
        };
        let lights = checked_slice(raw.lights, raw.light_count, "lights")?;
        let cameras = checked_slice(raw.cameras, raw.camera_count, "cameras")?;
        let lod_groups = checked_slice(raw.lod_groups, raw.lod_group_count, "lod_groups")?;
        let lod_levels = checked_slice(raw.lod_levels, raw.lod_level_count, "lod_levels")?;

        let scene = {
            let s = &raw.scene;
            extras::SceneExtras {
                creator: view.str(s.creator),
                filename: view.str(s.filename),
                original_file_path: view.str(s.original_file_path),
                version: s.version,
                ascii: s.ascii != 0,
                original_application: extras::Application {
                    vendor: view.str(s.original_vendor),
                    name: view.str(s.original_name),
                    version: view.str(s.original_version),
                },
                latest_application: extras::Application {
                    vendor: view.str(s.latest_vendor),
                    name: view.str(s.latest_name),
                    version: view.str(s.latest_version),
                },
                scene_props: view.props(s.scene_props)?,
                settings_props: view.props(s.settings_props)?,
                axes: [
                    extras::CoordinateAxis::from_code(s.axis_right),
                    extras::CoordinateAxis::from_code(s.axis_up),
                    extras::CoordinateAxis::from_code(s.axis_front),
                ],
                original_axis_up: extras::CoordinateAxis::from_code(s.original_axis_up),
                unit_meters: s.unit_meters,
                original_unit_meters: s.original_unit_meters,
                frames_per_second: s.frames_per_second,
                ambient_color: v3(s.ambient_color),
                default_camera: view.str(s.default_camera),
                time_mode: extras::TimeMode::from_code(s.time_mode),
                time_protocol: extras::TimeProtocol::from_code(s.time_protocol),
                snap_mode: extras::SnapMode::from_code(s.snap_mode),
            }
        };

        let nodes = checked_slice(raw.nodes, raw.node_count, "nodes")?
            .iter()
            .map(|node| {
                let kind = extras::AttributeKind::from_code(node.attribute_kind);
                let attribute = if kind == extras::AttributeKind::None {
                    None
                } else {
                    let typed = |what: &str| -> Result<usize, ImportError> {
                        usize::try_from(node.attribute_index).map_err(|_| {
                            ImportError::LoadFailed(format!(
                                "source properties: a {what} has no parameters"
                            ))
                        })
                    };
                    let light = if kind == extras::AttributeKind::Light {
                        let light = lights.get(typed("light")?).ok_or_else(|| {
                            ImportError::LoadFailed(
                                "source properties: light index out of range".to_owned(),
                            )
                        })?;
                        Some(extras::LightExtras {
                            color: v3(light.color),
                            intensity: light.intensity,
                            local_direction: v3(light.local_direction),
                            kind: extras::LightType::from_code(light.kind),
                            decay: extras::LightDecay::from_code(light.decay),
                            area_shape: extras::LightAreaShape::from_code(light.area_shape),
                            inner_angle: light.inner_angle,
                            outer_angle: light.outer_angle,
                            cast_light: light.cast_light != 0,
                            cast_shadows: light.cast_shadows != 0,
                        })
                    } else {
                        None
                    };
                    let camera = if kind == extras::AttributeKind::Camera {
                        let camera = cameras.get(typed("camera")?).ok_or_else(|| {
                            ImportError::LoadFailed(
                                "source properties: camera index out of range".to_owned(),
                            )
                        })?;
                        Some(extras::CameraExtras {
                            projection_mode: extras::ProjectionMode::from_code(
                                camera.projection_mode,
                            ),
                            resolution_is_pixels: camera.resolution_is_pixels != 0,
                            resolution: camera.resolution,
                            field_of_view_deg: camera.field_of_view_deg,
                            orthographic_extent: camera.orthographic_extent,
                            aspect_ratio: camera.aspect_ratio,
                            near_plane: camera.near_plane,
                            far_plane: camera.far_plane,
                            aspect_mode: extras::AspectMode::from_code(camera.aspect_mode),
                            aperture_mode: extras::ApertureMode::from_code(camera.aperture_mode),
                            gate_fit: extras::GateFit::from_code(camera.gate_fit),
                            aperture_format: extras::ApertureFormat::from_code(
                                camera.aperture_format,
                            ),
                            focal_length_mm: camera.focal_length_mm,
                            film_size_inch: camera.film_size_inch,
                            aperture_size_inch: camera.aperture_size_inch,
                            squeeze_ratio: camera.squeeze_ratio,
                        })
                    } else {
                        None
                    };
                    let lod_group = if kind == extras::AttributeKind::LodGroup {
                        let group = lod_groups.get(typed("LOD group")?).ok_or_else(|| {
                            ImportError::LoadFailed(
                                "source properties: LOD group index out of range".to_owned(),
                            )
                        })?;
                        Some(extras::LodGroupExtras {
                            relative_distances: group.relative_distances != 0,
                            ignore_parent_transform: group.ignore_parent_transform != 0,
                            use_distance_limit: group.use_distance_limit != 0,
                            distance_limit_min: group.distance_limit_min,
                            distance_limit_max: group.distance_limit_max,
                            levels: slice_range(
                                lod_levels,
                                group.level_first,
                                group.level_count,
                                "LOD levels",
                            )?
                            .iter()
                            .map(|level| extras::LodLevel {
                                distance: level.distance,
                                display: extras::LodDisplay::from_code(level.display),
                            })
                            .collect(),
                        })
                    } else {
                        None
                    };
                    Some(extras::AttributeExtras {
                        kind,
                        name: view.str(node.attribute_name),
                        props: view.props(node.attribute_props)?,
                        light,
                        camera,
                        lod_group,
                    })
                };
                Ok(extras::NodeExtras {
                    props: view.props(node.props)?,
                    rotation_order: extras::RotationOrder::from_code(node.rotation_order),
                    inherit_mode: extras::InheritMode::from_code(node.inherit_mode),
                    original_inherit_mode: extras::InheritMode::from_code(
                        node.original_inherit_mode,
                    ),
                    geometry_to_node: dmat(&node.geometry_to_node),
                    synthetic: extras::Synthetic::from_code(node.synthetic),
                    visible: node.visible != 0,
                    attribute,
                })
            })
            .collect::<Result<Vec<_>, ImportError>>()?;

        let material_textures = checked_slice(
            raw.material_textures,
            raw.material_texture_count,
            "material_textures",
        )?;
        let materials = checked_slice(raw.materials, raw.material_count, "materials")?
            .iter()
            .map(|material| {
                Ok(extras::MaterialExtras {
                    shader_type: extras::ShaderType::from_code(material.shader_type),
                    shading_model: view.str(material.shading_model),
                    props: view.props(material.props)?,
                    textures: slice_range(
                        material_textures,
                        material.texture_first,
                        material.texture_count,
                        "material textures",
                    )?
                    .iter()
                    .map(|texture| extras::MaterialTexture {
                        material_prop: view.str(texture.material_prop),
                        shader_prop: view.str(texture.shader_prop),
                        texture: texture.texture,
                    })
                    .collect(),
                })
            })
            .collect::<Result<Vec<_>, ImportError>>()?;

        let texture_layers = checked_slice(
            raw.texture_layers,
            raw.texture_layer_count,
            "texture_layers",
        )?;
        let textures = checked_slice(raw.textures, raw.texture_count, "textures")?
            .iter()
            .map(|texture| {
                Ok(extras::TextureExtras {
                    name: view.str(texture.name),
                    kind: extras::TextureKind::from_code(texture.kind),
                    filename: view.str(texture.filename),
                    absolute_filename: view.str(texture.absolute_filename),
                    relative_filename: view.str(texture.relative_filename),
                    uv_set: view.str(texture.uv_set),
                    wrap_u: extras::WrapMode::from_code(texture.wrap_u),
                    wrap_v: extras::WrapMode::from_code(texture.wrap_v),
                    uv_transform: (texture.has_uv_transform != 0).then(|| {
                        (
                            v3(texture.uv_translation),
                            Quat::from_xyzw(
                                texture.uv_rotation[0] as f32,
                                texture.uv_rotation[1] as f32,
                                texture.uv_rotation[2] as f32,
                                texture.uv_rotation[3] as f32,
                            )
                            .normalize(),
                            v3(texture.uv_scale),
                        )
                    }),
                    content: view.bytes(texture.content),
                    video: u32::try_from(texture.video).ok(),
                    layers: slice_range(
                        texture_layers,
                        texture.layer_first,
                        texture.layer_count,
                        "texture layers",
                    )?
                    .iter()
                    .map(|layer| extras::TextureLayer {
                        texture: layer.texture,
                        blend_mode: extras::BlendMode::from_code(layer.blend_mode),
                        alpha: layer.alpha,
                    })
                    .collect(),
                    props: view.props(texture.props)?,
                })
            })
            .collect::<Result<Vec<_>, ImportError>>()?;

        let videos = checked_slice(raw.videos, raw.video_count, "videos")?
            .iter()
            .map(|video| {
                Ok(extras::VideoExtras {
                    name: view.str(video.name),
                    filename: view.str(video.filename),
                    absolute_filename: view.str(video.absolute_filename),
                    relative_filename: view.str(video.relative_filename),
                    content: view.bytes(video.content),
                    props: view.props(video.props)?,
                })
            })
            .collect::<Result<Vec<_>, ImportError>>()?;

        let color_sets = checked_slice(raw.color_sets, raw.color_set_count, "color_sets")?;
        let color_values = checked_slice(raw.color_values, raw.color_value_count, "color_values")?;
        let edges = checked_slice(raw.edges, raw.edge_count.saturating_mul(2), "edges")?;
        let edge_smoothing = checked_slice(raw.edge_smoothing, raw.edge_count, "edge_smoothing")?;
        let edge_crease = checked_slice(raw.edge_crease, raw.edge_count, "edge_crease")?;
        let edge_visibility =
            checked_slice(raw.edge_visibility, raw.edge_count, "edge_visibility")?;
        let face_smoothing = checked_slice(raw.face_smoothing, raw.face_count, "face_smoothing")?;
        let face_hole = checked_slice(raw.face_hole, raw.face_count, "face_hole")?;
        let face_group = checked_slice(raw.face_group, raw.face_count, "face_group")?;
        let vertex_crease =
            checked_slice(raw.vertex_crease, raw.vertex_crease_count, "vertex_crease")?;
        let face_groups = checked_slice(raw.face_groups, raw.face_group_count, "face_groups")?;
        let extra_skins = checked_slice(raw.extra_skins, raw.extra_skin_count, "extra_skins")?;
        let extra_clusters = checked_slice(
            raw.extra_clusters,
            raw.extra_cluster_count,
            "extra_clusters",
        )?;
        let extra_skin_offsets = checked_slice(
            raw.extra_skin_offsets,
            raw.extra_skin_offset_count,
            "extra_skin_offsets",
        )?;
        let extra_influences = checked_slice(
            raw.extra_influences,
            raw.extra_influence_count,
            "extra_influences",
        )?;
        let dq_weights = checked_slice(raw.dq_weights, raw.dq_weight_count, "dq_weights")?;

        let meshes = checked_slice(raw.meshes, raw.mesh_count, "meshes")?
            .iter()
            .map(|mesh| {
                let corners = mesh.corner_count as usize;
                // Every face layer is `face_count` long, whichever the file carried.
                slice_range(
                    face_smoothing,
                    mesh.face_first,
                    mesh.face_count,
                    "face layers",
                )?;
                let logical = mesh.logical_count as usize;
                let color_sets = slice_range(
                    color_sets,
                    mesh.color_set_first,
                    mesh.color_set_count,
                    "color sets",
                )?
                .iter()
                .map(|set| {
                    let values = if set.value_first == u32::MAX {
                        Vec::new()
                    } else {
                        let count = corners.checked_mul(4).ok_or_else(|| {
                            ImportError::LoadFailed(
                                "source properties: color set overflows".to_owned(),
                            )
                        })?;
                        let first = set.value_first as usize;
                        color_values
                            .get(first..first.saturating_add(count))
                            .ok_or_else(|| {
                                ImportError::LoadFailed(
                                    "source properties: color set values out of range".to_owned(),
                                )
                            })?
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(|c| Vec4::new(c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32))
                            .collect()
                    };
                    Ok(extras::ColorSetExtras {
                        name: view.str(set.name),
                        index: set.index,
                        values,
                    })
                })
                .collect::<Result<Vec<_>, ImportError>>()?;
                let edge_range = |items: &[u8]| -> Result<Vec<bool>, ImportError> {
                    Ok(
                        slice_range(items, mesh.edge_first, mesh.edge_count, "edge layer")?
                            .iter()
                            .map(|&v| v != 0)
                            .collect(),
                    )
                };
                let face_range = |items: &[u8]| -> Result<Vec<bool>, ImportError> {
                    Ok(
                        slice_range(items, mesh.face_first, mesh.face_count, "face layer")?
                            .iter()
                            .map(|&v| v != 0)
                            .collect(),
                    )
                };
                let pair_first = mesh.edge_first.saturating_mul(2);
                let pair_count = mesh.edge_count.saturating_mul(2);
                let extra_skins = slice_range(
                    extra_skins,
                    mesh.extra_skin_first,
                    mesh.extra_skin_count,
                    "skin layers",
                )?
                .iter()
                .map(|skin| {
                    let offsets = slice_range(
                        extra_skin_offsets,
                        skin.offset_first,
                        (logical as u32).saturating_add(1),
                        "skin layer rows",
                    )?
                    .to_vec();
                    Ok(extras::SkinLayerExtras {
                        method: skinning_method_from_code(skin.method),
                        max_weights_per_vertex: skin.max_weights_per_vertex,
                        clusters: slice_range(
                            extra_clusters,
                            skin.cluster_first,
                            skin.cluster_count,
                            "skin layer clusters",
                        )?
                        .iter()
                        .map(|cluster| extras::ExtraCluster {
                            bone: cluster.bone,
                            name: view.str(cluster.name),
                            mesh_node_to_bone: dmat(&cluster.mesh_node_to_bone),
                            bind_to_world: dmat(&cluster.bind_to_world),
                        })
                        .collect(),
                        offsets,
                        influences: slice_range(
                            extra_influences,
                            skin.influence_first,
                            skin.influence_count,
                            "skin layer influences",
                        )?
                        .iter()
                        .map(|influence| (influence.cluster, influence.weight))
                        .collect(),
                    })
                })
                .collect::<Result<Vec<_>, ImportError>>()?;
                Ok(extras::MeshExtras {
                    node: mesh.node,
                    name: view.str(mesh.name),
                    props: view.props(mesh.props)?,
                    corner_first: mesh.corner_first,
                    corner_count: mesh.corner_count,
                    logical_first: mesh.logical_first,
                    logical_count: mesh.logical_count,
                    face_first: mesh.face_first,
                    face_count: mesh.face_count,
                    tangents_authored: mesh.tangents_authored != 0,
                    reversed_winding: mesh.reversed_winding != 0,
                    color_sets,
                    edges: slice_range(edges, pair_first, pair_count, "edges")?
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|pair| [pair[0], pair[1]])
                        .collect(),
                    edge_smoothing: if mesh.has_edge_smoothing != 0 {
                        edge_range(edge_smoothing)?
                    } else {
                        Vec::new()
                    },
                    edge_crease: if mesh.has_edge_crease != 0 {
                        slice_range(edge_crease, mesh.edge_first, mesh.edge_count, "edge crease")?
                            .iter()
                            .map(|&v| v as f32)
                            .collect()
                    } else {
                        Vec::new()
                    },
                    edge_visibility: if mesh.has_edge_visibility != 0 {
                        edge_range(edge_visibility)?
                    } else {
                        Vec::new()
                    },
                    face_smoothing: if mesh.has_face_smoothing != 0 {
                        face_range(face_smoothing)?
                    } else {
                        Vec::new()
                    },
                    face_hole: if mesh.has_face_hole != 0 {
                        face_range(face_hole)?
                    } else {
                        Vec::new()
                    },
                    face_group: if mesh.has_face_group != 0 {
                        slice_range(face_group, mesh.face_first, mesh.face_count, "face group")?
                            .to_vec()
                    } else {
                        Vec::new()
                    },
                    face_groups: slice_range(
                        face_groups,
                        mesh.face_group_first,
                        mesh.face_group_count,
                        "face groups",
                    )?
                    .iter()
                    .map(|group| extras::FaceGroup {
                        id: group.id,
                        name: view.str(group.name),
                    })
                    .collect(),
                    vertex_crease: if mesh.has_vertex_crease != 0 {
                        slice_range(
                            vertex_crease,
                            mesh.logical_first,
                            mesh.logical_count,
                            "vertex crease",
                        )?
                        .iter()
                        .map(|&v| v as f32)
                        .collect()
                    } else {
                        Vec::new()
                    },
                    subdivision: extras::SubdivisionExtras {
                        preview_levels: mesh.subdivision_preview_levels,
                        render_levels: mesh.subdivision_render_levels,
                        display_mode: extras::SubdivisionDisplayMode::from_code(
                            mesh.subdivision_display_mode,
                        ),
                        boundary: extras::SubdivisionBoundary::from_code(mesh.subdivision_boundary),
                        uv_boundary: extras::SubdivisionBoundary::from_code(
                            mesh.subdivision_uv_boundary,
                        ),
                    },
                    extra_skins,
                    dq_weights: slice_range(
                        dq_weights,
                        mesh.dq_first,
                        mesh.dq_count,
                        "dual-quaternion weights",
                    )?
                    .iter()
                    .map(|w| (w.logical_vertex, w.weight as f32))
                    .collect(),
                })
            })
            .collect::<Result<Vec<_>, ImportError>>()?;

        let pose_entries = checked_slice(raw.pose_entries, raw.pose_entry_count, "pose_entries")?;
        let poses = checked_slice(raw.poses, raw.pose_count, "poses")?
            .iter()
            .map(|pose| {
                Ok(extras::PoseExtras {
                    name: view.str(pose.name),
                    is_bind_pose: pose.is_bind_pose != 0,
                    entries: slice_range(
                        pose_entries,
                        pose.entry_first,
                        pose.entry_count,
                        "pose entries",
                    )?
                    .iter()
                    .map(|entry| extras::PoseEntry {
                        node: entry.node,
                        bone_to_world: dmat(&entry.bone_to_world),
                    })
                    .collect(),
                    props: view.props(pose.props)?,
                })
            })
            .collect::<Result<Vec<_>, ImportError>>()?;

        let layer_nodes = checked_slice(raw.layer_nodes, raw.layer_node_count, "layer_nodes")?;
        let display_layers = checked_slice(
            raw.display_layers,
            raw.display_layer_count,
            "display_layers",
        )?
        .iter()
        .map(|layer| {
            Ok(extras::DisplayLayerExtras {
                name: view.str(layer.name),
                visible: layer.visible != 0,
                frozen: layer.frozen != 0,
                ui_color: v3(layer.ui_color),
                nodes: slice_range(
                    layer_nodes,
                    layer.node_first,
                    layer.node_count,
                    "display layer nodes",
                )?
                .to_vec(),
                props: view.props(layer.props)?,
            })
        })
        .collect::<Result<Vec<_>, ImportError>>()?;

        let selection_nodes = checked_slice(
            raw.selection_nodes,
            raw.selection_node_count,
            "selection_nodes",
        )?;
        let selection_indices = checked_slice(
            raw.selection_indices,
            raw.selection_index_count,
            "selection_indices",
        )?;
        let selection_sets = checked_slice(
            raw.selection_sets,
            raw.selection_set_count,
            "selection_sets",
        )?
        .iter()
        .map(|set| {
            Ok(extras::SelectionSetExtras {
                name: view.str(set.name),
                props: view.props(set.props)?,
                nodes: slice_range(
                    selection_nodes,
                    set.node_first,
                    set.node_count,
                    "selection nodes",
                )?
                .iter()
                .map(|node| {
                    Ok(extras::SelectionNodeExtras {
                        node: u32::try_from(node.node).ok(),
                        include_node: node.include_node != 0,
                        vertices: slice_range(
                            selection_indices,
                            node.vertex_first,
                            node.vertex_count,
                            "selection vertices",
                        )?
                        .to_vec(),
                        edges: slice_range(
                            selection_indices,
                            node.edge_first,
                            node.edge_count,
                            "selection edges",
                        )?
                        .to_vec(),
                        faces: slice_range(
                            selection_indices,
                            node.face_first,
                            node.face_count,
                            "selection faces",
                        )?
                        .to_vec(),
                    })
                })
                .collect::<Result<Vec<_>, ImportError>>()?,
            })
        })
        .collect::<Result<Vec<_>, ImportError>>()?;

        let anim_keys = checked_slice(raw.anim_keys, raw.anim_key_count, "anim_keys")?;
        let anim_curves = checked_slice(raw.anim_curves, raw.anim_curve_count, "anim_curves")?;
        let anim_props = checked_slice(raw.anim_props, raw.anim_prop_count, "anim_props")?;
        let curve_of = |index: i32| -> Result<Option<extras::Curve>, ImportError> {
            let Ok(index) = usize::try_from(index) else {
                return Ok(None);
            };
            let curve = anim_curves.get(index).ok_or_else(|| {
                ImportError::LoadFailed("source properties: curve index out of range".to_owned())
            })?;
            Ok(Some(extras::Curve {
                keys: slice_range(anim_keys, curve.key_first, curve.key_count, "keys")?
                    .iter()
                    .map(|key| extras::RawKey {
                        time: key.time,
                        value: key.value,
                        interpolation: extras::Interpolation::from_code(key.interpolation),
                        left: (key.left_dx, key.left_dy),
                        right: (key.right_dx, key.right_dy),
                    })
                    .collect(),
                pre: extras::Extrapolation {
                    mode: extras::ExtrapolationMode::from_code(curve.pre_mode),
                    repeat_count: curve.pre_repeat,
                },
                post: extras::Extrapolation {
                    mode: extras::ExtrapolationMode::from_code(curve.post_mode),
                    repeat_count: curve.post_repeat,
                },
            }))
        };
        let anim_layers = checked_slice(raw.anim_layers, raw.anim_layer_count, "anim_layers")?
            .iter()
            .map(|layer| {
                Ok(extras::LayerCurves {
                    name: view.str(layer.name),
                    weight: layer.weight,
                    weight_is_animated: layer.weight_is_animated != 0,
                    blended: layer.blended != 0,
                    additive: layer.additive != 0,
                    compose_rotation: layer.compose_rotation != 0,
                    compose_scale: layer.compose_scale != 0,
                    props: view.props(layer.props)?,
                    anim: slice_range(
                        anim_props,
                        layer.anim_prop_first,
                        layer.anim_prop_count,
                        "animated properties",
                    )?
                    .iter()
                    .map(|prop| {
                        let target = match prop.target_kind {
                            1 => extras::ElementRef::Node(prop.target),
                            2 => extras::ElementRef::NodeAttribute(prop.target),
                            3 => extras::ElementRef::Material(prop.target),
                            4 => extras::ElementRef::Texture(prop.target),
                            5 => extras::ElementRef::Video(prop.target),
                            6 => extras::ElementRef::BlendChannel(prop.target),
                            7 => extras::ElementRef::DisplayLayer(prop.target),
                            8 => extras::ElementRef::AnimLayer(prop.target),
                            _ => extras::ElementRef::Unmapped {
                                element_type: prop.element_type,
                                name: view.str(prop.element_name),
                            },
                        };
                        Ok(extras::AnimPropCurves {
                            target,
                            prop_name: view.str(prop.prop_name),
                            default: v3(prop.default_value),
                            curves: [
                                curve_of(prop.curves[0])?,
                                curve_of(prop.curves[1])?,
                                curve_of(prop.curves[2])?,
                            ],
                        })
                    })
                    .collect::<Result<Vec<_>, ImportError>>()?,
                })
            })
            .collect::<Result<Vec<_>, ImportError>>()?;

        let stack_layers = checked_slice(raw.stack_layers, raw.stack_layer_count, "stack_layers")?;
        let animations = checked_slice(raw.anim_stacks, raw.anim_stack_count, "anim_stacks")?
            .iter()
            .map(|stack| {
                Ok(extras::ClipCurves {
                    name: view.str(stack.name),
                    props: view.props(stack.props)?,
                    clip: u32::try_from(stack.clip).ok(),
                    time_begin: stack.time_begin,
                    time_end: stack.time_end,
                    layers: slice_range(
                        stack_layers,
                        stack.layer_first,
                        stack.layer_count,
                        "stack layers",
                    )?
                    .to_vec(),
                })
            })
            .collect::<Result<Vec<_>, ImportError>>()?;

        Ok(SourceExtras {
            scene,
            nodes,
            materials,
            textures,
            videos,
            meshes,
            poses,
            display_layers,
            selection_sets,
            anim_layers,
            animations,
        })
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
            influence_cluster: &[u32],
            clusters: &[ReviewImportSkinCluster],
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
            scene.skin_influence_cluster = influence_cluster.as_ptr() as *mut u32;
            scene.skin_clusters = clusters.as_ptr() as *mut ReviewImportSkinCluster;
            scene.skin_cluster_count = clusters.len();
            scene
        }

        fn identity_cluster(bone: u32) -> ReviewImportSkinCluster {
            ReviewImportSkinCluster {
                bone,
                mesh_node: 0,
                world_to_bone_bind: Mat4::IDENTITY.to_cols_array(),
                mesh_node_to_bone: DMat4::IDENTITY.to_cols_array(),
                bind_to_world: DMat4::IDENTITY.to_cols_array(),
                name: std::ptr::null_mut(),
            }
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
            let clusters = [identity_cluster(1), identity_cluster(2)];
            let scene = skin_scene(
                &[0, 0, 1],
                &[0, 2, 3],
                &[1, 2, 2],
                &[0.75, 0.25, 1.0],
                &[0, 1, 1],
                &clusters,
            );
            let corner_map = marshal_corner_map(&scene).expect("valid corner map");
            let skin = marshal_skin(&scene)
                .expect("valid skin")
                .expect("a skinned scene marshals to Some");
            assert_eq!(corner_map, vec![0, 0, 1]);
            assert_eq!(skin.offsets, vec![0, 2, 3]);
            assert_eq!(skin.bones, vec![1, 2, 2]);
            assert_eq!(skin.weights, vec![0.75, 0.25, 1.0]);
            assert_eq!(skin.influence_cluster, vec![0, 1, 1]);
            assert_eq!(skin.clusters.len(), 2);
            assert_eq!(skin.clusters[1].bone, 2);
            assert_eq!(skin.clusters[1].world_to_bone_bind, Mat4::IDENTITY);
            assert_eq!(skin.influence_range(0), 0..2);
            assert_eq!(skin.validate(2, 3), Ok(()));
        }

        #[test]
        fn marshal_skin_rejects_a_null_array_with_a_nonzero_count() {
            // A count without its pointer must be a clean error naming the field,
            // not a `from_raw_parts` on null.
            let clusters = [identity_cluster(1)];
            let mut scene = skin_scene(&[0], &[0, 1], &[1], &[1.0], &[0], &clusters);
            scene.skin_bones = std::ptr::null_mut();
            assert!(matches!(
                marshal_skin(&scene),
                Err(ImportError::LoadFailed(message)) if message.contains("skin_bones")
            ));
            let mut scene = skin_scene(&[0], &[0, 1], &[1], &[1.0], &[0], &clusters);
            scene.skin_influence_cluster = std::ptr::null_mut();
            assert!(matches!(
                marshal_skin(&scene),
                Err(ImportError::LoadFailed(message)) if message.contains("skin_influence_cluster")
            ));
        }

        #[test]
        fn marshal_morph_sorts_entries_into_logical_rows() {
            // Two shapes over three logical vertices, emitted out of vertex order.
            let keyframes = [
                ReviewImportMorphKeyframe {
                    shape: 0,
                    target_weight: 1.0,
                },
                ReviewImportMorphKeyframe {
                    shape: 1,
                    target_weight: 1.0,
                },
            ];
            let channels = [
                ReviewImportMorphChannel {
                    name: std::ptr::null_mut(),
                    mesh_node: 0,
                    rest_weight: 0.25,
                    keyframe_first: 0,
                    keyframe_count: 1,
                },
                ReviewImportMorphChannel {
                    name: std::ptr::null_mut(),
                    mesh_node: 0,
                    rest_weight: 0.0,
                    keyframe_first: 1,
                    keyframe_count: 1,
                },
            ];
            let shapes = [
                ReviewImportMorphShape {
                    name: std::ptr::null_mut(),
                },
                ReviewImportMorphShape {
                    name: std::ptr::null_mut(),
                },
            ];
            let entry = |logical_vertex: u32, shape: u32, x: f32| ReviewImportMorphEntry {
                logical_vertex,
                shape,
                position: [x, 0.0, 0.0],
                normal: [0.0; 3],
            };
            let entries = [entry(2, 0, 2.0), entry(0, 1, 0.5), entry(2, 1, 2.5)];
            let mut scene: ReviewImportScene = unsafe { std::mem::zeroed() };
            scene.morph_channels = channels.as_ptr() as *mut ReviewImportMorphChannel;
            scene.morph_channel_count = channels.len();
            scene.morph_keyframes = keyframes.as_ptr() as *mut ReviewImportMorphKeyframe;
            scene.morph_keyframe_count = keyframes.len();
            scene.morph_shapes = shapes.as_ptr() as *mut ReviewImportMorphShape;
            scene.morph_shape_count = shapes.len();
            scene.morph_entries = entries.as_ptr() as *mut ReviewImportMorphEntry;
            scene.morph_entry_count = entries.len();

            let morph = marshal_morph(&scene, 3)
                .expect("valid morph")
                .expect("channels marshal to Some");
            assert_eq!(morph.channels.len(), 2);
            assert_eq!(morph.channels[0].rest_weight, 0.25);
            assert_eq!(morph.channels[1].keyframes[0].shape, 1);
            assert_eq!(morph.offsets, vec![0, 1, 1, 3]);
            assert_eq!(morph.shape, vec![1, 0, 1]);
            assert_eq!(morph.position[0].x, 0.5);
            assert_eq!(morph.position[1].x, 2.0);
            assert_eq!(morph.position[2].x, 2.5);
            assert_eq!(morph.validate(3, 1), Ok(()));

            // An entry past the logical range is a bridge drift, not a panic.
            let bad = [entry(7, 0, 1.0)];
            scene.morph_entries = bad.as_ptr() as *mut ReviewImportMorphEntry;
            scene.morph_entry_count = bad.len();
            assert!(matches!(
                marshal_morph(&scene, 3),
                Err(ImportError::LoadFailed(message)) if message.contains("source vertex 7 of 3")
            ));
        }

        #[test]
        fn marshal_animations_reads_tracks_and_key_ranges() {
            let vec3_keys = [
                ReviewImportVec3Key {
                    time: 0.0,
                    value: [0.0, 1.0, 0.0],
                },
                ReviewImportVec3Key {
                    time: 1.0,
                    value: [2.0, 1.0, 0.0],
                },
            ];
            let quat_keys = [ReviewImportQuatKey {
                time: 0.5,
                value: [0.0, 0.0, 0.0, 1.0],
            }];
            let tracks = [ReviewImportNodeTrack {
                node: 3,
                translation_first: 0,
                translation_count: 2,
                rotation_first: 0,
                rotation_count: 1,
                scale_first: 2,
                scale_count: 0,
            }];
            let scalar_keys = [ReviewImportScalarKey {
                time: 0.0,
                value: 0.5,
            }];
            let morph_tracks = [ReviewImportMorphTrack {
                channel: 1,
                first: 0,
                count: 1,
            }];
            let stacks = [ReviewImportAnimStack {
                name: std::ptr::null_mut(),
                time_begin: 0.0,
                time_end: 1.0,
                node_track_first: 0,
                node_track_count: 1,
                morph_track_first: 0,
                morph_track_count: 1,
            }];
            let mut scene: ReviewImportScene = unsafe { std::mem::zeroed() };
            scene.anim_stacks = stacks.as_ptr() as *mut ReviewImportAnimStack;
            scene.anim_stack_count = stacks.len();
            scene.anim_node_tracks = tracks.as_ptr() as *mut ReviewImportNodeTrack;
            scene.anim_node_track_count = tracks.len();
            scene.anim_vec3_keys = vec3_keys.as_ptr() as *mut ReviewImportVec3Key;
            scene.anim_vec3_key_count = vec3_keys.len();
            scene.anim_quat_keys = quat_keys.as_ptr() as *mut ReviewImportQuatKey;
            scene.anim_quat_key_count = quat_keys.len();
            scene.anim_morph_tracks = morph_tracks.as_ptr() as *mut ReviewImportMorphTrack;
            scene.anim_morph_track_count = morph_tracks.len();
            scene.anim_scalar_keys = scalar_keys.as_ptr() as *mut ReviewImportScalarKey;
            scene.anim_scalar_key_count = scalar_keys.len();

            let clips = marshal_animations(&scene).expect("valid animation");
            assert_eq!(clips.len(), 1);
            let clip = &clips[0];
            assert_eq!(clip.time_end, 1.0);
            assert_eq!(clip.tracks.len(), 1);
            assert_eq!(clip.tracks[0].node, 3);
            assert_eq!(clip.tracks[0].translation.len(), 2);
            assert_eq!(
                clip.tracks[0].translation[1].value,
                Vec3::new(2.0, 1.0, 0.0)
            );
            assert_eq!(clip.tracks[0].rotation.len(), 1);
            assert!(clip.tracks[0].scale.is_empty());
            assert_eq!(clip.morph_tracks[0].channel, 1);
            assert_eq!(clip.morph_tracks[0].keys[0].value, 0.5);
            assert_eq!(clip.validate(4, 2), Ok(()));

            // A track range past the key table is a bridge drift.
            let bad = [ReviewImportNodeTrack {
                translation_count: 5,
                ..tracks[0]
            }];
            scene.anim_node_tracks = bad.as_ptr() as *mut ReviewImportNodeTrack;
            assert!(matches!(
                marshal_animations(&scene),
                Err(ImportError::LoadFailed(message)) if message.contains("vec3 key range")
            ));

            // No stacks at all is simply no clips.
            let empty: ReviewImportScene = unsafe { std::mem::zeroed() };
            assert!(marshal_animations(&empty).expect("no animation").is_empty());
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
                model_from_bridge_scene(Path::new("mesh.fbx"), &scene, &|_| {}),
                Err(ImportError::LoadFailed(message))
                    if message.contains("index 3") && message.contains("3 vertices")
            ));
        }

        #[test]
        fn indices_within_the_vertex_buffer_load() {
            let vertices = blank_vertices(3);
            let scene = geometry_scene(&vertices, &[0, 1, 2]);
            let model = model_from_bridge_scene(Path::new("mesh.fbx"), &scene, &|_| {})
                .expect("a whole triangle");
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
            skin.validate(model.stats.vertex_count, model.nodes.len()),
            Ok(())
        );
        assert_eq!(model.corner_to_logical.len(), model.vertices.len());
        assert_eq!(skin.offsets.len(), model.stats.vertex_count + 1);
        assert!(
            !skin.clusters.is_empty(),
            "a skinned mesh binds at least one cluster"
        );
        assert!(
            !skin.deformers.is_empty(),
            "every skinned mesh node reports its deformer"
        );
        for deformer in &skin.deformers {
            assert!((deformer.mesh_node as usize) < model.nodes.len());
            assert!(deformer.max_weights_per_vertex > 0);
        }

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
        for &logical in &model.corner_to_logical {
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
        assert!(model.animations.is_empty(), "the cube carries no clips");
        assert_eq!(model.stats.clip_count, 0);
    }

    /// The rest local transforms the bridge captures must compose back to the
    /// world transforms it also captures — the check that a pose recomposed
    /// from them (and therefore every animated frame) lands where the file says.
    fn assert_rest_locals_recompose(model: &review_model::ModelData) {
        let ctx = review_model::AnimContext::new(model);
        assert!(
            review_model::anim::rest_locals_recompose(model, &ctx, 1e-3),
            "composing the rest local transforms must reproduce node_to_world"
        );
    }

    /// The skinned, multi-clip fixture: every stack imports as a clip with a
    /// playable range, every track names a real node, and the rest pose's
    /// skinning is consistent with the file's world transforms.
    #[test]
    fn import_carries_animation_clips() {
        let path = fixture("AN_ZombiedogLocomotion.fbx");
        if !path.exists() {
            eprintln!("skipping: {} is not present", path.display());
            return;
        }
        let model = load_model(&path).expect("the locomotion fixture must import");

        assert!(model.skin.is_some(), "the dog is skinned");
        assert!(
            model.animations.len() > 1,
            "the fixture carries several clips, got {}",
            model.animations.len()
        );
        assert_eq!(model.stats.clip_count, model.animations.len());
        assert!(model.frame_rate > 0.0, "the file declares a frame rate");
        for clip in &model.animations {
            assert!(!clip.name.is_empty(), "every clip is named");
            assert!(
                clip.time_end > clip.time_begin,
                "clip '{}' has an empty range {}..{}",
                clip.name,
                clip.time_begin,
                clip.time_end
            );
            assert!(
                !clip.tracks.is_empty(),
                "clip '{}' animates nothing",
                clip.name
            );
            for track in &clip.tracks {
                assert!((track.node as usize) < model.nodes.len());
            }
        }
        assert!(model.bounds.is_some());
        assert_rest_locals_recompose(&model);
        assert_eq!(model.validate_deform(), Ok(()));
    }

    /// The unskinned, rigidly animated fixture: node tracks and no skin, and at
    /// least one node ends the clip somewhere other than its rest transform.
    #[test]
    fn rigid_clip_moves_nodes() {
        let path = fixture("SM_Wall_Break_4x3m.fbx");
        if !path.exists() {
            eprintln!("skipping: {} is not present", path.display());
            return;
        }
        let model = load_model(&path).expect("the wall-break fixture must import");

        assert!(model.skin.is_none(), "the wall pieces are not skinned");
        assert!(!model.animations.is_empty(), "the fixture carries a clip");
        assert_rest_locals_recompose(&model);

        let ctx = review_model::AnimContext::new(&model);
        let mut pose = review_model::Pose::new(&model);
        let clip = &model.animations[0];
        review_model::anim::evaluate_pose(&model, &ctx, Some(clip), clip.time_end, &mut pose);
        let moved = model
            .nodes
            .iter()
            .zip(&pose.world)
            .any(|(node, world)| !world.abs_diff_eq(node.transform, 1e-4));
        assert!(moved, "the clip's last frame must move at least one node");
    }

    /// [`measure_clip_bounds`] — the deferred measurement `app` runs once the model
    /// is on screen — must give exactly what walking the whole mesh in every frame
    /// gives. `clip_bounds` narrows that walk to the corners the clip can actually
    /// move, the difference between a 5-second load and a 100-second one on a large
    /// scene, so this pins the narrowed answer to the definition it replaced, over
    /// real files rather than a synthetic one: a skinned character (the skin arm of
    /// the predicate), a rigid destructible (the node arm), and a locomotion clip.
    #[test]
    fn measured_clip_bounds_match_walking_every_corner() {
        for name in [
            "SK_Player_01.fbx",
            "AN_ZombiedogLocomotion.fbx",
            "SM_Wall_Break_4x3m.fbx",
        ] {
            let path = fixture(name);
            if !path.exists() {
                eprintln!("skipping: {} is not present", path.display());
                continue;
            }
            let model = load_model(&path).unwrap_or_else(|error| panic!("{name}: {error}"));
            if model.animations.is_empty() {
                continue;
            }

            let measured = crate::measure_clip_bounds(&model);
            assert_eq!(
                measured.len(),
                model.animations.len(),
                "{name}: one envelope per clip"
            );

            let ctx = review_model::AnimContext::new(&model);
            let fps = model.frame_rate_or_default();
            let mut pose = review_model::Pose::new(&model);
            let mut deform = review_model::DeformPose::default();
            for (clip, measured) in model.animations.iter().zip(&measured) {
                let mut expected = review_model::Bounds::EMPTY;
                for frame in 0..clip.frame_count(fps) {
                    review_model::anim::evaluate_pose(
                        &model,
                        &ctx,
                        Some(clip),
                        clip.frame_time(frame, fps),
                        &mut pose,
                    );
                    review_model::anim::build_palette(&model, &ctx, &pose, &mut deform);
                    if let Some(frame_bounds) =
                        review_model::anim::posed_bounds(&model, &ctx, &deform)
                    {
                        expected.include_point(frame_bounds.min);
                        expected.include_point(frame_bounds.max);
                    }
                }
                let measured = measured.expect("an animated clip has an envelope");
                assert_eq!(measured.min, expected.min, "{name} / {} min", clip.name);
                assert_eq!(measured.max, expected.max, "{name} / {} max", clip.name);
            }
        }
    }

    /// The staged import publishes a *drawable* model and leaves the two costly
    /// measurements to the caller: whatever the viewport needs on the first frame
    /// must already be there, and the deferred pair must not be.
    #[test]
    fn a_staged_import_is_drawable_but_unmeasured() {
        let path = fixture("SM_Wall_Break_4x3m.fbx");
        if !path.exists() {
            eprintln!("skipping: {} is not present", path.display());
            return;
        }
        let staged =
            crate::load_model_with_progress(&path, &|_| {}).expect("the fixture must import");

        // Everything the first frame draws with.
        assert!(!staged.vertices.is_empty(), "geometry");
        assert!(!staged.indices.is_empty(), "indices");
        assert!(!staged.nodes.is_empty(), "scene graph");
        assert!(staged.stats.draw_count > 0, "draw groups");
        assert!(staged.bounds.is_some(), "the camera frames on the bounds");
        assert!(!staged.has_degenerate_tangents(), "tangents");

        // …and neither of the two measurements that cost the most.
        assert_eq!(
            staged.stats.gpu_vertex_count, 0,
            "GPU Verts is measured after the model is up"
        );

        // The complete entry point makes them, so a test or batch caller still
        // gets a fully measured model.
        let complete = load_model(&path).expect("the fixture must import");
        assert_eq!(
            complete.stats.gpu_vertex_count,
            complete.count_gpu_vertices()
        );
        assert!(complete.stats.gpu_vertex_count > 0);
    }

    /// Every fixture's source-property capture must describe the model it came
    /// with: `load_model_full` runs the funnel guard, so an index that drifted
    /// from the geometry fails here rather than in an export.
    #[test]
    fn every_fixture_captures_valid_extras() {
        let dir = fixture("");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            eprintln!("skipping: {} is not present", dir.display());
            return;
        };
        let mut checked = 0;
        // `RVO_EXTRAS_FIXTURE=<name>` narrows the walk to one file, for
        // isolating a fixture that misbehaves.
        let only = std::env::var("RVO_EXTRAS_FIXTURE").ok();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("fbx"))
            {
                continue;
            }
            if let Some(only) = &only
                && path.file_name().is_none_or(|name| name != only.as_str())
            {
                continue;
            }
            let (model, extras) = crate::load_model_full(&path)
                .unwrap_or_else(|error| panic!("{} must import: {error}", path.display()));
            let extras = extras.unwrap_or_else(|| panic!("{} captured no extras", path.display()));
            // Beyond the guard: the parts cover the model, each node's props were
            // read, and the clip map points at the clips the model has.
            assert_eq!(extras.nodes.len(), model.nodes.len());
            assert_eq!(extras.materials.len(), model.materials.len());
            assert!(
                extras.nodes.iter().any(|node| !node.props.is_empty()),
                "{} has nodes without any authored property",
                path.display()
            );
            assert_eq!(
                extras
                    .animations
                    .iter()
                    .filter(|stack| stack.clip.is_some())
                    .count(),
                model.animations.len(),
                "{} stacks vs clips",
                path.display()
            );
            assert!(
                extras.scene.version > 0,
                "{} has no version",
                path.display()
            );
            eprintln!(
                "{}: v{} {} nodes, {} materials, {} textures, {} meshes, {} poses, {} layers, {} sets, {} stacks",
                path.file_name().unwrap().to_string_lossy(),
                extras.scene.version,
                extras.nodes.len(),
                extras.materials.len(),
                extras.textures.len(),
                extras.meshes.len(),
                extras.poses.len(),
                extras.display_layers.len(),
                extras.selection_sets.len(),
                extras.animations.len(),
            );
            checked += 1;
        }
        eprintln!("checked {checked} fixtures");
    }

    /// The skinned fixture carries the authored cluster matrices: `Transform`
    /// (mesh node → bone) and `TransformLink` (bind → world), which must be
    /// consistent with the palette matrix the viewer derives.
    #[test]
    fn skin_clusters_carry_authored_matrices() {
        let path = fixture("SK_Player_01.fbx");
        if !path.exists() {
            eprintln!("skipping: {} is not present", path.display());
            return;
        }
        let model = load_model(&path).expect("the fixture must import");
        let skin = model.skin.as_ref().expect("skinned");
        for cluster in &skin.clusters {
            assert!(cluster.bind_to_world.is_finite());
            assert!(cluster.mesh_node_to_bone.is_finite());
            // `TransformLink` is the bone's world at bind; its inverse composed
            // with the mesh node's world is what `mesh_node_to_bone` encodes.
            let mesh_world = model.nodes[cluster.mesh_node as usize].transform;
            let expected = cluster.bind_to_world.inverse() * mesh_world;
            let delta = (expected - cluster.mesh_node_to_bone).abs();
            let max = delta.to_cols_array().into_iter().fold(0f32, f32::max);
            assert!(max < 1e-3, "cluster {} drifts by {max}", cluster.name);
        }
    }

    /// The skinned fixture's clusters must be oriented correctly: at the rest
    /// pose a vertex owned by a single bone lands exactly on its baked position
    /// transformed by that cluster's skinning matrix, and the recomposed rest
    /// pose reproduces every world transform.
    #[test]
    fn rest_pose_skinning_matches_bind() {
        let path = fixture("SK_Player_01.fbx");
        if !path.exists() {
            eprintln!("skipping: {} is not present", path.display());
            return;
        }
        let model = load_model(&path).expect("the player fixture must import");
        assert_rest_locals_recompose(&model);

        let skin = model.skin.as_ref().expect("skinned");
        let ctx = review_model::AnimContext::new(&model);
        let mut pose = review_model::Pose::new(&model);
        let mut deform = review_model::DeformPose::default();
        review_model::anim::rest_pose(&model, &ctx, &mut pose);
        review_model::anim::build_palette(&model, &ctx, &pose, &mut deform);
        assert_eq!(
            deform.palette.len(),
            review_model::anim::palette_len(&model)
        );

        // The palette's node entries are identity at rest by construction; the
        // cluster entries are `bone_world * world_to_bone_bind`. For each corner
        // the CPU reference must agree with applying that blend by hand.
        let mut checked = 0;
        for corner in (0..model.vertices.len()).step_by(97) {
            let logical = model.corner_to_logical[corner] as usize;
            let range = skin.influence_range(logical);
            if range.is_empty() {
                continue;
            }
            let base = model.vertices[corner].position;
            let mut expected = glam::Vec3::ZERO;
            let mut total = 0.0;
            for influence in range {
                let entry = model.nodes.len() + skin.influence_cluster[influence] as usize;
                expected += deform.palette[entry].transform_point3(base) * skin.weights[influence];
                total += skin.weights[influence];
            }
            if (total - 1.0_f32).abs() > 1e-6 {
                expected /= total;
            }
            let (actual, _) = review_model::anim::deform_corner(&model, &ctx, &deform, corner);
            assert!(
                actual.abs_diff_eq(expected, 1e-4),
                "corner {corner}: {actual} vs {expected}"
            );
            checked += 1;
        }
        assert!(checked > 10, "sampled too few corners: {checked}");

        // The rest bounds are finite and of the same order as the bind-pose
        // buffer (the default pose may differ from the bind pose, but not by a
        // scene's worth).
        let mut bind = review_model::Bounds::EMPTY;
        for vertex in &model.vertices {
            bind.include_point(vertex.position);
        }
        let rest = model.bounds.expect("rest bounds");
        assert!(rest.size().max_element() > 0.0);
        assert!(
            rest.size().max_element() < bind.size().max_element() * 4.0,
            "rest {rest:?} vs bind {bind:?}"
        );
    }
}
