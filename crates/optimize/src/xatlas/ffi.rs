//! The vendored xatlas, through its C API (`third_party/xatlas/xatlas_c.h`):
//! the raw declarations, the maths functions its patch calls back into, and
//! [`atlas`], the one checked call the rest of the crate makes.
//!
//! This is the crate's xatlas `unsafe` surface (invariant 9), and the only one:
//! [`atlas`] validates every buffer and index before the call, owns the atlas
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

/// `xatlasMeshDecl`.
#[repr(C)]
struct MeshDecl {
    vertex_position_data: *const c_void,
    vertex_normal_data: *const c_void,
    vertex_uv_data: *const c_void,
    index_data: *const c_void,
    face_ignore_data: *const bool,
    face_material_data: *const u32,
    face_vertex_count: *const u8,
    vertex_count: u32,
    vertex_position_stride: u32,
    vertex_normal_stride: u32,
    vertex_uv_stride: u32,
    index_count: u32,
    index_offset: i32,
    face_count: u32,
    index_format: i32,
    epsilon: f32,
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
    fn xatlasAddMesh(atlas: *mut RawAtlas, decl: *const MeshDecl, mesh_count_hint: u32) -> i32;
    fn xatlasComputeCharts(atlas: *mut RawAtlas, options: *const ChartOptions);
    fn xatlasPackCharts(atlas: *mut RawAtlas, options: *const PackOptions);
    fn xatlasMeshDeclInit(decl: *mut MeshDecl);
    fn xatlasChartOptionsInit(options: *mut ChartOptions);
    fn xatlasPackOptionsInit(options: *mut PackOptions);
    /// Patch P3's: the chart growth's limits, for this thread.
    fn review_xatlas_set_growth(normal_limit: f32, check_projection: bool);
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

/// What [`atlas`] unwraps: a triangle mesh, and per face the group a chart may
/// not leave (xatlas's "material").
pub(crate) struct Input<'a> {
    /// `x, y, z` per vertex.
    pub positions: &'a [f32],
    /// `x, y, z` per vertex, or empty.
    pub normals: &'a [f32],
    pub indices: &'a [u32],
    pub face_group: &'a [u32],
}

/// One output vertex: the input vertex it came from, its chart ([`NO_CHART`]
/// for a vertex of a face xatlas left out, such as one with no area), and its
/// UV in the atlas's texels.
pub(crate) const NO_CHART: u32 = u32::MAX;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct OutputVertex {
    pub xref: u32,
    pub chart: u32,
    pub uv: [f32; 2],
}

/// The unwrapped mesh: the faces in input order, over new vertices (one input
/// vertex is split wherever charts meet at it).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Output {
    pub vertices: Vec<OutputVertex>,
    pub indices: Vec<u32>,
    pub chart_count: u32,
    /// Per chart, how xatlas flattened it: 0 planar, 1 orthogonal projection,
    /// 2 LSCM, 3 piecewise (an island it had to cut further), 4 invalid.
    pub chart_kinds: Vec<i32>,
    pub width: u32,
    pub height: u32,
}

