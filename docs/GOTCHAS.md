# Gotchas

Things that have already cost someone a day. Each one is a trap that looks like
working code, or a rule whose reason is not visible from the call site.
[ARCHITECTURE.md](ARCHITECTURE.md) has the crate map and the invariants these sit
under.

---

## Contents

- [Testing and CI](#testing-and-ci)
- [Skinning, animation and deformation](#skinning-animation-and-deformation)
- [GPU struct layout and the render passes](#gpu-struct-layout-and-the-render-passes)
- [Import and the FBX libraries](#import-and-the-fbx-libraries)
- [The Opt workspace](#the-opt-workspace)
- [Baked assets](#baked-assets)

---

## Testing and CI

**A real GPU is required for render checks.** Reserve CI for `cargo check`,
`clippy` and the unit tests. GPU render checks stay manual.

**`crates/optimize` carries two integration suites over the real fixtures** in
`assets/test_models`, both of which have already caught bugs the synthetic demo
cube could not: `tests/real_models.rs` (welds, LOD targets, per-triangle tag
survival, exclusions) and `tests/export_round_trip.rs`, which checks every written
FBX by **reading it back** through the vendored ufbx reader — the writer's own
return value only proves it did not error. Each test skips itself when its fixture
or a vendored tree is absent.

**The shaders are validated by the build, not by a test.** `fxc /WX` (or `xcrun
metal`) in `render`'s `build.rs` rejects a broken shader.

---

## Skinning, animation and deformation

### Rest pose ≠ bind pose

`ModelData::vertices` are world-baked in the **bind** pose, but a skinned model is
*displayed* in the file's **default** pose: the palette at rest is
`bone_rest_world × world_to_bone_bind` per cluster, not identity.

Anything that reads the vertex buffer on the CPU — the dimension-label BVH, the AO
bake, `recompute_bounds` — therefore sees the bind pose. `model.bounds` is measured
through `review_model::anim` at import, and the Opt workspace (which shows the bind
pose on purpose) passes no pose.

### Every mesh-derived overlay must copy its source corner's `deform` lane

The wireframe, normal lines, heat map and selection flash deform only because
their builders take the slot's lanes (`geometry::deform::corner_deform`). A
builder that pushes `NO_DEFORM` for mesh geometry silently draws the bind pose
over the skinned mesh.

Static geometry (grid, bounding box, pivot, UV) is `NO_DEFORM`; the skeleton
overlay uses `node_deform`.

### A clip's bounds cost the *animated* part of the scene, not the mesh

`anim::clip_bounds` re-measures only the corners the clip can move — a node it
tracks or a descendant of one, a bone reached through the skin, a vertex a tracked
blend shape has an offset for — and measures everything else once, at rest, which
is the pose it holds in every frame.

Walking the whole mesh per frame instead is what a game FBX punishes hardest. One
2.8M-triangle exterior carries a 2401-frame `Take 001` whose tracks reach 1.5% of
its triangles, and the naive walk spent **97 s of a 102 s load** re-deriving bounds
it already had — the parse itself is 1.6 s. Note that `needs_deform()` is true for
*any* clip, so a static scene with one leftover take takes this path too.

Keep the two halves in step: `corner_moves` mirrors `deform_corner`'s three
sources, and a new deform source added to one must be added to the other, or its
vertices will be measured at rest and the clip's envelope will come out too small.

### `ufbx_bake_anim` facts

- Linear keys are kept as authored; only cubic segments resample.
- Stepped keys become 1 ms pairs.
- `key_time_min` / `key_time_max` can extend **past** the stack range — so frame
  counts derive from the stack range × fps, and the evaluator holds the end values
  outside the keys.
- Blend-channel weights arrive as the `DeformPercent` element property (percent ÷
  100), and ufbx folds the channel weight into per-keyframe *effective* weights
  (`channel_effective_weights` is a verbatim port). Do not multiply the channel
  weight in again.

---

## GPU struct layout and the render passes

### Field order must match the shader

GPU struct field order must match the shader's uniform-block and vertex-input
layouts (invariant 11). The one shader source is
`crates/render/src/shaders/review.glsl`; update it in lockstep with the
`#[repr(C)]` `SceneUniforms` / `SceneVertex` structs in `scene/gpu_types.rs`, and
mind std140's 16-byte block packing.

Two things catch a mistake: `fxc /WX` in `build.rs` rejects a broken shader, and
each struct's `size_of` assertion against **shdc's generated struct** rejects a
layout that drifted. Re-run `scripts/gen-shaders.ps1` (or `.sh`) after editing the
GLSL, and commit what it writes.

### Structured-buffer structs are invariant-11 territory too

`InfluenceEntry`, `PaletteEntry` and `MorphEntry` in `gpu_types.rs` mirror the
HLSL structs at `t12..t15` byte for byte. The palette is three `float4` rows,
never `float3x4`, because matrix packing differs between cbuffers and structured
buffers. Each has a `const` size assertion.

`scene_vertex_size()` deliberately reports the 64-byte engine-equivalent vertex,
not `SceneVertex`'s 80 bytes — the deform lane is viewer-internal and must not
move the Opt overfetch figure.

### The scene pass is 2-MRT; keep three things in lockstep

The scene geometry pass has **two** colour targets: the scene fragment shaders
write location 0 (linear scene radiance) and location 1 (AO-eligible diffuse
ambient radiance). So every scene pipeline (mesh, line, uv-fill, skybox) declares
*two* colour formats and the offscreen pass binds *two* attachments plus their
MSAA resolves. Keep the three in step.

Both locations alpha-blend. Overlays (zero-normal verts) write 0 to location 1 so
they are not AO-darkened.

GTAO does **not** read location 1: it has its own single-sample mesh-only pass
(`fs_gtao_gbuffer`, one `SV_Target0` output of view normal `xyz` plus view Z `w`)
into a separate G-buffer target, which avoids MSAA edge averaging. GTAO is
horizon-based, with a structured 4×4 spatial dither to decorrelate slices, and
outputs a single scalar occlusion (`R8Unorm`). `post` darkens the diffuse ambient
by that scalar — an *additive correction* over location 0, so MSAA stays correct
and direct and emissive light are never darkened.

Tone mapping and sRGB encoding happen once in `post`, not in the scene shader.

### fxc notes

- Use `SampleLevel` (not `Sample`) for any texture read inside a loop or branch —
  non-uniform control flow.
- Guard a possibly-negative `pow` base with `max(x, 0.0)` so `/WX` does not reject
  it.

### Derived views: follow the ensure/free pattern

CPU-side vertex generation (grid, wireframe, face and vertex normal lines) lives
in `crates/render/src/geometry/`, one file per category;
`scene/resources.rs` (`ModelSlot` / `DerivedViews`) owns the draw list, the GPU
resource cache and the buffer upload.

Derived line views are built-on-demand and freed-on-off by
`SceneGpu::sync_line_views` (invariant 3): a view's buffer exists only while its
toggle is on, and is rebuilt live when its baked length or colour drifts. Add new
debug views by following that pattern.

A `sync_*` must compare against the **borrowed** frame inputs and only `to_vec()`
its bake key on the rebuild path, so a steady-state frame allocates nothing.

Geometry builders that filter hidden nodes go through `geometry::HiddenFilter`
rather than rebuilding the set-plus-length-guard by hand — the guard is what keeps
an out-of-range index out of `model.triangles.node`.

### Keep `model` and the camera/debug math host-agnostic

So that a future renderer swap only touches `render`.

---

## Import and the FBX libraries

### `ufbx_write`'s `ufbxw_view_*` buffers are borrowed, not copied

The pointer must stay valid until the save completes. `export_bridge.c` uses
`ufbxw_copy_*` throughout so nothing is borrowed past the call; reusing one
scratch buffer across meshes with a `view` was an access violation.

### FBX declares its unit rather than fixing one

Conventionally centimeters, while import normalizes every file to meters — so the
writer sets `UnitScaleFactor = 100`. Without it the geometry reads back exactly
100× too small, which no error reports.

### Recovering from a malformed index drops the whole triangle, never one corner

An index buffer is a flat corner stream, so `continue`-ing past a single bad
corner leaves a length that is no longer a multiple of three and shifts every
later triangle by one — plausible-looking garbage rather than a visible failure,
and trimming the tail afterwards does not undo it.

Both `submesh::partition` and `export::build_mesh` check all three corners first
and skip the triangle as a unit. `export` additionally consults the same check
from its per-face material loop, since skipping in one loop and not the other
mis-assigns every subsequent material.

---

## The Opt workspace

### Every run begins with a lossless index pass, and must

Import splits each face corner into its own vertex, so a mesh reaches `optimize`
with *no* shared vertices at all — a real 10,006-triangle asset arrives as 30,018
vertices. Every meshoptimizer operation works through the index buffer, so on that
mesh they are all no-ops: nothing for the vertex cache to reuse, no edge a collapse
may cross, a LOD chain that removes nothing.

`process::index_mesh` therefore merges vertices identical in *every* attribute
before any stack operation runs. They are byte-for-byte survivors, so nothing
visible changes — 30,018 → 5,284 on that asset, after which a 50% LOD target is hit
exactly. It is deliberately **not** a stack operation; skipping it is never useful.
The `Weld` operation is for the *lossy* merges (dropping normals or UVs from the
comparison, or a tolerance).

**The run's baseline (`ProcessedResult::source` / `source_metrics`) is measured
after that pass.** Quoting changes against the corner-split buffer credited the
user's operations with an "-82%" any engine cooker gets for free, and made the
baseline ACMR a meaningless 3.0. The indexed baseline also equals the stats panel's
`GPU Verts`, and a `real_models` test pins the two figures equal.

### A simplify that barely removes anything is usually attribute seams, not a bug

Import splits every face corner into its own vertex, so a mesh whose normals or
UVs differ at every corner — a scan with generated per-face normals — presents
*every* edge as a discontinuity, and a topology-preserving collapse cannot cross
one.

Measured: a real game asset welds 369k → 108k vertices and hits its LOD targets
with the default settings, while such a scan stalls at 249,882 → 249,880 triangles
until either a position weld (normals excluded) or the permissive flag unblocks
it. `process` detects the stall and says so.

The default weld comparing normals is *correct* for the tool's stated target of
static game meshes, so do not "fix" it by loosening the defaults.

### Re-keying operation ids is a two-pass job

`OptStack::reassign_ids` builds the whole old→new map before rewriting a single
per-node override. Rewriting them as the walk consumes the id space lets an
already-rewritten override collide with a later operation's *old* id and be
rewritten twice, silently reattaching the user's per-object settings to the wrong
operation — reachable with one reorder plus a preset round-trip.

---

## Baked assets

### IBL HDRs must stay finite

Bright suns in an HDR exceed `f16`'s maximum (65504), so the bake's
`load_equirect_from_file` clamps every channel to `F16_MAX` before the
`Rgba16Float` upload. Otherwise they become `inf`, the unbounded irradiance
integral turns to `NaN`, and the model shows black speckles with a dead spot at
the sun.

The `ibl_*` programs additionally clamp each *sampled* radiance to
`IBL_RADIANCE_CLAMP` in both convolutions (irradiance and prefilter) to kill
fireflies, and the skybox clamps `env * intensity` to f16 max so the intensity
multiply cannot re-overflow the HDR target. These clamps run at bake time, so the
shipped maps are already finite. Do not drop them.

### Embedded assets must exist before `cargo build`

- **HDR thumbnails** (`assets/thumbnails/T_HDR_*.png`) are `include_bytes!`-embedded
  by `crates/ui`. They are committed;
  `packaging/generate-hdr-thumbnails.ps1` (ImageMagick) regenerates them from
  `assets/textures/T_HDR_*.hdr`, and the installer build runs it automatically.
  Re-run it after adding or replacing an HDR.
- **Baked IBL maps** (`assets/ibl_baked/T_IBL_*.bin`) are `include_bytes!`-embedded
  by `crates/render`. Also committed; `packaging/generate-ibl-bake.ps1`
  regenerates them by running the `bake_ibl` tool, which needs a real GPU.

The three HDR cubes (env, irradiance, prefilter) are **BC6H** block-compressed
(`Bc6hRgbUfloat`, 16 bytes per 4×4 block, encoded at bake time with `intel_tex_2`);
the shared BRDF LUT stays raw little-endian f16 (`Rg16Float`). BC6H is core in
Direct3D 11 feature level 11_0 — the renderer's floor — so no explicit feature
request is needed.

The `.bin` byte layout is **mip-major with the six cube faces contiguous per mip**,
each face a row-major grid of BC6H blocks. The bake's `write_cube` /
`compress_bc6h_face` and the runtime `Texture::cube_block_compressed` must stay in
lockstep on it.

### The IBL bake is freshness-gated

The installer build invokes `generate-ibl-bake.ps1`, but it only runs the GPU bake
when a baked `.bin` is missing or older than an input that determines its bytes — a
source HDR, or `ibl.rs` / `review.glsl` / `bake_ibl.rs` / `rhi/bake.rs`, meaning an
IBL precompute constant or the encode path. Otherwise it is a fast no-op, so a
normal build touches no GPU. Pass `-Force` to re-bake regardless.

The shipping binary carries the baked maps, not the raw HDRs: `T_HDR_*.hdr` are
bake-tool inputs only.

Run the bake in `--release`, because the BC6H encoder's highest-quality profile is
slow. But note that sokol's validation layer is compiled *out* of a release build,
so if the bake misbehaves, reproduce it in debug where a bad call says so.

### Shader bytecode staleness is by content, not timestamp

`src/shaders/generated/bytecode.manifest` records the SHA-256 of each generated
source and of the blob compiled from it, and is committed beside them. Commit it
whenever you commit bytecode.

Timestamps cannot answer this once two OSes commit blobs: a shader rebuilt on the
Mac reaches Windows as a new source next to a blob one revision behind, both
stamped by `git checkout` at the same instant.
`packaging/check-shader-bytecode.{ps1,sh}` fails a release build on any mismatch,
and the installer build runs it. `build.rs` additionally warns when `review.glsl`
is newer than everything generated from it — the one thing the manifest cannot see,
namely that you edited the shader and never ran `scripts/gen-shaders`.
