# Field-guided retopology for the Opt stack: `Remesh` + `Shrinkwrap`

## Context

TriFlow (arXiv 2606.20131) is a learned retopologizer: SDF proxy -> learned
nearest-vertex field -> watershed clustering -> constrained QEM -> a new mesh with
artist-like topology. Its network, licence (ADPNCL, non-commercial), and runtime
(PyTorch + CUDA, 1.3 GB weights, 31 s/shape on an A6000) make it unusable
in-app. Everything after the network, however, is deterministic geometry
processing that already exists in permissively licensed form: TriFlow is
structurally "Instant Meshes with the field learned instead of computed".

This plan adds that non-learned pipeline to the Opt stack for **static meshes
only** (skinned / blend-shaped nodes are left untouched with a warning):

- **`Remesh`** op: per node, regenerate the surface as evenly sized, curvature-
  aligned **Triangles**, **Mostly quads** (Instant Meshes, BSD-3) or **Only
  quads** (QuadriFlow, MIT, manifold input only), at a density given either as a
  **ratio** of the node's current triangles or an **absolute** face count split
  across nodes by area. Materials, UVs, vertex colors and normals come back by
  **projection** onto the source surface. Quads are emitted as **real polygons**:
  the viewport draws a quad wireframe, the stats card counts them as Polys, and
  the export writes them as quads.
- **`Shrinkwrap`** op (final phase, pure Rust): narrow-band SDF with a
  generalized-winding-number sign, marching cubes, one closed manifold shell per
  node. It fuses kitbashed parts and is what makes Only quads work on game assets.

User decisions (already made): vendor both engines, phased; Shrinkwrap is in
scope as the last phase; density has both modes with a toggle. CPU only (the
optimizer runs on a worker thread that cannot touch sokol; see conversation).
Assumptions taken here, overridable at review: `deterministic` defaults **on**;
the ratio reads against the node's **current triangles** (matches the Tris figure
on the stats card), with a quad counted as two.

Two code facts drive the design (verified):

1. `TriangleData::to_face` is all-or-nothing (`crates/model/src/geometry.rs:112-139`):
   a level cannot carry faces for one node and not another. So corner-run emission
   is a **per-level mode**: once any piece carries a rebuilt polygon list, `assemble`
   emits every piece in the import's corner-run layout (one vertex per face corner,
   contiguous, in face order; `TopologyFace { first_index = vertex offset,
   index_count }`; see `crates/model/src/demo.rs:13-24,125-132`). With that layout
   `wireframe_edge_indices` (`crates/render/src/geometry/wireframe.rs:37-84`),
   `mesh_group_stats` (`crates/model/src/stats.rs:197-223`) and the export all see
   real quads, unchanged.
2. The exporter writes one FBX control point per level vertex
   (`crates/optimize/src/export/mesh.rs:53-74`, `local_vertex`). A corner-run level
   exported as-is would be a quad soup, so `LevelCarry` gains a `control_point`
   map (per level vertex, the indexed vertex it was expanded from) and
   `local_vertex` keys on it.

The `Submesh` stays **indexed** after a Remesh (exact-welded vertices + a
`PolygonCarry` describing the quads, flagged `rebuilt`), so every downstream rule
still works: a later Weld/VertexFetch renumbers the carry (`submesh/mesh.rs:143`),
Filter/Prune/reorders reconcile by content (`mesh.rs:265`), a simplify clears it
(`ops.rs:221`), and `meshopt::analyze` measures a real indexed mesh.

---

## Phase A: Instant Meshes (Triangles + Mostly quads)

### A.1 Vendor Eigen + Instant Meshes core

```
third_party/eigen/            Eigen/ from release 3.4.0 (no unsupported/, doc/, test/), COPYING.MPL2, NOTICE.txt
                              (build defines EIGEN_MPL2_ONLY; whole dir vendored for the same reason meshoptimizer's is)
third_party/instant-meshes/   LICENSE.txt (BSD-3), NOTICE.txt (patched-tree template of third_party/ufbx-write/NOTICE.txt),
                              review.patch, src/ (compute set only: aabb.h adjacency bvh cleanup common.h dedge extract field
                              hierarchy meshstats normal smoothcurve subdivide + whatever their include closure pulls;
                              NOT viewer/gui/main/glutil/meshio/batch), shim/tbb/*.h
```

