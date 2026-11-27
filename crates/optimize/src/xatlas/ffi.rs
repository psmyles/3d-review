//! The vendored xatlas, through its C API (`third_party/xatlas/xatlas_c.h`):
//! the raw declarations, the maths functions its patch calls back into, and
//! [`pack`], the one checked call the rest of the crate makes: pack charts
//! flattened elsewhere (the Unwrap operation segments and flattens its own).
//!
//! This is the crate's xatlas `unsafe` surface (invariant 9), and the only one:
//! [`pack`] validates every buffer and index before the call, owns the atlas
//! for exactly as long as it reads it, and copies the output into owned
//! vectors, re-checking each count and index against the input.
//!
//! The structs mirror `xatlas_c.h` field for field; C `enum`s are `int`-sized,
//! C `bool` is Rust's. Keep them in lockstep with the vendored header when
//! refreshing it.

#![allow(unsafe_code, reason = "invariant 9: the vendored xatlas C API")]
#![cfg(has_xatlas)]

use std::ffi::c_void;

/// `xatlasChart`.
#[repr(C)]
struct Chart {
    face_array: *mut u32,
    atlas_index: u32,
    face_count: u32,
    kind: i32,
    material: u32,
}

/// `xatlasVertex`.
#[repr(C)]
#[derive(Clone, Copy)]
struct RawVertex {
    atlas_index: i32,
    chart_index: i32,
    uv: [f32; 2],
    xref: u32,
}

/// `xatlasMesh`.
#[repr(C)]
struct RawMesh {
    chart_array: *mut Chart,
    index_array: *mut u32,
    vertex_array: *mut RawVertex,
    chart_count: u32,
    index_count: u32,
    vertex_count: u32,
}

/// `xatlasAtlas`.
#[repr(C)]
struct RawAtlas {
    image: *mut u32,
    meshes: *mut RawMesh,
    utilization: *mut f32,
    width: u32,
    height: u32,
    atlas_count: u32,
    chart_count: u32,
    mesh_count: u32,
    texels_per_unit: f32,
}

const INDEX_FORMAT_UINT32: i32 = 1;
const ADD_MESH_SUCCESS: i32 = 0;

/// `xatlasUvMeshDecl`.
#[repr(C)]
struct UvMeshDecl {
    vertex_uv_data: *const c_void,
    index_data: *const c_void,
    face_material_data: *const u32,
    vertex_count: u32,
    vertex_stride: u32,
    index_count: u32,
    index_offset: i32,
    index_format: i32,
}

/// `xatlasChartOptions`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct ChartOptions {
    param_func: *const c_void,
    pub max_chart_area: f32,
    pub max_boundary_length: f32,
    pub normal_deviation_weight: f32,
    pub roundness_weight: f32,
    pub straightness_weight: f32,
    pub normal_seam_weight: f32,
    pub texture_seam_weight: f32,
    pub max_cost: f32,
    pub max_iterations: u32,
    pub use_input_mesh_uvs: bool,
    pub fix_winding: bool,
}

/// `xatlasPackOptions`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct PackOptions {
    pub max_chart_size: u32,
    pub padding: u32,
    pub texels_per_unit: f32,
    pub resolution: u32,
    pub bilinear: bool,
    pub block_align: bool,
    pub brute_force: bool,
    pub create_image: bool,
    pub rotate_charts_to_axis: bool,
    pub rotate_charts: bool,
}

unsafe extern "C" {
    fn xatlasCreate() -> *mut RawAtlas;
    fn xatlasDestroy(atlas: *mut RawAtlas);
    fn xatlasAddUvMesh(atlas: *mut RawAtlas, decl: *const UvMeshDecl) -> i32;
    fn xatlasComputeCharts(atlas: *mut RawAtlas, options: *const ChartOptions);
    fn xatlasPackCharts(atlas: *mut RawAtlas, options: *const PackOptions);
    fn xatlasUvMeshDeclInit(decl: *mut UvMeshDecl);
    fn xatlasChartOptionsInit(options: *mut ChartOptions);
    fn xatlasPackOptionsInit(options: *mut PackOptions);
}

impl Default for ChartOptions {
    /// xatlas's own defaults.
    fn default() -> Self {
        let mut options = std::mem::MaybeUninit::<ChartOptions>::uninit();
        // SAFETY: `xatlasChartOptionsInit` writes every field of the struct it is
        // given (it assigns a default-constructed `xatlas::ChartOptions`), and the
        // pointer is to a properly sized and aligned `ChartOptions`.
        unsafe {
            xatlasChartOptionsInit(options.as_mut_ptr());
            options.assume_init()
        }
    }
}

impl Default for PackOptions {
    /// xatlas's own defaults.
    fn default() -> Self {
        let mut options = std::mem::MaybeUninit::<PackOptions>::uninit();
        // SAFETY: as for `ChartOptions`: the init call writes every field.
        unsafe {
            xatlasPackOptionsInit(options.as_mut_ptr());
            options.assume_init()
        }
    }
}

// The maths xatlas's patch P1 redirects here (`third_party/xatlas/NOTICE.txt`):
// the same bits on every OS (D27).

/// `acosf` for xatlas.
#[unsafe(no_mangle)]
pub extern "C" fn review_xatlas_acosf(x: f32) -> f32 {
    review_model::exact::acos(x)
}

