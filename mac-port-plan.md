# 3D Review - macOS port plan

3D Review shipped as a Windows-only `winit` + Direct3D 11 + `egui` (via `egui-directx11`) executable.
This document is the resolved plan for making it run on macOS with the **minimum amount of
platform-specific code**, the same governing constraint - and the same shape - as the Fire port
(`/Users/chandan/Dev/fire/mac-port-plan.md`, referred to below as *Fire §n* / *Fire Dn*): **one
shared shell, `winit` for the window and `sokol_gfx` as the one drawing API, with a ~200-line
device/swapchain leaf per OS.** Every applicable learning from that port is carried over and named
where it applies; where this project differs from Fire, the difference and its reason are stated.

It is a companion to `CLAUDE.md`, not a replacement: the crate boundaries, the data flow, the
`ModelData` funnel, the Opt workspace, skinning, and every UI invariant are unchanged. What changes
is the *GPU layer* (`crates/render/src/rhi/` and the shaders), the egui renderer, a handful of shell
leaves in `app`, and the build/distribution chain. The port runs **Windows first** (Fire D13): the
shared-shell migration is the risk, and it has to be gated on a Windows box against `main`.

Status: **Phase 0 done on Windows** (D21, D22), plus **Phase 1 steps 1–3 and most of step 4**. The
backend swap has happened: the device, the swapchain, the frame flow, the egui chrome, the Tex
viewport and the 3D + UV scenes (MSAA included) run on sokol_gfx, and `app` no longer depends on
`windows` at all. Two things are still to come in step 4: **GTAO** and the **Opt workspace's
comparison view** (the latter still parked in `crates/render/src/port_pending/`, with
`render_opt_scene` a stub that clears the frame to the viewport background). Sections are written in the present tense of the finished port so they
can become the description once it lands; the *Status* column of §2 and the phase list in §8 say
what is actually done.

---

## 1. The governing decision

Two ways to run on a second OS: a second native shell (AppKit + Metal beside Win32 + D3D11) or one
shared shell. Fire chose the shared shell to minimise platform-specific code, and measured that
`winit` + `sokol_gfx` costs nothing on its primary metric. The same applies here, with one
difference in scale: Fire's renderer was a single fullscreen triangle; this one is a 2-MRT MSAA
scene pass, GTAO, IBL/PBR with BC6H cubes, GPU skinning over storage buffers, an Opt workspace with
two resident meshes, and a Tex viewport - ~1700 lines of HLSL over 20 entry points. So the per-OS
*leaf* stays ~200 lines, but the shared *renderer* rewrite (`rhi/` → sokol_gfx, HLSL → sokol-shdc
GLSL, egui-directx11 → our own egui renderer) is the bulk of the work, and it lands on Windows
first, where every step can be checked against the current output by eye.

What survives untouched because it never had D3D11 in it: `model` (incl. `anim`, `bvh`), `import`
(the ufbx bridge), `optimize` (meshoptimizer + ufbx_write), `prof`, the whole `ui` crate (egui
native windowing; its only GPU contact is plain value types), `render`'s camera / config / geometry
/ material-state / texture-decode modules, and `app`'s input, shortcuts, loading, undo, animation,
texture-manager, opt and window-state logic. That is the payoff of invariants 2, 9 and 10.

---

## 2. Decisions

Each is stated with the reason and what it costs, so a future revision can revisit it. The
decisions marked *owner* were put to the project owner as questions before this plan was written
and are settled.