**TBB shim** (`shim/tbb/`, namespace `tbb`, on the include path ahead of everything
so vendored `#include <tbb/...>` resolves to it with no source edit): `blocked_range`,
`parallel_for` (range and index forms over a once-created `std::thread` pool sized
by `available_parallelism`, overridable via env `REVIEW_REMESH_THREADS`),
`parallel_reduce` as **fixed chunking in index order with a sequential join** (so
float results are thread-count independent), `parallel_sort` = `std::sort`,
`spin_mutex` on `std::atomic_flag` with nested `scoped_lock`, `concurrent_vector`
= mutex-guarded `std::vector` exposing only the members the vendored code uses
(discovered by compiling), `task_scheduler_init` no-op. `review.patch` is expected
to be tiny (silence `std::cout` progress, include fixes) and documented per hunk.

### A.2 Build wiring: `crates/optimize/build.rs`

Add `cargo:rustc-check-cfg=cfg(has_instant_meshes)` and `build_instant_meshes()`
after `build_ufbx_write()`, following the two-build split already there:

- Gate on `../../third_party/instant-meshes/src/field.cpp` and
  `../../third_party/eigen/Eigen/Core`; missing -> return, no cfg.
- Vendored: `cpp(true)`, sorted `src/*.cpp`, includes `src`, `shim`, `eigen`,
  defines `EIGEN_MPL2_ONLY`, `EIGEN_NO_DEBUG`, `NDEBUG`, **`opt_level(2)` in every
  profile** (Eigen at -O0 makes a 100k-tri remesh take minutes under `cargo test`),
  flags copied from `crates/psd/build.rs:38-52` (`/std:c++17`, `/EHsc`,
  `-std=c++17`) plus `/bigobj`, `warnings(false)`, `.compile("instant_meshes")`.
- Bridge: `src/remesh_bridge.cpp`, same includes/flags, `warnings(true)`, `/wd` by
  number only for Eigen-header warnings, emitted first.
- `cargo:rustc-cfg=has_instant_meshes`; rerun-if-changed on tree, bridge, header.

`lib.rs:34` -> `#![cfg_attr(not(any(has_meshopt, has_ufbxw, has_instant_meshes)), forbid(unsafe_code))]`;
add `mod remesh_ffi;` beside `mod export_ffi;` and `pub mod remesh;`.

### A.3 C ABI: `src/remesh_bridge.h` / `.cpp`, `src/remesh_ffi.rs`

**Ownership: opaque result handle + `review_remesh_free`, wrapped in a `Drop` guard
inside the one unsafe module.** Count-then-fill (invariant 7) fits a parse whose
counts are cheap; here the output size is known only after the whole solve, so
count->fill would run it twice or hide state in C. The handle keeps C++ the only
allocator of its own memory (no CRT mismatch).

```c
#define RVO_REMESH_ERROR_LENGTH 512
enum { RVO_REMESH_ENGINE_INSTANT_MESHES = 0, RVO_REMESH_ENGINE_QUADRIFLOW = 1 };
typedef struct { const float *positions; size_t vertex_count; const uint32_t *indices; size_t index_count; } rvo_remesh_input;
typedef struct { uint32_t engine, rosy /*2|4|6*/, posy /*3|4*/, face_count; float crease_angle_deg /*<0 off*/;
                 int32_t extrinsic, align_to_boundaries; uint32_t smooth_iterations; int32_t pure_quad, deterministic;
                 int32_t adaptive_scale, min_cost_flow; /* phase B */ } rvo_remesh_options;
typedef struct rvo_remesh_result rvo_remesh_result;                       /* opaque, bridge-owned */
int  review_remesh_run(const rvo_remesh_input*, const rvo_remesh_options*, rvo_remesh_result **out, char *error, size_t error_length);
const float    *review_remesh_positions  (const rvo_remesh_result*, size_t *vertex_count);
const uint32_t *review_remesh_corners    (const rvo_remesh_result*, size_t *corner_count);
const uint32_t *review_remesh_face_offsets(const rvo_remesh_result*, size_t *face_count); /* face_count+1 entries */
void review_remesh_free(rvo_remesh_result*);
```