/// `sinf` for xatlas.
#[unsafe(no_mangle)]
pub extern "C" fn review_xatlas_sinf(x: f32) -> f32 {
    review_model::exact::sin(x)
}

/// `cosf` for xatlas.
#[unsafe(no_mangle)]
pub extern "C" fn review_xatlas_cosf(x: f32) -> f32 {
    review_model::exact::cos(x)
}

/// `atan2f` for xatlas.
#[unsafe(no_mangle)]
pub extern "C" fn review_xatlas_atan2f(y: f32, x: f32) -> f32 {
    review_model::exact::atan2(y, x)
}

/// Charts flattened elsewhere, packed: per vertex its UV in the atlas's texels
/// (`None` for a vertex of no chart: one only zero-area faces use), and the
/// atlas's size.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Packed {
    pub uvs: Vec<Option<[f32; 2]>>,
    pub width: u32,
    pub height: u32,
}

/// Pack charts already flattened: `uvs` per vertex, in texels (`texels_per_unit`
/// in `options` should be 1), `indices` three per face, `chart` one per face.
/// A face's chart keeps it apart from every other chart's, even where their
/// UVs meet.
pub(crate) fn pack(
    uvs: &[[f32; 2]],
    indices: &[u32],
    chart: &[u32],
    options: &PackOptions,
) -> Result<Packed, String> {
    let vertex_count = uvs.len();
    if vertex_count == 0 || indices.is_empty() || !indices.len().is_multiple_of(3) {
        return Err("no whole triangles".to_owned());
    }
    if chart.len() != indices.len() / 3 {
        return Err("one chart per face is needed".to_owned());
    }
    if indices.iter().any(|&index| index as usize >= vertex_count) {
        return Err("an index addresses no vertex".to_owned());
    }
    if uvs.iter().flatten().any(|value| !value.is_finite()) {
        return Err("a UV is not finite".to_owned());
    }
    let (Ok(vertex_count_u32), Ok(index_count_u32)) =
        (u32::try_from(vertex_count), u32::try_from(indices.len()))
    else {
        return Err("too large for xatlas".to_owned());
    };

    /// Destroys the atlas on every path out.
    struct Owned(*mut RawAtlas);
    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: the pointer came from `xatlasCreate`, is non-null (checked
            // where it was made) and is destroyed exactly once, here.
            unsafe { xatlasDestroy(self.0) }
        }
    }
    // SAFETY: `xatlasCreate` takes nothing and returns a new atlas or null.
    let raw = unsafe { xatlasCreate() };
    if raw.is_null() {
        return Err("xatlas could not allocate an atlas".to_owned());
    }
    let owned = Owned(raw);

    let mut decl = std::mem::MaybeUninit::<UvMeshDecl>::uninit();
    // SAFETY: `xatlasUvMeshDeclInit` writes every field of the declaration.
    let mut decl = unsafe {
        xatlasUvMeshDeclInit(decl.as_mut_ptr());
        decl.assume_init()
    };
    decl.vertex_uv_data = uvs.as_ptr().cast();
    decl.vertex_stride = 8;
    decl.vertex_count = vertex_count_u32;
    decl.index_data = indices.as_ptr().cast();
    decl.index_count = index_count_u32;
    decl.index_format = INDEX_FORMAT_UINT32;
    decl.face_material_data = chart.as_ptr();
    // SAFETY: every pointer in `decl` points into a slice that outlives the
    // call, sized as its count says (checked above); xatlas copies them.
    let added = unsafe { xatlasAddUvMesh(owned.0, &decl) };
    if added != ADD_MESH_SUCCESS {
        return Err(format!("xatlas refused the charts (error {added})"));
    }
    let charts = ChartOptions::default();
    // SAFETY: a live atlas holding one UV mesh, and option structs laid out as
    // the C API's, with no parameterization callback.
    unsafe {
        xatlasComputeCharts(owned.0, &charts);
        xatlasPackCharts(owned.0, options);
    }
    // SAFETY: `owned.0` is live; after `xatlasPackCharts` its fields describe
    // `mesh_count` meshes, each with `vertex_count` vertices and `index_count`
    // indices, valid until the atlas is destroyed (which `owned` does after
    // this function has copied everything out).
    let atlas = unsafe { &*owned.0 };
    if atlas.mesh_count != 1 || atlas.meshes.is_null() || atlas.atlas_count > 1 {
        return Err("xatlas did not pack into one atlas".to_owned());
    }
    // SAFETY: `meshes` holds `mesh_count` (1) meshes.
    let mesh = unsafe { &*atlas.meshes };
    if mesh.vertex_count as usize != vertex_count
        || mesh.index_count as usize != indices.len()
        || mesh.vertex_array.is_null()
    {
        return Err("xatlas changed the charts' vertices".to_owned());
    }
    // SAFETY: the array holds `vertex_count` vertices (xatlas's output
    // contract), non-null (checked).
    let raw_vertices =
        unsafe { std::slice::from_raw_parts(mesh.vertex_array, mesh.vertex_count as usize) };
    Ok(Packed {
        uvs: raw_vertices
            .iter()
            .map(|vertex| (vertex.chart_index >= 0).then_some(vertex.uv))
            .collect(),
        width: atlas.width,
        height: atlas.height,
    })
}
