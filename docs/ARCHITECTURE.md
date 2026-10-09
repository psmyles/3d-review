# Architecture

How 3D Review is put together, and the rules that keep it that way. The
[README](../README.md) covers what the program does and how to build it; this
file covers why the code is shaped the way it is. [GOTCHAS.md](GOTCHAS.md)
collects the traps that have already cost someone a day.

---

## Contents

- [Data flow](#data-flow)
- [Crate map](#crate-map)
- [Invariants](#invariants)
- [Settled decisions](#settled-decisions)
- [Platform decisions](#platform-decisions)

---

## Data flow

Data and commands each flow one way.

```
input / file drop
      ↓
    app  ──→  import  ──→  model  ──→  render  ──→  GPU
      │       (FBX →       (shared    (cameras,
      │       ModelData)   Arc)       resources, passes)
      │                                    ↓
      │                          app opens the frame, the renderer
      │                          records its passes and composites,
      │                          then ui paints its chrome over it
      │                          in the same swapchain pass
      ↑                                    │
      └──────────  UiOutput intents  ──────┘
```

`app` is the only coordinator. It owns the event loop, the redraw timing and
every piece of mutable session state; it drives `import`, hands the resulting
`ModelData` to `render`, and applies the intents `ui` emits. `render` and `model`
never depend on `ui` or `app`.

The Opt workspace branches off at `model`: `optimize` turns a `ModelData` plus an
operation stack into one `ModelData` per LOD level on a worker thread, and those
go back through the same `render` path as the source mesh.

**Model revisions must be unique across every mesh the renderer is handed**, not
merely increasing within one. `SceneGpu` keys its mesh-buffer cache on the
revision alone, so two different meshes sharing a number leaves one of them stale
on screen. `app` issues them all from one counter (`App::next_model_revision`):
the source mesh and each processed Opt level.

---

## Crate map

Nine crates. The boundaries carry real weight — see [Invariants](#invariants) 2,
9 and 10.

### `app` — `review-app`

The winit `ApplicationHandler`, the event loop, and the only place renderer state
is mutated. Fully safe (`#![forbid(unsafe_code)]`), with no platform GPU
dependency: `Gpu::attach` reads the window's raw handle itself.

One `impl App` block per concern, one file each:

| File | Concern |
| --- | --- |
| `main.rs` | `fn main`, the `App` struct, the `ApplicationHandler` impl, window + GPU startup — and nothing else |
| `events.rs` | `UserEvent`: the cross-module message bus every worker posts back through |
| `redraw.rs` | Redraw pacing — `RedrawScheduler` and the `about_to_wait` body that drives it |
| `frame.rs` | The per-frame render loop, including the camera-animation tick |
| `input.rs` | Pointer / scroll / resize / pinch routing, the drag mode, the zoom sensitivities, the framing safe area |
| `shortcuts.rs` | Keyboard dispatch (the help tables in `ui` mirror it) |
| `flycam.rs` | The RMB-held WASD/QE flycam: held-key direction bits, integrated once per frame |
| `loading.rs` | The model-load funnel — drag-drop, Ctrl+O, double-click, CLI/file association |
| `ui_intents.rs` | Applying `UiOutput` intents to the renderer (invariant 2, made concrete) |
| `pick.rs` | The viewport pick: the ray, the per-view geometry, the posed-pick cache |
| `gate.rs` | The performance gate stamp (`--gate-out <file>`) |
| `dialog.rs` | Every native file dialog |
| `texture_manager.rs` | Scene texture pool, off-thread decode, disk auto-reload |
| `animation.rs` | The animation clock and pose evaluation |
| `window_state.rs` | Window position/size restore (saved on exit, never in a gate run), monitor validation, refresh-rate query |
| `settings.rs` | The per-user `settings.cfg`: the locale, **Remember settings** and the option values it keeps, the recent-files list. A read error other than "not found" skips the next save rather than overwriting lines the viewer does not own |
| `recent.rs` | File > Open Recent |
| `update.rs` | Help > Check for Updates, over the OS `curl` on a worker |
| `logging.rs` | The log from `app`'s side: where the day's file lives, the session header, the handover into the Log window |
| `explain.rs` | The optimizer errors, warnings and export notes a user can act on, turned into catalog text (`review_optimize::notice`'s `OptWarning` / `ExportNote` are typed, so every one has a key) |
| `docs_dir.rs` | Where the manual's images live on this install |
| `undo.rs` | The unified undo/redo snapshot stack |
| `opt/` | The Opt workspace: `mod.rs` (`OptSubsystem`, its state and the latest-request-wins rules), `process.rs` (scheduling runs, the worker, folding progress / previews / results back), `export.rs`, `preset.rs` |

**GPU bring-up.** `Gpu::start` runs on the *first line of main*, on its own
thread — device creation needs no window and is the longest single item on the
launch path — and is joined by `GpuBringUp::attach` in `resumed`, which puts the
swapchain on the window.

**Per-frame draw order.** `begin_frame` → the renderer's offscreen passes → egui
tessellation *outside* any pass → the one swapchain pass (composite, then chrome)
→ `finish`.

**Dialogs.** Nothing in this crate may call `rfd` inline from a winit callback: on
macOS a modal run loop entered from one aborts the process. Every dialog opens on
a worker thread and answers through `UserEvent::DialogDone`, one at a time. The
single exception is the startup error box, which `main` raises *after* `run_app`
has returned. A request carries its own subject with it (the LOD chain to export,
the serialized preset), so the answer acts on what the user was looking at when
they asked rather than on whatever the state has since become.

**Model loading is staged.** The worker publishes the model as soon as it is
*drawable* and keeps measuring against the `Arc` it just sent, posting each result
as its own event. See [The staged import](#the-staged-import) below.

**Opt.** `OptSubsystem` is created only on first entry into the workspace, so a
session that never opens it pays nothing. One run is ever in flight: edits
arriving mid-run mark it dirty and it respawns once with the latest stack, so
dragging a slider coalesces without a debounce timer, and a result whose
generation has been superseded is dropped rather than shown. Export runs on its
own worker the same way.

**Background work** starts through `thread::Builder::spawn`, and a failure to
spawn is a notice, never a panic. Each job that shows progress owns its own
activity card: `ui`'s `begin_activity` returns an `ActivityId` that the job's
messages carry and `update_activity` / `end_activity` take, so two jobs running
at once (a load and an Opt rebuild, say) never write into each other's card.

### `model` — `review-model`

Host-agnostic data only. Depends on `glam` and nothing else: no GPU API, no
`egui`, no `winit`, no importer types.

- `lib.rs` — `Vertex`, `Bounds`, `MaterialImportDefaults`, `TopologyFace`,
  `ModelStats`, `TriangleData` (grouped per-triangle face/material/node arrays
  plus a `validate` lockstep guard), `ModelData`, `recompute_bounds`,
  `demo_cube_model`. Plus the deform data: `SceneNode::rest_local` (the per-node
  rest TRS the clips override), `ModelData::corner_to_logical` (render corner →
  DCC vertex), `SkinData` (CSR weights over logical vertices plus the `clusters`
  table carrying each (mesh node, bone) bind matrix), `MorphData` (blend-shape
  channels / keyframes / per-logical-vertex offset CSR), `AnimationClip` (baked
  `NodeTrack`s and `MorphTrack`s and the stack's time range), and
  `ModelData::validate_deform` (the funnel guard for all of it). `SkinCluster`
  also carries the authored `mesh_node_to_bone` / `bind_to_world` matrices and
  the cluster name, for the export.
- `extras.rs` — `SourceExtras`, the source-property capture the exporter gives
  back (see [The source-property capture](#the-source-property-capture)), with
  `validate` and `ExtrasCounts`.
- `anim.rs` — pose evaluation. `AnimContext`, `Pose`, `evaluate_pose`
  (parents-first recomposition, hold outside keys, ufbx's in-between blend rule
  via `channel_effective_weights`), `build_palette` → `DeformPose`, and the CPU
  reference `deform_corner` / `clip_bounds` that the shader and the tests mirror.
- `bvh.rs` — a per-mesh-part triangle BVH, used for occlusion of the dimension
  labels and by the Opt AO bake.

A clip's **motion envelope is not stored here.** It is measured after the model
is on screen and lives in `UiState::clip_bounds` — see
[The staged import](#the-staged-import).

### `import` — `review-import`

`load_model` / `load_model_with_progress` / `load_fbx`, `ImportError`, the unsafe
FFI (`repr(C)` mirror structs, `checked_slice`, `model_from_bridge_scene`), the
vendored ufbx C plus the bridge, and `build.rs` (`cc`, `cfg(has_ufbx)`).

Files: `src/lib.rs` (the public API — errors, the progress and stage types,
the four `load_model*` entry points, `measure_clip_bounds`), `src/startup.rs`
(the Windows `STARTUPINFO` launch hint, which is FFI but not FBX), and the
`#[cfg(has_ufbx)]` `src/ffi/`, layered so the `unsafe` is a leaf rather than a
theme: `ffi/raw_scene.rs` and `ffi/raw_extras.rs` are the `#[repr(C)]` mirrors
(data only), `ffi/bridge.rs` the one call into C plus the `Drop` handles that
own the scene and the capture, `ffi/raw.rs` the unsafe pointer-to-slice and
pointer-to-string helpers and the safe accessor built on each — one method per
bridge array, returning a slice that borrows the struct owning it, which is the
last of the `unsafe` — and `ffi/marshal_model.rs` + `ffi/marshal_extras.rs` the
majority of the crate by line count, which `deny(unsafe_code)` and read only
through those accessors, so no raw pointer reaches them at all. Plus `src/ufbx_bridge.c`, `src/ufbx_bridge.h`,
`src/ufbx_extras.c`, `src/ufbx_extras.h`, `build.rs`. Fixture-driven checks of
the public API live in `tests/fbx_fixtures.rs`.

The bridge captures each node's rest local TRS; the skin cluster table (per
mesh-bearing node: `geometry_to_bone × inverse(geometry_to_world)`) with a
per-influence cluster index and deformer method; the blend shapes (offsets
pre-rotated into baked world orientation); and every animation stack baked with
`ufbx_bake_anim` (file frame rate, no key reduction; `DeformPercent` element
tracks become morph channels) — each with the same two-pass count→fill
discipline. The Rust side sorts the morph entries into a CSR, validates through
`validate_deform`, and measures `bounds` at the rest pose.

The scene loads with `UFBX_INHERIT_MODE_HANDLING_HELPER_NODES` so `parent ×
local` recomposes `node_to_world` exactly.

#### Cancelling a load

A superseded load stops rather than parsing to completion: `app` shares its
load-generation counter with the import worker as a `CancelToken`, the bridge's
progress callback answers ufbx's continue/cancel, and the worker checks the same
token before each post-publish measuring stage. Dropping the result on arrival
was never enough on its own — the parse and the measuring ran either way, which
on a large file is several seconds of work for something nobody will see.

**ufbx reports a cancelled parse two different ways, and only one of them says
"cancelled".** Stopping the plain read path fails with `UFBX_ERROR_CANCELLED`,
but the progress check inside the bit stream (`ufbxi_bit_refill`) only sets an
internal flag and feeds the inflater zeroes, so it surfaces as `Bad DEFLATE data`
— indistinguishable from a genuinely corrupt file. Most binary FBX is
deflate-compressed, so that is the common path, and reading `error.type` alone
turned a load the user had superseded into an error toast. The bridge's progress
context records our own answer and checks that first.

#### The staged import

`load_model_with_progress` stops where the viewport stops caring: geometry,
materials, the scene graph, the rest-pose bounds the camera frames on, and
tangents. It reports an `ImportProgress` per stage as it goes — the `Reading`
stage carries a real fraction, taken from ufbx's own `progress_cb` through the
bridge, which is what puts a moving bar on the loading card for a large file.

Two measurements are deliberately left out, because nothing on screen needs them
and together they are the majority of a large load:

- **each clip's motion envelope** (`measure_clip_bounds`), which only framing and
  the bounding box on a *selected* clip use;
- **the per-draw-group table** (`ModelData::mesh_group_stats`), which
  `stats.gpu_vertex_count` sums — left `0`, which the stats card reads as "not
  measured yet" and simply omits.

`app` makes both on the import worker *after* publishing the mesh and folds each
into the UI as it lands. `load_model` is the complete import, for tests and batch
callers.

On a 2.8M-triangle, 120 MB scene that is the mesh on screen in ~2.1 s, its clip
envelope at ~2.8 s and its stats at ~5.7 s, against ~5.2 s of blank viewport if
the whole thing is measured up front.

#### The source-property capture

Everything the viewer never reads but the exporter must give back is
`review_model::SourceExtras` (`crates/model/src/extras.rs`): every explicit
property of every node / attribute / material / texture / video / mesh / layer
(`Prop`, carrying ufbx's type and flags), rotation order, inherit mode, geometric
transforms, light / camera / null / LOD-group parameters, texture paths and
embedded content, layered textures, every color set, edges with their smoothing /
crease / visibility, face smoothing / holes / groups, vertex creases, subdivision
settings, the second and later skin deformers, dual-quaternion weights, authored
bind poses, display layers, selection sets, scene metadata and settings, and the
**authored animation curves** of every animated property. Every enum is our own
mirror; nothing from ufbx leaks into `model`.

The C side captures it in `ufbx_extras.c` inside the one bridge call (the same
count→fill and the same free as the geometry — invariant 7). The Rust side
marshals it *after* the mesh is published: `load_model_staged` returns the model
plus a `PendingExtras`, `app` runs `marshal` on the import worker
(`ImportStage::Extras`, "Reading source properties") and posts
`UserEvent::SourceExtrasReady`, which lands in `App::scene_extras` and travels
with every Opt run and export request. `validate` drops a malformed capture with
a toast rather than publishing it. Measured cost: ≤3 % of time-to-mesh on the
largest fixtures, so the C entry was not split. `load_model_full` returns
`(ModelData, Option<SourceExtras>)` for tests and batch callers.

### `render` — `review-render`

Cameras, per-view configuration, materials, geometry building, the shaders, and
the whole GPU layer.

**Configuration and cameras.**

- `config.rs` — `ShadingMode`, `VertexColorMode`, `ActiveMaterial`,
  `CameraProjection`, `AntiAliasing`, `EnvironmentSettings`, `GtaoSettings`,
  `TonemapSettings`, `SceneDebugOptions`, `RendererConfig`.
- `camera.rs` — `OrbitCamera` (framing / orbit / pan / zoom, ortho and
  perspective, reversed-Z infinite perspective, plus the flycam's `look` — which
  turns about the eye rather than the pivot — and `fly`), `UvCamera`,
  `CameraTransition`
  (0.3 s ease-in-out cubic) and the shared `ease_in_out_cubic` curve the chrome's
  own animations reuse.
- `lib.rs` — the `Renderer` façade.

**`src/rhi/` — the sole home for the drawing API and for the backend's types.**
Nothing outside `rhi` names an `sg::` type, a pixel format enum or a window
handle. Every resource goes through a wrapper, every pixel format is `Format`,
and every failure is a `GpuError`. That is what makes the renderer's shape
independent of the API underneath it.

| File | Contents |
| --- | --- |
| `mod.rs` | `Gpu` (device + swapchain), `Frame` (one frame in flight, borrowing the `Gpu` so "one frame at a time" is a compile-time fact) |
| `job.rs` | `SwapchainJob`, a draw recorded before the swapchain pass and replayed inside it |
| `make.rs` | Creating a sokol resource validated, with the pool slot freed if creation failed |
| `sokol_log.rs` | The sokol logger — without it validation failures are silent — handing each line to the `log` facade |
| `error.rs` | `GpuError` / `GpuResult` / `ResourceKind`, `require_valid`, the `.resource(kind, label)` combinator |
| `format.rs` | `Format` and `SCENE_*_FORMAT`; its `sg()` is `pub(in crate::rhi)`, which stops an `sg::PixelFormat` leaking out |
| `present.rs` | `PresentStatus` |
| `shader.rs` | The generated `ShaderDesc` with this host's committed bytecode swapped in, and the `bytecode!` macro that picks the per-OS blob |
| `pipeline.rs`, `buffer.rs` | Pipelines; `TransientBuffer` (per-frame geometry stream), immutable `VertexBuffer`/`IndexBuffer`, `StorageBuffer<T>` for the deform tables |
| `texture.rs`, `mips.rs`, `sampler.rs` | Textures, the CPU mip chain, samplers |
| `target.rs` | Offscreen colour + depth attachments, plus the single-sample twin sokol resolves into at 2×+ MSAA |
| `bindings.rs` | What a draw reads, as one value re-applied after every `apply_pipeline` — sokol has no sticky slot state |
| `gpu_profiler.rs` | The `--tracy` GPU profiler: the `Zone` set, the Tracy GPU context and spans, the arming flag and the message channel |
| `bake.rs` | The offline bake's headless sokol device, `CubeTarget`, `Target2D` (feature `bake` only) |

`Frame` carries a `Drop` guard that closes an abandoned pass, so one bad frame
cannot wedge the next. `SwapchainJob` is a draw *recorded* before the swapchain
pass exists and replayed when it opens: every `Renderer::render_*` runs before
that pass and there is only ever one of it per frame, so the composite is
deferred while the offscreen passes are issued directly.

**`src/rhi/backend/` — the device and swapchain leaf, one module per OS**, and the
only platform GPU code in the workspace. `d3d11.rs` creates the device, points
`sg_environment` at it, owns the DXGI flip-model swapchain, hands sokol a
render-target view per frame, presents, and answers `supported_sample_counts`
(sokol only reports MSAA as a yes/no). `metal.rs` does the same with a
`CAMetalLayer`. Each also carries the leaves for what sokol has no notion of:
`GpuTimer` (timestamp queries) and, gated on `bake`, `read_image_subresource`.

The two modules are twins aliased as `backend`, **not a trait**: anything added to
one must be added to the other.

The Windows swapchain is created with `ALLOW_TEARING` where the factory offers it,
and a vsync-**off** present passes the matching flag; without that, a flip-model
`Present(0, 0)` still queues behind DWM and `finish(false)` returns the refresh
interval rather than the frame's own cost. Inert for the viewer, which always
presents with vsync on, but it is what the performance gate measures.
`dxgi_format` is deliberately exhaustive with **no** catch-all arm — the one it
used to have made a wrong-format staging texture, and `CopySubresourceRegion`
between mismatched formats is a silent no-op.

**The rest of `render`.**

- `egui_sokol.rs` — the egui renderer. One program, one interleaved vertex
  stream, a texture per egui id with a **CPU shadow** it is recreated from (sokol
  has no sub-rectangle image update, and egui patches its atlas by sub-rect), and
  a sampler per distinct `TextureOptions`. `prepare` runs outside any pass,
  `paint` inside the swapchain pass, `free_textures` after the frame.
- `geometry/` — CPU vertex generation, one file per category: `vertex`, `grid`,
  `mesh`, `select`, `uv`, the debug views one file per view (`wireframe`,
  `markers` for the pivot and bounding box, `normal_lines`, `uv_seams`), plus
  `deform.rs` (the per-model
  `DeformLayout`: each corner's 16-byte `deform` lane and the influence / morph
  tables it indexes).
- `material/` — the editable per-material table (the `material` uniform block,
  texture bindings 5-11, the anisotropic `material_sampler`) and its path-keyed
  LRU texture cache. The GPU table is keyed on the model revision, the mode and,
  in Unique mode, the part count, so the Opt split's two meshes never share one.
- `texture.rs` — source-texture decode by magic-byte dispatch: PSD via
  `review-psd`, JPEG via zune's fast path, PNG/TGA/TIFF/HDR/BMP/GIF/PNM via the
  `image` crate. Plus filename channel auto-detect.
- `tex.rs` / `tex/gpu.rs` — what the Tex viewport is asked to draw, and its image
  and checker draws (two deferred `SwapchainJob`s, deliberately outside the scene
  MRT/tonemap path so the displayed texel equals the stored texel).
- `scene/` — `gpu.rs` (`SceneGpu`, the frame's sync and pass recording),
  `slot.rs` (`ModelSlot`: per-model GPU state — mesh buffers, derived views,
  selection/visibility draw lists, each with its bake key), `resources.rs` (the
  mesh and UV builds), `line_views.rs` (the derived line overlays),
  `draw_lists.rs` (selection / hover / visibility index orderings),
  `deform_gpu.rs` (the skinning and morph tables), `targets.rs` (`TargetSet`),
  `gtao.rs` + `ao_accum.rs` (ambient occlusion and its convergence), `uv.rs`,
  `pipelines.rs`, `uniforms.rs`, `draw.rs`, `gpu_types.rs` (the `#[repr(C)]`
  uniforms and `SceneVertex`), `opt.rs` (the Opt split and overlay views). Each
  derived resource's `sync_*` sits beside the thing it builds, and every one has
  a free arm on the path that leaves its view (invariant 3).
- `ibl.rs` — HDR image-based lighting. At runtime the env cube, irradiance,
  prefilter and shared BRDF LUT are **loaded** (`IblMaps::from_baked`, a pure
  upload, no startup precompute) from offline-baked assets. The precompute lives
  in the `bake` feature and the `bake_ibl` binary.

`SceneGpu` holds an **active/idle `ModelSlot` pair** so the Opt workspace can keep
the source and processed meshes both resident and alternate between them within a
frame for one `mem::swap`; a single slot would rebuild both meshes on every
alternation. `render` and `render_uv` claim the source slot explicitly.
`render_opt` draws the split — each half into its **own** `TargetSet` sized to
half the viewport, composited into its own half via a viewport rect. Two sets, not
one: the composite is a deferred `SwapchainJob`, so both halves' passes have run
before either composite does and a shared set would show the second view in both.
Two half-width sets cost what one full-width set does, and the second is released
the moment a single view is drawn — as is everything else only Opt uses (the ghost
wireframe, the idle slot's derived views) the moment `render`, `render_uv` or the
Tex path runs (`SceneGpu::release_opt_views`).

**Shaders are one source**: `src/shaders/review.glsl`, all sixteen programs in
sokol-shdc's annotated GLSL. `scripts/gen-shaders.{sh,ps1}` turns it into the
checked-in `src/shaders/generated/` — per-backend HLSL5 and MSL sources plus
shdc's Rust reflection (bind slots, attribute locations, a struct per uniform
block) — and `build.rs` compiles this host's half to committed bytecode beside it,
freshness-gated, with jobs *discovered* by filename rather than listed in a table.
The runtime `include_bytes!`s the bytecode; nothing compiles a shader at run time,
and a broken shader is a build error.

### `ui` — `review-ui`

The egui chrome, built on egui's **native windowing**. Option tools are native
`egui::Window`s (collapsible, closable, non-resizable, multi-open via
`UiState::panels_open`); the Outliner (left) and Inspector (right) are dockable,
resizable `egui::Panel`s; the toolbar and status bar are `egui::Panel` bands.

The side panels are in **every** workspace, and each workspace shows its own
Outliner tabs. `OutlinerTab::available` (`state/outliner.rs`) is the one table:
3D shows Scene, Materials, Textures and Animations (the last only for a file with
clips); UV shows Scene and Textures; Tex shows Textures; Opt shows Scene and
Materials. Each workspace remembers its own tab. The UV viewport lays out the
Outliner's node selection, or every node when nothing is selected, and never a
hidden one (`geometry::UvNodeScope`, part of `sync_uv_view`'s bake key).

A texture is not a scene `Selection` — nothing in the viewport highlights it. A
Textures-tab click makes it the current texture (`texture_view.selected`) and
points the Inspector at it (`UiState::texture_inspected`); any scene selection
hands the Inspector back, through the same `release_stack_selection` that already
did so for the Opt stack pane.

`draw_overlay` takes the frame's root `&mut Ui` (egui shows panels into a `Ui`,
not onto the `Context`) and carves the bands out of it; floating chrome — `Window`,
`Area`, the layer painters — still addresses `ui.ctx()`.

The 3D/UV scene and the composite are drawn by the renderer into the same frame
*before* egui, which paints its chrome over them with a transparent central
viewport. The Tex viewport (`texture_view.rs`) handles only interaction (pan,
zoom, fit, background fill, channel pick) in the central area the side panels
leave, and hands the image to the renderer: `ui` owns no GPU state. Its image is
picked in the Textures tab, and its measured properties are in the Inspector.

Modules: `theme`, `state`, `assets`, `widgets`, `overlay`, `toolbar`,
`status_bar`, `stats`, `texture_view`, `gizmo`, `dimensions`, `help`, `transport`
(the playback controls, drawn in the status bar's centre span only while a clip
is selected in the 3D workspace), `notifications`, plus `panels/` (one file per tool: `anti_aliasing`,
`bounding_box`, `environment`, `normals`, `gtao`, `tonemap`, `uv_checker`,
`vertex_colors`, `wireframe`, `material_mode`; plus `inspector`, `outliner`, and
`opt_stack` / `opt_inspector`). The Outliner is itself a directory —
`panels/outliner/{mod,tree,rows,nav,materials,textures,animations}.rs`. Pooled-texture
thumbnails, shared by the Textures tab and the Inspector, are `widgets/thumbnail.rs`.

**Notifications.** `notifications.rs` is ours, drawn on plain egui —
`egui-notify` is gone, because it slid every toast in and out with no way to stop
it, anchored only to the four screen corners, and turned a multi-part result into
one box per line. One `egui::Area` anchored bottom-centre of the free viewport
(offset by `UiState::chrome_insets`, the open side panels' widths) holds a column
of cards. A card is a header over a body: the header carries only the kind —
`Warning`, `Success`, `Error`, `Info`, or `Working` while a job runs — then a
divider edge to edge, then the message wrapped beneath it. The message used to be
the header, clipped to one line, which threw away the half of an error that said
what went wrong. `mode` notices stay a bare single line (`Notice::compact`).

The styling is egui's own throughout rather than a second look: `Frame::window`
for the card, `spacing.window_margin` for the header's and body's padding (the
card's own frame has none, which is what lets the divider span the full width),
one `TextStyle::Heading` for the header row, `window_stroke` for the divider's
colour and thickness, egui's two-stroke close glyph, `text_color` /
`weak_text_color` for the body, `egui::ProgressBar` for the bar, and
`error_fg_color` / `warn_fg_color` / `hyperlink_color` / `strong_text_color` for
the kind tints. Only `NOTICE_SUCCESS` is ours, because egui has no green. `success`/`info`/`mode` expire
after `motion::NOTIFICATION_EVENT`; `warning`/`error` and any `report(kind, title,
lines)` with a body stay until dismissed. A keyed push (`mode`, `error_keyed`)
rewrites its slot in place. Deadlines run on egui's clock and resolve on the first
frame a card is shown, hovering pauses them, and one `request_repaint_after` per
frame keeps a sticky card off the redraw loop. `begin_activity` names a background
job; `update_activity` rewrites the stage line and the bar in place.

**Opt state.** `opt_state.rs` holds the `OptStack` the chrome edits, the
comparison-view settings and the last run's measured figures. Ownership follows
the convention already used for selection and the hidden-mesh set: `UiState` owns
it, the chrome edits it in place, and bumping `stack_revision` is what tells `app`
to reprocess and what undo compares. Only the file-dialog actions (export, preset
load/save) travel as an `OptIntent`.

Three modules exist for invariant 12: `labels.rs` maps every foreign enum variant
to a catalog message, `widgets/tooltip.rs` is the one tooltip shape (title,
paragraph, manual link), and `help.rs` is the in-app manual — a native
`egui::Window` over `docs::Page`, the page table this crate's `build.rs` generates
from `docs/book/src`. Page *text* is compiled in, so Help works on a broken
install; only images are read from disk, and one that is not there renders as its
alt text.

### `optimize` — `review-optimize`

Mesh optimization over vendored meshoptimizer. Depends on `review-model` only —
it hands back plain `ModelData` and never touches GPU or UI types.

Layers, bottom up:

1. `ffi.rs` (raw `extern "C"` declarations) and `meshopt/` (the checked safe
   wrappers) — together the entire unsafe surface for meshoptimizer. The option
   bitmasks (`meshopt/options.rs`) sit outside the `has_meshopt` gate, since the
   stack's parameter types need them in either build; `build.rs` pins the header
   version and every bound declaration.
2. `submesh/` — splits a `ModelData` into per-(node, material) pieces, the unit
   that can survive a simplify, since meshoptimizer returns a new index buffer
   with no triangle correspondence.
3. `ops.rs` — one function per operation; `remesh/`, `shrinkwrap/`, `shading.rs`
   and `ao.rs` for the operations too large for one function.
4. `process/` — indexes the mesh losslessly (see
   [GOTCHAS](GOTCHAS.md#every-opt-run-begins-with-a-lossless-index-pass), this is
   what makes any meshoptimizer call do anything at all), then walks the stack,
   fans out the LOD chain, reassembles a `ModelData` per level and measures it,
   alongside the source's own buffer counts so the overlay's deltas subtract like
   from like.

`shading.rs` owns the derived shading bases. meshoptimizer's normal and tangent
generators answer per corner; `Submesh::split_by_corner` turns that into vertices
on the pieces, before `assemble` (see
[GOTCHAS](GOTCHAS.md#normals-and-tangents-are-per-corner-the-split-happens-on-the-pieces)).
It holds the Recalculate Normals operation and `finish_bases`, each level's last
step: smooth normals a normal-blind weld left stale, then rebuild tangents when
any operation invalidated them.

`stack/` is the serializable operation stack the UI edits, and `stack/limits.rs`
the range every numeric setting may take — one table read by both the
inspector's sliders and `OptStack::sanitize`, which clamps a preset loaded from
disk into the same ranges (a preset is a file anyone can edit). `preset.rs` is its
versioned JSON envelope. The two operations that simplify share one
`SimplifySettings` (flattened on the wire, so older presets still load) and one
`process::simplify_submeshes` — the LOD op fans its output out into levels,
`Reduce` writes its own back in place.

**Everything the stack did not change is carried through.** A `Submesh` holds,
beside its vertices, the per-vertex rows a vertex remap must follow
(`VertexRows`: skin, extra skin layers, DQ weights, blend-shape offsets; plus
extra color sets and vertex creases) and a `PolygonCarry` — the source's faces
and edges with their smoothing / crease / hole / group / visibility layers, over
local vertices. Operations keep them by class (see
[GOTCHAS](GOTCHAS.md#polygons-ride-the-pipeline-as-a-carry-and-each-operation-class-treats-it-one-way)):
a vertex remap gathers rows and rewrites corners, an operation that keeps
triangles whole reconciles the polygons by triangle content, a simplify clears
them. `assemble` hands each level back as a `ModelData` that *deforms* — its own
`SkinData` / `MorphData` over the level's vertices, the source's clips cloned,
bounds at rest — plus a `LevelCarry` of what the renderer has no use for
(polygons, color sets, creases, extra skins, DQ weights, a source-corner map).
The processed `ModelData` still leaves `faces` / `triangles.to_face` empty: the
corner-run layout the renderer assumes cannot survive a weld, so the polygons
reach only the exporter.

**Retopology** (`remesh/`) is our own: partition the surface into even regions,
collapse each to one vertex, clean up, pair triangles into quads, and project the
source's attributes back on (`remesh/solve.rs` is the per-object order; see
[Settled decisions](#settled-decisions) and [GOTCHAS](GOTCHAS.md#remesh)).
`shrinkwrap/` rebuilds a closed shell around an object, from a distance field or
voxels. Both run one object per core through `parallel.rs`, which also holds
`sweep` / `sweep_reduce`: the parallel pass the stages *inside* one object are
built from, over a **fixed** 65536-element chunk so a reduction folds per chunk
and merges in chunk order. That constant, not the thread count, is what the
arithmetic depends on — it is the whole determinism argument — so a sweep's
chunks must be whole elements (a per-vertex `[f64; 3]`, never a flat `f64`
stream, where 65536 is not a multiple of three). Each object gets its share of
the machine (`cores / objects`) for its own inner parallelism. Cancellation is
checked between passes (`cancel.rs`'s `CancelToken`, over the same generation
counter `app` supersedes runs by), so a cancelled rebuild costs at most the pass
in flight.

`notice.rs` holds what a run or an export has to say about its result as values
(`OptWarning`, `ExportNote`) rather than sentences, so `app` renders them through
the catalog (invariant 12). `replace_file.rs` writes a file by replacing it, so a
failed write never destroys what was there.

`export/` and `export_bridge.c` write a LOD chain out as FBX via vendored
ufbx_write (suffixed siblings in one file or one file per level, rebuilt or
flattened hierarchy), giving back **every source property the stack did not
change**: the authored node graph (props, pivots, rotation order, inherit type,
geometric transforms, user properties with the `U` flag; ufbx's synthetic helper
nodes are skipped and their children re-parented), bones / lights / cameras /
nulls / LOD groups, materials as authored (Lambert / Phong / custom shading
model, every prop, textures with their paths as authored, embedded content,
layered textures — a source without a capture gets Phong with
`ShininessExponent = (10·smoothness)²` and `ReflectionFactor = metallic`), quads
and n-gons wherever no operation rebuilt the buffer, every color set and topology
layer, authored tangents (synthesized ones are not written), skins with their
authored cluster matrices, extra layers, DQ weights and bind poses, blend shapes,
the authored animation curves of every animated property, display layers,
selection sets, and the scene settings and metadata. The FBX version is always
7700. `ExportReport.notes` lists only genuine losses: a level written as
triangles because a simplify rebuilt it, an unmapped animation target, an export
issued before the capture landed. Output is written in the source file's own
unit (`UnitScale`), since import normalizes every file to meters.

`ufbxw_probe.c` (non-release profiles only, `cfg(has_ufbxw_probe)`) writes a
test-only scene exercising every writer patch; `tests/ufbxw_patches.rs` reads it
back through ufbx.

### `localization` — `review-localization` and `localization-build` — `review-localization-build`

The localization system behind invariant 12, split in two because the typed
message keys have to be generated *inside* each consuming crate: an unused `pub
const` in a library warns about nothing, but an unused private one is `dead_code`,
and that is what stops the catalog filling up with messages no screen shows.

`review-localization` is the runtime — `Key`, `tr`, `tr_args`, `init`, the negotiated
bundle chain, and the embedded catalogs. Its `build.rs` embeds every locale's
`.ftl` files and cross-checks each translation against English: an id English does
not define, or a message whose variables differ from the English ones, fails the
build. Two Fluent defaults are overridden and both matter — `set_use_isolating`
is off, because the isolation marks Fluent brackets variables in have no glyph in
either bundled font, and the bundle is built with `new_concurrent`, because the
default one memoizes in a `RefCell` and a `OnceLock` cannot hold a `!Sync` value.

`review-localization-build` is the parser, used by every consumer's `build.rs`. It depends
on `fluent-syntax` alone — a build script should compile the parser and nothing
else — and it is the same parser the runtime loads through, so "it built" means
"it loads". `generate_keys` writes a `Key` const per message and attribute, plus a
typed formatter for every message that takes variables, so a missing argument is a
compile error rather than a `{$name}` left in the text.

The English catalog is `crates/localization/locales/en/*.ftl`, one file per area. Every
id is prefixed with its file's stem: that is what keeps ids unique inside the one
flat bundle, what the generated const name drops, and what lets one catalog serve
three crates without handing each of them the others' keys (`ui` takes `common-`
and `ui-`, `app` takes `app-`, `shell-macos` takes `menu-`).

### `annotate` — `review-annotate`

Review comments stored **inside the FBX**, in pure safe Rust with no ufbx, GPU or
window dependency — so the viewer, the `review-comments` CLI (`src/bin`) and any
other tool read a file's comments through exactly the same code. The format other
tools read against is `docs/review-comments-format.md`: a `KString` user property
`ReviewComments` (flag `U`) on the `Model` each thread is about, holding versioned
JSON.

- `fbx/` reads and **losslessly patches** that property. A binary file is parsed
  into a node tree whose property bytes stay opaque slices (compressed arrays are
  never inflated) and rewritten with every record's absolute end offset
  recomputed; an unedited rewrite is byte-identical to the input, and the footer
  is re-padded at the new length keeping its writer's convention (the Autodesk SDK
  pads 4 bytes more than Blender). An ASCII file is spliced one line at a time;
  `&` in the stored JSON is written as `\u0026`, since ASCII FBX has no escape for
  it. Reading a binary file **streams** it (`scan_file`): it seeks from record to
  record by their end offsets and reads only the `Model` and `Connections`
  records, so the viewer reads a 200 MB file's comments without a second copy of
  it in memory.
- `thread` / `codec`: the thread types and their JSON. Unknown fields, unknown
  anchor kinds and threads that fail to decode are all kept verbatim and written
  back, and a payload from a newer version is shown read-only — an older viewer
  never destroys what it doesn't understand.
- `mapping` pairs the file's objects with the importer's nodes. **ufbx lists nodes
  by depth, not in file order** (measured on a rigged fixture) and adds nodes the
  file never had, so the pairing walks both trees from the root and matches each
  parent's children by name, *k*-th to *k*-th, stepping over synthetic nodes.
- `report` lists the comments of one file or many as JSON or Markdown, and
  `baseline` narrows a listing to what changed since an earlier `--json` one
  (the CLI's `--baseline`). It matches threads by `id` across every file, so a
  moved file reports nothing new, and messages by author, time and text, never
  by time alone: `time` is the writer's clock, so a reply written before the
  last check and pushed after it would otherwise be missed.

`app/src/comments.rs` reads them on the import worker, right after the source
properties (whose synthetic-node flags the mapping needs), and posts them as their
own event; each frame it resolves every anchor to a world point for the pins — a
surface pin through the current pose, falling back to its stored local point when
the host mesh's polygon/vertex counts no longer match the ones recorded with it.

`tests/fixtures.rs` runs the patcher over every file in `assets/test_models`:
byte-identical unedited rewrites, every imported node mapped, patched files read
back identically by this crate *and* by ufbx with the scene otherwise unchanged,
removal restoring the original bytes, and the streaming scan agreeing with the
in-memory one.

### `psd` — `review-psd`

A safe `decode_psd` over a C-ABI bridge to psd_sdk (C++), returning a PSD's merged
composite as RGBA8. Vendors psd_sdk as **source** and compiles it with `cc`
alongside the C-ABI `wrapper.cpp`; `bindings.rs` is committed Rust hand-kept
against `wrapper.h`, so no bindgen or libclang runs at build time.

The decode is staged — open (header only), check, decode, read — so the wrapper
refuses what psd_sdk would mishandle *before* psd_sdk allocates for it: only 8,
16 and 32-bit Grayscale or RGB, 1 to 56 channels, and the same size caps the Rust
side applies (`PsdError::Unsupported`). The vendored tree is **patched**, not an
unmodified snapshot: its RLE decoder was unchecked and its image-data section
sized buffers with 32-bit arithmetic, and `vendor/review.patch` + `NOTICE.txt`
record the bounds checks added to both. `tests/synthetic.rs` builds malformed
PSDs in-test (grayscale with alpha, 1-bit, too many channels, truncated RLE) and
requires each to decode correctly or be refused with an error - never a crash.

### `shell-macos` — `review-shell-macos`

The two macOS shell pieces: the Launch Services open hook and the menu bar. Every
entry point is a no-op stub off macOS, so it is depended on unconditionally with
no `cfg` at the call site.

### `prof` — `review-prof`

The guarded Tracy helpers every instrumented crate shares: `zone!`, `plot!`,
`frame_mark`, `thread_name`, `msg`. `tracy_client`'s own macros panic when no
client is running, so each wrapper checks `Client::running()` first. It re-exports
`tracy_client`, and the macros reach it as `$crate::tracy_client`, so a crate that
only opens zones needs no tracy dependency of its own. Each consumer's `prof.rs`
is a re-export of this crate, not a copy.

### Vendored trees

| Path | What |
| --- | --- |
| `third_party/ufbx` | `ufbx.c` / `ufbx.h` — FBX reading |
| `third_party/meshoptimizer` | The Opt operations |
| `third_party/ufbx-write` | `ufbx_write.c` / `.h` — the Opt FBX export. **Patched** (`review.patch`, listed in `NOTICE.txt`): property flags + explicit values, the `Null` attribute, topology layers, LOD groups + layered textures, curve nodes with only the curves asked for |
| `crates/psd/vendor/Psd` | PSD decoding |
| `vendor/sokol-rust` | The graphics API bindings |

The first three are compiled only if present, and the workspace still builds
without them; the dependent features report themselves unavailable.

---


### File layout

One file per concern, and a file that outgrows its concern gets split into a
directory module rather than a longer file. The parent keeps the type or entry
point the module is *for* and re-exports its children, so a split never changes a
path outside the crate — `review_model::ModelData`, `review_ui::UiState`,
`crate::process::ProcessedLod` all resolve exactly as they did when each was one
file. Tests live beside the code they exercise, not beside the code they happen
to have been written next to.

Two files are deliberately long and stay that way. `optimize/src/export/write.rs`
is one 650-line function because it is a strict lifetime chain — each `Vec` of
bridge structs borrows the storage of the one above it, so every local has to
live in one stack frame. The C bridges (`import/src/ufbx_bridge.c`,
`optimize/src/export_bridge.c`) are one translation unit each because every
helper in them is `static` and the crossing into C is meant to be one call.

## Invariants

> **Lean on the existing system; don't hand-roll another.** Before building any
> mechanism, reach for what is already there — first the platform or framework
> primitive (egui's native `Window` / `Panel` / `Grid` / `Slider`, winit
> facilities, an `rhi` wrapper), then an existing helper in this codebase (a
> `theme` token, a `widgets` primitive, an `import` or `render` funnel). The UI
> chrome was *migrated away* from a hand-rolled `egui::Area` and pixel-rect layout
> system precisely because re-implementing window management by hand made every
> new panel come out broken. If the existing system genuinely cannot do the job,
> say so and extend it in one place rather than forking a parallel one.

**1. Single source of geometry; no defensive copies.** `ModelData` owns the mesh
buffers. They are uploaded to the GPU by `bytemuck` cast and shared as
`Arc<ModelData>`. Never `.clone()` or re-`Vec` the geometry to build a debug view
— derived structures (wireframe, normal lines) index *into* the shared
vertex/index/topology data.

**2. Data and commands flow one way each.** `ui` never owns or mutates renderer or
model internals: it holds plain values plus displayed stats and emits `UiOutput`
*intents*. `app` is the only coordinator that applies those intents to the
`Renderer`. Commands go UI→app; data goes app→UI. `render` and `model` must not
depend on `ui` or `app`.

**3. Derived-on-demand, free-on-switch.** Anything a debug view needs (wireframe
edges, face-normal lines, vertex-normal lines) is built when that view turns on
and dropped when it turns off. The steady-state shaded view holds **zero** derived
buffers. Implemented for the line views by `SceneGpu::sync_line_views` in
`render/src/scene/line_views.rs`, which is where every other overlay's
`sync_*` / `release_*` pair lives too (`scene/resources.rs` keeps the mesh
upload, `scene/slot.rs` the per-model state and bake keys, `scene/draw_lists.rs`
the selection and visibility index ordering). A builder reached from only *one* viewport still needs its free
arm on the path that leaves that viewport: `sync_uv_view` is called from
`render_uv` alone, so `release_uv_views` runs from the 3D path's `sync_frame`.

**4. Heavy derived views are GPU compute and capability-gated.** Compute-based
views (none ship yet) read buffers already on the GPU and write transient storage
buffers. Gate them on what the device reports — sokol's `query_features` /
`query_limits` / `query_pixelformat` — and disable them in the UI when the active
device cannot run them. Never crash. The one capability gate today is the scene
MSAA picker, which asks `backend::supported_sample_counts` because sokol reports
MSAA only as a yes/no; the texture path already asks `query_limits` for the
largest 2D image the device takes.

**5. Faithful stats.** The Model Stats panel reads source DCC counts carried
through import in `ModelStats` (original polygon and vertex counts), **never**
post-triangulation render counts. Every stat shown must be a real measured value.
`stats_grid` shows only measured values (Draws, Polys, Tris, Verts, GPU Verts,
Vtx Splits, UV Sets, FPS); `fps` is fed from the `app` render loop. Do not
reintroduce placeholder rows. **The viewer's corner-split vertex buffer is an
internal layout, not a stat**: the panel reports the DCC count (`vertex_count`)
and the engine cost (`gpu_vertex_count`, unique vertices per draw group via
`ModelData::count_gpu_vertices`); the corner count is surfaced nowhere.

**6. The redraw loop lives in `app`.** Redraw is driven by winit events and active
camera animation — redraw-on-demand, never by UI state mutation. Continuous
redraw only while a camera transition, an interaction or a clip is live.

**7. One C extraction, one funnel.** The `ufbx_scene → flat arrays` walk is
written once in `crates/import/src/ufbx_bridge.c` and is the *only* path FBX
import takes. Any future caller of that C core must produce identical
`ModelData`. Keep the two-pass count→fill discipline and free every buffer on
every error path.

**8. No hardcoded visual values in UI components.** Every colour, font size,
border width, radius, spacing, opacity and animation duration in `crates/ui` must
come from the central semantic theme, not an inline literal. The only exception is
a value computed at runtime from state (a per-axis gizmo colour, say). Token names
are **semantic** (`panel_bg`, `selection`, `gizmo_ball`), not `dark_grey_6`.
Tokens live in `crates/ui/src/theme/` — one file per group (`color.rs`,
`size.rs`, `font.rs`, `motion.rs`) with `theme/mod.rs` holding `apply_visuals`
and the conversions; add the token there first, then reference it.
**Every `size` and `font` token is in egui points and is used raw** — a point is
already the DPI-independent unit, since egui multiplies by `pixels_per_point` when
it rasterizes, so there is nothing to convert. `theme::chrome_height()` is the one
derived value, and it takes no scale. The tokens used to be "design pixels" divided
by `pixels_per_point` at the use site; that is a second correction on top of egui's
and cancels it, pinning the hand-painted chrome to physical pixels so it halved in
apparent size on a 2× display while the native `Window`/`Panel` chrome beside it
did not. Don't reintroduce a px-to-point conversion.

**9. All unsafe and all C/FFI lives in `import`, `psd` and `optimize`** — plus the
scoped GPU site below and `shell-macos`'s one Objective-C hook. No unsafe leaks
into `model` or `ui`, nor into `render`'s geometry, material or camera modules.
Before `slice::from_raw_parts`, null-check the pointer and treat len 0 as empty
(`checked_slice`). Free the C scene on **both** success and error paths.

**The compiler enforces it.** `app`, `model`, `ui`, `log`, `prof` and
`localization` are `#![forbid(unsafe_code)]`. `render`, `import` and `optimize`
are `#![deny(unsafe_code)]` at the crate root, and each sanctioned module opts
back in with a scoped `#![allow(unsafe_code, reason = "...")]` saying why — so a
new `unsafe` anywhere else is a build error, and the list of `allow`s *is* the
list of unsafe modules:

- `import`: `ffi/bridge.rs` (the one call into C and the handles that free what
  it returns), `ffi/raw.rs` (pointer-to-slice leaves), `startup.rs` (the Windows
  launch hint), `tracy_alloc.rs` (the `GlobalAlloc` wrapper). The marshallers are
  `deny`, and read only through `raw.rs`'s safe accessors.
- `optimize`: `ffi.rs` + `meshopt/`, `export_ffi.rs` + `export/write.rs`, and the
  test-only `probe.rs`.
- `render`: `rhi/backend/{d3d11,metal}.rs` and the `extern "C"` logger callback
  in `rhi/sokol_log.rs`.

The workspace also denies `clippy::undocumented_unsafe_blocks`, which is what
makes "every unsafe carries a `// SAFETY:` rationale" a build error rather than a
convention.

The sanctioned exceptions:

- **psd_sdk FFI** (`crates/psd`). The C-ABI bridge to vendored psd_sdk C++ that
  decodes a PSD's merged composite, so the viewer reads layered PSDs without a
  bundled ImageMagick. The unsafe is confined to `decode_psd`, which validates
  header dimensions with checked arithmetic before sizing the output buffer.
  `render`'s `texture.rs` calls it through the safe API only.
- **meshoptimizer + ufbx_write FFI** (`crates/optimize`). The two libraries are
  bound differently *because their APIs differ*. meshoptimizer's is already flat
  over raw pointers, so `ffi.rs` declares its entry points directly and
  `meshopt/` holds every call — each wrapper validating the mesh preconditions
  before it (whole-triangle index buffers, in-range indices, exact stream lengths,
  `checked_mul` destination sizes) and re-validating the returned element count
  after. ufbx_write's is handle-and-setter-based, so driving it from Rust would
  spread unsafe across a hundred call sites and leak its scene lifetime into Rust;
  instead `export_bridge.c` does the whole write in one call taking flat arrays,
  validating the payload up front and freeing the scene on every path. Those
  modules plus `export/write.rs`'s single call site are the whole unsafe surface:
  `ops`, `process`, `stack`, `preset` and `submesh` are ordinary safe Rust, and
  the crate `forbid`s `unsafe_code` outright when neither vendored tree is
  present.
- **The platform GPU leaf** (`crates/render/src/rhi/backend/`) is the only place
  the renderer's unsafe lives, and it is a few hundred lines per OS. Everything
  sokol_gfx draws with — pipelines, buffers, targets, textures, samplers — is safe
  Rust over its C API, so what used to be ~2000 lines of pervasive COM unsafe is
  now that leaf plus the one `extern "C"` logger callback in `rhi/sokol_log.rs`.

Every unsafe in `render` touches only GPU plumbing — never model geometry, camera
math or material logic, which stay safe.

**10. `model` is host-agnostic.** It depends only on `glam` — no GPU API, `egui`,
`winit` or importer types. This is what kept the renderer swappable (wgpu → D3D11
→ sokol_gfx); do not add rendering or UI dependencies to `model`.

**11. GPU structs are `#[repr(C)]` plus `bytemuck` `Pod`/`Zeroable`.** Match the
shader's uniform-block and vertex-input layout exactly (std140's 16-byte block
packing); do not reorder fields without updating `review.glsl`. Nothing about this
is caught at run time: a uniform upload is sized from the Rust struct and only
rejects a payload *larger* than the block, so a field added on one side alone
uploads happily and the shader reads every later field shifted.

Every such struct therefore carries **two** `const` assertions beside it: a
literal `size_of::<T>() == N`, and `size_of::<T>() == size_of::<generated::T>()`
against shdc's reflection. The second is what pins the invariant to the *shader*
rather than to a hand-typed number. Add both with the struct; they are the only
thing that turns this invariant into a build error. Vertex attribute locations are
pinned the same way, by a test asserting the generated `ATTR_*` constants against
the layout every program sharing `vs_main` expects. Within the scene programs
each uniform block has its own slot (scene VS 0, scene FS 1, material 2), so a
scene draw never binds one slot to two structs. The single-purpose programs —
post, GTAO, the depth chain, Tex, egui, the bake — each bind their own block at 0,
and nothing draws with two of them at once.

**12. No inline user-visible strings; one documentation source.** Every string a
user can read — a label, a tooltip, a panel title, a notice, a dialog title or
filter, the window title, a menu item — is a Fluent message in
`crates/localization/locales/<lang>/*.ftl`, reached through a **generated typed key**
(`keys::<file>::NAME`, or the typed formatter for a message that takes variables),
never a literal at the call site.

`review-localization-build` turns the English catalog into those keys inside each
consuming crate at build time, so a key that does not exist is a compile error and
one that nothing names is a `dead_code` warning that `-D warnings` turns into a
failed build. A message that takes variables gets no key constant at all, only its
typed formatter — so the formatter is what goes unused, and is reported, when no
screen shows the message. `crates/localization/tests/no_inline_strings.rs` parses
`ui`, `app` and `shell-macos` and fails on a literal handed to any text sink —
egui's widgets and builders, `painter.text`, `LayoutJob`, the window title — or
routed there through a `let` or a formatter argument; it carries self-tests for
each sink, since a gate that silently misses one is worse than none. A literal
that genuinely is not user-visible is marked `// localization: exempt <reason>`.

Widgets take `impl Into<egui::WidgetText>`, into which a `Key` converts, and a
runtime value — a file name, a count — is formatted *through* a message with a
variable rather than concatenated beside one. A control's tooltip is a `Tip`: a
title, a paragraph saying what the control is for, and a link to its manual page.

**Catalog and manual are written in characters a keyboard can type**, and two
tests keep them that way (`localization/tests/typeable_characters.rs`,
`help_pages.rs::every_page_is_typeable_ascii`): a hyphen for every dash, three
dots for an ellipsis, `x` for a multiplication sign. Typographic punctuation
cannot be typed by whoever edits the line next, so it comes back mixed with the
ASCII equivalent one line up; it is invisible in a diff; and it depends on the
bundled fonts having the glyph, which is exactly what made Fluent's own isolation
marks render as tofu. Icon glyphs are not catalog text and are exempt — the Opt
stack's reorder arrows, the remove mark, the Help window's Back arrow — because
they stand in for icons and have no ASCII form that does not look broken.

**A catalog is wrapped for reading, and the runtime unwraps it.** Fluent keeps
every newline a multiline value was written with, so a description wrapped at the
file's margin arrived as hard lines and broke a tooltip a third of the way across
its width. `review_localization`'s `unwrap_source_wrapping` applies markdown's rule
instead: a single newline is the source's wrapping and becomes a space, a blank
line is a deliberate paragraph break and survives. Text reaches a widget as one
run and the widget breaks it where it actually runs out of room.

**Errors are the one deliberate exception.** `ImportError`, `OptError`,
`GpuError` and `PsdError` keep their English `Display`: they are diagnostics, they
are what a bug report quotes, and their crates must not learn about catalogs to
say them. `app` localizes the *lead* line and passes the diagnostic through as
`{ $detail }`; the three optimizer errors a user can act on get their own text in
`app/src/explain.rs`. What a run or an export has to *say* — a warning, a note
about what the file lost — is not an error, so it is a typed value
(`review_optimize::OptWarning` / `ExportNote`) that `explain.rs` renders through
the catalog, exhaustively. Enums in `render` / `optimize` / `model` / `import` keep
`label()` as a stable English identifier for logs and reports, and get their
display name through `ui`'s `labels.rs` map — which is exhaustive, so a variant
added upstream is a compile error rather than an English word on a translated
screen.

**The manual is written once.** It is the mdBook under `docs/book/src/<lang>/`,
and that same markdown is embedded into `ui` at build time and shown by the Help
window, by every panel's `?`, and by `F1`. A page is addressed by its file path
(`Page::PanelsAmbientOcclusion`), so a panel naming a page that does not exist, or
a page linking to one, is a build error rather than a 404 on the published site.
Only images ship on disk (`docs/<lang>/images/`, found by `app`'s `docs_dir()`);
the text never depends on the install being intact.

---

## Settled decisions

These were argued once and are not worth reopening without new evidence.

**Spend development time freely to make the release runtime fast, efficient and
accurate.** Offline and bake-time cost — CPU, wall-clock, build complexity — is
*not* a constraint; the shipped binary's speed, memory and VRAM footprint, and
output accuracy are what matter. Always prefer the highest-quality,
most-precomputed option for anything done ahead of runtime: the IBL maps are baked
offline with no startup precompute, and the BC6H cubes use the encoder's slowest,
**highest-quality** profile, because that work happens once on a dev machine and
only sharpens what runs.

**Native stack: direct winit plus sokol_gfx as the one drawing API on both
Windows and macOS** (Direct3D 11 and Metal underneath), with one small device
and swapchain leaf per OS. Not `eframe`, not wgpu, and not a second native shell
per platform. `egui` is an overlay drawn by our own sokol renderer.

The renderer was migrated off wgpu first, to shrink the shipped binary, startup
time and baseline RAM — one device creation instead of probing every DX12 adapter.
The workspace carries **zero** wgpu, naga, pollster or egui-wgpu. That ban stands,
and so does the "no second shell" one: **do not reintroduce wgpu, and do not add a
per-OS renderer.** Anything sokol genuinely cannot do becomes a named leaf beside
the device (mips, timestamp queries, readback, exact MSAA counts), not a fork.

**UI chrome uses egui's native windowing** (`Window` / `Panel`) and stock widgets
(`Grid` / `Slider` / `DragValue` / `ComboBox`), styled from egui's default
`Visuals::dark()` plus a few theme-token overrides. The old hand-rolled
`egui::Area` and pixel-rect panel system is gone; do not bring it back. Styling
exceptions: keep the bundled Inter (proportional) and JetBrains Mono fonts, and
numeric value boxes (`DragValue`) render in monospace while everything else is
proportional.

**The internal format is our own `ModelData` superset** — flat parallel buffers
plus original face topology plus source stats. FBX is the only import format,
parsed by vendored ufbx through a single C bridge. Do not round-trip through glTF:
it drops quad topology and changes vertex counts.

**Shading is one scene program** (`@program mesh`) covering the shaded, unlit,
wireframe, uv-checker and vertex-colour paths; tone mapping and sRGB encoding live
in the `post` program, and the model wireframe is a depth-tested line-list draw in
the scene pass. The whole set is generated per backend by sokol-shdc and compiled
offline to committed bytecode by `build.rs`; the runtime does no shader
compilation. The Tex image is drawn by its own minimal fullscreen-triangle
pipeline, outside the scene MRT and tonemap path, so channel isolation is a
uniform swizzle and the displayed texel equals the stored texel.

**Crate boundaries are load-bearing** (invariants 2, 9, 10). Keep them.

**Retopology is ours, in Rust, and the vendored engines are gone.** Remesh was
built on Instant Meshes (triangles, mostly quads) and QuadriFlow (only quads)
behind a C++ bridge. Three things made that untenable and none was tunable: a
field extraction *synthesizes* a surface, so on the thin open shells game assets
are full of it tore holes; neither engine took a cancel hook, so an edit mid-run
waited out the object in the solver; and QuadriFlow was serial as vendored while
Instant Meshes' field solve gave a different mesh per thread count.
`crates/optimize/src/remesh/` replaces all of it: partition the surface into even
regions and **collapse** each one to a vertex. A collapse only ever removes edges
from a mesh that was already whole, so it cannot open a hole or drop a border —
which is the entire point. Its parallel stages sweep fixed chunks, it is
cancellable between passes, and it gives the same mesh at any thread count, so `Reproducible` and
`Thorough layout` are gone as settings rather than defaulted. **Do not
reintroduce a vendored retopologizer.** Two consequences are load-bearing and
documented for users in `docs/book/src/en/opt/remesh.md`: a rebuild can never
produce *more* faces than the input has, and its face sizes are less uniform than
a lattice-based extraction's ([GOTCHAS.md](GOTCHAS.md#remesh)). Quads are real:
`remesh/quads.rs` pairs the finished triangles up, guided by the cross field.

Two papers were assessed as replacement engines (2026-09-19) and neither is
adoptable. *SQuadGen* (arXiv 2604.27329, MIT, microsoft/SQuadGen) is an
802M-parameter diffusion model needing CUDA 13 and PyTorch on Linux and 30-60 s
per shape on an A100, and its stated scope is coarse, simple base layouts —
detailed geometry is filtered out of its training data. *LATO* (arXiv
2603.06357) is a mesh *generator*; even geometry-conditioned its fidelity to the
input is a Hausdorff distance of 0.08 of the shape's extent, orders of magnitude
worse than what `tests/remesh_fidelity.rs` measures here. Three classical parts
of SQuadGen are worth keeping in mind: its extraction (seeded region growing plus
feature-aware shortest-edge collapse) is structurally what `remesh/` already
does; its refinement ends with a nearest-point projection onto the input, which
we **measured as harmful** ([GOTCHAS.md](GOTCHAS.md#snapping-the-rebuild-back-onto-the-source-surface-makes-it-worse));
and its dataset-side quad recovery — rectangularity-sorted edge merging,
direction-aligned loop growing, loop shifting, a Blossom-Quad fallback — is a
licence-clean, non-neural triangle-to-quad converter and the best candidate if
the quad merge is ever extended.

**Engine-cooked compressed textures (KTX2, DDS) are intentionally not a goal.**
Artists test *source* assets (PNG, TGA, …); KTX2 and DDS are produced inside an
engine's content pipeline and never hand-authored or carried, so the viewer is
never handed one. We do use GPU block compression internally — the baked IBL cubes
are BC6H — but that is our own offline bake, not an import path.

---

## Platform decisions

The cross-platform work was planned as a numbered list of decisions, and source
comments cite them by number (`D9`) or, for the structural rules below the
table, by name (`Platform decisions: one swapchain pass`). This is what each
one settled.

| # | Decision |
| --- | --- |
| D1 | sokol_gfx is the one drawing API on both OSes, with a small device/swapchain leaf per OS — not wgpu, not a renderer per platform. |
| D2 | A measured performance gate (`--gate-out`, `scripts/gate.{ps1,sh}`): interleaved release launches of two builds compared on startup, frame time, RAM and VRAM. |
| D3 | egui is drawn by our own sokol renderer (`egui_sokol.rs`), replacing `egui-directx11`. |
| D4 | One annotated-GLSL shader source (`review.glsl`) compiled per backend by sokol-shdc. |
| D5 | Shader bytecode is committed per host and its freshness judged by **content hash**, not timestamps — mtimes stop meaning anything once two hosts commit blobs. |
| D6 | Dropping `egui-directx11` also unpinned egui, which that crate had held at 0.33. |
| D7 | Mip chains are generated on the CPU; sokol has no `GenerateMips`. sRGB is averaged in linear light so the result matches what the hardware produced. |
| D8 | GPU bring-up starts on the first line of `main`, on its own thread, and is joined when the window appears. |
| D9 | Native dialogs run on a worker thread and answer through the event loop; a modal run loop entered from a winit callback aborts the process on macOS. |
| D10 | File commands chord with the **primary** modifier — `Ctrl` on Windows and Linux, `Cmd` on macOS. `app`'s `shortcuts::primary_held` matches it; `ui`'s `primary_modifier()` is the one place that names it on screen, passed into a message as a variable, so no label hard-codes "Ctrl". |
| D11 | macOS builds are Apple silicon only, so the packaging script checks the binary's architecture rather than producing a universal binary. |
| D12 | The macOS build is signed and notarized by hand on the dev Mac (`scripts/build-mac.sh`, the twin of `packaging/build-windows-installer.ps1`), and its `.fbx` association is `LSHandlerRank = Alternate`: the viewer volunteers in Open With without taking the extension from whatever already owns it. |
| D13 | Windows first, then macOS: the shared code and the gate risk were in the Windows migration, and the Mac is leaves plus packaging. |
| D14 | A `.fbx` opened from Finder arrives as an Apple event, not `argv[1]`; without the open hook the file association does nothing. |
| D15 | The macOS menu bar is a second door onto the existing commands, never a second implementation; its items route through the event loop rather than acting in the callback. |
| D16 | Trackpad pinch zooms the camera. A pinch delta is a scale *fraction*, so its constant is what a whole gesture is worth, not a per-notch step. |
| D17 | Window placement is saved under the OS config directory (`%APPDATA%\3D Review`, `~/Library/Application Support/3D Review`). |
| D18 | GPU timing is a per-OS leaf beside the device (timestamp and disjoint queries), since sokol has no notion of it. |
| D19 | The IBL precompute is offline only, behind the `bake` feature, on a headless sokol device; image readback is another per-OS leaf. |
| D20 | The swapchain is plain UNORM, not an sRGB format — the flip model disallows `*_SRGB` — so the composite, egui and Tex shaders encode sRGB themselves. |
| D21 | `vendor/sokol-rust` is a pinned upstream checkout, `exclude`d in the root manifest so `-D warnings` does not lint it. `--exclude` on the command line is not enough: cargo lints every path dependency inside the workspace directory as local. |
| D22 | psd_sdk is vendored as source and compiled with `cc` beside a C-ABI wrapper, with committed hand-kept bindings, so a clean checkout links without a prebuilt library. |

The structural rules the decisions add up to:

| Rule | What it says |
| --- | --- |
| **`rhi` owns the backend** | Nothing outside `render/src/rhi/` names an `sg::` type, a backend pixel format or a window handle. Every resource goes through a wrapper, every format is `Format`, every failure is a `GpuError`, and invalid input (a zero dimension, a refused drawable) is an error there rather than a sokol validation abort. |
| **One swapchain pass** | Exactly one swapchain pass per frame — Metal presents inside `sg_end_pass`, so a second one would double-present. Everything a `Renderer::render_*` draws onscreen is therefore recorded as a deferred `SwapchainJob` and replayed when that pass opens; offscreen passes are issued directly. |
| **Platform leaves** | The complete inventory of per-OS code: `rhi/backend/{d3d11,metal}.rs` (device, swapchain, GPU timer, bake readback), `crates/shell-macos` (open-file events, menu bar), and the packaging scripts. Nothing else in the workspace names an OS. |
| **Toolchain** | `rust-toolchain.toml` tracks stable rather than pinning a version, and shader edits are the only thing that needs a platform shader compiler (D5): a checkout that does not touch `review.glsl` builds from the committed bytecode. |