`remesh_bridge.cpp` (one TU, `extern "C"`): validates input (index range, `%3`,
finite, `rvo_mul_overflows` on every sizing), wraps the run in `try/catch
(std::exception)` -> error string (Instant Meshes throws `std::runtime_error`),
and mirrors `batch.cpp`'s call order: `build_dedge` -> adjacency -> smooth normals
-> mesh stats -> `scale` from `face_count` -> `subdivide` if needed ->
`MultiResolutionHierarchy` (`setF/setV/setN/setAdj`, `build(deterministic)`,
`resetSolution`, `setScale`) -> boundary constraints when `align_to_boundaries`
-> orientation then position optimisation per level coarse-to-fine (drive
`optimize_orientations` / `optimize_positions` directly rather than the
`Optimizer`, which owns its own worker thread) -> `extract_graph` ->
`extract_faces(..., posy, scale, crease, fill_holes=true, pure_quad, bvh,
smooth_iterations)`. Copies `F` (posy x nF; `F(2,f)==F(3,f)` collapsed to a
3-corner face; <3 distinct corners dropped) and `O` into the result.

`remesh_ffi.rs` (`#![cfg(has_instant_meshes)]`): `#[repr(C)]` mirrors in header
field order, `ERROR_LENGTH = 512` duplicated from the header, five `unsafe extern
"C"` decls, `RemeshResult(NonNull<..>)` with `Drop => review_remesh_free` and
null-checked slice accessors, and the single unsafe call site
`pub(crate) fn run(input: &RemeshInput<'_>, options: &RemeshOptions) -> Result<RemeshOutput, OptError>`
which **copies** the three views into owned `Vec`s, re-validates (offsets
monotone, last == corners.len(), corners < vertex count, faces >= 3 corners) and
drops the guard. Twin in `src/remesh/unavailable.rs` (`#![cfg(not(has_instant_meshes))]`)
returning `OptError::RemeshUnavailable` (new variant naming `third_party/instant-meshes`),
following `meshopt/unavailable.rs`. It surfaces as a per-op warning, not a run failure.

### A.4 Parameters: `src/stack/remesh.rs` (re-export from `stack/mod.rs` + `lib.rs:60-64`)

```rust
pub enum RemeshTopology { Triangles, #[default] QuadDominant }   // PureQuads in phase B
impl RemeshTopology { pub const ALL; pub fn label(self) -> &'static str; pub(crate) fn rosy_posy(self) -> (u32,u32) /* (6,3) | (4,4) */ }
pub enum RemeshDensity { #[default] Ratio, Absolute }             // ALL, label
#[serde(default)] pub struct RemeshParams {
    pub topology: RemeshTopology, pub density: RemeshDensity,
    pub ratio: f32 /*1.0*/, pub faces: u32 /*5000*/,
    pub sharp_edges: bool /*false*/, pub crease_angle: f32 /*30 deg*/,
    pub align_to_boundaries: bool /*true*/, pub smooth_iterations: u32 /*2*/, pub deterministic: bool /*true*/,
}
```

`OpKind::Remesh(RemeshParams)` in `stack/mod.rs:294-317` (after `Reduce`), `ALL`
array length 10 (test at `:515` -> ten), `label() => "Remesh"`, `description()`,
`alters_geometry() => true` (this alone triggers `Rebuild { tangents: true }` via
`process/mod.rs:233-279`). Presets: additive, `PRESET_VERSION` unchanged.

### A.5 Density resolution: `src/remesh/mod.rs` (pure, unit-tested)

`resolve_budgets(nodes: &[(node, triangles_now, area, RemeshParams)]) -> Vec<NodeBudget { node, faces }>`,
params per node from `resolve_op(stack, op, node)` (`process/mod.rs:324`).
Ratio: `faces = round(triangles * ratio * factor)`, factor 0.5 for QuadDominant,
1.0 for Triangles. Absolute: nodes still on the global params share `faces` by
area; a node whose override sets its own count gets exactly that and leaves the
pool. Floor `MIN_FACES = 4`; skip (warn) a proxy under `MIN_INPUT_TRIANGLES = 16`.

### A.6 The operation: `src/remesh/{mod,proxy,project,manifold,layout}.rs`

