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

Status: **Phase 0 and Phase 1 done on Windows** (the merge excepted), and **Phase 2 steps 1–7 done
on the Mac** — everything but the notarization credential. The backend swap is complete on both
OSes: every draw path — the frame flow, the egui chrome, the Tex viewport, the 3D and UV scenes
with MSAA and GTAO, and the Opt workspace's comparison view — runs on sokol_gfx, over Direct3D 11
on Windows and Metal on macOS, with a ~200-line device/swapchain leaf per OS and nothing else
platform-specific in the renderer. The viewport is **pixel-identical** to a `main` build on the
models checked, and **all three D2 budgets pass** on Windows with the frame **43–61 % faster** and
memory slightly *down*.

On the Mac the viewer builds, runs and draws; the shell leaves are in (Finder opens, the menu bar,
⌘ as the primary modifier, pinch-zoom, the config directory); Retina needed no code; the Tracy GPU
profiler and the offline IBL bake both work on Metal; `scripts/gate.sh` has recorded the baseline
numbers; and `scripts/build-mac.sh` produces a signed, verified `.dmg`. What is left is the Phase 1
merge, notarization (which needs a `notarytool` keychain profile), and Phase 3's documentation
sweep. Sections are written in the present tense of the finished port so they can become the
description once it lands; the *Status* column of §2 and the phase list in §8 say what is actually
done.

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
| D1 | **Shared shell: winit + sokol_gfx on both OSes** (*owner*); the only per-OS GPU code is `render/src/rhi/backend/{d3d11,metal}.rs` (device + swapchain), aliased as `backend` exactly like Fire's `render/mod.rs` | Minimum platform code; two full renderers (the D3D11 rhi + a Metal twin behind a trait) is the maintenance Fire D1 rejected | `rhi/` rewritten; Windows re-measured (D2) | Shipped on both OSes (Phase 1 step 3/4; Phase 2 step 2) |
| D2 | **Windows migrates too, gated on three measurements** (*owner*) against `main` on the same box, same fixture, release build: (a) cold + warm startup to first presented frame with a model loaded; (b) steady-state frame time (GPU + CPU) on a fixed test model with GTAO on + 4× MSAA + a skinned clip playing; (c) private RAM + VRAM at idle with that model. Budgets: ≤ 10 ms startup median, ≤ 0.3 ms frame median, ≤ 5 % RAM/VRAM | The invariants stress runtime speed and footprint (`CLAUDE.md` §4: "spend dev-time freely to make the release runtime fast") | A harness: `scripts/gate.ps1` + a `--gate-out` stamp in `app` (Fire's `ttfp.rs` shape, extended with a frame-time and memory dump) | Measured on Windows (Phase 1 step 8): startup +4.8 ms, frame −0.53 ms, RAM −2.8 %, VRAM −1.0 % — all three pass. Mac baseline recorded (Phase 2 step 6), no cross-OS budget |
| D3 | **Keep egui; write `render/src/egui_sokol.rs`**, a sokol_gfx egui renderer, replacing `egui-directx11` on both OSes (*owner*) | No sokol egui backend exists; switching to Dear ImGui like Fire would rewrite the whole `ui` crate and retire the native-windowing invariant | ~300–400 lines + one `@program`; consumes egui's `ClippedPrimitive`s + `TexturesDelta` (§3.4) | Shipped (step 3) — 380 lines |
| D4 | **One sokol-shdc annotated-GLSL source** (`render/src/shaders/review.glsl`) generates per-backend HLSL5 + MSL *and* the `ShaderDesc` reflection into `render/src/shaders/generated/`, checked in; build.rs compiles the host's set to bytecode (*owner*) | Fire D4/D24: nothing compiles a shader at runtime, a broken shader is a build error, one source instead of two hand-kept twins | The HLSL is rewritten once (~1700 lines); `fxc` on Windows, the Metal toolchain on macOS (D5) | Shipped on both OSes: the MSL half compiles to `.metallib` on the Mac (Phase 2 step 2), warning-free under `-Werror` |
| D5 | **Bytecode is committed and freshness-gated on both OSes** - `.dxbc` (as today) *and* `.metallib` - compiled only when the generated source is newer or the blob is missing and the toolchain is present; missing blob + missing toolchain is a build error | Keeps this repo's existing "a no-fxc box builds from the committed blobs" property and extends it to the Mac, so the toolchain floor for someone who never edits a shader is Command Line Tools, not full Xcode. *Diverges from Fire* (which compiles into `OUT_DIR` on every build) on purpose | A shader edit must be followed by a rebuild on **both** OSes before commit, or one platform ships stale bytecode; `build.rs` warns when a blob is older than its generated source, and both packaging scripts fail on it | Shipped: both hosts' blobs committed, freshness keyed on content (`bytecode.manifest`), enforced by `packaging/check-shader-bytecode.{ps1,sh}`. Plus `generated/review.glsl.sha256`, which closes the one link the manifest could not see |
| D6 | **egui bumped to 0.36.1** (`egui`, `egui-winit`, `egui-notify 0.23`) in the same port (*owner*) | Dropping egui-directx11 removes the only 0.33 pin | API churn in `ui` folded into the port; verify egui-notify 0.23 tracks 0.36 at execution, else pin egui to the newest it supports | Shipped (step 3); egui-notify 0.23 does track 0.36 |
| D7 | **CPU mip chain**, uploaded in the one `sg_make_image` | sokol_gfx has no `GenerateMips`; sRGB textures are averaged in linear light so the result matches the hardware path | ~5 ms per 4K texture | Shipped (step 4 stage 1) as `rhi/mips.rs`, but built **at upload**, not on the decode worker: the chain differs between the raw and sRGB uploads of the same pixels, so it is not a property of the decoded image (see step 4) |
| D8 | **GPU bring-up on its own thread from the first line of `main`; the window is created before the join** (Fire D18) | Device creation is the longest startup item (~135 ms for D3D11 on Fire's box) and needs no window | `App::start` reorders; `backend::Device: Send` | Shipped (step 3) |
| D9 | **Every `rfd` dialog runs on a worker thread and answers through `UserEvent`** (Fire §3.5: nothing called from a winit callback may pump a loop of its own - on macOS AppKit aborts the process) | Six blocking sites today (`loading.rs`, `texture_manager.rs`, three in `opt.rs`, the startup box in `main.rs`), three of them *inside* `render()`; on macOS this is a reliable crash, not a glitch | `Dialog` enum + `UserEvent::DialogDone`, one dialog at a time; the startup error box moves to after `run_app` returns | Shipped (step 5) — `app/src/dialog.rs` |
| D10 | **`Primary` modifier: Ctrl on Windows, ⌘ on macOS**; `help.rs`/`stats.rs` labels say "Ctrl"/"Cmd" per OS | Cmd is the only acceptable file-command chord on a Mac | `shortcuts.rs` matches `SUPER` on macOS; the Alt+RMB zoom-drag stays Option | Shipped (step 5); still `Ctrl` on this OS, `Cmd` arrives with the `cfg` |
| D11 | **Apple Silicon only** (Fire D10, *owner*); every vendored C tree (ufbx, meshoptimizer, ufbx_write, psd_sdk) is compiled from source by `cc`, so nothing is prebuilt per target | No Intel users to serve; unlike Fire's HEIF `.a`s there is no prebuilt native dep to re-vendor | `lipo` check in `build-mac.sh` | Shipped (Phase 2 step 7) |
| D12 | **Build, sign, notarize only on the dev Mac via `scripts/build-mac.sh`** (Fire D11/D12, *owner*); `Info.plist` heredoc from `product.json`; `.icns` from the 1024² master `assets/icons/application-logo.png`; `.fbx` in `CFBundleDocumentTypes` with `LSHandlerRank = Alternate` | Mirrors `packaging/build-windows-installer.ps1`; keeps the Developer ID cert off any shared machine. `Alternate` volunteers for `.fbx` in "Open With" without taking it from anything | `scripts/dev-app.sh` for the unsigned dev bundle - needed to test Finder opens at all, since a bare binary is not what `open` delivers files to | Shipped (Phase 2 step 7); signed `.dmg` verified. The notarytool profile is **account-level, not per-product** — one Apple ID + team + app-specific password notarizes everything this team signs — so `build-mac.sh` defaults to the existing `fire` profile rather than a product-derived name it would then always need a flag to override |
| D13 | **Windows first, then macOS** (*owner*) | The shared code and the gate risk are the Windows migration; mac is leaves + packaging | The Mac waits one phase | Decided |
| D14 | **Open-file events via `openfiles.rs`** (Fire's `class_addMethod` hook adding `application:openURLs:` to winit's delegate) feeding `App::open_model_from_path`; opens that arrive before the window are held and handed to `start()` | Launch Services gives a fresh launch *no argv* and a running app *no new process*; without it `.fbx` association does nothing on macOS | ~150 lines, in the new `crates/shell-macos` rather than in `app`, which stays `forbid(unsafe_code)`; no IPC needed (one window, one process, no single-instance socket) | Shipped (Phase 2 step 3); verified cold and on a running app |
| D15 | **Minimal `muda` menu bar** (App / File: Open… ⌘O, New ⌘N / Window) with `with_default_menu(false)`; `Open…` routes through D9 (*owner*) | A Mac app without a menu bar reads as broken; winit's default menu would replace ours wholesale, and it is where ⌘Q comes from, so ours must carry Quit | ~120 lines `menubar.rs` in `crates/shell-macos`; accelerators intercept keys before winit sees them, so only the two app items carry one | Shipped (Phase 2 step 3) |
| D16 | **Pinch → wheel zoom** on the orbit, UV and Tex cameras (Fire D15); gesture math stays in logical px, rendering in physical px | Trackpad users | `WindowEvent::PinchGesture` through the same `zoom_active_camera` the wheel uses, NaN-filtered | Shipped (Phase 2 step 3) |
| D17 | **`window.cfg` via `dirs::config_dir()`** (`%APPDATA%` on Windows - the same path as today - `~/Library/Application Support` on macOS) | Drop the `APPDATA` env read in `window_state.rs`, which fails soft on macOS today | `dirs 6` | Shipped (step 5); a unit test pins the Windows path unchanged |
| D18 | **Tracy GPU profiler: per-OS leaves over sokol's native handles** (*owner*). D3D11: the existing timestamp-query code unchanged, over `sg_d3d11_device()` / `sg_d3d11_device_context()` (the immediate context; `ctx.End(query)` between sokol calls is valid). Metal: per-pass zones are *not* achievable (sokol owns the command buffer and encoder; Apple silicon has no draw-boundary counter sampling) - the leaf brackets sokol's buffer with two sentinel command buffers on `sg_mtl_command_queue()` and reports their `GPUEndTime`s as one "GPU frame" zone, plus `sg_query_stats()` counts as plots on both OSes | Recorded honestly | `rhi/gpu_profiler.rs` keeps its `Zone` API plus a backend-written `Zone::Frame`; `zone_*` are no-ops on Metal; Xcode's Metal profiler covers per-pass timing there | Shipped on both (Phase 1 step 6; Phase 2 step 5) — 226 GPU zones captured on Metal |
| D19 | **`bake_ibl` on sokol_gfx** (*owner*): `Baker` = headless `Gpu` (device, `sg_setup`, no swapchain); `CubeTarget` = one cube image + 6×mips attachment views (`mip_level` + `slice`); render every pass for one environment → `sg_commit` → per-OS readback leaf `backend::read_image_subresource`: D3D11 staging copy + `Map(READ)` over `sg_d3d11_query_image_info` (today's body); Metal blit from the private texture to a shared `MTLBuffer` on our own command buffer from `sg_mtl_command_queue()`, committed *after* `sg_commit` (queue order = commit order) + `waitUntilCompleted`. The old RTV/SRV-hazard `unbind_*` calls go away (passes are closed) | Fire D23: the whole dev pipeline runs on the Mac; a re-bake must not need Windows | sokol uses unretained command-buffer references, so every image stays alive until its readback returns | Shipped on both (Phase 1 step 6; Phase 2 step 5). Byte-identical on Windows; on Metal **numerically equivalent, not byte-identical** — ≤0.021 on a 0..1 BRDF LUT, GPU float precision, so the committed Windows bake stays the one shipped set |
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
* **Two-host bytecode discipline (D5), enforced rather than remembered:**
  `crates/render/build.rs` records what each committed blob was compiled from in
  `src/shaders/generated/bytecode.manifest` — SHA-256 of the generated source and of the blob, one
  row per program, both hosts' rows in the one file — and rebuilds a blob whose row no longer
  matches. `packaging/check-shader-bytecode.ps1` fails the installer build on any mismatch, and
  warns (never fails) about the other host's rows, since a Windows installer ships no `.metallib`.
  The Mac's `build-mac.sh` owes the mirror of that check.

  Content, not mtimes, and the reason is specific: a shader edited on the Mac arrives here as a new
  generated source beside a blob a revision behind, both written by `git checkout` in the same
  instant. "Is the source newer than the blob?" is a coin toss on the one occasion the answer
  matters, which is why the old timestamp gate had to go. The digest is SHA-256 rather than
  something smaller only so both packaging shells can recompute it (`Get-FileHash`,
  `shasum -a 256`) without a toolchain.

  What no mechanism catches: editing `review.glsl` and forgetting to run `scripts/gen-shaders`. The
  blobs would match their committed sources perfectly and the whole set would simply be a revision
  behind. `build.rs` warns when `review.glsl` is newer than everything generated from it — the one
  staleness question timestamps *can* answer, because it is asked of a working copy where the edit
  just happened.
* **Dev bundle:** `scripts/dev-app.sh` wraps `target/{debug,release}/3d-review` in an unsigned
  `.app` (`com.psmyles.3d-review.dev`); the only way to exercise D14 and Retina before packaging.
* **Gate harness (D2):** `scripts/gate.ps1` (Windows; a bash twin later for mac-vs-mac numbers):
  N interleaved launches of A and B with a fixture FBX, reading the `--gate-out <file>` stamp that
  `app` writes (`crates/app/src/gate.rs`) — first present with the model up, then median frame time
  over 300 frames of a scripted orbit with GTAO + 4× + a clip playing. One warm-up launch of each
  before measuring; the first launch after a build is an outlier (Fire step 7). *Built in step 8*,
  with two deliberate departures from this sketch:

  * **Both ends of a measurement live where they can be read honestly.** The stamp carries the
    wall clock at first present; the *process-creation* end is `Process.StartTime`
    (`GetProcessTimes`) read by the script, because `app` is `forbid(unsafe_code)` and has no
    business calling it. Memory is sampled from outside too — the stamp is written and the process
    then idles with the model resident for four seconds, which is exactly the state D2 asks about,
    so private bytes come from `Get-Process` and VRAM from the `GPU Process Memory` performance
    counter rather than from a `DXGI_QUERY_VIDEO_MEMORY_INFO` call that the backend leaf would then
    owe a `MTLDevice.currentAllocatedSize` twin for. Nothing was added to the shipped binary to
    learn what Windows already reports about any process.
  * **`--gate-out` ships in the release binary** rather than behind a `gate` feature. A
    feature-gated harness measures a binary nobody ships; the cost of not gating it is one
    `Option<Gate>` test per frame.

  The baseline needs the same instrumentation, and `main` predates it, so the patch that adds it is
  committed as `scripts/gate-baseline.patch` — the measurement is reproducible rather than taken on
  trust, and `crates/app/src/gate.rs` inside it is byte-identical to the port's copy.

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

   *Stage 4 (GTAO) done.* The three passes are back — a single-sample mesh-only G-buffer (view
   normal + Z) with its own depth, then the fullscreen occlusion and 5×5 bilateral blur into `R8` —
   and the composite binds the blurred result instead of its placeholder. The pass model made one
   thing simpler than D3D11: **nothing is unbound between them**. A sokol pass ends before the next
   begins, so a target a pass wrote is free for the next to sample, and the old `unbind_ps_srvs`
   calls that hand-guarded that hazard have no successor.

   One defect, and it is the interesting part of this stage. `review.glsl` (step 2) repurposed the
   two previously-unused `w` slots of `gtao_params` to carry the **target size in pixels**, because
   sokol has no `GetDimensions` and the shader cannot ask; the D3D11 uniform builder this stage
   ported from left both at zero, and nothing complains — the horizon search simply becomes one
   pixel wide, which is a scene with no ambient occlusion in it at all. It was found by arithmetic
   rather than by eye: the shaded cube read 21 % brighter than `main` on its side faces and *exactly
   equal* on its top, and the shape of that difference — a term proportional to the ambient
   attachment, zero where nothing occludes — is the composite's AO compose and nothing else. Two
   hypotheses died on the way (a mis-ported shading path; a difference in the environment maps), the
   second ruled out by the skybox coming back pixel-identical, which exercises the env cube, the
   composite and the tone map but no AO.

   Worth recording as a method note: an A/B run of `main` with GTAO disabled reported *no*
   difference, which pointed away from AO entirely and cost an hour. The probe had been silently
   dropped from the worktree by a later edit. **Check that a probe is actually in the binary you are
   about to measure**, not just that you wrote it.

   Verified: clippy clean, tests green, and the whole viewport diffed against a `main` build in a
   worktree at 98,400 sample points per model — `meter_cube.fbx`, `SM_Speaker_01a.fbx` and
   `SK_Player_01.fbx` come back **pixel-identical**, maximum channel-sum delta 0, with IBL on and
   off. Zero output on sokol's validation channel.

   *Stage 5 (the Opt comparison view) done, and step 4 with it.* `scene/opt.rs` replaces the parked
   `scene_opt.rs`: the split, the x-ray and wireframe ghosts, and the ghost's tint through the
   `selection_color` uniform, all over pipelines that already existed.

   The one design change §3.2 called for and this stage had to make: **the split needs two target
   sets.** The parked D3D11 version rendered both halves through one set and composited in between;
   with the composite a deferred job, both halves' passes have run before either composite does, so
   a shared set shows the second view in both — which is exactly what the first run of this stage
   did, the source half rendering the processed mesh. The seven per-view targets are now a
   `TargetSet`, `SceneGpu` holds a second one built only by the split (at half width, so the pair
   costs what one full-width set did) and released the moment a single view is drawn.

   Two things fell out of the pass model rather than being ported: the ghost needs no
   "restore the frame's uniform afterwards" step, because sokol applies uniforms per draw and the
   next draw's own call is the restore; and the split's black backdrop is the swapchain pass's clear
   rather than a `clear_backbuffer` of its own.

   Verified against a `main` build in a worktree, both layouts, same window and model
   (`SM_Speaker_01a.fbx` with one Reduce operation, so the two meshes genuinely differ): the
   **overlay is pixel-identical** (0 of 98,400 samples differ), and the **split** differs on 2.7 % of
   samples — traced to the chrome, not the renderer. The Opt split is the only path that renders
   into the UI's *measured* chrome-free rect rather than the whole backbuffer, and that rect is one
   pixel narrower under egui 0.36 than under 0.33 (left edge 462 vs 461), which scales each half by
   a fraction of a pixel. Every full-backbuffer path, the overlay included, matches exactly.
5. **Shell leaves on Windows**: `rfd` on workers (D9), `dirs` config dir (D17), `Primary` modifier
   plumbing (D10, still Ctrl here), the startup error box after `run_app`, `ScaleFactorChanged`
   forwarding. *Done.* `ScaleFactorChanged` turned out to have landed already in step 3, with the
   backend leaf that owns it; the other four are this step.

   **D9** is the one with substance. Six blocking `rfd` calls became one `dialog.rs`: a `Dialog`
   request (carrying whatever its answer will act on) goes to a worker, the worker blocks on the OS
   dialog, and `UserEvent::DialogDone` applies the answer on the main thread — the shape
   `loading.rs` and `texture_manager.rs` already had. Two things the blocking version got for free
   had to be arranged: **one dialog at a time** (a modal made a second unreachable; nothing freezes
   now, so `App::ask` drops a request that arrives while one is up), and **the payload is captured
   when the dialog opens, not when it closes** — the workspace stays live behind the picker, so
   an export or a preset save that re-read `self` on the way out would race a background run or a
   slider drag with no visible cause. The request carries its own subject out and back instead.
   The startup error box is the one dialog that still blocks, and legitimately: `main` raises it
   after `run_app` has returned, where there is no loop left to re-enter.

   Verified on the running viewer, since none of this is unit-testable. Synthetic input is
   unavailable in this environment (`GetForegroundWindow` returns 0, so neither `mouse_event` nor
   `keybd_event` lands anywhere), so a temporary `REVIEW_DIALOG_PROBE` drove it and was removed
   afterwards. With the picker up: the dialog window is a `#32770` on a **worker** thread id, never
   the winit thread's; the viewer window resized 2575×1407 → 1280×800 *behind* it and came
   back correctly re-laid-out and re-rendered, which is the whole claim — the old code would
   have been frozen inside the OS modal loop. Cancelling it (`WM_CLOSE`) round-tripped to the main
   thread and re-armed three times, each on a fresh worker thread, with no leaked `dialog_open`. A
   canned answer posted through the same path loaded a different model, framed it and filled the
   Outliner, so the accept side is covered too. An injected startup failure raised the error box
   from `main`'s own stack with the viewer window already gone — 32 threads left, not 97.

   One thing that looked like a bug and was not: an oversized window (20000 px tall, past D3D11's
   16384 limit) makes `ResizeBuffers` fail and `GetBuffer` fail after it, which reads alarming in
   the log. It recovers completely on the next in-range resize, exactly as `d3d11.rs`'s comment
   claims. Left alone.
6. **Profiler + bake** (D18, D19) on D3D11: `--tracy` GPU zones match the old capture; `bake_ibl`
   reproduces the committed `.bin`s byte-for-byte (same GPU, same encoder settings). *Done, and the
   byte-for-byte criterion is met on all 19 files* — which also says the shdc-generated HLSL from
   `review.glsl` is bit-identical in effect to the hand-written `ibl.hlsl` it replaced.

   **D18** splits the way §5 wants a leaf split: `backend::GpuTimer` *measures* (a ring of D3D11
   timestamp + disjoint queries), `rhi/gpu_profiler.rs` *reports* (the `Zone` set, the Tracy GPU
   context, the spans), and they meet at `backend::FrameTimings` so the Metal twin — which can only
   bracket a whole frame — drops in without touching the Tracy side. It records into the same
   immediate context sokol submits through, using our own `backend::Device` handles rather than
   `sg_d3d11_device_context()`: the same objects, since we created them and handed them to
   `sg_setup`, without rebuilding a COM pointer from a `*const c_void`.

   One design change against the parked code, and it is a correctness fix rather than a preference.
   The old profiler *declared* which zones a frame would encode (`frame_mask(gtao_active)`) and
   skipped the rest at readback. Timestamp queries are reused across ring slots, so a query left
   un-`End`-ed this frame still reads back what it held four frames ago — a declared-but-unwritten
   zone reports those stale ticks as a perfectly plausible duration. And the Tex viewport renders no
   scene pass at all while the UV path does not go through `record_view`, so on those frames
   `end_frame` would `End` a disjoint query that was never `Begin`-ed. The mask is now **measured**:
   `GpuTimer::timestamp` records a bit per slot it actually writes, a zone is emitted only when both
   halves were written by the frame the timings belong to, and an `open` flag makes the timestamp and
   close calls no-ops on a frame that never opened one.

   Verified with a real capture (`tracy-capture` 25 s against a `--tracy` run, redraws forced by
   resizing, then `tracy-csvexport -g`): all five zones present with sane relative costs — GTAO
   Occlusion 428 µs median > Scene Geometry 309 > GTAO Blur 137 > Composite 84 > GTAO G-Buffer 40 —
   275–276 events each across ~280 drained frames, and the five `sg_query_stats` plots reporting
   (5 passes, 83 draws, 7 pipeline binds, 1712 uniform bytes per frame).

   **D19** is smaller than the parked version because sokol does most of what it did. `rhi/bake.rs`
   is a headless `Gpu` (device, `sg_setup`, no swapchain), a `CubeTarget` (one attachment view per
   face/mip via `ImageViewDesc`'s `mip_level` + `slice`, plus a cube texture view) and a `Target2D`;
   what went away is every RTV, every SRV slot, and the manual unbinding — the RTV/SRV hazard that
   once baked six flat black cubes cannot arise when a pass is closed by `sg_end_pass`. What is left
   that sokol cannot do is readback, which is the `backend::read_image_subresource` leaf.

   Two bugs on the way, both worth recording because both were **silent**:

   * sokol *rejects* an empty `sg_apply_bindings` rather than ignoring it, and the BRDF LUT pass is
     the one that reads nothing at all. `SwapchainJob` already carried an `Option` for exactly this
     rule and the bake helper did not.
   * `dxgi_format` in the backend ended in `_ => R8G8B8A8_UNORM`. That was harmless while
     `supported_sample_counts` was its only caller — it asks about the two scene formats and nothing
     else — and silently wrong the moment the readback reused it: the BRDF's `Rg16Float` source got
     an `Rgba8` staging texture, and `CopySubresourceRegion` between mismatched formats is a no-op
     that reports success. The LUT read back as its own cleared zeroes, which in `Rg16Float` is
     byte-identical to "the pass never ran". It is now exhaustive with no catch-all, so adding a
     `Format` is a compile error here.

   Two method notes. The bake's `reject_degenerate` guard is what turned both of those into a clean
   failure instead of six destroyed environments — it overwrites committed assets in place, and it
   refused to write. And the first run was `--release`, where sokol's validation layer is **compiled
   out**: the empty-bindings rejection was invisible until the same run was repeated in debug. Run a
   misbehaving bake in debug before reasoning about it.
7. **Cleanup**: `hlsl/` deleted and its hand-listed `build.rs` jobs with it (step 6 removed its last
   reader — the parked bake was the only thing still naming `hlsl/ibl.*.dxbc`); the last
   `#[allow(dead_code)]` naming the port removed; `port_pending/` is already empty but for its
   README, so this is the commit that deletes it; clippy clean on Windows. (`windows` left `app` and `egui-directx11` left the workspace in step 3, ahead of
   schedule — both were one-line consequences of the swap rather than cleanup.) *Done.*

   `src/hlsl/` is gone — five hand-written sources and the twenty committed `.dxbc` blobs — and
   with it `build.rs`'s `LEGACY_SHADERS` table and the `legacy_jobs` resolver that read it. One
   discovered job list over `src/shaders/generated/` is all that remains, and since SPIRV-Cross
   names every generated entry point `main`, `ShaderJob` no longer carries an entry-point field at
   all: the compile step is now exactly "every `review_*_hlsl5_*.hlsl` in that directory, stage
   from the filename". `port_pending/` and its README went in the same commit, as that README
   asked. No `.hlsl` filename is named anywhere in the docs any more either — every reference is
   now to a `review.glsl` program.

   The `#[allow(dead_code)]`s the step called for turned out to be attached to **live** code:
   `ghost_wireframe_buf`, `ghost_wireframe_baked` and `release_ghost_wireframes` in
   `scene/resources.rs`, each reading "the Opt comparison view is the last stage of the port" — a
   stage that landed back in step 4. That is the lesson worth keeping: an `#[allow]` whose lint no
   longer fires is *itself* silent, so a migration-scoped suppression outlives its reason by
   default and nothing in the build says so. `#[expect]` is the form that self-retires —
   `unfulfilled_lint_expectations` fires the moment the code becomes live — and is what a
   temporary suppression should have used.

   Verified: clippy `-D warnings` clean across the workspace in both the default and
   `--features bake` configurations, 298 tests green, the viewer relaunched and captured
   unchanged, and `FXC_FORCE=1 cargo build -p review-render` recompiles all 30 generated sources
   through the rewritten job list and reproduces every committed `.dxbc` **byte for byte** — which
   is what says the job-list rewrite changed no shader.
8. **Gate (D2)** with `scripts/gate.ps1` against `main`; fix or document any regression; merge.
   *Measured; nothing needed fixing or excusing.* Five interleaved runs per build plus a discarded
   warm-up each, release builds of both, same box, 1600×960, 4× MSAA, GTAO on, vsync off.

   | Measure | `main` | port | Δ | budget |
   | --- | --- | --- | --- | --- |
   | **SK_Player_01** — 141 978 tris, skinned, clip playing | | | | |
   | startup (ms) | 1423.6 | 1428.4 | **+4.8** (+0.3 %) | +10 ms ✔ |
   | frame (ms, median) | 0.879 | 0.346 | **−0.532** (−60.6 %) | +0.3 ms ✔ |
   | frame (ms, p95) | 1.054 | 0.489 | −0.565 (−53.6 %) | — |
   | cpu/frame (ms) | 0.764 | 0.301 | −0.463 (−60.6 %) | — |
   | private (MB) | 382.7 | 372.0 | **−10.7** (−2.8 %) | +5 % ✔ |
   | GPU dedicated (MB) | 260.1 | 257.5 | **−2.6** (−1.0 %) | +5 % ✔ |
   | **xyzrgb_dragon** — 249 882 tris, static | | | | |
   | startup (ms) | 841.4 | 812.3 | −29.2 (−3.5 %) | +10 ms ✔ |
   | frame (ms, median) | 0.592 | 0.337 | −0.255 (−43.1 %) | +0.3 ms ✔ |
   | frame (ms, p95) | 1.087 | 0.531 | −0.556 (−51.2 %) | — |
   | cpu/frame (ms) | 0.437 | 0.163 | −0.274 (−62.7 %) | — |
   | private (MB) | 412.6 | 410.5 | −2.1 (−0.5 %) | +5 % ✔ |
   | GPU dedicated (MB) | 288.4 | 284.2 | −4.2 (−1.5 %) | +5 % ✔ |

   Two fixtures rather than D2's one: the skinned one is what D2 specifies, and the dragon is there
   because the skinned fixture is CPU-bound at this size and a GPU-side regression could have hidden
   behind that. Both are comfortably inside every budget, and the frame is the *opposite* of a
   regression — the port is 43–61 % faster per frame, with the CPU half falling by about the same
   proportion, which is where a swapped drawing API would be expected to show up.

   **A measurement bug worth recording, because it made the frame figure meaningless and looked
   entirely plausible.** The first runs reported a median frame time of *exactly* 8.34 ms on both
   builds — 1000/120, this monitor's refresh — despite presenting with a sync interval of 0. A
   flip-model swapchain without `ALLOW_TEARING` still queues behind DWM, which releases one buffer
   per vblank, so `present(false)` measured the display rather than the frame. The fix is the flag
   on the swapchain plus `DXGI_PRESENT_ALLOW_TEARING` on a vsync-off present, in both builds; it is
   inert for the shipped viewer, which always presents with vsync on. With it the same runs report
   0.35 ms and 0.88 ms. The lesson is the shape of the number, not the flag: a median that lands on
   a round multiple of the refresh interval is measuring the compositor, whatever it is labelled.

   **One number moved the wrong way, and D2 does not budget it: the release binary is
   13 842 432 → 16 345 088 bytes (+2.50 MB, +18 %).** Sections: `.text` +2.07 MB, `.rdata`
   +0.41 MB, `.data` +0.53 MB — so it is mostly code. The vendored sokol C accounts for at most
   0.6 MB of that (`sokol-rust.lib` is 610 KB, of which `sokol_gfx.o` is 247 KB) and the generated
   shader payload for under 0.2 MB, which leaves roughly 1.5 MB unattributed. That runs against the
   footprint argument the wgpu migration was made on, so it deserves one focused look (a `cargo
   bloat` pass on an unstripped build) — but it is a follow-up, not a gate: D2's three measurements
   are startup, frame and memory, and all three pass.

### Phase 2 - macOS — **steps 1–7 done**, notarization excepted

1. **Done.** Mac setup per §7, then the tests that can run at all today:
   `cargo test -p review-model -p review-import -p review-optimize -p review-psd` — **182 green**.
   **Not** `--workspace`, and there is no "first light without a line of new code" — that was
   written before the shape of the port was settled and is wrong. `rhi/backend/mod.rs` declared only
   `#[cfg(windows)] mod d3d11`, so `review-render` did not compile on macOS at all until step 2
   wrote `metal.rs`, and `review-ui` depends on `review-render`, so `ui` and `app` followed it. What
   step 1 *did* prove is the half that has nothing to do with the GPU: that all four vendored C
   trees (ufbx, meshoptimizer, ufbx_write, psd_sdk) compile under Apple clang and that the parsers,
   the optimizer and the PSD decoder agree with Windows — which is exactly what would have been
   miserable to debug later, tangled up in a new backend.
2. **Done.** `rhi/backend/metal.rs` from Fire (+ `supported_sample_counts`, and a `present(vsync)`
   that sets `displaySyncEnabled` since Metal has no per-present sync interval); `build.rs`'s Metal
   half per (program, stage) with the committed-blob gate (D5), its rows joining the Windows ones in
   the one `bytecode.manifest`, and the toolchain found by *running* `xcrun metal --version` rather
   than `--find`, since Command Line Tools ship a stub that resolves and then fails at first
   compile. All 30 MSL sources compile warning-free under `-Werror -O3`.

   Verified by eye against the Windows build: the shaded model with IBL/PBR, GTAO, the grid, the
   axis gizmo, the stats overlay, the Outliner/Inspector/toolbar/status-bar chrome, a skinned
   character with its 72-bone skeleton, and 109–114 FPS. **The MSAA menu offers 1/2/4, not the
   1/2/4/8 predicted here**: Apple silicon answers `supportsTextureSampleCount:` no at 8, which is
   the case invariant 4's capability gate exists for.

   Two things found on the way and fixed rather than left. `build.rs`'s "review.glsl is newer than
   generated/" warning fired on **every** macOS build of a clean tree — git wrote review.glsl 6 ms
   after the generated directory and the check compared mtimes strictly; a tolerance cannot fix it
   (a real edit *is* followed by a build seconds later), so `gen-shaders.{sh,ps1}` now records the
   digest of the review.glsl it ran on as `generated/review.glsl.sha256` and build.rs compares
   content, closing the one staleness link the manifest could never see. And `window_state.rs`'s
   config-directory test had a Windows arm only; it now has the macOS twin D17 names.
3. **Done.** Leaves: the open hook (D14), the menu bar (D15), `Primary = ⌘` + labels (D10, already
   in from Phase 1 step 5), pinch (D16), `dev-app.sh`.

   **The open hook and the menu bar live in a new crate, `crates/shell-macos`, not in `app`** — a
   departure from §6, which listed objc2 as a dependency of `app` itself. `class_addMethod` on
   winit's delegate needs `unsafe`, and `crates/app` is `#![forbid(unsafe_code)]` (invariant 9);
   weakening that on the OS being ported to is the wrong trade when the project already isolates
   `unsafe` behind named crates (`psd`, `optimize`, the GPU backend leaf). `app` hands the crate a
   callback and gets back paths and a two-variant command enum; every entry point is a no-op stub
   off macOS, so `main.rs` gained no `cfg`.

   Verified with the dev bundle: a cold `open -a` came up *showing* the model with no argv, and an
   `open -a` at a running instance loaded a second one. Note for anyone testing by hand that plain
   `open <app> <file>` is **not** the test — it opens the two independently and sends the `.fbx` to
   the system default handler; `-a` is what routes it through Launch Services.
4. **Done — no code needed.** The Windows port had already done the work, because Windows has HiDPI
   too: winit's `inner_size()` is physical pixels, so the layer's drawable and every offscreen scene
   target are physical-sized, and `scene_viewport_px` already rounds each edge to a whole physical
   pixel (with a test at 2×). Measured on a 2× display rather than assumed: the toolbar band is
   **73 physical px** and the status bar **64**, exactly their design-pixel tokens.
5. **Done.** Profiler / bake leaves for Metal (D18 / D19).

   D18 needed one shared change: `Zone` gains a `Frame` variant only a backend leaf ever writes.
   D3D11 never writes those slots and Metal writes nothing else, so — because which zones a frame
   reported is *measured* rather than declared — a capture on either OS shows exactly what that OS
   could measure, with no `cfg` in `gpu_profiler.rs`. The Tracy baseline had to become the lowest
   *written* timestamp rather than `times[0]` (the scene pass's begin), which would have calibrated
   Metal's whole timeline against a zero. Captured with `tracy-capture`: 226 GPU zones beside 225
   CPU ones.

   **The Metal bake does not reproduce the committed `.bin`s byte for byte** — this step anticipated
   "BC6H-identical or a documented tolerance", and the measurement says which. The BRDF LUT (pure
   math, no sampling, 1024-byte rows involving no padding at all) differs in 3.5 % of its values by
   at most 0.021 on a 0..1 range, mean 2.3e-4. Across a prefilter cube the difference rate falls
   with mip size — 21 % / 11 % / 6.7 % / 5.1 % / 6.0 % — tracking convolution sample count, and the
   two mips whose rows are actually padded are the *least* different, where a de-padding bug would
   have made them ~100 %. So it is GPU float precision, not the leaf. The committed Windows bake
   stays the one set of bytes both hosts embed, and `build-mac.sh` deliberately runs no re-bake.
6. **Done.** `scripts/gate.sh`, with one difference of substance: **no cross-OS budget**, since
   holding a Mac against a Windows PC compares two machines rather than two builds. It measures one
   build and records it, or two interleaved with D2's budgets on the delta. Process creation comes
   from `proc_pidinfo`'s `pbi_start_tvsec` (the analogue of `Process.StartTime`, read by the script
   rather than by the `forbid(unsafe_code)` viewer); memory is the physical footprint, and there is
   **no VRAM row** because Apple silicon's unified memory already counts it inside that figure.

   Recorded on this Mac (release, 1920×1280, 4× MSAA, GTAO on, a clip playing, 5 runs, medians):

   | fixture | startup | frame | footprint |
   | --- | --- | --- | --- |
   | `SK_Player_01` (141 978 tris) | 941.0 ms | 3.416 ms | 842 MB |
   | `SM_Ammo_Crate_01a` (8 055 tris) | 199.0 ms | 3.368 ms | 628 MB |

   The startup spread between them is the FBX import, which is the same host-agnostic code on both
   OSes; the frame time is flat between them, which is what a fill-bound 4×-MSAA + GTAO frame looks
   like. Also found here: `gate.rs`'s `WINDOW_SIZE` comment claimed 150 % was the worst display
   scale its 1600×960 request had to survive, but on a 2× Mac the viewer's 960×640-logical minimum
   wins and the window comes up 1920×1280. Nothing is broken by it — the clamp is a property of the
   display, not the build, and both harnesses already refuse to compare runs whose stamped sizes
   disagree — but the comment was stating a guarantee the constant does not have.
7. **Done but for notarization.** `scripts/build-mac.sh` (Fire's, with the `.fbx` document type and
   `application-logo.png` → `.icns`), plus `packaging/check-shader-bytecode.sh`, the mirror §7 said
   this script owed: it hard-fails on a stale `.metallib` and only warns about the Windows rows,
   the inverse of the PowerShell twin.

   Two departures from Fire's script. **The icon keeps its alpha** — Fire composited its flame onto
   an opaque background because a floating flame reads as unfinished, but this master is a cube
   whose faces run nearly corner to corner, so the real problem is macOS masking a legacy icon into
   a rounded rect and clipping the cube's own corners; an 8 % inset fixes that and invents no
   background colour. And **no IBL re-bake**, for the reason step 5 measured.

   Run end to end with `--no-notarize`: a signed, verified 11.5 MB `dist/3D Review-0.2.1.dmg`, and
   the packaged bundle launches, opens a model through Launch Services and draws at 110 FPS.
   **Notarization is the one step not yet exercised**, though it is no longer blocked: the
   credential a notarytool profile holds is an Apple ID, a team ID and an app-specific password,
   all three of which belong to the *account* rather than to a product, so the profile already on
   this Mac notarizes anything this team signs. Deriving the default profile name from the product
   was therefore the wrong convention — it would have meant a `--notarize-profile` flag on every
   release to reach a credential the machine already had — and the default is the existing `fire`
   profile instead. The script still prints the `store-credentials` line if it is ever missing.

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