/// Chart, parameterize and pack `input`. `Err` says what was wrong with it, or
/// what xatlas refused.
/// `whole_islands` lifts the two limits xatlas's chart growth has beyond its
/// costs (patch P3): a chart then grows over everything its face group allows,
/// and only its flattening can split it.
pub(crate) fn atlas(
    input: &Input<'_>,
    charts: &ChartOptions,
    pack: &PackOptions,
    whole_islands: bool,
) -> Result<Output, String> {
    let vertex_count = input.positions.len() / 3;
    if !input.positions.len().is_multiple_of(3) || vertex_count == 0 {
        return Err("no vertices".to_owned());
    }
    if !input.normals.is_empty() && input.normals.len() != input.positions.len() {
        return Err("normals do not match the positions".to_owned());
    }
    if !input.indices.len().is_multiple_of(3) || input.indices.is_empty() {
        return Err("no whole triangles".to_owned());
    }
    if input.face_group.len() != input.indices.len() / 3 {
        return Err("one group per face is needed".to_owned());
    }
    if input
        .indices
        .iter()
        .any(|&index| index as usize >= vertex_count)
    {
        return Err("an index addresses no vertex".to_owned());
    }
    if input.positions.iter().any(|value| !value.is_finite()) {
        return Err("a position is not finite".to_owned());
    }
    let (Ok(vertex_count_u32), Ok(index_count_u32)) = (
        u32::try_from(vertex_count),
        u32::try_from(input.indices.len()),
    ) else {
        return Err("too large for xatlas".to_owned());
    };
    let mut charts = *charts;
    charts.param_func = std::ptr::null();

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

    let mut decl = std::mem::MaybeUninit::<MeshDecl>::uninit();
    // SAFETY: `xatlasMeshDeclInit` writes every field of the declaration.
    let mut decl = unsafe {
        xatlasMeshDeclInit(decl.as_mut_ptr());
        decl.assume_init()
    };
    decl.vertex_position_data = input.positions.as_ptr().cast();
    decl.vertex_position_stride = 12;
    if !input.normals.is_empty() {
        decl.vertex_normal_data = input.normals.as_ptr().cast();
        decl.vertex_normal_stride = 12;
    }
    decl.index_data = input.indices.as_ptr().cast();
    decl.index_format = INDEX_FORMAT_UINT32;
    decl.vertex_count = vertex_count_u32;
    decl.index_count = index_count_u32;
    decl.face_material_data = input.face_group.as_ptr();

    // SAFETY: every pointer in `decl` points into a slice that outlives the
    // call, sized as its count says (checked above: positions and normals are
    // `vertex_count * 3` floats, the indices `index_count` `u32`s all below
    // `vertex_count`, the groups one per face); xatlas copies them before
    // returning. `owned.0` is a live atlas.
    let added = unsafe { xatlasAddMesh(owned.0, &decl, 1) };
    if added != ADD_MESH_SUCCESS {
        return Err(format!("xatlas refused the mesh (error {added})"));
    }
    // SAFETY: a live atlas holding one mesh, and option structs laid out as
    // the C API's, with no parameterization callback. The limit is a plain
    // thread-local store, read by the chart growth on this same thread.
    unsafe {
        if whole_islands {
            review_xatlas_set_growth(2.0, false);
        } else {
            review_xatlas_set_growth(0.707, true);
        }
        xatlasComputeCharts(owned.0, &charts);
        xatlasPackCharts(owned.0, pack);
    }

    // SAFETY: `owned.0` is live; after `xatlasPackCharts` its fields describe
    // `mesh_count` meshes, each with `vertex_count` vertices and `index_count`
    // indices, valid until the atlas is destroyed (which `owned` does after
    // this function has copied everything out).
    let atlas = unsafe { &*owned.0 };
    if atlas.mesh_count != 1 || atlas.meshes.is_null() {
        return Err("xatlas produced no mesh".to_owned());
    }
    // SAFETY: `meshes` holds `mesh_count` (1) meshes.
    let mesh = unsafe { &*atlas.meshes };
    if mesh.index_count as usize != input.indices.len() {
        return Err("xatlas changed the face count".to_owned());
    }
    if mesh.vertex_array.is_null() || mesh.index_array.is_null() {
        return Err("xatlas produced an empty mesh".to_owned());
    }
    // SAFETY: the arrays hold `vertex_count` vertices and `index_count`
    // indices (xatlas's output contract), both non-null (checked).
    let (raw_vertices, raw_indices) = unsafe {
        (
            std::slice::from_raw_parts(mesh.vertex_array, mesh.vertex_count as usize),
            std::slice::from_raw_parts(mesh.index_array, mesh.index_count as usize),
        )
    };
    let mut vertices = Vec::with_capacity(raw_vertices.len());
    for vertex in raw_vertices {
        if vertex.xref as usize >= vertex_count {
            return Err("xatlas produced a vertex from no input vertex".to_owned());
        }
        vertices.push(OutputVertex {
            xref: vertex.xref,
            chart: u32::try_from(vertex.chart_index).unwrap_or(NO_CHART),
            uv: vertex.uv,
        });
    }
    if raw_indices
        .iter()
        .any(|&index| index as usize >= vertices.len())
    {
        return Err("xatlas produced an index past its vertices".to_owned());
    }
    let chart_kinds = if mesh.chart_array.is_null() {
        Vec::new()
    } else {
        // SAFETY: `chart_array` holds `chart_count` charts (xatlas's output
        // contract), non-null (checked).
        unsafe { std::slice::from_raw_parts(mesh.chart_array, mesh.chart_count as usize) }
            .iter()
            .map(|chart| chart.kind)
            .collect()
    };
    Ok(Output {
        vertices,
        indices: raw_indices.to_vec(),
        chart_count: atlas.chart_count,
        chart_kinds,
        width: atlas.width,
        height: atlas.height,
    })
}