Dispatch beside the AO arm in `process/pipeline.rs:185`:
`if matches!(op.kind, OpKind::Remesh(_)) { crate::remesh::remesh_submeshes(submeshes, op, stack, model, warnings); return 0.0; }`
and add `OpKind::Remesh(_)` to the `Reduce | SimplifyLod | BakeAo => Ok(())` arm at
`:205`. Change `apply_op`'s first parameter to `&mut Vec<Submesh>` (both callers,
`process/mod.rs:191` and `:218`, hold `Vec`s) because Remesh **replaces** a node's
pieces.

Per node, in first-seen order:

1. Skip if `is_excluded`; skip with a warning if any piece has skin/morph/DQ rows.
2. **Proxy** (`proxy.rs`): concatenate the node's pieces, exact-weld by position
   bits across materials (as `ao.rs:181`), drop degenerate triangles, record area.
3. **Projection source** (`project.rs`, `ProjectionSource::build`): pieces
   concatenated **without** welding as a scratch `ModelData { vertices, indices,
   ..Default }` plus parallel `uv_channels`, `color_channels`, `triangle_material`
   and triangle adjacency from shared index-buffer edges; `Bvh::build` over it.
4. `manifold::report(&proxy) -> { boundary_edges, nonmanifold_edges }`; warn once
   when non-manifold ("the remesh may leave holes there").
5. `remesh_ffi::run` with `topology.rosy_posy()`, the budget, crease angle (or -1).
   `Err` -> warning, pieces untouched. Empty -> warning.
6. **Canonicalize** (`layout.rs`): sort faces by lowest-corner position bits,
   renumber vertices by first use, rotate each face to its lowest vertex. Makes the
   output independent of thread scheduling so bytes compare.
7. **Orientation guard**: flip a face whose normal opposes the source normal at
   its centroid.
8. **Projection**: per face, `bvh.closest_point(centroid)` -> `T_f`, `material =
   triangle_material[T_f]`. Per corner: closest point on the **2-ring
   adjacency neighbourhood** of `T_f` (interior barycentrics if inside a triangle,
   else extrapolated on `T_f`). Because two corners of one output vertex from
   adjacent faces in the same attribute region resolve to the same source point,
   they are bit-identical and the weld in step 10 merges them; corners straddling
   a real seam stay split, so seams land on output edges and no face tears.
   Attributes: `Vertex::uv`, every `uv_channels[c]`, `vertex_color`, extra
   `color_channels` (clamped), `normal` (barycentric, normalised, face normal
   fallback); `tangent` default (rebuilt); `vertex_crease` dropped. Parallel over
   faces with `std::thread::scope` round-robin chunks as `ao.rs:261-277`.
9. **Split by material** into one corner-run `Submesh` per material present:
   one vertex per corner, fan triangulation (quads on the shorter diagonal),
   `polygons: Some(PolygonCarry { face_offsets, corners, source_face: vec![u32::MAX; n]
   /* must be filled: retain_faces indexes it at polygons.rs:72 */, triangle_face,
   layers empty, edges empty, rebuilt: true })`, `source_corner` empty.
10. `ops::weld(&mut piece, &WeldParams::default())`: the exact weld goes through
    `apply_vertex_remap`, which renumbers the carry (`mesh.rs:188-206`). The piece is
    now indexed with quads over shared vertices.
11. `splice` the new pieces in at the node's first old piece.

New: `PolygonCarry.rebuilt: bool` (default false). `LevelCarry.control_point: Vec<u32>`
(empty = identity).

### A.7 Closest-point query: `crates/model/src/bvh.rs`

```rust
pub struct ClosestPoint { pub triangle: u32, pub point: Vec3, pub distance_squared: f32, pub barycentric: Vec3 }
impl Bvh { pub fn closest_point(&self, model: &ModelData, query: Vec3, max_distance: f32) -> Option<ClosestPoint>; }
pub fn closest_point_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> (Vec3, Vec3);   // Ericson; public like ray_triangle_t
```

Traversal reuses the fixed `MAX_STACK` array of `closest_hit_with`, nearest-child
first by box distance, pruning boxes farther than the best. Tests: brute-force
parity over a grid around the demo cube, on-surface query -> distance 0 with
interior barycentrics, beyond `max_distance` -> None, empty model -> None.