| # | Decision | Reason | Cost | Status |
| -- | -------- | ------ | ---- | ------ |
| D1 | **Shared shell: winit + sokol_gfx on both OSes** (*owner*); the only per-OS GPU code is `render/src/rhi/backend/{d3d11,metal}.rs` (device + swapchain), aliased as `backend` exactly like Fire's `render/mod.rs` | Minimum platform code; two full renderers (the D3D11 rhi + a Metal twin behind a trait) is the maintenance Fire D1 rejected | `rhi/` rewritten; Windows re-measured (D2) | Shipped on Windows (step 3); the scene lands in step 4 |
| D2 | **Windows migrates too, gated on three measurements** (*owner*) against `main` on the same box, same fixture, release build: (a) cold + warm startup to first presented frame with a model loaded; (b) steady-state frame time (GPU + CPU) on a fixed test model with GTAO on + 4× MSAA + a skinned clip playing; (c) private RAM + VRAM at idle with that model. Budgets: ≤ 10 ms startup median, ≤ 0.3 ms frame median, ≤ 5 % RAM/VRAM | The invariants stress runtime speed and footprint (`CLAUDE.md` §4: "spend dev-time freely to make the release runtime fast") | A harness: `scripts/gate.ps1` + a `--gate-out` stamp in `app` (Fire's `ttfp.rs` shape, extended with a frame-time and memory dump) | Planned |
| D3 | **Keep egui; write `render/src/egui_sokol.rs`**, a sokol_gfx egui renderer, replacing `egui-directx11` on both OSes (*owner*) | No sokol egui backend exists; switching to Dear ImGui like Fire would rewrite the whole `ui` crate and retire the native-windowing invariant | ~300–400 lines + one `@program`; consumes egui's `ClippedPrimitive`s + `TexturesDelta` (§3.4) | Shipped (step 3) — 380 lines |
| D4 | **One sokol-shdc annotated-GLSL source** (`render/src/shaders/review.glsl`) generates per-backend HLSL5 + MSL *and* the `ShaderDesc` reflection into `render/src/shaders/generated/`, checked in; build.rs compiles the host's set to bytecode (*owner*) | Fire D4/D24: nothing compiles a shader at runtime, a broken shader is a build error, one source instead of two hand-kept twins | The HLSL is rewritten once (~1700 lines); `fxc` on Windows, the Metal toolchain on macOS (D5) | Planned |
| D5 | **Bytecode is committed and freshness-gated on both OSes** - `.dxbc` (as today) *and* `.metallib` - compiled only when the generated source is newer or the blob is missing and the toolchain is present; missing blob + missing toolchain is a build error | Keeps this repo's existing "a no-fxc box builds from the committed blobs" property and extends it to the Mac, so the toolchain floor for someone who never edits a shader is Command Line Tools, not full Xcode. *Diverges from Fire* (which compiles into `OUT_DIR` on every build) on purpose | A shader edit must be followed by a rebuild on **both** OSes before commit, or one platform ships stale bytecode; `build.rs` warns when a blob is older than its generated source, and both packaging scripts fail on it | Planned |
| D6 | **egui bumped to 0.36.1** (`egui`, `egui-winit`, `egui-notify 0.23`) in the same port (*owner*) | Dropping egui-directx11 removes the only 0.33 pin | API churn in `ui` folded into the port; verify egui-notify 0.23 tracks 0.36 at execution, else pin egui to the newest it supports | Shipped (step 3); egui-notify 0.23 does track 0.36 |
| D7 | **CPU mip chain**, uploaded in the one `sg_make_image` | sokol_gfx has no `GenerateMips`; sRGB textures are averaged in linear light so the result matches the hardware path | ~5 ms per 4K texture | Shipped (step 4 stage 1) as `rhi/mips.rs`, but built **at upload**, not on the decode worker: the chain differs between the raw and sRGB uploads of the same pixels, so it is not a property of the decoded image (see step 4) |
| D8 | **GPU bring-up on its own thread from the first line of `main`; the window is created before the join** (Fire D18) | Device creation is the longest startup item (~135 ms for D3D11 on Fire's box) and needs no window | `App::start` reorders; `backend::Device: Send` | Shipped (step 3) |
| D9 | **Every `rfd` dialog runs on a worker thread and answers through `UserEvent`** (Fire §3.5: nothing called from a winit callback may pump a loop of its own - on macOS AppKit aborts the process) | Six blocking sites today (`loading.rs`, `texture_manager.rs`, three in `opt.rs`, the startup box in `main.rs`), three of them *inside* `render()`; on macOS this is a reliable crash, not a glitch | `Dialog` enum + `UserEvent::DialogDone`, one dialog at a time; the startup error box moves to after `run_app` returns | Planned |
| D10 | **`Primary` modifier: Ctrl on Windows, ⌘ on macOS**; `help.rs`/`stats.rs` labels say "Ctrl"/"Cmd" per OS | Cmd is the only acceptable file-command chord on a Mac | `shortcuts.rs` matches `SUPER` on macOS; the Alt+RMB zoom-drag stays Option | Planned |
| D11 | **Apple Silicon only** (Fire D10, *owner*); every vendored C tree (ufbx, meshoptimizer, ufbx_write, psd_sdk) is compiled from source by `cc`, so nothing is prebuilt per target | No Intel users to serve; unlike Fire's HEIF `.a`s there is no prebuilt native dep to re-vendor | `lipo` check in `build-mac.sh` | Planned |
| D12 | **Build, sign, notarize only on the dev Mac via `scripts/build-mac.sh`** (Fire D11/D12, *owner*); `Info.plist` heredoc from `product.json`; `.icns` from the 1024² master `assets/icons/application-logo.png`; `.fbx` in `CFBundleDocumentTypes` with `LSHandlerRank = Alternate` | Mirrors `packaging/build-windows-installer.ps1`; keeps the Developer ID cert off any shared machine. `Alternate` volunteers for `.fbx` in "Open With" without taking it from anything | `scripts/dev-app.sh` for the unsigned dev bundle - needed to test Finder opens at all, since a bare binary is not what `open` delivers files to | Planned |
| D13 | **Windows first, then macOS** (*owner*) | The shared code and the gate risk are the Windows migration; mac is leaves + packaging | The Mac waits one phase | Decided |
| D14 | **Open-file events via `openfiles.rs`** (Fire's `class_addMethod` hook adding `application:openURLs:` to winit's delegate) feeding `App::open_model_from_path`; opens that arrive before the window are held and handed to `start()` | Launch Services gives a fresh launch *no argv* and a running app *no new process*; without it `.fbx` association does nothing on macOS | ~130 lines copied; no IPC needed (one window, one process, no single-instance socket) | Planned |
| D15 | **Minimal `muda` menu bar** (App / File: Open… ⌘O, New ⌘N / Window) with `with_default_menu(false)`; `Open…` routes through D9 (*owner*) | A Mac app without a menu bar reads as broken; winit's default menu would replace ours wholesale, and it is where ⌘Q comes from, so ours must carry Quit | ~150 lines `menubar.rs`, `cfg(target_os = "macos")`; accelerators intercept keys before winit sees them, so only the menu items carry one | Planned |
| D16 | **Pinch → wheel zoom** on the orbit, UV and Tex cameras (Fire D15); gesture math stays in logical px, rendering in physical px | Trackpad users | `WindowEvent::PinchGesture`, factor `1 + delta`, NaN-filtered | Planned |
| D17 | **`window.cfg` via `dirs::config_dir()`** (`%APPDATA%` on Windows - the same path as today - `~/Library/Application Support` on macOS) | Drop the `APPDATA` env read in `window_state.rs`, which fails soft on macOS today | `dirs 6` | Planned |
| D18 | **Tracy GPU profiler: per-OS leaves over sokol's native handles** (*owner*). D3D11: the existing timestamp-query code unchanged, over `sg_d3d11_device()` / `sg_d3d11_device_context()` (the immediate context; `ctx.End(query)` between sokol calls is valid). Metal: per-pass zones are *not* achievable (sokol owns the command buffer and encoder; Apple silicon has no draw-boundary counter sampling) - the leaf brackets sokol's buffer with two sentinel command buffers on `sg_mtl_command_queue()` and reports their `GPUEndTime`s as one "GPU frame" zone, plus `sg_query_stats()` counts as plots on both OSes | Recorded honestly | `rhi/gpu_profiler.rs` keeps its `Zone` API; `zone_*` are no-ops on Metal; Xcode's Metal profiler covers per-pass timing there | Planned |
| D19 | **`bake_ibl` on sokol_gfx** (*owner*): `Baker` = headless `Gpu` (device, `sg_setup`, no swapchain); `CubeTarget` = one cube image + 6×mips attachment views (`mip_level` + `slice`); render every pass for one environment → `sg_commit` → per-OS readback leaf `backend::read_image_subresource`: D3D11 staging copy + `Map(READ)` over `sg_d3d11_query_image_info` (today's body); Metal blit from the private texture to a shared `MTLBuffer` on our own command buffer from `sg_mtl_command_queue()`, committed *after* `sg_commit` (queue order = commit order) + `waitUntilCompleted`. The old RTV/SRV-hazard `unbind_*` calls go away (passes are closed) | Fire D23: the whole dev pipeline runs on the Mac; a re-bake must not need Windows | sokol uses unretained command-buffer references, so every image stays alive until its readback returns | Planned |
| D20 | **The swapchain backbuffer is plain UNORM and the composite/egui/tex shaders sRGB-encode their own output** - `R8G8B8A8_UNORM` on D3D11 (today's contract), `BGRA8Unorm` on Metal (`CAMetalLayer` refuses RGBA8); `SWAPCHAIN_FORMAT` lives in the backend module | Fire D20. The swapchain pass needs no depth (the scene is offscreen), so Fire's `metal.rs` is reusable **as-is** | Channel order only | Shipped on Windows (step 3) |
| D21 | **`vendor/sokol-rust` vendored** (Fire's pin, floooh/sokol-rust @ b22a545) and **`exclude`d from the workspace** so `-D warnings` does not lint upstream | Fire D22 + its step-8 finding: `--exclude` on the command line does not work, because cargo lints every path dependency as local; `exclude = [...]` in the root manifest does | A pinned tree updated by hand; `sg_swapchain` / `ShaderDesc` shape changes land there | Shipped (Phase 0) |
| D22 | **PSD: vendor `psd_sdk` source + a `read_merged_rgba8` wrapper into `crates/psd`, compile with `cc`, keep the committed `bindings.rs`** (*owner*) | The prebuilt `fire_psd.lib` is gitignored (`*.lib`) and absent - the crate cannot link from a clean checkout on *any* OS today; source + `cc` is the model ufbx / meshoptimizer / ufbx_write already use and needs no bindgen or libclang at build time | `PsdNativeFile.cpp` excluded off Windows (the wrapper reads through its own in-memory `psd::File`); `cpp(true)` links `c++`. Fire's current `psd-sdk-sys` wrapper has a *newer* ABI (`fire_psd_read_merged`, a 16-byte info struct); the vendored wrapper here keeps the older one the committed bindings declare | Shipped (Phase 0) |

Not adopted from Fire, and why: the single-instance socket / `interprocess` (D5/D6 there) - this
viewer is one window per process and has no instance model; CI (Fire D23's mac leg) - deferred by
the owner, verification stays manual on the two dev machines; Dear ImGui - see D3.

---

## 3. Architecture after the port

```
Explorer / Finder open
        │  Windows: 3d-review.exe "path"     macOS: Apple Event → application:openURLs:  (D14)
        ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  3d-review (one process, one window)                                        │
│  main(): GPU bring-up thread starts here (D8) → winit loop (Wait/WaitUntil) │
│                                                                             │
│  app     : ApplicationHandler, input, redraw pacer (unchanged), UiOutput    │
│            intents, workers (import / texture decode / opt / export /       │
│            file dialogs D9) → EventLoopProxy<UserEvent>                     │
│  ui      : egui chrome (unchanged) → FullOutput                             │
│  render  : Renderer façade (unchanged API) → SceneGpu / TexGpu / EguiGpu    │
│            over rhi = thin sokol_gfx wrappers                               │
│            rhi::backend = d3d11.rs | metal.rs  (device + swapchain, ~200 l) │
│  frame   : offscreen passes (scene MRT MSAA, GTAO ×3) →                     │
│            swapchain pass { composite (viewport rect ×1 or ×2), egui }      │
│            → sg_commit → backend.present                                    │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 3.1 The new `rhi` (thin sokol wrappers; checked against the vendored header)

```
crates/render/src/rhi/
  mod.rs             Gpu (owns backend::Device + Swapchain, sg::setup/shutdown), Frame, SwapchainJob,
                     GpuError/GpuResult, Format, `pub use backend`
  backend/d3d11.rs   Fire's d3d11.rs + the debug-layer fallback + 3 leaves     #[cfg(windows)]
  backend/metal.rs   Fire's metal.rs verbatim + 3 leaves                        #[cfg(target_os = "macos")]
  shader.rs          generated ShaderDesc + this OS's bytecode (Fire's make_shader, generalised)
  pipeline.rs        Pipeline/PipelineDesc + the surviving enums (VertexFormat, Topology, Cull,
                     DepthCompare, DepthState, BlendMode, DepthBias); InputElement = (location, format)
  buffer.rs          VertexBuffer, IndexBuffer (immutable), StorageBuffer<T> (immutable | dynamic_update)
  texture.rs         Texture = sg::Image + texture sg::View; rgba8_mipped uses mips.rs
  mips.rs            CPU mip chain (port of Fire's render/mips.rs; sRGB-aware)
  sampler.rs, target.rs (ColorTarget/DepthTarget own attachment + resolve + texture views; TargetSet)
  gpu_profiler.rs    same Zone API; body cfg(windows); Metal frame-span probe (D18)
  bake.rs            Baker = headless Gpu, CubeTarget (6×mips attachment views), Target2D   [feature bake]
crates/render/src/shaders/{review.glsl, generated/…}      §4
crates/render/src/egui_sokol.rs                            D3
```

The three leaves each backend module carries beside `Device` / `Swapchain` / `SWAPCHAIN_FORMAT`:
`supported_sample_counts(color, depth)`, `read_image_subresource(image, mip, slice, w, h, bpp)`
(bake), and the profiler probe (D18). Like Fire's `render/mod.rs`, the two modules are twins
aliased as `backend`, not a trait: the target set is closed, and the alias keeps `cfg` out of every
shared file. Anything added to one must be added to the other.

Contract changes the callers see (mechanical, ~90 signatures): `GpuError { Resource{kind,label},
InvalidArg, Backend(String), DeviceLost{reason} }` replaces `windows::core::Result`; `Format`
replaces `DXGI_FORMAT` (`Rgba8, Rgba8Srgb, Rgba16F, Rg16F, R8, Bc6hUf16, Depth32F` +
`backend::SWAPCHAIN_FORMAT`); every `device: &ID3D11Device` / `ctx: &ID3D11DeviceContext`
parameter disappears - sokol is a process singleton, the `Gpu` value is the proof of setup, and
each constructor `debug_assert!(sg::isvalid())`; `DynamicConstantBuffer` is **deleted** in favour of
`rhi::apply_uniforms::<T>(slot, &T)` (which asserts `size_of::<T>()` against the generated block
size); `bind_*` / `unbind_*` / `resolve()` are deleted (bindings are an `sg::Bindings` *value*
re-applied after every `apply_pipeline`; the D3D11 backend `ClearState`s at every `end_pass`;
resolves happen in `end_pass`); `Gpu::resize` becomes infallible; `Gpu::new(&Window, w, h)` reads
the raw window handle itself, so `app` drops `windows` and `win32_hwnd` and stays
`#![forbid(unsafe_code)]`. `App`'s field order must put `gpu` last, *and* every wrapper's `Drop`
guards on `sg::isvalid()` as Fire's do. sokol's logger is installed in `Gpu::new` and forwards to
`eprintln!` + `review_prof::msg`, because validation failures are otherwise silent.

### 3.2 Rendering - the sokol pass model

sokol_gfx is a process singleton: `sg_setup` once on the bring-up thread (Fire does this; sokol has
no thread affinity, the join is the synchronisation), every later call on the main thread.
**Exactly one swapchain pass per frame**: the Metal backend calls `presentDrawable:` inside
`sg_end_pass`, so a second swapchain pass double-presents. Today the composite clears and owns the
backbuffer per view, so it becomes a *deferred job*:

```rust
let Some(mut frame) = gpu.begin_frame() else { skip };   // acquire; None on 0-size / refused drawable
renderer.render_scene(&mut frame, &scene_frame)?;         // offscreen passes now; composite → frame.jobs
egui_renderer.prepare(&egui_ctx, full_output)?;           // tessellate + texture deltas, outside any pass
frame.begin_swapchain_pass();                             // clear(frame.clear) → replay jobs, viewport per job
egui_renderer.paint(&frame, ppp);                         // inside the same pass, after the composites
frame.finish(vsync) -> PresentStatus                      // end_pass, commit, backend.present
egui_renderer.free_textures();                            // after the frame
```

A `SwapchainJob` is `{ pipeline, bindings, uniform bytes + slot, viewport, vertex_count }` - all
`Copy` sokol ids, so the renderer keeps no borrow into the frame. Per frame, inside one `sg_commit`:

1. **Scene pass** - offscreen, `attachments{ colors[0]=color(MSAA), colors[1]=ambient(MSAA),
   resolves[0..1]=the single-sample twins, depth=D32F(MSAA) }`, clear both colours to `0` and depth
   to `0.0` (Reversed-Z, `GreaterEqual`). Same draw order as today. The ghost overlay's three
   scene-uniform rewrites become three `apply_uniforms` calls (free).
2. **GTAO** (when on) - three passes exactly as today: G-buffer (single-sample RGBA16F + own depth),
   occlusion (R8), blur (R8).
3. **Swapchain pass** - `sg_pass.swapchain` filled by `backend::Swapchain::acquire`
   (`current_drawable` / `render_view`), `depth_format = None`, `sample_count = 1`; the pass owns
   the clear (`frame.clear`, black by default - the Tex viewport's solid backgrounds just set it).
   Then the composite job(s) with `sg_apply_viewport` to the scene rect, then egui (D3), then
   `sg_end_pass`. The Tex viewport queues checker + image jobs instead. The startup black frame is
   this pass with no jobs - the same code path, no special case.
4. `sg_commit`; `backend::Swapchain::present()` (D3D11: `Present(1,0)` → `PresentStatus`, incl.
   `DeviceLost` from `DXGI_ERROR_DEVICE_REMOVED/RESET` as today; Metal: already presented, so
   `present` only releases the drawable).

**Opt split**: today both halves render through *one* shared target set and are composited in
between; with deferred composites both halves' results must be alive at swapchain time, so
`SceneGpu` holds **two `TargetSet`s at half width** (the same memory as one full-width set); single
views use set 0 at full size. Overlay and UV are single views.

Resize: `backend.resize(w,h)` (drop the RTV + `ResizeBuffers` / `setDrawableSize`) + the existing
`SceneGpu::sync_targets` reallocation; `ScaleFactorChanged` → `backend.set_scale_factor` (Metal
`contentsScale`, D3D11 no-op) - a new event `app` must forward, or a window dragged between
displays of different backing scale goes soft (Fire step 6). Device lost: `acquire` returning
`false` skips the frame; relaunch is the recovery, as today.

**Every declared view slot must be bound** (sokol validates it), so `SceneGpu` owns four 1-element
dummy storage buffers for models without deform tables or without morph tables - today's "leave
`t14`/`t15` unbound" is a validation error under sokol. `bind_scene_shared`'s "bind every material
slot" rule already matches this.

The redraw pacer in `app` is unchanged and still necessary: `frame.rs`'s own note ("we can't rely
on the swapchain to pace us") holds for Metal, where the block is at `nextDrawable`, not at present.

### 3.3 What sokol cannot do, and the leaf for each

| Need | sokol_gfx | Resolution |
| --- | --- | --- |
| GPU `GenerateMips` (every material + Tex texture) | none | CPU mip chain on the decode worker (D7) |
| Timestamp queries → Tracy | none | per-OS leaf (D18) |
| CPU readback (bake only) | none | per-OS leaf (D19) |
| Exact MSAA sample-count support | only `sg_query_pixelformat().msaa` (yes/no) | `backend::supported_sample_counts()` - `CheckMultisampleQualityLevels` / `supportsTextureSampleCount:` (Metal on Apple silicon: 1, 2, 4, 8 - the 16× option disappears there) |
| Sub-rect image update (egui atlas patches) | `sg_update_image` is whole-image, once per frame | CPU shadow of each egui texture; recreate the immutable image on change (§3.4) |
| Max 2D texture size for egui-winit | `sg_query_limits().max_image_size_2d` | replaces the hardcoded `Some(16384)` in `init_shell` |

Everything else maps 1:1: immutable vertex/index buffers, uniform blocks via `sg_apply_uniforms`
(uniform-block slots are per stage, so `SceneUniforms` is applied twice - a VS block and an FS block,
same bytes; `MaterialUniform` once per draw range is fine - sokol appends into a per-frame uniform
buffer, sized in `sg_desc.uniform_buffer_size`), read-only storage buffers on the vertex stage for
skinning (D3D11 `ByteAddressBuffer` via shdc, MSL `[[buffer(8+)]]`), the dynamic palette via
`sg_update_buffer` (once per frame - already true, it updates on `pose_revision`), BC6H cube images
with per-mip data (the committed `.bin` layout is *exactly* sokol's: mip-major, six faces
contiguous in +X −X +Y −Y +Z −Z - the D3D subresource re-indexing goes away), the RG16F LUT,
R8/RGBA16F/D32F attachments, per-attachment blend (both MRTs alpha-blend, as now), depth bias,
cull, line lists, the `Uint4` vertex attribute, the five samplers incl. aniso 16 and the mixed
min-linear/mag-point one, viewports for the Opt split, scissor for egui. Pipelines carry
`sample_count`, and sokol validates it against the pass, so a missed AA rebuild is a logged
validation error rather than silent garbage.

### 3.4 The egui renderer (`render/src/egui_sokol.rs`, D3)

`render` gains an `egui` dependency (for the `epaint` types); `egui-directx11` leaves `app`.
One `egui` program: vertex `pos Float2 @0, uv Float2 @1, color Ubyte4N @2` (the 20-byte
`epaint::Vertex`, size-asserted), `u32` indices, no cull/depth, `sample_count 1`, colour format
`SWAPCHAIN_FORMAT`, blend `src One / dst OneMinusSrcAlpha`, alpha `OneMinusDstAlpha / One` -
egui's premultiplied contract; the fragment is `texture * color` with no gamma conversion, the
contract egui-directx11 already relied on (D20). Uniform: `screen_size_points`; the VS maps points
→ NDC with Y flipped. Geometry: one vertex + one index buffer with `write_transient`, grown before
the write, written once per frame from the concatenated `ClippedPrimitive`s; per mesh
`vertex_buffer_offsets[0]` / `index_buffer_offset` + `sg::draw`. Scissor: `clip_rect × ppp` →
`sg_apply_scissor_rect(origin_top_left)`, clamped; `paint()` first restores the full viewport the
composite jobs narrowed. Textures: `HashMap<TextureId, { image, view, shadow: Vec<u8>, options }>`;
each `set` delta is applied into the CPU shadow (whole or sub-rect), dirty textures are destroyed
and recreated as immutable images after all deltas (the atlas mutates for a few frames after
startup or a DPI change, then never), `free` entries after the frame. Only `TextureId::Managed` is
in use (icons and HDR thumbnails go through `load_texture`). Samplers are cached by `TextureOptions`.

---

## 4. Shader

`crates/render/src/shaders/review.glsl` becomes the one source (sokol-shdc annotated GLSL, Vulkan
syntax, separate `texture2D`/`textureCube` + `sampler`). `scripts/gen-shaders.{ps1,sh}` (Fire's
script, both OSes; it fetches `sokol-shdc` into `.tools/` on demand) runs shdc twice - `-l
hlsl5:metal_macos -f bare` for the per-backend sources and `-f sokol_rust` for the reflection - into
`crates/render/src/shaders/generated/`, checked in and normalised (rustfmt + a path scrub) so either
OS regenerates identical text. `build.rs` compiles the host's sources to committed, freshness-gated
bytecode (D5): `fxc /T vs_5_0|ps_5_0 /E main /O3 /WX` per (program, stage) on Windows; `xcrun -sdk
macosx metal -c -O2` + `metallib` per (program, stage) on macOS - one library per stage because
SPIRV-Cross names every entry `main0`. Each pipeline's shader takes the generated
`<program>_shader_desc(sg::query_backend())` and swaps `source` for the bytecode, as Fire does.

Programs - 15, one `@program` each (today's 20 fxc jobs fold into these; 30 DXBC + 30 metallib
blobs). `vs_main` is compiled once per program that uses it (shdc dead-strips; harmless):

| Program | Today | Pipelines |
| --- | --- | --- |
| `mesh` | `vs_main` + `fs_main` | mesh, mesh_double_sided, fill_overlay, uv_fill |
| `line` | `vs_main` + `fs_line` | line, line_overlay (also the UV wire) |
| `selection` | `vs_main` + `fs_selection` | selection, x-ray ghost |
| `gtao_gbuffer` | `vs_main` + `fs_gtao_gbuffer` | single-sample |
| `skybox` | `vs_skybox` + `fs_skybox` | `gl_VertexIndex`, no vertex input |
| `gtao`, `gtao_blur` | `vs_fullscreen` + `fs_gtao` / `fs_blur` | `GetDimensions` → the unused `params.w` / `config.w` (block stays 96 B) |
| `post` | `vs_fullscreen` + `fs_post` | composite |
| `tex_image`, `tex_checker` | `vs_fullscreen` + `fs_image` / `fs_checker` | |
| `egui` | new | D3 |
| `ibl_equirect`, `ibl_irradiance`, `ibl_prefilter`, `ibl_brdf` | bake | compiled only under the `bake` feature |

Binding plan for the scene programs (explicit `layout(binding=N)` everywhere so one `sg::Bindings`
value serves every scene pipeline): UB 0 `scene_vs`, UB 1 `scene_fs` (both `SceneUniforms`), UB 2
`material`; views 0 checker, 1–4 IBL (irradiance, prefilter, BRDF, env), 5–11 the seven material
slots, 12–15 the deform storage buffers; samplers 0 checker, 1 IBL, 2 material. Other programs
start at 0. `@block`s: `srgb`, `fullscreen_vs`, `scene_uniforms`, `scene_bindings`, `deform`.

Rules the rewrite must keep (from the HLSL inventory + Fire §4):

* **Uniform blocks obey shdc's type rules**: members are `float/vec2/vec3/vec4/int/ivec*/mat4` only -
  the `uint` fields of `PostUniforms`/`TexUniforms` and the slot-flag bitfield become `int` (same
  bits, Rust `u32` → `i32`); no `mat3` members (post's `mat_from_cols` becomes a local
  `mat3(c0,c1,c2)`, column-major in GLSL). Every block is `std140`; the generated Rust struct is the
  layout, and each existing `#[repr(C)]` struct keeps its `const _: () = assert!(size_of::<T>() == N)`
  **plus** a new `assert!(size_of::<T>() == size_of::<generated::T>())` - invariant 11 now catches
  drift against the generated source, not only against a hand-typed number.
* **Storage structs are std430**: `InfluenceEntry {uint; float}` (8 B) and `PaletteEntry {vec4×3}`
  (48 B) are unchanged; `MorphEntry` is declared as **seven scalars** (`uint shape; float px,py,pz,
  nx,ny,nz`) so it stays 28 bytes - a `vec3` member would pad it to 48 and shift every entry.
  *Check at first regeneration:* shdc's Rust emitter may stamp `#[repr(C, align(16))]` on every
  struct; if the generated `MorphEntry` comes out at 32 bytes, the assertion is against the
  hand-computed std430 stride (28), which is what the generated HLSL/MSL actually index with.
* **HLSL → GLSL mechanics**: `mul(M,v)` → `M*v`; GTAO's `proj[row][col]` reads flip to
  `proj[col][row]`; `lerp/saturate/frac/fmod/rsqrt/atan2` → `mix/clamp/fract/mod/inversesqrt/atan`;
  vertex inputs `layout(location=0..5) in` with `uvec4` at 5; MRT outputs `layout(location=0/1) out`.
  A unit test asserts the generated `ATTR_<prog>_*` constants equal the literal locations for every
  program that shares `vs_main`.
* Every texture read in non-uniform control flow is `textureLod`/`textureGrad` (already the HLSL's
  `SampleLevel` discipline; Fire's edge-flicker bug is the reason). `fs_main` keeps sampling every
  material slot at the top, under uniform control flow.
* `gl_FragCoord` is top-left on both backends (`sg_features.origin_top_left`), matching today's
  `SV_Position` reads in `tex` and `gtao`; `gl_VertexIndex` builds the fullscreen triangles.
* `discard`, `switch`, bit ops, `textureLod` on cubes, `[loop]` (drop the attribute) all carry over.
* `srgb_to_linear`/`linear_to_srgb`, duplicated across three HLSL files today, become one `@block`.

---

## 5. Platform leaves (the complete inventory)

Everything below is the platform-specific code that remains. Nothing else in the workspace names an
OS.

| Concern | Windows | macOS | Shared via |
| --- | --- | --- | --- |
| Window, loop, DPI, DnD, theme, placement, refresh rate | - | - | `winit` |
| GPU device + swapchain | `rhi/backend/d3d11.rs` (~230 l, from Fire) | `rhi/backend/metal.rs` (~220 l, from Fire, unchanged) | `sokol_gfx` |
| Supported MSAA counts | `CheckMultisampleQualityLevels` in d3d11.rs | `supportsTextureSampleCount:` in metal.rs | `backend::supported_sample_counts()` |
| Shader bytecode | HLSL → DXBC (`fxc`) | MSL → `.metallib` (`xcrun metal`) | one shdc source (D4) |
| Tracy GPU zones | `sg_d3d11_device_context()` + the existing queries | frame-level at best (D18) | `rhi/gpu_profiler.rs` |
| Bake readback | staging copy via `sg_d3d11_query_image_info` | blit to a shared `MTLBuffer` via `sg_mtl_query_image_info` + `sg_mtl_command_queue` | `rhi/bake.rs` |
| Config dir (`window.cfg`) | `%APPDATA%\3D Review` | `~/Library/Application Support/3D Review` | `dirs` |
| File dialogs, startup error box | - | - | `rfd` on a worker (D9) |
| Open-file from the OS | argv (unchanged) | `openfiles.rs` (`application:openURLs:`) | both → `open_model_from_path` |
| Launcher show state | `startup_show_maximized` (exists, cfg'd) | `false` (exists) | `import` |
| Menu bar | none | `menubar.rs` (`muda`) | - |
| Primary modifier | Ctrl | ⌘ (`SUPER`) | `shortcuts.rs` + `help.rs` labels |
| Pinch | n/a | `PinchGesture` → zoom | `input.rs` |
| Window icon / metadata | `winresource` (exists) | `Info.plist` + `.icns` (bundle; `with_window_icon` is a no-op) | `product.json` |
| Installer | Inno Setup (unchanged) | `scripts/build-mac.sh` → signed, notarized `.dmg` | - |
| File association | `HKCU` ProgID (unchanged) | `CFBundleDocumentTypes`, `LSHandlerRank = Alternate` | - |

Removed by the port: `rhi/`'s D3D11 COM (all of it - device, swapchain, pipeline, buffer, target,
texture, sampler; ~2000 lines), `egui-directx11`, `win32_hwnd`, the `%APPDATA%` read, the
unconditional `windows` dep in `app` (the `unsafe` moves into `rhi/backend/*.rs`, which becomes
invariant 9's sanctioned GPU site), and `render/build.rs`'s hand-listed 20 fxc jobs.
`crates/app/build.rs`, `import`, `optimize`, `ui`, `model`, `prof` need nothing.

---

## 6. Crates

**Shell / render.** `winit` (pin `=0.30.x`, the one Fire uses), `sokol` (path `vendor/sokol-rust`),
`egui`/`egui-winit` `=0.36.1`, `egui-notify 0.23` (D6). `windows 0.62` becomes a
`[target.'cfg(windows)']` dep of `render` only (for `backend/d3d11.rs`, the profiler leaf and the
bake leaf); `app` loses it. `dirs 6`; `rfd 0.15` kept, running on a worker.

**macOS** (`render` and `app`, `[target.'cfg(target_os = "macos")']`, versions = winit 0.30's own
objc2 family so nothing extra compiles): `objc2 0.5`, `objc2-foundation 0.2` (NSGeometry, NSArray,
NSString, NSURL), `objc2-metal 0.2` (MTLDevice, MTLPixelFormat, MTLDrawable, MTLTexture,
MTLResource, + MTLCommandQueue / MTLCommandBuffer / MTLBlitCommandEncoder / MTLBuffer for the bake
leaf), `objc2-quartz-core 0.2` (CALayer, CAMetalLayer, objc2-metal), `objc2-app-kit 0.2` (NSView,
NSResponder, objc2-quartz-core, NSApplication), `muda 0.19` (`default-features = false`), `libc`.
`rfd` pulls a second objc2 family (0.6); the two coexist because nothing passes objc2 types between
them - the glue only hands sokol_gfx raw pointers.

**Unchanged.** `glam`, `bytemuck`, `image`, `zune-*`, `half`, `cc`, `notify`, `serde*`,
`thiserror`, `tracy-client`, `intel_tex_2` (bake).

**Dropped.** `egui-directx11`; `windows` from `app`; `windows-sys` stays only for `import`'s
`startup_show_maximized`.

**PSD (D22).** `crates/psd/vendor/Psd/` = psd_sdk source (from Fire's `crates/psd-sdk-sys/vendor/Psd`,
same commit `f514495`), `crates/psd/src/wrapper.cpp` = a `read_merged_rgba8`-ABI wrapper matching
the committed `bindings.rs` (12-byte `fire_psd_info`), `build.rs` = `cc::Build::new().cpp(true)`
over `wrapper.cpp` + every `Psd/*.cpp` except `*_Linux.cpp`, `*_Mac.cpp` and (off Windows)
`PsdNativeFile.cpp`; `flag_if_supported` for both `/std:c++17 /EHsc` and `-std=c++17`. No bindgen.
`CLAUDE.md` invariant 9's psd paragraph is rewritten to say "source + `cc`".

---

## 7. Development environment

Fire §7 applies, with these deltas.

* **A Mac needs:** Xcode + the Metal toolchain (`xcodebuild -downloadComponent MetalToolchain`;
  verify by *running* `xcrun -sdk macosx metal --version` - `xcrun --find metal` finds a stub that
  exists before the toolchain does) **only when editing shaders** (D5); rustup stable with
  `aarch64-apple-darwin`; nothing else - every native dep is `cc`-compiled source. No libclang, no
  vcpkg, no ImageMagick (the thumbnails are committed). `sokol-shdc` is fetched by
  `gen-shaders.sh` into `.tools/` on demand (osx_arm64 / win32).
* **Windows needs:** as today (the `x64 Native Tools` prompt for `cc`; `fxc` from the Windows SDK
  only when editing shaders), plus `sokol-shdc` for `gen-shaders.ps1`.
* **`rust-toolchain.toml`**: add one with `channel = "stable"` and both targets.
* **`.cargo/config.toml`**: none exists; keep it that way (Fire's `LIBCLANG_PATH` trap: cargo's
  `[env]` cannot be made host-conditional).
* **Clippy scope:** `exclude = ["vendor/sokol-rust"]` in the root manifest (D21). `cargo clippy
  --workspace --all-targets -- -D warnings` must pass on *both* hosts, because each host's clippy
  never sees the other's leaf. It **aborts at the first failing crate**, so a run that reports one
  crate's lints says nothing about the crates downstream of it — after fixing, re-run to the end
  rather than assuming the list was complete. A toolchain bump can also turn a clean tree red on
  code nobody touched; fix those rather than `-A`-ing them, since a `future_incompat` warning
  becomes a hard error later.
* **Tests:** everything headless (`model`, `import`, `render`'s geometry / material / camera,
  `optimize` + its fixture suites) runs on both. Added by the port: the egui renderer's
  mesh→buffer packing, the CPU mip chain (against a hardware-matching reference), the
  generated-struct size assertions, the `.bin`-to-`sg_image_data` slicing.
* **Dev bundle:** `scripts/dev-app.sh` wraps `target/{debug,release}/3d-review` in an unsigned
  `.app` (`com.psmyles.3d-review.dev`); the only way to exercise D14 and Retina before packaging.
* **Gate harness (D2):** `scripts/gate.ps1` (Windows; a bash twin later for mac-vs-mac numbers):
  N interleaved launches of A and B with a fixture FBX, reading the `--gate-out <file>` stamp that
  `app` writes (process-creation → first present via `GetProcessTimes` / `proc_pidinfo`, then
  median frame time over 300 frames of a scripted orbit with GTAO + 4× + a clip playing, then
  `PROCESS_MEMORY_COUNTERS` / `DXGI_QUERY_VIDEO_MEMORY_INFO` ↔ `task_info` /
  `MTLDevice.currentAllocatedSize`), then exits. One warm-up launch before measuring; the first
  launch after a build is an outlier (Fire step 7).

---

## 8. Phases

### Phase 0 - unblock the checkout (either OS, ~½ day) — **done on Windows**

1. **D22**: vendor the psd_sdk source + wrapper into `crates/psd`, `cc` build, delete the `.lib`
   link. Verify: `cargo test -p review-psd` + a PSD texture decodes in the viewer on Windows;
   `cargo test -p review-psd` on the Mac.
   *Done.* `crates/psd/vendor/Psd/` (upstream `src/Psd/` @ `f514495`) + `src/wrapper.{h,cpp}`
   compiled by `cc`; `src/bindings.rs` unchanged, so the ABI is the older
   `fire_psd_read_merged_rgba8` / 12-byte `fire_psd_info` those bindings declare, and the wrapper
   is byte-identical to the source the retired `.lib` was built from — no behaviour change. The
   viewer check is now a repeatable test rather than a manual one:
   `crates/psd/tests/real_psd.rs` decodes `assets/test_textures/T_Sides_D.psd` (skips when
   absent). Still to run on the Mac.
2. Bump `crates/app`'s version to match `product.json` (the build.rs warning fires today).
   *Done* — 0.2.0 → 0.2.1; the warning no longer fires.
3. Vendor `vendor/sokol-rust` (Fire's pin), `exclude` it, add `rust-toolchain.toml`. `cargo check
   --workspace` still green on Windows (nothing uses it yet).
   *Done.* 94 files at `b22a545`, `exclude`d in the root manifest, provenance in
   `vendor/NOTICE.txt`; it compiles standalone on Windows (its build.rs builds the sokol C for
   D3D11), and nothing depends on it yet, so `[workspace.dependencies] sokol` is deliberately not
   declared until Phase 1 step 3 wires it in.

Found while doing it:

* **The workspace clippy gate had gone red on current stable** (1.98.1) from three lints on
  untouched code — *fixed during Phase 1, thirteen sites in all*. `float_literal_f32_fallback`
  (`crates/ui/src/widgets.rs:532`) was the one that mattered: a future-incompat warning, so it
  becomes a hard error on a later toolchain. `byte_char_slices` (3× `crates/render/src/texture.rs`)
  became byte-string literals. `chunks_exact_to_as_chunks` (9× across `model`,
  `render/geometry` and `optimize`'s `real_models` test) became `as_chunks::<3>().0`, which hands
  back `&[[u32; 3]]` rather than `&[u32]` — the triangle shape moves into the type and the
  remainder is dropped identically, so nothing changes at run time. `slice::as_chunks` stabilised
  in 1.88, exactly the workspace floor, so no MSRV bump. Worth knowing for the next such sweep:
  **clippy aborts at the first failing crate**, so `model`'s five sites were hiding `render`'s and
  `optimize`'s — a run that reports one crate's lints is not evidence the rest are clean.
* **`fire_psd_info.channels` is the document's raw `channelCount`** (not fixed; it predates the
  port and does not block Phase 1), which counts spot/extra
  channels, and `toolbar.rs:530` keys the Tex viewport's alpha segment on `channels == 2 | 4`. An
  RGB PSD with one spot channel reports 5 and so reads as having no alpha. Fire hit this and fixed
  it in its newer wrapper by reporting the *composite's* channel count instead; that fix is not
  carried here, because it changes the ABI's meaning and Phase 0 was deliberately behaviour-
  preserving. Worth doing on its own.

### Phase 1 - Windows on winit + sokol_gfx (branch `shell/winit-sokol`), gated

Ordered so each step is verifiable by eye against the current renderer on the same machine.

1. **`rhi` façade first, D3D11 underneath.** Introduce `GpuError`/`GpuResult` and `Format`, and
   remove `ID3D11Device`/`Context` from every signature outside `rhi/` (~90 sites in `scene/*`,
   `material/d3d.rs`, `tex_d3d.rs`, `ibl.rs`, `lib.rs`); `windows::core::Result` disappears from
   `render`'s public API. Pure refactor, pixel-identical, tests green. This is what makes step 4 a
   swap instead of a rewrite.
   *Done.* `rhi/error.rs` + `rhi/format.rs` are new; every constructor now takes `&Gpu` (the two
   `(device, ctx)` parameter pairs collapse into one handle) and ends in
   `.resource(kind, label)`, so a failure names the resource instead of only an HRESULT.
   `Format::dxgi()` is `pub(in crate::rhi)`, which is what *enforces* the boundary rather than
   just documenting it — `grep -r 'windows::\|ID3D11\|DXGI_' crates/render/src` outside `rhi/`
   is now empty. `Gpu` gained an **optional** swapchain: `Gpu::headless()` is the bake's device,
   so `Baker` is already "a headless `Gpu`" as D19 wants, and `rhi::bake` no longer creates a
   device of its own. Three deviations worth knowing: `Gpu::new` still takes an `HWND` and
   `d3d11_device()` / `d3d11_context()` / `d3d11_backbuffer_rtv()` remain `pub` purely for
   `egui-directx11` (all four go in step 3); `Gpu::resize` is still fallible (step 3); and
   `Texture::cube_block_compressed` lost its `block_bytes` parameter, which now comes from the
   `Format`. Verified: `cargo clippy --workspace --all-targets -- -D warnings` clean with no `-A`
   escapes (the three stale-toolchain lints Phase 0 found were cleared in the same pass), the same
   for `-p review-render --features bake`, all tests green, and the viewer eye-checked against
   `SK_Player_01.fbx` — skinned mesh in its default pose, IBL/PBR shading, GTAO, grid + axes,
   Outliner and toolbar all unchanged. The UV / Tex / Opt viewports compile and share the same
   `rhi` calls but were **not** eye-checked: synthetic clicks do not reach the winit window from
   an agent session, so that check is one manual click each.
2. **Shaders first, no runtime change.** Write `review.glsl` (all 15 programs, §4) +
   `scripts/gen-shaders.ps1/.sh`; generate; the new `build.rs` compiles the generated HLSL to
   committed DXBC with the existing freshness gate (D5). Verify: shdc accepts it, `fxc /WX`
   compiles all 30, `generated/review.rs` exposes the expected constants/structs, the size
   assertions compile - this is where the `MorphEntry` alignment and `uint` questions are answered
   before any renderer code exists. Keep the old `.hlsl` until step 4 renders correctly.
   *Done.* All 15 programs in `crates/render/src/shaders/review.glsl`; shdc emits 60 per-backend
   sources + the reflection into `generated/` (checked in), and `fxc /WX /O3` compiles all 30
   DXBC blobs clean. build.rs **discovers** the generated jobs by filename rather than listing
   them, so adding a program touches only `review.glsl`; the hand-listed legacy jobs stay beside
   them until step 7. The old `hlsl/` set still draws every frame — nothing includes the new
   bytecode yet.

   The three open questions are answered:

   * **`MorphEntry` is 28 bytes.** shdc's Rust emitter stamps `align(4)` on a scalar-only struct
     (only vec/mat members get `align(16)`), so the seven-scalar declaration the plan called for
     does hold at 28. `InfluenceEntry` 8, `PaletteEntry` 48, all matching.
   * **`uint` in a uniform block is rejected**, as expected; `PostUniforms`' and `TexUniforms`'
     flags are `int` in the shader and come back as `i32` in the generated struct. The Rust
     structs still declare `u32` — same bits, same size, and the assertions pass — so flipping
     them is step 3/4's business, not a silent change here.
   * **A uniform block belongs to exactly one stage** (`sg_shader_uniform_block.stage`), so
     `SceneUniforms` really is declared twice, at UB 0 (VS) and UB 1 (FS). GLSL puts block members
     in global scope, so the two would collide on member names — the fix is a block *instance
     name* (`su.` / `sc.`), which shdc accepts. The same applies to the IBL face block (0 VS /
     1 FS), which the plan had not anticipated.

   Four things worth knowing that the plan did not cover:

   * **A storage buffer must hold exactly one flexible array of a struct** — shdc rejects a bare
     `float[]`. The morph-weight buffer is therefore `struct MorphWeight { float value; }`, the
     same 4 bytes.
   * **shdc renumbers the HLSL registers** it emits (a `binding=12` storage buffer can land on
     `t0`), and strips resources a shader declares but never reads. Neither matters — the runtime
     binds by the *sokol* slot and the reflection carries the mapping — but it does mean the
     register plan in the HLSL comments is no longer the register plan on the GPU. The declared
     slots come out exactly as §4 specifies (0 checker, 1–4 IBL, 5–11 material, 12–15 deform),
     pinned by a test.
   * **A shader must declare only the views it uses**, since sokol validates that every declared
     view is bound: the blanket-include approach would force the skybox and line pipelines to bind
     seven material textures they never sample.
   * **The generated `review.rs` needs `clippy::all` silenced** at the module declaration, for the
     same reason `vendor/sokol-rust` is `exclude`d — it is machine-written, and today it trips
     `large_const_arrays` (shdc emits each shader source as a multi-KB `const [u8; N]`).
   * **`cargo fmt --all` reformats the vendored sokol tree**, and D21's `exclude` does not stop it:
     the exclude keeps the crate out of the build and lint graph, but fmt follows *path
     dependencies*, so it only started happening once `review-render` depended on sokol. It
     rewrote 60-odd upstream files to a style that is neither upstream's (their rustfmt.toml is
     half nightly-only options, silently dropped on stable) nor ours — every line of which would
     conflict at the next pin bump. The fix is `disable_all_formatting = true` in
     `vendor/sokol-rust/rustfmt.toml`, since rustfmt reads the nearest config above each file and
     the `ignore` option is nightly-only; it is the one local modification to the pinned tree, and
     `vendor/NOTICE.txt` records it.

   Two deviations, both deliberate: the `sokol` workspace dependency lands **here** rather than in
   step 3, because step 2's own verification (the generated reflection compiling, with the size
   assertions against it) requires the crate that `generated/review.rs` imports; and the four
   bake-only IBL programs are compiled by build.rs unconditionally rather than under the `bake`
   feature, so the committed blob set stays complete — `cfg` gates which ones are
   `include_bytes!`'d, which is what decides binary size. Measured cost of carrying sokol unused:
   +17.9 KB on the release binary (13,824,000 → 13,841,920), the linker having dropped nearly all
   of it.
3. **rhi core + app loop, scene stubbed** (one big-bang commit): `backend/d3d11.rs` (Fire's + the
   debug-layer fallback + `supported_sample_counts`), `Gpu`/`Frame`/jobs, `shader / pipeline /
   buffer / texture / sampler / target / mips`, `egui_sokol.rs` (D3) + the egui 0.36 bump (D6), the
   new `frame.rs` flow (§3.2), `sg_setup` on the bring-up thread (D8), `Renderer::render_*`
   temporarily returning `Ok(())`. Verify: black clear + the full egui chrome, resize, vsync
   pacing, every panel and the HDR thumbnails at 100 % and 150 %.

   *Done.* The viewer runs on sokol_gfx: `rhi/backend/d3d11.rs` is the whole platform GPU surface
   (~330 lines incl. the WARP fallback), `Gpu::start` brings the device up on its own thread from
   the first line of `main` and `GpuBringUp::attach` joins it once the window exists, `Frame` owns
   the single swapchain pass, and `egui_sokol.rs` paints the chrome into it. `app` lost its
   `windows` dependency entirely (D3's `win32_hwnd` and the three `d3d11_*` escape hatches from
   step 1 are gone, and `Gpu::resize` is infallible — all four of step 1's loose ends closed here).
   Verified: clippy clean workspace-wide, all 20 test binaries pass, and an interactive session
   against `SK_Player_01.fbx` — chrome, Outliner tree with its icons, Inspector, stats card, axis
   gizmo, toasts, status bar, workspace switching, the four option windows, and all six HDR
   thumbnails in the Environment dropdown — with **zero** output on sokol's validation channel.
   The Tex workspace's background buttons were clicked through to confirm the clear-colour path
   end to end. Not checked: 100 % display scale (the box is at 150 %, and changing it is the
   user's setting, not ours) — `theme::px` makes the chrome physically size-invariant anyway, and
   that logic is untouched by this step.

   How it was sequenced, and one deviation worth knowing:

   * **The renderer was verified on egui 0.33 first, then bumped.** D6 folds the bump into this
     step, and it did land here — but doing both at once would have made a broken panel ambiguous
     between "my renderer" and "the API migration". So the egui renderer was landed against the
     known-good 0.33 chrome, eye-checked, and only then bumped; the second eye-check then had one
     suspect. The bump costs ~10 lines in the renderer (0.36's `TexturesDelta.set` is a map of
     *several* deltas per texture, and `free` is a set) and about 30 in `ui`.
   * **egui 0.36 replaced `SidePanel`/`TopBottomPanel` with one `Panel`, shown into a `Ui` rather
     than onto the `Context`** — `Context::run` is gone in favour of `run_ui(input, |ui| …)`. So
     `draw_overlay` now takes the frame's root `&mut Ui` and the chrome carves its bands out of
     that; `Window`/`Area`/layer painters still address `ui.ctx()`. Builders renamed with it
     (`default_width`/`default_height` → `default_size`, `width_range`/`height_range` →
     `size_range`, `exact_height` → `exact_size`, `show_inside` → `show`), plus
     `is_using_pointer` → `egui_is_using_pointer` and `set_style` → `all_styles_mut`.
   * **`TexturesDelta` panics if dropped un-applied** in 0.36 — a genuinely good check, and it
     caught the headless `stats_rows` test immediately. The renderer therefore takes it *by value*
     and `clear()`s it only after every delta has been applied, so an early `?` on a failed upload
     still trips the assertion rather than swallowing it.
   * **The parked D3D11 code lives in `crates/render/src/port_pending/`, not only in git history.**
     Ten files, not declared as modules, so nothing there is compiled, linted or formatted; its
     README lists what returns in which step, plus the exact `Cargo.toml` lines step 6 must
     restore. Step 4 is then a diff against something rather than a rewrite from memory.
   * **The `bake` feature and the `bake_ibl` binary were removed, not left declared and empty**, so
     a `--features bake` build cannot pass by compiling nothing. Re-baking the IBL maps is blocked
     until step 6; the shipped viewer is unaffected, since it embeds the committed maps.
   * **The wrappers grew only as far as step 3 exercises them** — `pipeline.rs` has two vertex
     formats and one blend mode, `buffer.rs` has only the transient stream, `target.rs` and
     `mips.rs` do not exist yet, and `SwapchainJob` is deferred to step 4 with the composite that
     produces one. That is deliberate rather than lazy: the workspace lints deny warnings, so a
     wrapper nobody calls is a build error, and a wrapper nobody calls is also a wrapper nobody has
     checked. Each arrives with the stage that needs it.
   * A handful of still-live-but-unused modules (`geometry/`, `material/{mode,state}`,
     `scene/gpu_types`) carry a scoped `#[allow(dead_code)]` naming the port, because their only
     consumers are in `port_pending`. They stay compiled — and unit-tested — because none of them
     touches the GPU and all of them are what step 4 ports *against*. Those allows come off with
     the stages that use them again.
   * Release binary: 13,841,920 → **11,580,416** bytes. Recorded rather than claimed as a win —
     the scene renderer is parked and therefore not compiled, so most of that is code that comes
     back in step 4. What it does say is that sokol's C core plus our own egui renderer cost
     nothing like what `egui-directx11` + the D3D11 `rhi` did. The real number is step 8's.
   * `packaging/generate-ibl-bake.ps1` learned that the bake is parked: a normal packaging build
     gets a one-line notice and a green no-op (the committed maps are current, and nothing that
     determines their bytes is editable right now), while `-Force` or a genuinely missing map
     throws with the reason and a pointer to the README. Both paths were exercised. Its freshness
     input list also picked up `review.glsl` in place of the deleted `ibl.hlsl`.
4. **Renderer, in this order, each stage eye-checked against `main` on the same model:** Tex
   viewport (two fullscreen jobs, CPU mips) → scene single-sample without GTAO (pipeline set,
   mesh/index buffers, IBL cube upload, checkers, material table as `apply_uniforms` + patched
   bindings, grid / lines / overlays / selection, composite job; check shading, background
   gradient, tone-map operators, buffer views, selection flash) → skinning storage buffers +
   dummies (clips, skin-weight view) → MSAA (resolve views, AA menu from the leaf) → GTAO (dims via
   uniform) → Opt split (two target sets) + ghost overlay → UV viewport.

   *Stage 1 (Tex viewport) done.* `src/tex/gpu.rs` replaces the parked `tex_d3d.rs`: the two
   fullscreen programs `tex_checker` / `tex_image` from the generated bytecode, the path-keyed LRU
   upload cache unchanged, and channel isolation still one uniform. Two things arrived with it:

   * **`SwapchainJob` (the deferred draw §3.2 called for) landed here rather than with the
     composite**, because the Tex viewport needs it for exactly the same reason: every
     `Renderer::render_*` runs *before* the frame's one swapchain pass opens. It is
     `{pipeline, optional bindings, one uniform block inline, vertex count}` — all `Copy` sokol
     ids, so a queued draw keeps no borrow — and the queue lives on `Gpu` rather than `Frame` so a
     steady-state frame allocates nothing. Deliberately *not* carrying a viewport rect yet: the
     Opt split is what needs one, and a knob nobody sets is a knob nobody has checked.
   * **The CPU mip chain (D7) is built at upload, not on the decode worker.** The chain is not a
     property of the decoded image alone — the Tex viewport uploads the same pixels raw and a
     material slot uploads them sRGB, and the two chains differ, because hardware `GenerateMips`
     averages an sRGB view in *linear light* and a UNORM view in stored bytes. `rhi/mips.rs` does
     both (the sRGB conversion through a 256-entry forward table and a 4096-entry inverse one, so
     a 4K chain costs no `powf` per texel), and the cost is a few milliseconds on the frame a
     texture is first looked at. Moving it to the worker would mean carrying *two* chains per
     decoded image; revisit with the material stage, which is the one that would benefit.

   Verified: clippy clean workspace-wide, all 20 test binaries pass (four new unit tests pin the
   mip filter, incl. the linear-light sRGB average), and three runs against
   `assets/test_textures/T_Sides_D.psd` — checker background with the image composited over it and
   the background showing where the shader discards outside the rect; a solid grey background with
   the image minified to 12 % (cleanly filtered, so the chain is being sampled); and a 6×
   magnification showing hard texel edges, which is the min-linear/mag-point sampler behaving.
   Channel isolation checked on G and R. **Zero** output on sokol's validation channel across all
   three.

   *Stage 2 (the scene, single-sample, no GTAO) done*, and with it the UV viewport and the skinning
   storage buffers that were sequenced as their own stages — both came along because the scene port
   is wholesale rather than incremental: `scene/{pipelines,resources,gpu}.rs`, `material/gpu.rs`,
   `rhi/target.rs` and `ibl.rs`'s runtime half replace the parked D3D11 files, and `render_scene` /
   `render_uv_scene` draw again. `scene/resources.rs` is a near-mechanical port (every `sync_*` lost
   its `&Gpu` and nothing else); `scene/gpu.rs` is a rewrite, because the pass and binding model
   changed underneath it.

   What the sokol model actually changed, beyond the mechanical:

   * **Bindings are per *program*, not per pass**, and shdc decides what a program declares by what
     it *reads*. There is no `bind_scene_shared` any more: each draw assembles the set its own
     program asks for, and binding one resource too many is as much a validation failure as binding
     one too few. Three cases had to be discovered rather than deduced — `fs_main` never samples the
     environment cube (the skybox does), so the mesh binds three IBL maps and not four; `fs_line`
     reads nothing from the fragment-stage scene block, so shdc strips it and the line pipelines
     apply only the vertex block while `fs_selection` takes both; and `vs_main` declares all four
     deform storage buffers, so **every** program sharing it must bind all four — which is why
     `SceneGpu` owns one-element `DeformDummies` for a model with no skin. §3.2 anticipated the
     dummies; it did not anticipate the other two.
   * **`SwapchainJob` grew a viewport rect** and the composite became one, while the offscreen scene
     pass is issued directly — the split §3.2 called for.
   * **Front faces are counter-clockwise, and sokol's default is the opposite.** The old D3D11
     rasterizer set `FrontCounterClockwise = TRUE`; sokol's `face_winding` defaults to CW, so the
     pipelines must say `Ccw`. This is the one defect that survived to the eye-check, and it is
     worth knowing how it *looked*: not a subtle shading difference but a closed mesh rendering as
     its own interior — back-face culling keeping the far side of every solid — which reads as a
     translucent model with the grid showing through it. Two other hypotheses (a broken depth test,
     a reversed-Z mix-up) fit that symptom equally well, and both were wrong; what settled it was
     probing the depth buffer by flipping the line pipelines' compare function and watching *where*
     the grid survived.

   Deliberately not carried over in this stage: MSAA (which follows immediately), GTAO (the three
   passes and their targets), and the Opt comparison view. Each is an addition to the sokol code
   rather than a port of the parked D3D11, so those files left `port_pending/` with this stage.

   Verified: clippy clean workspace-wide, all 20 test binaries pass (the `SceneVertex` layout test
   came back, now asserting the sokol attribute list against the struct's field offsets), and a
   side-by-side against a `main` build in a worktree, same window and same model:
   `meter_cube.fbx` (opaque, identically framed, identical face shading, grid correctly occluded)
   and `SK_Player_01.fbx` (skinned into the file's default pose, same silhouette, same materials,
   same IBL). The only visible difference is the missing ambient-occlusion contrast in creases,
   which is the GTAO stage. **Zero** output on sokol's validation channel. Not compared against
   `main`: the UV viewport (it draws its grid, islands and edges, but its own eye-check belongs to
   its stage) — and, as in step 1, no workspace or panel reachable only by clicking, since synthetic
   input does not reach the winit window from an agent session.

   *Stage 3 (MSAA) done.* A `ColorTarget` at 2×+ now carries a single-sample twin: the attached
   image is multisampled, the twin holds the texture view, and a `resolve_attachment` view over it
   goes in the pass's `resolves` slot. **sokol resolves at `end_pass`**, so the old explicit
   `ColorTarget::resolve()` — and the ordering rule that it had to run after the RTVs were unbound —
   has no successor at all; there is nothing left to forget. The two ends are *exclusive* usages
   (`color_attachment` on the multisampled image, `resolve_attachment` on the twin, never both),
   `DepthTarget` takes the same sample count, `PipelineDesc` carries one, and `sync_targets` rebuilds
   the pipeline set before the targets so a failure leaves a consistent pair. The capability clamp
   (invariant 4) is `Frame::clamp_msaa`, which lives on the frame because the query needs the device
   and the scene renderer is handed a frame, not a `Gpu`; the raw request is cached so the adapter is
   only re-asked when it moves.

   Verified: clippy clean, tests green, and measured rather than eyeballed — the silhouette edge of
   `meter_cube.fbx` at row 800 goes `0,0,0 → 242,227,217` single-sample and
   `0,0,0 → 60,57,54 → 242,227,217` at 4×, and that coverage pixel is **byte-identical** to `main`'s
   at the same row (main's interior differs only by its GTAO darkening). The rebuild path was run
   rather than reasoned about: a temporary hook cycled the level through Off/2×/4×/8×/16× every 20
   frames for ~45 s, captures taken mid-run show the edge alternating between aliased and resolved,
   and sokol's validation channel stayed **silent** for the whole run.
5. **Shell leaves on Windows**: `rfd` on workers (D9), `dirs` config dir (D17), `Primary` modifier
   plumbing (D10, still Ctrl here), the startup error box after `run_app`, `ScaleFactorChanged`
   forwarding.
6. **Profiler + bake** (D18, D19) on D3D11: `--tracy` GPU zones match the old capture; `bake_ibl`
   reproduces the committed `.bin`s byte-for-byte (same GPU, same encoder settings).
7. **Cleanup**: `hlsl/` deleted and its hand-listed `build.rs` jobs with it; the last
   `#[allow(dead_code)]` naming the port removed; `port_pending/` empty and gone; clippy clean on
   Windows. (`windows` left `app` and `egui-directx11` left the workspace in step 3, ahead of
   schedule — both were one-line consequences of the swap rather than cleanup.)
8. **Gate (D2)** with `scripts/gate.ps1` against `main`; fix or document any regression; merge.

### Phase 2 - macOS

1. Mac setup per §7; `cargo test --workspace` green (everything is headless). First light without
   a line of new code, as in Fire.
2. `rhi/backend/metal.rs` from Fire verbatim (+ `supported_sample_counts`); `build.rs`'s
   `compile_shaders_metal` per (program, stage) with the committed-blob gate (D5); first
   `cargo run`. Verify: shaded model, GTAO, MSAA levels offered = 1/2/4/8, a skinned clip playing,
   Opt split, UV, Tex, the egui chrome - each by eye against the Windows build's screenshots.
3. Leaves: `openfiles.rs` (D14), `menubar.rs` (D15), `Primary = ⌘` + labels (D10), pinch (D16),
   `dev-app.sh`. Verify with `open -a` on a cold app, on a running app, and a Finder drag onto
   the Dock icon.
4. Retina: physical-pixel scene targets from `scale_factor` (`theme::px` already converts the
   chrome); verify a 2× screenshot's chrome heights and that the scene viewport rect maps to whole
   physical pixels.
5. Profiler / bake leaves for Metal (D18 / D19); run `bake_ibl` on the Mac and diff the `.bin`s
   against the committed ones (expect BC6H-identical or a documented tolerance).
6. Measure (Fire's `ttfp.sh` twin as `gate.sh`): the startup breakdown; no cross-OS budget, the
   number to beat is the next mac build's.
7. `scripts/build-mac.sh` (Fire's, with the `.fbx` document type + `application-logo.png` →
   `.icns`), sign, notarize, staple, `.dmg`; the `spctl -a -t open` check.

### Phase 3 - cleanup

Delete any D3D11 drawing code left behind; rewrite `CLAUDE.md` §1 invariant 9 (the GPU site is
`rhi/backend/*` + the two leaves), §2 (`rhi` = sokol wrappers), §3 (build on both OSes), §4
(supersede D3D11 with sokol_gfx; the wgpu ban stands), §5/§6 (shader source, mip chain, sample
counts); retire the stale `docs/PROJECT_STATE.md` / `docs/RENDERING_PIPELINE.md` (both still
describe wgpu) in favour of an updated pipeline doc; third-party notices for sokol (Zlib) and
psd_sdk.

---

## 9. Risks

- **The renderer rewrite is large, not a leaf** (~2000 lines of `rhi` + 1700 of shader + the egui
  renderer). Mitigation: step 1's façade refactor, then a strict port order with an eye-check per
  stage, on the OS whose output is known-good.
- **Things in the current code that make steps harder than they look**: the composite owning and
  clearing the backbuffer (→ pass clear + deferred jobs); the shared target set in the Opt split
  (→ two sets); `bind_scene_shared` binding once per pass (→ a bindings value re-applied after every
  `apply_pipeline`, plus two scene-uniform applies); `unbind_*` and `resolve()` (→ delete);
  `t14`/`t15` left unbound (→ dummies); `Gpu::resize` returning `Result` into `input.rs` (→
  infallible); `GenerateMips` on the immediate context (→ CPU chain on the material sync path,
  then onto the decode worker); the `ID3D11DeviceContext` threaded through every
  `record_*`/`sync_*`; `App` field order vs `sg::shutdown`.
- **sokol's `dynamic_update` usage is marked deprecated** in the vendored header. The palette uses
  it; the fallback is `write_transient` every frame from the kept scratch buffer.
- **Uniform-block and storage-struct layout drift** between the generated Rust structs and the
  hand `#[repr(C)]` ones. Mitigation: the paired size assertions (§4); `MorphEntry` as scalars.
- **`MaterialUniform` per draw range through `sg_apply_uniforms`** - a model with thousands of
  ranges could exceed `uniform_buffer_size` (default 4 MB/frame; 96 B aligned to 256 on Metal ≈
  16k ranges). Set the size explicitly; ranges are per material/part, not per triangle.
- **egui atlas recreation**: a delta arrives every frame while fonts warm up; recreating a 2048²
  image each frame for a few frames is fine, but a texture that changed *every* frame forever
  (none today) would need `stream_update`. Watch the HDR thumbnails and icons (egui-managed, static).
- **The Metal MSAA menu shrinks** (no 16×). The AA toggle already gates on the reported list.
- **Tracy GPU on Metal is frame-level at best** (D18). Accepted.
- **D5's two-host bytecode discipline**: a shader edit committed from one OS ships stale bytecode
  for the other. `build.rs` warns when a blob is older than its generated source; a pre-release
  check in both packaging scripts fails on it.
- **`rfd` on a worker + `set_parent`**: Fire proved the pattern on both OSes; keep one dialog at a
  time (`dialog_running`).
- **The toolchain floor on the Mac for shader edits** is full Xcode + the 839 MB Metal toolchain
  (D5 keeps it off everyone else). The stub-`metal` failure is near-silent; the build must *run*
  the tool, not `--find` it.
- **Notarization**: budget an afternoon for the first submission; Fire's script already encodes
  hardened runtime + timestamp + `ditto` packaging, and the same Developer ID / `notarytool`
  keychain profile serves both products.