### A.8 Corner-run emission: `process/assemble.rs` + `remesh/layout.rs`

`assemble` sets `corner_run = any piece has polygons.rebuilt`. In that mode each
non-empty piece is expanded by `corner_run(piece) -> CornerRun { vertices,
uv_channels, color_channels, vertex_crease, source_corner, control_point, indices,
faces: Vec<TopologyFace>, to_face }`: a carried face's corners pushed contiguously,
its triangles re-expressed over the run, `NO_FACE` triangles and carry-less pieces
as 3-corner faces; deform rows (a skinned neighbour node) gathered through
`VertexRows::gather(&control_point)`. `assemble` fills `model.faces`,
`model.triangles.to_face` (offset by running face count), `carry.control_point`,
and `carry.polygons[..].corners` in run numbering; `TriangleData::validate`
asserted in debug. Outside the mode nothing changes. Update the doc comment at
`assemble.rs:18-25`.

Invariant 5 (never surface the corner count): `measured_stats` takes the indexed
vertex count (sum of `piece.vertices.len()`); `process/mod.rs:292` measures with
`measure_submeshes(&level.submeshes, ..)` instead of re-partitioning the assembled
model (identical outside the mode).

Export: `export/mesh.rs:53-74` keys `local_of_global` by
`carry.control_point.get(g).copied().unwrap_or(g)`. `source_face == u32::MAX` and
empty `source_edge` are already tolerated (`mesh.rs:268`, `:554`); no "written as
triangles" note fires (the piece is non-empty); the existing selection-set note at
`anim.rs:384` reports lost members.

### A.9 Ordering advice (`process/mod.rs:251-267` area)

A `Reduce`/`SimplifyLod` enabled **below** the last `Remesh`: "A simplifier runs
after Remesh and rebuilds its faces as triangles ... Move Remesh below it to keep
the quads." Remesh after a simplify is fine. Bake AO in either order is fine.
Warnings are English diagnostics through `Warnings` (invariant 12's exception),
prefixed `Remesh: '<node name>' ...`.

### A.10 UI (`crates/ui`)

- `panels/opt_inspector.rs:87-101`: `OpKind::Remesh(params) => remesh_params(ui, params).map(OpKind::Remesh)`;
  `remesh_params` in the copy-edit-writeback shape of `bake_ao_params` (`:190-252`):
  `labeled_combo` over `RemeshTopology::ALL` / `RemeshDensity::ALL`;
  `labeled_slider_with_value` on `ratio` or `faces` by density; `labeled_checkbox`
  sharp edges (+ crease slider when on), align to boundaries; smoothing slider;
  deterministic checkbox; `REMESH_EXPLAINED` in `color::TEXT_MUTED`. Every `Tip`
  `.page(Page::OptRemesh)`.
- Range consts in the `range` block of `crates/ui/src/state/mod.rs` (~`:84-108`):
  `REMESH_RATIO_MIN 0.05 / MAX 2.0`, `REMESH_FACES_MIN 100 / MAX 200_000`,
  `REMESH_CREASE_MIN 5.0 / MAX 90.0`, `REMESH_SMOOTH_MAX 10`.
- `labels.rs:217-246`: `OP_REMESH` / `OP_REMESH_DESCRIPTION` arms; new
  `remesh_topology()` / `remesh_density()` mappers; both `ALL`s added to
  `every_model_and_optimize_enum_variant_resolves` (`:388-418`).
- `panels/opt_stack.rs:117-123`: add-time side effect turns the wireframe overlay
  on (add-only, like the AO material switch).
- Fluent (`crates/localization/locales/en/`, the only locale): `ui-enums.ftl`
  `ui-enums-op-remesh` + `.description`, `ui-enums-remesh-triangles`,
  `-quad-dominant = Mostly quads`, `-density-ratio`, `-density-absolute`;
  `ui-opt.ftl` `ui-opt-remesh-{topology,density,ratio,faces,sharp-edges,crease-angle,
  align-boundaries,smoothing,deterministic}` + `.description`, `ui-opt-remesh-explained`.
  ASCII only.
- `app/src/explain.rs`: arm for `OptError::RemeshUnavailable`.

### A.11 Docs

`docs/book/src/en/opt/remesh.md` (H1 "Remeshing a model"; H2s: When to remesh /
Triangles or quads / How dense / Sharp edges and open borders / Materials, UVs and
colors come back by projection / What is lost / Where it goes in the list /
Working with a messy model [filled in phase C]); `SUMMARY.md:44` line after
simplify.md (yields `Page::OptRemesh` automatically via `crates/ui/build/pages.rs`);
`opt/operations.md` bullet + a sentence in "What survives". `CLAUDE.md`: §2
optimize entry (`src/remesh/`, `remesh_bridge.cpp`, `remesh_ffi.rs`), third_party
list, invariant 9's sanctioned-exception paragraph, and the "polygons ride the
pipeline as a carry" gotcha (they now can reach the renderer in corner-run levels).

### A.12 Tests

- `model/src/bvh.rs`: the four closest-point tests (A.7).
- `remesh/layout.rs`: `corner_run_of_a_rebuilt_quad_carry_validates` (hand-built
  2-quad `Submesh` as `submesh/mod.rs:130` does; `TriangleData::validate` passes;
  contiguous runs; `to_face` is the fan map; `control_point` maps corners to
  indexed vertices); `canonicalize_is_order_independent`.
- `remesh/project.rs`: on `demo_cube_model()`, +Z-face centroid -> material 0 and
  bilinear UVs; two corners of one shared vertex on one cube face project
  bit-identically; a corner across a cube edge does not weld.
- `remesh/mod.rs`: `resolve_budgets` ratio / absolute / override / floor;
  `manifold::report` on the cube (0/0) and an open quad (4 boundary edges).
- `process/mod.rs`: `a_level_with_a_rebuilt_carry_emits_corner_run_faces`; keep
  the existing no-topology test as the control.
- `crates/optimize/tests/remesh.rs` (`#![cfg(all(has_meshopt, has_instant_meshes))]`,
  `common::{fixture, run}`): monkey.fbx Mostly quads at absolute 2000 ->
  `polygon_count` within 25% of target, quad share >= 0.8, `assert_consistent`-style
  checks, no "left as is" warning; Triangles -> `polygon_count == triangle_count`;
  SM_column04.fbx -> every output material is a source material; determinism ->
  two `deterministic` runs bit-identical (plus one under `REVIEW_REMESH_THREADS=1`
  in its own test binary); exclusion -> excluded node unchanged; Weld+VertexFetch
  below Remesh keeps `polygon_count`.
- `tests/export_round_trip.rs`: `remeshed_quads_read_back_as_quads` -> export the
  quad monkey, `reimport`, `loaded.stats.polygon_count == level polygon_count`,
  `polygon_count < triangle_count`, DCC vertex count == the level's indexed count,
  bounds within 2% of extent (the remesh moves the surface by a fraction of an edge).
- Update `stack/mod.rs:515` count test; l10n tests run unchanged.

---

## Phase B: QuadriFlow (Only quads)

- **Vendor** `third_party/quadriflow/` from `blender/blender` `extern/quadriflow`
  (upstream 27a6867 with `patches/blender.patch` applied: Boost graph replaced by
  `patches/boykov_kolmogorov_max_flow.hpp`, lemon-1.3.1 bundled under `3rd/`):
  `src/` minus `main.cpp`/`loader.cpp`, the patch header, lemon headers + the `.cc`
  files Blender's CMake lists, `LICENSE.txt` (MIT), `NOTICE.txt` naming Blender's
  arrangement as the source. Shares `third_party/eigen`. `build_quadriflow()` in
  `build.rs` gated on `has_quadriflow`, serial (no OpenMP), `EIGEN_MPL2_ONLY`,
  `opt_level(2)`, `/bigobj`, `/EHsc`; sets `REVIEW_HAS_QUADRIFLOW` for the bridge.
- **Bridge**: `engine == RVO_REMESH_ENGINE_QUADRIFLOW` dispatches to a second
  function under `#ifdef REVIEW_HAS_QUADRIFLOW`, mirroring Blender's
  `quadriflow_capi.cpp`: `Parametrizer` flags (`preserve_sharp`, `preserve_boundary`,
  `adaptive_scale`, `minimum_cost_flow`, seed 0 when deterministic), load V/F,
  `NormalizeMesh`, `Initialize(face_count)`, orientation + position fields,
  `ComputeIndexMap()` false -> error, de-normalise `O_compact`/`F_compact` into the
  same result. No new Rust FFI decls; `RemeshOptions` gains `engine`,
  `adaptive_scale`, `min_cost_flow`.
- **Params**: `RemeshTopology::PureQuads` (`ALL` 3, `ui-enums-remesh-pure-quads =
  Only quads`), `RemeshParams { adaptive_scale, min_cost_flow }` shown only for it.
- **Manifold gate + fallback**: `manifold::report` must show 0 non-manifold edges
  and every vertex one fan (add the vertex check); boundaries allowed. Otherwise
  warn "... not a manifold mesh (N bad edges, M bad vertices), so Only quads fell
  back to Mostly quads; run Shrinkwrap first" and run (4,4) through Instant
  Meshes. A runtime QuadriFlow error takes the same fallback with its message.
- **Tests** (`#[cfg(has_quadriflow)]` in `tests/remesh.rs`): monkey PureQuads ->
  every face `index_count == 4`, count within 30%; cube with a duplicated face ->
  fallback warning text and still quads; seed-0 determinism. Docs section "Why
  Only quads sometimes falls back".

---

## Phase C: Shrinkwrap (pure Rust)

`OpKind::Shrinkwrap(ShrinkwrapParams { resolution: u32 /*128, 32..=512*/, offset: f32
/*0.0 m*/, keep_largest_shell: bool /*true*/ })`, `ALL` 11, `alters_geometry() => true`,
whole-scene dispatch like Remesh, per node, static only; materials/UVs/colors come
back through the same `remesh::project` path, so Shrinkwrap alone yields a textured
proxy and Only quads after it always passes the manifold gate.

```
crates/optimize/src/shrinkwrap/
  mod.rs      shrinkwrap_submeshes + per-node driver
  grid.rs     GridDesc { origin, voxel, dims } from node bounds padded 3 voxels; voxel = max_extent / resolution;
              block-sparse 8x8x8 storage allocated only where a dilated triangle AABB touches (4 B dist + 1 B sign per cell);
              MAX_CELLS = 64 M -> warn and lower resolution
  sdf.rs      narrow-band unsigned distance via Bvh::closest_point (band = 2 voxels); outside the band +-band by sign
  winding.rs  generalized winding number: exact solid angle near field + Barill et al. dipole far field over a private
              triangle-cluster octree; sign = winding > 0.5 (robust to holes, self-intersections, inverted shells)
  mc.rs       marching cubes (256-case tables) over band cells, vertices welded by edge key -> closed manifold;
              keep_largest_shell drops small shells
  voxel.rs    voxel_sample(grid, flat_index, source) -> VoxelSample: the per-voxel pass as a pure function of a flat
              index so it can move to a compute shader later
```

Parallel over blocks in Morton order round-robin (`std::thread::scope`, as
`ao.rs:261`); block outputs independent, so scheduling cannot change the result.
Output: one manifold triangle `Submesh` per node, split per material by projection,
then `ops::weld`; no polygon carry. Warnings: resolution lowered, empty result
(node thinner than a voxel), skinned node skipped. UI/l10n/docs as for Remesh
(`ui-enums-op-shrinkwrap`, `ui-opt-shrinkwrap-*`, "Working with a messy model").
Tests: `winding_number_is_one_inside_the_cube_and_zero_outside`,
`marching_cubes_of_a_sphere_sdf_is_closed_and_manifold` (edge-use 2 everywhere,
Euler 2), `voxel_sample_is_a_pure_function_of_its_index` (per-block == whole-grid);
integration: monkey Shrinkwrap at 96 -> `manifold::report` 0/0, bounds within 2
voxels; Shrinkwrap + Only quads -> all quads, no fallback warning.

---

## Verification (per phase, MSVC on PATH)

```
cargo check -p review-model -p review-optimize
cargo test -p review-model bvh
cargo test -p review-optimize
cargo test -p review-ui labels
cargo test -p review-localization
cargo clippy --workspace --all-targets -- -D warnings
$env:REVIEW_REQUIRE_FIXTURES='1'; cargo test -p review-optimize --test remesh --test export_round_trip
scripts/check.ps1
```

Availability: rename `third_party/instant-meshes` (later `quadriflow`) and confirm
`cargo check --workspace` passes with the op warning `RemeshUnavailable`; add a
twin of `lib.rs`'s `availability_matches_the_vendored_tree`.

Manual viewport (A): `SM_column04.fbx` -> Opt -> add Remesh (wireframe turns on)
-> quads in the processed half and the ghost wireframe; Polys near target, Tris
about double in Mostly quads; Triangles -> Polys == Tris; Absolute 3000 on
`SM_Ammo_Crate_01a.fbx` -> per-node density proportional to size; exclude a node ->
untouched while the level still draws quads elsewhere; Verts stays the indexed
count; export, reopen -> quad wireframe, per-face materials, UV islands;
`lucy.fbx` shows the "Optimizing mesh..." card; slider drag coalesces. (B) Only
quads on monkey -> no triangles; non-manifold asset -> one warning, quads shown.
(C) Shrinkwrap on `stylized_tree_branch_01.fbx` -> one closed shell; then Only
quads -> no fallback.

## Future: UV layout generation (standalone op, not in these phases)

An auto-unwrap op (`GenerateUvs`, engine xatlas, MIT, vendored the same way; a
whole-scene op since one atlas packs every node) is deliberately **out of scope**:
it replaces the source's layout and invalidates its textures, so it is a separate
choice from Remesh, which keeps the layout by projection. Its priority rises after
phase C, because a Shrinkwrap shell projects a patchwork of the fused parts'
islands. Two hooks are reserved now so nothing here has to be retrofitted:

1. **A fourth carry rule: vertex split.** The three existing rules
   (`submesh/mod.rs:20-38`) cover merge (`apply_vertex_remap`), keep-whole
   (`reconcile_triangles`) and rebuild (`clear_polygons` / the `rebuilt` carry).
   Unwrapping re-points a face's corners at *new* vertices along new seams; xatlas
   returns exactly that as a per-output-vertex `xref` to the source vertex. When
   the op lands, `PolygonCarry` and `VertexRows` gain a `split_vertices(map)`
   beside `apply_vertex_remap` (corners rewritten per (face, corner), rows
   gathered from the source vertex). Remesh does not need it: it rebuilds the
   carry outright, and nothing in phases A-C assumes a vertex can only merge.
2. **Per-level UV channel growth.** `assemble` sizes `uv_channels` from the
   source's channel count, and `ModelData` keeps that array empty unless there is
   more than one set. A lightmap-UV op *adds* a set, so the level will need its
   own channel count and an entry in `uv_set_names`; the export already writes
   every set. Keep `assemble`'s channel handling in one place (the corner-run
   expansion in A.8 already threads `uv_channels` per piece) so this is a local
   change later.

## Risks

- **Eigen compile time / footprint**: whole `Eigen/` (~5 MB) rather than a pruned
  subset (Geometry pulls SVD/LU/QR/Householder/Cholesky); MSVC 1-2 min at /O2 on
  the three big TUs; `/bigobj`. `opt_level(2)` everywhere is the test-time mitigation.
- **Determinism across thread counts**: relies on the shim's ordered reduce plus
  Rust-side canonicalization; fallback is single-threaded when `deterministic`.
- **TBB shim**: member set discovered by compiling (compile error, never silent);
  `spin_mutex` non-recursive; pool created once.
- **Non-manifold input to Instant Meshes** completes but may leave holes; Phase C is
  the real fix.
- **Tiny pieces** below the thresholds are left untouched with a warning.
- **Corner-run levels** cost the viewer the corner count in memory (same as the
  source's own buffer); never shown as a stat.
- **Seam extrapolation** can push UVs slightly outside an island; raise the ring or
  clamp to the ring boundary if visible.
- **QuadriFlow `ComputeIndexMap` can fail on valid manifolds**; the runtime
  fallback covers it.
- **Shrinkwrap memory** bounded by block-sparse storage + `MAX_CELLS`; the winding
  far-field tree is O(triangles) per node per run (add a Tracy zone).
