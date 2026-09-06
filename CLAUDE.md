# CLAUDE.md — 3D Review (Rust)

A **native** 3D model-audit viewer (think F3D / Autodesk FBX Review), running on
**Windows and macOS from one shared shell**. Drag-drop an **FBX**, inspect game
assets, switch through debug views. Built on `winit` (window/event loop) +
**`sokol_gfx`** (GPU, over Direct3D 11 on Windows and Metal on macOS) + `egui`
(overlay UI, through our own sokol renderer) + vendored `ufbx` (FBX parsing via a
C bridge). Pure-Rust, no web/Electron layer. **Target priority: Windows** — it is
the platform the D2 performance budgets gate, and the one a change is checked
against first.

> **The GPU layer is fully on sokol_gfx, on both OSes** (`mac-port-plan.md` Phase 1
> and Phase 2): every draw path — the frame flow, the egui chrome, the Tex viewport,
> the 3D and UV scenes with MSAA and ambient occlusion, and the Opt workspace's
> comparison view — plus the `--tracy` GPU profiler and the offline `bake_ibl` tool.
> The viewport is pixel-identical to the old Direct3D 11 build on the models checked.
> Nothing is parked or duplicated: the hand-written `src/hlsl/` set and
> `src/port_pending/` are both gone, so `src/shaders/review.glsl` is the only shader
> source in the workspace.
>
> Two things differ per OS and are *measured* rather than assumed. The MSAA menu
> offers 1/2/4/8/16 on Windows and **1/2/4** on Apple silicon, which is what
> invariant 4's capability gate is for. And a fresh `bake_ibl` reproduces every
> committed `.bin` byte for byte on Windows but only *numerically* on Metal (≤0.021
> on a 0..1 BRDF LUT — GPU float precision), so the committed Windows bake is the one
> set of bytes both hosts embed and the Mac packaging script deliberately runs no
> re-bake.

Deeper docs: the crate map + data flow live in §2 below; `PROJECT_STATE.md`
(architecture, status, risk register), `RENDERING_PIPELINE.md` (render-pass
detail), `materials and textures plan.md` (the active materials/textures
roadmap), `TODO.md` (running notes), and `mac-port-plan.md` (the macOS port:
the planned migration of the GPU layer to `winit` + `sokol_gfx` on both OSes, its
decisions, phases and risks — see §4's note).

## 1. Invariants — the rules an agent will break if not told

> **Lean on the existing system; don't hand-roll your own.** Before building any
> mechanism, reach for what's already there — first the platform/framework
> primitive (e.g. egui's native `Window`/`Panel`/`Grid`/`Slider`, winit
> facilities, an `rhi` wrapper), then an existing helper in this codebase (a
> `theme` token, a
> `widgets` primitive, an `import`/`render` funnel). The UI chrome was *migrated
> away* from a hand-rolled `egui::Area` + pixel-rect layout system precisely
> because re-implementing window management by hand made every new panel come out
> broken. Do not reintroduce bespoke layout/window/styling machinery when a native
> primitive or shared helper covers it. If the existing system genuinely can't do
> the job, say so and extend it in one place — don't fork a parallel one.

1. **Single source of geometry; no defensive copies.** `ModelData`
   (`crates/model`) owns the mesh buffers. They are uploaded to the GPU by
   `bytemuck` cast and shared as `Arc<ModelData>`. Never `.clone()` / re-`Vec`
   the geometry to build a debug view — derived structures (wireframe, normal
   lines) index *into* the shared vertex/index/topology data.
2. **Data and commands flow one way each.** `ui` never owns or mutates renderer
   or model internals: it holds plain values + displayed stats and emits
   `UiOutput` *intents* (e.g. gizmo orbit/snap). `app` is the only coordinator
   that applies those intents to the `Renderer`. Commands go UI→app; data goes
   app→UI. `render` and `model` must not depend on `ui` or `app`.
3. **Derived-on-demand, free-on-switch.** Anything a debug view needs
   (wireframe edges, face-normal lines, vertex-normal lines, future overdraw /
   barycentric buffers) is built when that view turns on and dropped when it
   turns off. The steady-state shaded view holds **zero** derived buffers.
   Implemented for the line views by `SceneGpu::sync_line_views` in
   `render/src/scene/resources.rs` (build-on-demand, free-on-off, live param
   rebuild) — which is where every other `sync_*`/`release_*` pair lives too.
   A builder reached from only *one* viewport still needs its free arm on the
   path that leaves that viewport: `sync_uv_view` is called from `render_uv`
   alone, so `release_uv_views` runs from the 3D path's `sync_frame`.
4. **Heavy derived views are GPU compute and capability-gated.** Compute-based
   views (normals/tangents/overdraw, post-MVP) read buffers already on the GPU
   and write transient storage buffers. Gate them on what the device reports —
   `sg_query_features()` / `sg_query_limits()` / `sg_query_pixelformat()`, plus
   `backend::supported_sample_counts` where sokol's answer is only a yes/no — and
   disable them in the UI when the active device can't run them, never crash.
5. **Faithful stats.** The Model Stats panel reads source DCC counts carried
   through import in `ModelStats` (original polygon/vertex counts), **never**
   post-triangulation render counts. Every stat shown must be a real measured
   value. `stats_grid` shows only measured values (Draws/Polys/Tris/Verts/GPU
   Verts/Vtx Splits/UV Sets/FPS); `fps` is fed from the `app` render loop. Don't
   reintroduce placeholder rows. **The viewer's corner-split vertex buffer is an
   internal layout, not a stat**: the panel reports the DCC count (`vertex_count`)
   and the engine cost (`gpu_vertex_count`, unique vertices per draw group via
   `ModelData::count_gpu_vertices`); the corner count is surfaced nowhere.
6. **The redraw loop lives in `app`.** Redraw is driven by `winit` events and
   active camera animation (redraw-on-demand), never by UI state mutation.
   Continuous redraw only while a camera transition or interaction is live.
7. **One C extraction, one funnel.** The `ufbx_scene → flat arrays` walk is
   written once in `crates/import/src/ufbx_bridge.c` and is the *only* path FBX
   import takes. Any future caller of that C core must produce identical
   `ModelData`. Keep the two-pass count→fill discipline and free every buffer on
   every error path.
8. **No hardcoded visual values in UI components.** Every color / font size /
   border width / radius / spacing / opacity / animation duration in `crates/ui`
   must come from a
   central semantic theme (a `theme` module), not inline literals. The only
   exception is a value computed at runtime from state (e.g. a per-axis gizmo
   color). Token names are **semantic** (`panel_bg`, `selection`, `gizmo_ball`),
   not `dark_grey_6`. Tokens live in `crates/ui/src/theme.rs` (`color`, `size`,
   `font`, `motion` submodules + `apply_visuals`); add the token there first, then
   reference it. **`size` tokens are design pixels, not egui points** — convert at
   the use site with `theme::px(ctx, …)` (or `theme::chrome_height`, which `app`
   uses to reserve the toolbar/status-bar band). Reading a `size` token raw
   against a value already in points silently over-reserves on any HiDPI display;
   the few tokens that genuinely *are* points say so in their doc comment.

### Rust-specific invariants

9. **All `unsafe` and all C/FFI lives in `crates/import`, `crates/psd` and
   `crates/optimize`** —
   *plus* the scoped GPU site below. No `unsafe` leaks into `model` / `ui`,
   nor into `render`'s geometry / material / camera modules. Before
   `slice::from_raw_parts`, null-check the pointer and treat len 0 as empty
   (`checked_slice`). Free the C scene on **both** success and error paths (no leak).
   **Sanctioned exception — psd_sdk FFI** (`crates/psd`, `review-psd`): the C-ABI
   bridge to the vendored psd_sdk C++ that decodes a PSD's merged composite (source
   art), so the viewer reads layered PSDs without a bundled ImageMagick. It vendors
   psd_sdk as **source** (`vendor/Psd/`, an unmodified upstream `src/Psd/` snapshot)
   and compiles it with `cc` alongside the C-ABI `src/wrapper.cpp`, exactly as
   `import` does with ufbx; `src/bindings.rs` is committed Rust hand-kept against
   `src/wrapper.h`, so no `bindgen`/libclang runs at build time (see that crate's
   `vendor/NOTICE.txt`). The `unsafe` is confined to
   its `decode_psd`, which validates header dimensions with checked arithmetic before
   sizing the output buffer. `render`'s `texture.rs` calls it through the safe API
   only — no `unsafe` there.
   **Sanctioned exception — meshoptimizer + ufbx_write FFI** (`crates/optimize`,
   `review-optimize`): the mesh-optimization core and FBX writer behind the Opt
   workspace. Unlike psd it vendors **source** — `third_party/meshoptimizer` (an
   unmodified upstream `src/` snapshot; C++ with no STL/exceptions behind a pure-C
   API) and `third_party/ufbx-write` (`ufbx_write.h/.c`, plain C) — each compiled
   by `cc` in its `build.rs` and existence-gated on `cfg(has_meshopt)` /
   `cfg(has_ufbxw)` exactly as `import` gates ufbx: delete either tree and the
   workspace still builds, with the dependent operations returning
   `OptError::Unavailable`. The two libraries are bound differently *because their
   APIs differ*: meshoptimizer's is already flat over raw pointers, so `src/ffi.rs`
   declares its entry points directly and `src/meshopt.rs` holds every call — each
   wrapper validating the mesh preconditions before it (whole-triangle index
   buffers, in-range indices, exact stream lengths, `checked_mul` destination
   sizes) and re-validating the returned element count after. ufbx_write's is
   handle-and-setter-based, so driving it from Rust would spread `unsafe` across a
   hundred call sites and leak its scene lifetime into Rust; instead
   `src/export_bridge.c` does the whole write in one call taking flat arrays
   (`src/export_ffi.rs` declares it), validating the payload up front and freeing
   the scene on every path. It hands every buffer over with `ufbxw_copy_*` rather
   than the borrowing `ufbxw_view_*`, so nothing is borrowed past the call. Those
   modules plus `src/export.rs`'s single call site are the whole `unsafe` surface:
   `ops`/`process`/`stack`/`preset`/`submesh` are ordinary safe Rust, and the crate
   `forbid`s `unsafe_code` outright when neither vendored tree is present. `app`
   calls it through the safe API only.
   **Sanctioned exception — the platform GPU leaf** (`crates/render/src/rhi/backend/`)
   is the *only* place the renderer's `unsafe` lives, and it is ~200 lines per OS:
   `d3d11.rs` creates the device, hands it to `sg_setup`, owns the DXGI swapchain,
   hands sokol_gfx a render-target view per frame and presents; `metal.rs` does the
   same with an `MTLDevice` and a `CAMetalLayer` hosted on winit's `NSView`. They are
   twins aliased as `backend`, not a trait — anything added to one must be added to
   the other. Everything sokol_gfx draws with — pipelines,
   buffers, targets, textures, samplers — is safe Rust over its C API, so what used
   to be ~2000 lines of pervasive COM `unsafe` is now that leaf plus the one
   `extern "C"` logger callback in `rhi/mod.rs`. `crates/app` is **fully safe**
   (`#![forbid(unsafe_code)]` — as are `model` and `ui`) and, since `Gpu::attach`
   reads the window's raw handle itself, no longer depends on `windows` at all.
   **Sanctioned exception — the macOS shell leaf** (`crates/shell-macos`,
   `review-shell-macos`): the Launch Services open hook (D14) and the `muda` menu bar
   (D15). It is its own crate for exactly the reason `app` is `forbid(unsafe_code)`:
   teaching winit's application delegate to answer `application:openURLs:` is
   `class_addMethod` and nothing else, and that `unsafe` belongs in a named, bounded
   site rather than loose in the shell. It knows nothing of models, cameras, frames or
   GPU state — `app` hands it a callback and gets back paths and a two-variant
   `MenuCommand` — and every entry point is a no-op stub off macOS, so `main.rs`
   carries no `cfg` for it.
   Every `unsafe` carries a `// SAFETY:` rationale and touches only GPU plumbing —
   never model geometry, camera math, or material logic, which stay safe.
   *(`crates/render/src/rhi/bake.rs` and the `bake`-gated half of `ibl.rs` are safe
   sokol code; the bake's one `unsafe` is `read_image_subresource` in the backend
   leaf, where the rest of the D3D11 `unsafe` already lives.)*
10. **`crates/model` is host-agnostic.** It depends only on `glam` — no GPU API,
    `egui`, `winit`, or importer types. This is what kept the renderer swappable
    (wgpu → D3D11 → sokol_gfx); don't add rendering/UI deps to `model`.
11. **GPU structs are `#[repr(C)]` + `bytemuck` `Pod`/`Zeroable`.** Match the
    shader's uniform-block / vertex-input layout exactly (std140's 16-byte block
    packing) — don't reorder fields without updating `review.glsl`. Nothing about
    this is caught at run time: a uniform upload is sized from the Rust struct and
    only rejects a payload *larger* than the block, so a field added on one side
    alone uploads happily and the shader reads every later field shifted. Every
    such struct therefore carries **two** `const` assertions beside it: a literal
    `size_of::<T>() == N`, and `size_of::<T>() == size_of::<generated::T>()`
    against shdc's reflection — the second is what pins the invariant to the
    *shader* rather than to a hand-typed number. Add both with the struct; they
    are the only thing that turns this invariant into a build error. Vertex
    attribute locations are pinned the same way, by a test asserting the generated
    `ATTR_*` constants against the layout every program that shares `vs_main`
    expects. Each uniform block also gets its own slot across all programs (scene
    VS 0 / scene FS 1 / material 2), so a slot never means two different structs.

## 2. Where things live

```
crates/
  app/      review-app: winit ApplicationHandler, event loop, input routing
            (LMB orbit / RMB pan / wheel zoom / F frame / drag-drop /
            double-click-open), egui_winit wiring, the GPU bootstrap (`Gpu::start`
            on the *first line of main*, on its own thread — device creation needs
            no window and is the longest launch item — joined by
            `GpuBringUp::attach` in `resumed`, which puts the swapchain on the
            window; `app` has zero `unsafe` and no platform GPU dependency at all)
            + the per-frame draw order (`begin_frame` → the renderer's offscreen
            passes → egui tessellation *outside* any pass → the one swapchain pass
            {composite, chrome} → `finish`), redraw timing, applies UiOutput back
            to Renderer. One `impl App` block per concern, one file each:
            `fn main`, the `ApplicationHandler` impl and the window + GPU startup
            path -> src/main.rs; the per-frame render loop ->
            src/frame.rs; pointer/scroll/resize routing -> src/input.rs; the
            keyboard dispatch (which `ui`'s help.rs tables must mirror; file
            commands chord with the **primary** modifier — `Ctrl` here, `Cmd` on
            macOS, D10 — and `ui`'s `primary_key!` is what names it on screen) ->
            src/shortcuts.rs; the model-load funnel — drag-drop, Ctrl+O,
            double-click, CLI/file-association — plus the one
            `reset_ui_for_new_model` both paths share -> src/loading.rs;
            applying `UiOutput` intents to the Renderer (invariant 2's concrete
            realization) -> src/ui_intents.rs; the selection-flash animation ->
            src/selection_flash.rs;
            the D2 gate stamp (`--gate-out <file>`: load the fixture, play a
            scripted orbit, write one JSON stamp of startup + frame timings, then idle
            with the model resident so `scripts/gate.ps1` can sample memory from
            outside, and exit) -> src/gate.rs;
            every native file dialog — opened on a worker thread and answered
            through `UserEvent::DialogDone` (`mac-port-plan.md` D9), one at a time
            -> src/dialog.rs. Nothing in this crate may call `rfd` inline from a
            winit callback: on macOS a modal run loop entered from one aborts the
            process. The single exception is the startup error box, which `main`
            raises *after* `run_app` has returned. A request carries its own
            subject with it (the LOD chain to export, the serialized preset), so
            the answer acts on what the user was looking at when they asked rather
            than on whatever the state has since become;
            scene texture pool + off-thread decode + disk-auto-reload
            (an `impl App` block) -> src/texture_manager.rs;
            the animation clock + pose evaluation (`AnimationSubsystem`, an
            `impl App` block: advances `UiState::animation.time` while playing,
            re-evaluates the pose through `review_model::anim` when the clip or
            time moved, bumps `pose_revision`, keeps `ui.bounds` on the selected
            clip's envelope, and is the `anim_playing` term of the redraw pacing)
            -> src/animation.rs;
            window position/size restore via the OS config dir (`dirs`, D17:
            `%APPDATA%\3D Review` on Windows — the same path as before —
            `~/Library/Application Support/3D Review` on macOS), monitor-geometry
            validation + the refresh-rate query -> src/window_state.rs;
            unified undo/redo snapshot stack -> src/undo.rs;
            the Opt workspace's processing loop -> src/opt.rs (an `impl App` block
            + `OptSubsystem`, created only on first entry into the workspace so a
            session that never opens it pays nothing). One run is ever in flight:
            edits arriving mid-run mark it dirty and it respawns once with the
            latest stack, so dragging a slider coalesces without a debounce timer,
            and a result whose generation has been superseded is dropped rather
            than shown. Export runs on its own worker the same way.
  model/    review-model: host-agnostic data only (glam dep only).
            Vertex, Bounds, MaterialImportDefaults, TopologyFace, ModelStats,
            TriangleData (grouped per-triangle face/material/node arrays +
            `validate` lockstep guard), ModelData, recompute_bounds,
            demo_cube_model -> src/lib.rs, plus the deform data: `SceneNode::
            rest_local` (the per-node rest TRS the clips override), `ModelData::
            corner_to_logical` (render corner → DCC vertex), `SkinData` (CSR
            weights over logical vertices + the `clusters` table carrying each
            (mesh node, bone) bind matrix), `MorphData` (blend-shape channels /
            keyframes / per-logical-vertex offset CSR), `AnimationClip` (baked
            `NodeTrack`s + `MorphTrack`s, the stack's time range, its motion
            envelope `bounds`), and `ModelData::validate_deform` (the funnel
            guard for all of it). Pose evaluation — `AnimContext`, `Pose`,
            `evaluate_pose` (parents-first recomposition, hold outside keys,
            ufbx's in-between blend rule via `channel_effective_weights`),
            `build_palette` → `DeformPose`, and the CPU reference
            `deform_corner` / `clip_bounds` the shader and tests mirror ->
            src/anim.rs; per-mesh-part triangle BVH (occlusion for the dimension
            labels) -> src/bvh.rs
  import/   review-import: load_model/load_fbx, ImportError, the unsafe FFI
            (repr(C) mirror structs, checked_slice, model_from_bridge_scene),
            the vendored ufbx C + bridge, build.rs (cc, cfg(has_ufbx)).
            -> src/lib.rs, src/ufbx_bridge.c/.h, build.rs. The bridge also
            captures each node's rest local TRS, the skin cluster table (per
            mesh-bearing node: `geometry_to_bone × inverse(geometry_to_world)`)
            + per-influence cluster index + deformer method, the blend shapes
            (offsets pre-rotated into baked world orientation), and every
            animation stack baked with `ufbx_bake_anim` (file frame rate, no key
            reduction; "DeformPercent" element tracks → morph channels), each
            with the same count→fill discipline. The Rust side sorts the morph
            entries into a CSR, validates through `validate_deform`, and — for a
            model that deforms — measures `bounds` at the rest pose and each
            clip's envelope on the import worker. Loads with
            `UFBX_INHERIT_MODE_HANDLING_HELPER_NODES` so `parent × local`
            recomposes `node_to_world` exactly.
  optimize/ review-optimize: mesh optimization for the Opt workspace, over
            vendored meshoptimizer v1.2 (invariant 9's third FFI site). Depends on
            `review-model` only — hands back plain `ModelData`, never touches GPU
            or UI types. Layers, bottom up: src/ffi.rs (raw `extern "C"` decls) +
            src/meshopt.rs (the checked safe wrappers; together the entire `unsafe`
            surface) -> src/submesh.rs (splits a `ModelData` into per-(node,
            material) pieces — the unit that can survive a simplify, since
            meshoptimizer returns a new index buffer with no triangle
            correspondence) -> src/ops.rs (one function per operation) ->
            src/process.rs (indexes the mesh losslessly — see §6, this is what
            makes any meshoptimizer call do anything at all — then walks the stack,
            fans out the LOD chain, reassembles a `ModelData` per level + measures
            it, alongside the source's own buffer counts so the overlay's deltas
            subtract like from like). src/stack.rs is the serializable
            operation stack the UI edits; src/preset.rs is its versioned JSON
            envelope. The two operations that simplify share one
            `SimplifySettings` (flattened on the wire, so older presets still
            load) and one `process::simplify_submeshes` — the LOD op fans its
            output out into levels, `Reduce` writes its own back in place. Processed meshes are pure triangles and carry **no** face
            topology (`faces` / `triangles.to_face` left empty — the corner-run
            layout a `TopologyFace` describes cannot survive welding; every
            `render` consumer already falls back to per-triangle behaviour).
            src/export.rs + src/export_bridge.c write a LOD chain out as FBX via
            vendored ufbx_write (suffixed siblings in one file or one file per
            level; rebuilt or flattened hierarchy; source materials, untextured).
            Output is always triangulated and declares `UnitScaleFactor = 100`,
            since import normalizes every file to meters while FBX's conventional
            unit is centimeters. build.rs compiles third_party/meshoptimizer and
            third_party/ufbx-write with `cc` when present (`cfg(has_meshopt)` /
            `cfg(has_ufbxw)`).
  prof/     review-prof: the guarded Tracy helpers (`zone!`, `plot!`,
            `frame_mark`, `thread_name`, `msg`) every instrumented crate shares.
            `tracy_client`'s own macros panic when no client is running, so each
            wrapper checks `Client::running()` first. It re-exports `tracy_client`
            and the macros reach it as `$crate::tracy_client`, so a crate that
            only opens zones needs no tracy dependency of its own (`ui` and
            `optimize` have none; `app` and `import` keep one for `Client::start`
            and the allocation hooks). Each consumer's `src/prof.rs` is a
            re-export of this crate, not a copy. -> src/lib.rs
  shell-macos/ review-shell-macos: the two macOS shell leaves, and nothing else —
            the Launch Services open hook (`openfiles.rs`, D14: `class_addMethod` on
            winit's application delegate, so a Finder double-click / `open(1)` / a
            drop on the Dock icon reaches the viewer, which on this OS arrives as an
            Apple event and *never* as argv) and the `muda` menu bar (`menubar.rs`,
            D15: App / File {New ⌘N, Open… ⌘O} / Window, everything else a predefined
            item on AppKit's own responder chain — and where ⌘Q comes from). Its own
            crate because the delegate hook needs `unsafe` and `app` is
            `#![forbid(unsafe_code)]`; `app` hands it a callback and gets back paths
            and a two-variant `MenuCommand`, so it depends on neither `winit` nor any
            crate of ours. Every entry point is a no-op stub off macOS.
            -> src/lib.rs, src/openfiles.rs, src/menubar.rs
  psd/      review-psd: safe `decode_psd` over a C-ABI bridge to psd_sdk (C++),
            returning a PSD's merged composite as RGBA8 (invariant 9's third FFI
            site). Vendors psd_sdk as SOURCE (`vendor/Psd/`) and compiles it with `cc`
            alongside the C-ABI `src/wrapper.cpp`; `src/bindings.rs` is committed Rust
            hand-kept against `src/wrapper.h`, so no bindgen/libclang runs.
            -> src/lib.rs, src/wrapper.cpp, src/wrapper.h, src/bindings.rs, build.rs,
            vendor/
  render/   review-render: per-view config/option types (ShadingMode,
            VertexColorMode, ActiveMaterial, CameraProjection, AntiAliasing,
            EnvironmentSettings, GtaoSettings, TonemapSettings, SceneDebugOptions,
            RendererConfig) -> src/config.rs; OrbitCamera (framing/orbit/pan/zoom/
            ortho+persp, Reversed-Z infinite perspective), UvCamera (2D UV
            viewport), CameraTransition (0.3s ease-in-out cubic) and the shared
            `ease_in_out_cubic` curve the chrome's own animations re-use ->
            src/camera.rs; the `Renderer` façade -> src/lib.rs.
            The GPU layer:
              * src/rhi/ — the SOLE home for the drawing API (sokol_gfx) **and for
                the backend's types**: nothing outside `rhi` names an `sg::` type, a
                pixel format enum or a window handle. Every resource goes through a
                wrapper, every pixel format is `Format` (format.rs), and every
                failure is a `GpuError` (error.rs) — which is what makes the
                renderer's shape independent of the API underneath it
                (`mac-port-plan.md` §3.1). mod.rs (`Gpu` = the device + the window's
                swapchain, brought up on its own thread by `Gpu::start` and joined by
                `GpuBringUp::attach`; `Frame` = one frame in flight, borrowing the
                `Gpu` so "one frame at a time" is a compile-time fact, carrying the
                clear colour, the single `begin_swapchain_pass` and every draw verb,
                with a `Drop` guard that closes an abandoned pass so one bad frame
                cannot wedge the next; plus the sokol logger, without which
                validation failures are silent); error.rs (`GpuError`/`GpuResult`/
                `ResourceKind`, `require_valid` for sokol's invalid-id reports and
                the `.resource(kind, label)` combinator for the backend leaf's
                HRESULTs); format.rs (`Format` + `SCENE_*_FORMAT`; its `sg()` is
                `pub(in crate::rhi)`, which is what stops an `sg::PixelFormat`
                leaking back out); present.rs (`PresentStatus`); shader.rs (the
                generated `ShaderDesc` with this host's committed bytecode swapped
                in, and the `bytecode!` macro that picks the per-OS blob);
                pipeline.rs, buffer.rs (`TransientBuffer`, the per-frame geometry
                stream, plus the immutable `VertexBuffer`/`IndexBuffer` and the
                `StorageBuffer<T>` the deform tables live in), texture.rs, mips.rs (the
                CPU mip chain, D7 — sokol has no `GenerateMips`; sRGB is averaged in
                linear light so the result matches what the hardware produced),
                sampler.rs, target.rs (the offscreen colour + depth attachments: an
                image plus the views a pass attaches and a later pass samples, and at
                2×+ MSAA a single-sample twin sokol resolves into at `end_pass`; the
                GTAO targets are single-sample by design, and `scene/gpu.rs` groups the
                seven of them into a `TargetSet` the Opt split holds *two* of),
                bindings.rs (what a draw reads, as one value re-applied after every
                `apply_pipeline` — sokol has no sticky slot state, so the old
                `bind_*`/`unbind_*` pairs have no successor); gpu_profiler.rs (the
                `--tracy` GPU profiler — the `Zone` set, the Tracy GPU context and the
                spans, over the backend's `GpuTimer` — plus the arming flag and the
                Tracy message channel; which zones a frame encodes is **measured**, not
                declared, because timestamp queries are reused across ring slots and a
                declared-but-unwritten zone would report ticks from four frames ago as
                a plausible duration). `SwapchainJob` (in
                mod.rs beside `Frame`) is a draw *recorded* before the swapchain pass
                exists and replayed when it opens — every `Renderer::render_*` runs
                before that pass, and there is only ever one of it per frame, so the
                composite is deferred while the offscreen passes are issued directly.
              * src/rhi/backend/ — the device + swapchain leaf, one module per OS and
                **the only platform GPU code in the workspace** (invariant 9's
                sanctioned `unsafe`). d3d11.rs creates the device, points
                `sg_environment` at it, owns the DXGI flip-model swapchain, hands
                sokol a render-target view per frame, presents, and answers
                `supported_sample_counts` (sokol only reports MSAA as a yes/no).
                metal.rs is its twin: an `MTLDevice`, a `CAMetalLayer` hosted on
                winit's `NSView` (layer-*hosting*, not layer-backed, so AppKit does
                not redraw it behind us), a drawable per frame, and
                `supportsTextureSampleCount:`. Four things differ there and each is
                load-bearing — a `BGRA8Unorm` backbuffer (a `CAMetalLayer` refuses
                RGBA8; a storage channel *order* only, so D20 holds), sokol presenting
                inside `sg_end_pass` rather than us (a second present would
                double-present), the frame blocking at `nextDrawable` rather than at
                present, and vsync being the layer's `displaySyncEnabled` rather than
                an argument to a present call.
                The swapchain is created with `ALLOW_TEARING` where the factory
                offers it, and a vsync-**off** present passes the matching flag: without
                that a flip-model `Present(0, 0)` still queues behind DWM, so
                `finish(false)` returned the refresh interval rather than the frame's
                own cost (which is what the D2 gate measures). Inert for the viewer,
                which always presents with vsync on.
                It also carries the two leaves for what sokol has no notion of:
                `GpuTimer` (timestamp + disjoint queries, D18) and, `bake`-gated,
                `read_image_subresource` (a staging copy + `Map(READ)`, D19). Its
                `dxgi_format` is deliberately exhaustive with **no** catch-all arm —
                the one it used to have made a wrong-format staging texture, and
                `CopySubresourceRegion` between mismatched formats is a silent no-op.
                The two modules are twins aliased as `backend`, not a trait:
                anything added to one must be added to the other.
              * src/egui_sokol.rs — the egui renderer (D3), which replaced
                `egui-directx11`. One program, one interleaved vertex stream, a
                texture per egui id with a **CPU shadow** it is recreated from (sokol
                has no sub-rectangle image update, and egui patches its atlas by
                sub-rect), and a sampler per distinct `TextureOptions`. `prepare`
                runs outside any pass, `paint` inside the swapchain pass, and
                `free_textures` after the frame.
              * src/scene/gpu_types.rs — the #[repr(C)] scene/post/GTAO uniforms +
                SceneVertex, each with a `const` size assertion **against shdc's
                generated struct** as well as a literal, so invariant 11 is pinned to
                the shader rather than to a hand-typed number.
              * src/rhi/bake.rs — the offline bake's headless sokol_gfx (a device, no
                swapchain), the render-target `CubeTarget` (one attachment view per
                face/mip plus a cube texture view) and `Target2D`, and the one-call
                fullscreen pass they are drawn with. `[feature bake]` only; the runtime
                never touches it. sokol's closed passes are what retired the old
                `unbind_*` dance — a render target cannot still be bound when the next
                pass samples it.
            CPU vertex generation -> src/geometry/ (vertex/grid/mesh/select/
            debug_lines/uv, plus deform.rs — the per-model `DeformLayout`: each
            corner's 16-byte `deform` lane and the influence / morph tables it
            indexes; `rhi::StructuredBuffer` uploads them to VS `t12..t15`, the
            palette + shape weights re-uploaded only on `pose_revision`, and
            `vs_main` applies morph deltas then the normalised skinning blend so
            the mesh, the GTAO G-buffer, the selection flash and every
            mesh-derived overlay deform through the one shader; the skeleton
            overlay attaches its vertices to node entries and needs no CPU
            rebuild); editable per-material table (cbuffer `b1` + `t5..t11` +
            aniso sampler) + path-keyed texture cache -> src/material/ (state / mode,
            plus gpu — the uploaded table and its path-keyed LRU cache); source-texture decode
            (magic-byte dispatch: PSD via `review-psd`, JPEG via zune's fast path,
            PNG/TGA/TIFF/HDR/BMP/GIF/PNM via the `image` crate — no ImageMagick) +
            filename channel auto-detect -> src/texture.rs; what the Tex viewport is
            asked to draw (`TexBackground` — a solid fill *is* the frame's clear now
            — and the placed `TexImage`) -> src/tex.rs, with its image + checker draw
            (two deferred `SwapchainJob`s, deliberately outside the scene MRT/tonemap
            path so the displayed texel equals the stored texel) -> src/tex/gpu.rs.
            Shaders are ONE source: src/shaders/review.glsl, all 15 programs in
            sokol-shdc's annotated GLSL. `scripts/gen-shaders.{sh,ps1}` turns it into
            the checked-in src/shaders/generated/ — per-backend HLSL5 + MSL sources
            and shdc's Rust reflection (bind slots, attribute locations, a struct per
            uniform block) — and build.rs compiles this host's half to committed
            bytecode beside it (`fxc /WX /O3`, freshness-gated, jobs *discovered* by
            filename). The runtime `include_bytes!`s the bytecode; nothing compiles a
            shader at run time, and a broken shader is a build error. That reflection
            is also what every `#[repr(C)]` GPU struct asserts its size against, so
            invariant 11 is pinned to the shader rather than to a hand-typed number.
            Per-model GPU state (mesh buffers, derived views, selection/visibility
            draw lists, each with its bake key) lives in a `ModelSlot`; `SceneGpu`
            holds an **active/idle pair** so the Opt workspace can keep the source
            and processed meshes both resident and alternate between them within a
            frame for one `mem::swap` — a single slot would rebuild both meshes on
            every alternation. `render`/`render_uv` claim the source slot
            explicitly. `render_opt` (src/scene/opt.rs) draws the split — each half
            into its **own** `TargetSet` sized to half the viewport, composited into
            its own half via a viewport rect. Two sets, not one: the composite is a
            deferred `SwapchainJob`, so both halves' passes have run before either
            composite does and a shared set would show the second view in both. Two
            half-width sets cost what one full-width set does, and the second is
            released the moment a single view is drawn. The overlay is one view (one
            mesh solid, the other a ghost: the x-ray reuses the selection-flash fill,
            the wireframe ghost the line pipeline).
            Scene depth is `Depth32Float`, Reversed-Z. The offscreen scene pass is
            **2-MRT** linear-HDR (`R16G16B16A16_FLOAT`): location 0 = linear scene
            radiance, location 1 = AO-eligible diffuse-ambient radiance; GTAO has a
            *separate single-sample* view-normal/Z G-buffer (its own mesh-only pass) +
            raw/blurred `R8` occlusion, and the composite darkens the ambient by the
            scalar AO factor. HDR image-based lighting: at runtime the env cube +
            irradiance + prefilter + shared BRDF LUT are **loaded**
            (`IblMaps::from_baked`, a pure upload — no startup precompute) from
            offline-baked assets (`assets/ibl_baked/`) for the PBR shaded path +
            skybox. The three HDR cubes ship **BC6H** block-compressed
            (`Bc6hRgbUfloat`, ~8× smaller than `Rgba16Float`, GPU-native so no
            decode); the shared BRDF LUT stays `Rg16Float`. The precompute that bakes
            + BC6H-encodes them (`review.glsl`'s four `ibl_*` programs run on a
            headless sokol device via `rhi::bake`, then `intel_tex_2` encodes BC6H)
            compiles only into the
            offline `bake_ibl` tool (render's `bake` feature, src/bin/bake_ibl.rs) ->
            src/ibl.rs. The model wireframe is a plain LineList drawn in the scene
            pass via the line pipeline (depth-tested against the mesh so hidden-face
            edges are occluded; fixed 1px hardware width) -> src/scene/gpu.rs +
            src/geometry/ (`wireframe_lines`).
  ui/       review-ui: egui chrome built on egui's **native windowing**, not a
            hand-rolled layout system. Option tools are native `egui::Window`s
            (collapsible/closable, non-resizable, multi-open via
            `UiState::panels_open`); the Outliner (left) + Inspector (right) are
            dockable, resizable `egui::Panel`s; the toolbar + status bar are
            `egui::Panel` bands (interiors still hand-laid via
            `scope_builder` — the one remaining rework step). `draw_overlay` takes
            the frame's root `&mut Ui` (egui shows panels into a `Ui`, not onto the
            `Context`) and carves the bands out of it; floating chrome — `Window`,
            `Area`, the layer painters — still addresses `ui.ctx()`. The 3D/UV scene
            + the composite are drawn by the renderer into the same frame *before*
            egui, which paints its chrome over them with a transparent central
            viewport; the Tex viewport (`texture_view.rs`) handles only interaction
            (pan/zoom/fit, background fill, channel pick) and hands the image to the
            renderer — `ui` owns no GPU state. Plus axis
            gizmo, stats overlay, bounding-box dimension labels, startup help
            overlay; emits UiOutput intents. Thin root re-exports; modules: theme/
            state/assets/widgets/overlay/toolbar/status_bar/stats/texture_view/gizmo/
            dimensions/help/transport (the bottom-centre playback card, drawn
            only while a clip is selected in the 3D workspace; it edits the
            plain `UiState::animation` in place, like selection)
            + panels/ (mod.rs = width-pinning dispatch; one file per tool:
            anti_aliasing, bounding_box, environment, normals, gtao, tonemap,
            uv_checker, vertex_colors, wireframe, material_mode; plus inspector +
            outliner for the side panels, and opt_stack + opt_inspector for the Opt
            workspace). The Outliner is itself a directory — panels/outliner/
            {mod.rs = entry point + tab dispatch, tree.rs = tree-model
            construction, rows.rs = row painting, nav.rs = keyboard nav + click
            semantics, materials.rs = the materials tab, animations.rs = the
            clip list, a tab that exists only while the model carries clips and
            the workspace is not Opt}. -> src/lib.rs + src/*.rs
            The Opt workspace's own state (the `OptStack` the chrome edits, the
            comparison-view settings, and the last run's measured figures) ->
            src/opt_state.rs. Ownership follows the convention already used for
            selection and the hidden-mesh set: `UiState` owns it, the chrome edits
            it in place, and bumping `stack_revision` is what tells `app` to
            reprocess and what undo compares; only the file-dialog actions (export,
            preset load/save) travel as an `OptIntent`. In Opt the left side panel
            splits horizontally — the scene tree on top, the operation stack in a
            resizable nested `egui::Panel` band below — and the Inspector
            retargets at the selected operation's parameters, the export settings,
            or the selected object's overrides.
third_party/ufbx/   vendored ufbx.c / ufbx.h (compiled only if present)
third_party/meshoptimizer/  vendored meshoptimizer v1.2 src/ (compiled only if
                    present; see its NOTICE.txt for the pinned commit)
third_party/ufbx-write/     vendored ufbx_write.c / .h — the FBX writer behind
                    the Opt export (compiled only if present; upstream is
                    work-in-progress, so the commit is pinned and the export
                    round-trip test re-reads what it writes)
assets/icons/       PNG toolbar/gizmo icons (include_bytes!)
assets/test_models/ local FBX fixtures for manual checks
```

Data flow: input/file-drop → `app` → `import` (FBX→`ModelData`) → `model`
(shared) → `render` (camera + GPU resources) → `app` opens the frame, the renderer
records its passes and composites, then `ui`'s chrome is painted over it in the same
swapchain pass → `app` (applies UI intents, requests redraw). See §2 above + `PROJECT_STATE.md`. The Opt workspace branches
off that at `model`: `optimize` turns a `ModelData` plus an operation stack into
one `ModelData` per LOD level on a worker thread, and those go back through the
same `render` path as the source.

> **Model revisions must be unique across *every* mesh the renderer is handed**,
> not merely increasing within one. `SceneGpu` keys its mesh-buffer cache on the
> revision alone, so two different meshes sharing a number leaves one of them
> stale on screen. `app` issues them all from one counter
> (`App::next_model_revision`) — the source mesh and each processed Opt level.

## 3. Build & run

**Windows:** use the **x64 Native Tools Command Prompt for VS 2022** (MSVC `cl`
must be on PATH so `cc` can compile `ufbx.c`). **macOS:** Xcode Command Line Tools
are enough (Apple clang compiles every vendored C tree); the full Metal toolchain
(`xcodebuild -downloadComponent MetalToolchain`) is needed **only** to edit shaders
— verify it by *running* `xcrun -sdk macosx metal --version`, since CLT ships a stub
that `xcrun --find` locates and that then fails at first compile.

- `cargo run -p review-app` — launch the viewer.
- `cargo check --workspace` — fast type check.
- `cargo clippy --workspace --all-targets -- -D warnings` — lint; **run before
  claiming done.**
- `cargo fmt --all` — format.
- `cargo test --workspace` — unit tests (model / import / render); add alongside
  changes. (The shaders are validated by `fxc /WX` in render's `build.rs`, not a
  test.)
- `cargo build --release` — release binary.
- `scripts/gate.ps1 -A <baseline exe> -B <exe> -Fixture <fbx>` — the D2 gate
  (`mac-port-plan.md`): interleaved release launches of two builds, comparing
  startup / frame time / RAM + VRAM against a budget on each delta. Needs a real
  GPU and a quiet box. The baseline build is `main` plus
  `scripts/gate-baseline.patch`, which adds the same `--gate-out` stamp to a branch
  that predates it. `scripts/gate.sh` is the Mac twin, with **no cross-OS budget** —
  comparing a Mac against a Windows PC compares two machines, not two builds — so it
  also has a one-build mode that just records the numbers the next Mac build has to
  beat. There is no VRAM row there: unified memory already counts it in the footprint.
- `scripts/dev-app.sh [--debug] [model.fbx]` (macOS) — wrap the built binary in an
  unsigned `.app` and launch it. The only way to exercise the menu bar, the Dock, and
  the Launch Services open path at all: a bare executable gets none of them, and is
  not what `open` hands files to. It launches with `open -a`, which is load-bearing —
  plain `open <app> <file>` sends the `.fbx` to the system default handler instead.
- `scripts/build-mac.sh` — the release chain (D12): `.icns`, `Info.plist` with the
  `.fbx` document type, codesign with the hardened runtime, notarize, staple, `.dmg`,
  and the `spctl -a -t open` check. Run by hand on the dev Mac so the Developer ID
  cert never leaves that keychain. `packaging/check-shader-bytecode.sh` (which it runs
  first) is the mirror of the `.ps1`: it hard-fails on a stale `.metallib` and only
  warns about the Windows rows.
- `cargo run --release -p review-render --features bake --bin bake_ibl` — the
  offline IBL re-bake (needs a real GPU; it creates its own headless device). Writes
  `assets/ibl_baked/` **in place**, and validates each payload before writing so a
  dead pass cannot destroy a good map. `--release` because the BC6H encoder's
  highest-quality profile is slow — but note that sokol's validation layer is compiled
  *out* of a release build, so if the bake misbehaves, reproduce it in debug where a
  bad call says so.
- `scripts/gen-shaders.ps1` (or `.sh`) — regenerate `crates/render/src/shaders/
  generated/` after editing `review.glsl`. Fetches `sokol-shdc` into the
  gitignored `.tools/` on first run; **commit what it writes**, since a plain
  build never runs it. The bytecode beside it is per-host (D5): a shader edit
  committed from one OS ships stale bytecode for the other until that host
  rebuilds, and build.rs says so. It also writes `generated/review.glsl.sha256` —
  the digest of the shader the set was generated *from*, which is the one staleness
  question `bytecode.manifest` cannot answer (it chains a blob to its generated
  source, and this is the link above that). Editing `review.glsl` and forgetting to
  run this script is what that catches, and build.rs warns on it.

Pinned (workspace deps): `winit 0.30`, `sokol` (a vendored checkout — see
`vendor/NOTICE.txt`), `windows 0.62` (Direct3D 11/DXGI, a `cfg(windows)` dep of
`render` **only**, for the backend leaf), the `objc2 0.5` family
(`objc2-metal`/`-quartz-core`/`-app-kit`/`-foundation` 0.2 — the versions winit
already pulls, so the Mac build compiles nothing extra; a `cfg(target_os = "macos")`
dep of `render` for the Metal leaf and of `shell-macos` for the shell ones) +
`muda 0.19` (the menu bar), `egui`/`egui-winit 0.36` +
`egui-notify 0.23`, `glam 0.30`, `bytemuck 1`, `thiserror 2`, `rfd 0.15`,
`dirs 6` (the per-user config dir `window.cfg` lives in),
`image 0.25` (png + hdr + tga/tiff/jpeg/pnm), `half 2`,
`zune-image`/`zune-core 0.5` (JPEG-only + simd; the texture fast path), `cc 1`
(build dep), `tracy-client 0.18`. Edition 2024.

> **MSRV:** `rust-version = "1.88"`.
> The FFI's `unsafe extern "C" { … }` blocks are the idiomatic (and, under
> Edition 2024, required) form. Bump the floor only when adopting a feature
> that needs a higher version.

`build.rs` (import) compiles `ufbx.c` + the bridge with `cc` **only when**
`third_party/ufbx/ufbx.{c,h}` exist, and sets `cfg(has_ufbx)`; without them the
workspace still builds and FBX import returns a clear error. `build.rs` (render)
compiles **this host's half** of the generated shader sources to committed bytecode —
`fxc` → `.dxbc` on Windows, `xcrun metal -Werror -O3` → `.metallib` on macOS — but
only when a blob is stale and the compiler is present (a box with neither uses the
committed blobs). The source list, the compiler and the blob extension are `cfg`-chosen
from one shared skeleton, and the jobs are discovered from the filenames in
`src/shaders/generated/`, so adding a program touches only `review.glsl` — there
is no hand-listed job table to keep in step with it.

**Stale means "does not match", not "is older than"** — `src/shaders/generated/
bytecode.manifest` records the SHA-256 of each generated source and of the blob
compiled from it, and is committed beside them. Commit it whenever you commit
bytecode. Timestamps cannot answer this once two OSes commit blobs (D5): a shader
rebuilt on the Mac reaches Windows as a new source next to a blob one revision
behind, both stamped by `git checkout` at the same instant. `packaging/check-
shader-bytecode.{ps1,sh}` fails a release build on any mismatch (each hard-fails on
its own host's blobs and only warns about the other's; both packaging builds run
theirs). The one thing the manifest cannot see — that you edited `review.glsl` and
never ran `scripts/gen-shaders` — is caught by `generated/review.glsl.sha256`, which
those scripts write and `build.rs` compares against the file's own digest.

## 4. Locked decisions — DO NOT RE-LITIGATE

- **Spend dev-time freely to make the release runtime fast, efficient, and
  accurate.** Offline/bake-time cost — CPU, wall-clock, build complexity — is
  *not* a constraint; the shipped binary's speed, memory/VRAM footprint, and
  output accuracy are what matter. So always prefer the highest-quality /
  most-precomputed option for anything done ahead of runtime: e.g. the IBL maps
  are baked offline (no startup precompute) and the BC6H cubes use the encoder's
  slowest, **highest-quality** profile (`very_slow_settings`) because that work
  happens once on a dev machine and only sharpens what runs. When a choice trades
  dev-time effort for a better runtime, take it.
- Native stack: direct `winit` + **`sokol_gfx` as the one drawing API on both
  Windows and macOS** (Direct3D 11 and Metal underneath), with one ~200-line
  device/swapchain leaf per OS in `render/src/rhi/backend/` — not `eframe`, not
  wgpu, and not a second native shell per platform (`mac-port-plan.md` D1). `egui`
  is an overlay drawn by our own sokol renderer (`render/src/egui_sokol.rs`, D3).
  The renderer was migrated off `wgpu` first, to shrink the shipped binary, startup
  time and baseline RAM (one device creation instead of probing every DX12 adapter);
  the workspace carries **zero** wgpu/naga/pollster/egui-wgpu. That ban stands, and
  so does the "no second shell" one: **do not reintroduce wgpu, and do not add a
  per-OS renderer** — anything sokol genuinely cannot do becomes a named leaf beside
  the device (mips, timestamp queries, readback, exact MSAA counts), not a fork.
- UI chrome uses egui's **native windowing** (`Window` / `Panel`) and stock widgets (`Grid` / `Slider` / `DragValue` /
  `ComboBox`), styled from egui's default `Visuals::dark()` plus a few theme-token
  overrides. The old hand-rolled `egui::Area` + pixel-rect panel system is gone —
  do **not** bring it back (see the lead-in to §1). Styling exceptions: keep the
  bundled Inter (proportional) + JetBrains Mono fonts; numeric value boxes
  (`DragValue`) render in monospace, everything else proportional.
- Internal format is the own `ModelData` superset (flat parallel buffers +
  original face topology + source stats). FBX is the only MVP import format,
  parsed by vendored `ufbx` through a single C bridge — don't round-trip through
  glTF (drops quad topology, changes vertex counts).
- Shading is one scene program (`@program mesh` in `shaders/review.glsl`) covering
  shaded / unlit / wireframe / uv-checker / vertex-color paths; tone mapping + sRGB
  encoding live in the `post` program, and the model wireframe is a depth-tested
  line-list draw in the scene pass. The whole set is generated per backend by
  sokol-shdc and compiled offline to committed bytecode by `build.rs` (`fxc` here);
  the runtime does no shader compilation. `3D`, `UV` and `Tex`
  viewports are all implemented; the `Tex` image is drawn by its own minimal
  fullscreen-triangle pipeline (`TexGpu`, outside the scene MRT/tonemap path) so
  channel isolation is a uniform swizzle and the displayed texel equals the stored
  texel.
- Crate boundaries are load-bearing (invariants 2, 9, 10) — keep them.

## 5. Current state

> **Everything below draws again.** The GPU port (`mac-port-plan.md` Phase 1 step 4)
> is complete: the frame flow, the egui chrome, the Tex viewport, the 3D and UV scenes
> — GPU skinning, the material table, the IBL/PBR shaded path, the skybox, every
> derived overlay, dynamic scene MSAA and the GTAO passes — and the Opt workspace's
> split and ghost-overlay comparison all run on sokol_gfx. Against a `main` build in a
> worktree the viewport is **pixel-identical** on the models checked. The steps left
> are the Tracy GPU profiler and the offline IBL bake (step 6), the shell leaves
> (step 5) and the cleanup (step 7).

MVP: native window + Direct3D 11 viewport + egui chrome; FBX import via ufbx (drag-drop,
`Ctrl+O`, double-click empty viewport, command-line/file-association path);
orbit/pan/zoom + frame-on-`F` + home reset + 45° WASD orbit steps; grid with
axes; shaded / unlit / wireframe / shaded+wireframe, source-color / UV-checker /
vertex-color materials, plus bounding-box, face- and vertex-normal debug
overlays; orthographic/perspective toggle; animated axis gizmo (orbit +
snap-to-axis); a 2D UV viewport (independent pan/zoom, UV channel picker, wire
layout, shaded fill, per-island coloring); a 2D Tex viewport (a fullscreen image
draw via `TexGpu` over the scene texture pool: texture picker, RGB/R/G/B/A channel
isolation — a shader uniform swizzle, so switching is instant; the `A` segment
auto-hides for opaque images — mipmapped (a CPU chain, D7), pan (LMB-drag) / zoom
(wheel or RMB-drag) / `F`-to-fit, black/white/grey/checker
background fill, and a real-values stats panel). Windows packaging (exe icon/resource
metadata + Inno Setup installer) is present.

**Opt workspace** (mesh optimization, static meshes only): a fourth mode sharing
every 3D control. The user builds a re-orderable stack of meshoptimizer
operations — weld, filter degenerate/duplicate triangles, prune components,
reduce the mesh in place or generate a LOD chain (both over the same standard /
attribute-preserving / sloppy simplifiers with the option flags; `Reduce`
rewrites the mesh every later step and the export carry, `Generate LODs` fans
out into extra levels beside it), a `Bake AO to Vertex Colors` op (deterministic
CPU cosine-hemisphere raycasts via `review-model`'s `Bvh::ray_occluded` against
bake-scoped occluder concatenations of the *visible* submeshes — excluded
objects still occlude, they just aren't written, while **Outliner-hidden nodes
neither occlude nor bake** (`ProcessInput.hidden_nodes` carries the visibility
snapshot; `sync_opt` reruns the stack on an eye toggle only while a bake is
enabled) and **the occluders partition by the `_LOD<n>` name suffix** (own node
or nearest named ancestor): game FBXs carry their whole LOD chain as co-located
siblings, and raycasting one LOD against another's near-coincident surfaces
shreds the result, so each LOD bakes only against its own group plus every
suffix-less node, suffix-less nodes bake against the lowest LOD present, and an
artist can bake an entire visible chain in one run; parallel over
`std::thread::scope`; write
target defaults to the alpha channel, sRGB encode optional for the RGB-family
targets; adding it auto-switches the viewport to the matching Vertex Colors
mode, and the exporter already writes vertex colors so it needs nothing), and
the vertex-cache / overdraw / vertex-fetch reorders — with
per-object exclusions, and saves it as a JSON preset. Every edit reprocesses on a
worker (latest-request-wins, a notice only if it runs long) and the result is in
the viewport as soon as it lands, undoable through the same snapshot stack as
everything else. Comparison is a vertical split (optionally camera-synced) or a
single view with one mesh ghosted over the other and an `X` A/B swap — the LOD
picker and the layout / camera-sync / swap icons sit centred in the status bar,
over the split's divider, since they describe the viewport as a whole. A second
stats card carries the processed mesh's measured counts and the ACMR / ATVR /
overdraw / overfetch figures meshoptimizer measured, each with its change against
the source tinted green or red (every figure there is one where lower is better)
— which is what makes the reorder operations, invisible in the viewport, worth
having. That card is up as soon as the workspace is opened: a run with nothing
enabled produces no mesh but still measures the source, so the baseline is
readable before anything is added. The overlay layout adds a legend naming which
mesh is shaded and which is the ghost. An explicit export writes the chain to FBX via vendored ufbx_write.
Startup is untouched: nothing Opt-specific is built until the workspace is first
opened, and a stack with nothing enabled schedules no run.

**Skinning + animation.** A skinned mesh rests in the file's *default* pose,
skinned onto the skeleton as drawn (not the bind pose its buffers hold): GPU
linear-blend skinning over each vertex's exact influence run, weights normalised
by their sum as ufbx does, plus rigid node animation and blend shapes through the
same vertex-shader deform stage. Every FBX animation stack imports as a clip
(baked at the file's frame rate, no key reduction; frame counts come from the
stack range, never key counts) and is listed in the Outliner's `Animations`
tab — shown only for animated files and never in Opt. Clicking a clip selects it
paused on its first frame and shows the bottom-centre transport (go-to-start /
step / play-pause / step / scrubber / `frame / total   s` / loop-on-by-default /
speed); clicking it again returns to the rest pose. `Space` and `,` `.` mirror
the transport. While a clip is selected, `F` and the bounding box use its motion
envelope, measured once at import. Playback state is view state (never undone);
the clock and pose live in `app` (`animation.rs`), the pose math in
`review_model::anim`. Dual-quaternion skins are evaluated as linear and the
Inspector says so; the Opt workspace draws the bind pose (its processed meshes
carry no skin) and the dimension-label BVH / AO bake use the bind-pose buffers.

The `ui` crate was migrated off the old hand-rolled `egui::Area` + pixel-rect
layout system onto egui's **native windowing**: option tools are native
`egui::Window`s (collapsible/closable, non-resizable, multi-open via
`UiState::panels_open`), the Outliner/Inspector are dockable `egui::Panel`s,
the toolbar/status bar are `egui::Panel` bands, and panel bodies use stock
`Grid`/`Slider`/`DragValue`/`ComboBox` widgets dispatched by
`panels::draw_panel_body` (which pins each panel to one width). Styling derives
from egui's default `Visuals::dark()` installed once at startup
(`theme::init_style`) plus a few token overrides — custom accent colors were
dropped, but the Inter (proportional) + JetBrains Mono fonts are kept; only the
numeric `DragValue` value boxes render in monospace (left-aligned, fixed width),
everything else proportional. All visual values still come from the central
`theme` module (invariant 8). Derived line views are freed on view-off and the
normal length/color sliders update live (invariant 3, via `scene/resources.rs`
`sync_line_views`). The stats panel shows only measured values (invariant 5).
Still pending: the toolbar/status-bar *interiors* are the last hand-laid
(`scope_builder`) bit awaiting a native-layout rebuild.

Rendering pipeline (see `PROJECT_STATE.md` + `RENDERING_PIPELINE.md`): the scene
renders into **offscreen linear-HDR MRT targets** composited by a fullscreen post
pass; **anti-aliasing** is dynamic scene MSAA (Off/2×/4×/8×/16×, gated on
`CheckMultisampleQualityLevels` for the scene color + depth formats), on the
status-bar AA button; **HDR image-based lighting + PBR** is the default Shaded
look — six baked HDR environments (each with a preview thumbnail shown in the
Environment dropdown), baked irradiance/prefilter/BRDF-LUT maps loaded by `ibl.rs`,
an optional skybox, and a live 0–360° environment yaw rotation (applied at sample
time in the scene program via `projection_params.y`, so it never rebuilds the maps);
**Ambient Occlusion** (GTAO internally) is on by default — horizon-based occlusion
(a structured 4×4 spatial dither decorrelates slices) + 5×5 bilateral blur over a
*separate single-sample* view-normal/view-Z G-buffer (its own mesh-only pass, not
an MSAA MRT). Composed in post as an additive correction over the full radiance:
post darkens the AO-eligible diffuse ambient (location 1) by a scalar AO factor and
leaves location 0 otherwise intact, so direct + emissive light are never darkened.
UI knobs are Radius / Intensity / Thickness / Quality; user-facing strings stay
"Ambient Occlusion". **Tone mapping** is on by default — the composite applies a
selectable operator (Khronos PBR Neutral / Linear / Reinhard / ACES / AgX) to the
linear-HDR radiance before sRGB encoding; toggling it off is a linear pass-through.
The status bar's right group holds the IBL / Ambient Occlusion / Tonemapper /
Anti-aliasing toggles (left-click toggles, right-click opens each tool's options
panel — Environment / Ambient Occlusion / Tonemapper / Anti Aliasing).

The renderer is **fully linear-HDR with Reversed-Z scene depth**, on a **2-MRT**
scene pass: location 0 carries linear radiance (tone mapping + `linear_to_srgb`
happen in the `post` program); location 1 is the AO-eligible diffuse ambient radiance (IBL
diffuse + analytic fill only), which post darkens by the scalar GTAO factor. Scene
depth is `Depth32Float` cleared to 0 with `GreaterEqual` and infinite reversed
perspective (`perspective_infinite_reverse_rh`); the egui renderer then draws the
chrome into the same swapchain pass, after the composite and with no depth. The model wireframe is a plain
`LineList` drawn inside the scene pass via the line pipeline, so it depth-tests
against the mesh (Reversed-Z `GreaterEqual`, no depth write) and edges on hidden
faces are occluded, while the scene MSAA antialiases it (the trade-off is fixed 1px
hardware line width).

Known gaps: in-app load-error/warning display, GPU-buffer visualization, and
additional import formats (glTF/OBJ) are post-MVP (`TODO.md`). Importing
engine-cooked compressed textures (KTX2/DDS) is intentionally **not** a goal:
artists test *source* assets (PNG/TGA/…); KTX2/DDS are produced inside an
engine's content pipeline and never hand-authored or carried, so the viewer is
never handed one. (We do use GPU block compression internally — the baked IBL
cubes are BC6H — but that's our own offline bake, not an import path.) Tests
exist (model / import / render / optimize unit tests, run headless in CI; the HLSL
is `fxc`-validated at build time); GPU render checks stay manual.

`crates/optimize` additionally carries two integration suites over the real
fixtures in `assets/test_models`, both of which have already caught bugs the
synthetic demo cube could not: `tests/real_models.rs` (welds, LOD targets,
per-triangle tag survival, exclusions) and `tests/export_round_trip.rs`, which
checks every written FBX by **reading it back** through the vendored ufbx reader —
the writer's own return value only proves it didn't error. Each test skips itself
when its fixture or a vendored tree is absent.

## 6. Gotchas

- Real GPU required for render checks; reserve CI for `check` / `clippy` /
  import unit tests.
- **Rest pose ≠ bind pose.** `ModelData::vertices` are world-baked in the bind
  pose, but a skinned model is *displayed* in the file's default pose: the palette
  at rest is `bone_rest_world × world_to_bone_bind` per cluster, not identity.
  Anything that reads the vertex buffer on the CPU (the dimension-label BVH, the
  AO bake, `recompute_bounds`) sees the bind pose; `model.bounds` and each clip's
  `bounds` are therefore measured through `review_model::anim` at import, and the
  Opt workspace (which shows the bind pose on purpose) passes no pose.
- **Every mesh-derived overlay must copy its source corner's `deform` lane.** The
  wireframe, normal lines, heat map and selection flash deform only because their
  builders take the slot's lanes (`geometry::deform::corner_deform`); a builder
  that pushes `NO_DEFORM` for mesh geometry silently draws the bind pose over the
  skinned mesh. Static geometry (grid, bounding box, pivot, UV) is `NO_DEFORM`;
  the skeleton overlay uses `node_deform`.
- **Structured-buffer structs are invariant-11 territory too.** `InfluenceEntry`
  / `PaletteEntry` / `MorphEntry` in `gpu_types.rs` mirror the HLSL structs at
  `t12..t15` byte for byte (the palette is three `float4` rows, never `float3x4`,
  because matrix packing differs between cbuffers and structured buffers); each
  has a `const` size assertion. `scene_vertex_size()` deliberately reports the
  64-byte engine-equivalent vertex, not `SceneVertex`'s 80 bytes — the deform lane
  is viewer-internal and must not move the Opt overfetch figure.
- **`ufbx_bake_anim` facts.** Linear keys are kept as authored (only cubic
  segments resample), stepped keys become 1 ms pairs, and `key_time_min/max` can
  extend past the stack range — so frame counts derive from the stack range × fps
  and the evaluator holds the end values outside the keys. Blend-channel weights
  arrive as the `DeformPercent` element property (percent ÷ 100), and ufbx folds
  the channel weight into per-keyframe *effective* weights
  (`channel_effective_weights` is a verbatim port) — don't multiply the channel
  weight in again.
- **ufbx_write's `ufbxw_view_*` buffers are borrowed, not copied** — the pointer
  must stay valid until the save completes. `export_bridge.c` uses `ufbxw_copy_*`
  throughout so nothing is borrowed past the call; reusing one scratch buffer
  across meshes with a `view` was an access violation. Related: FBX *declares* its
  unit rather than fixing one, conventionally centimeters, while import normalizes
  every file to meters — so the writer sets `UnitScaleFactor = 100`. Without it the
  geometry reads back exactly 100× too small, which no error reports.
- **Every Opt run begins with a lossless index pass, and must — and the run's
  baseline (`ProcessedResult::source`/`source_metrics`) is measured *after* it.**
  Quoting changes against the corner-split buffer credited the user's operations
  with an "-82%" any engine cooker gets for free, and made the baseline ACMR a
  meaningless 3.0; the indexed baseline also equals the stats panel's `GPU Verts`
  (a `real_models` test pins the two figures equal). Import splits
  each face corner into its own vertex, so a mesh reaches `optimize` with *no*
  shared vertices at all (a real 10006-triangle asset arrives as 30018 vertices).
  Every meshoptimizer operation works through the index buffer, so on that mesh
  they are all no-ops — nothing for the vertex cache to reuse, no edge a collapse
  may cross, a LOD chain that removes nothing. `process::index_mesh` therefore
  merges vertices identical in *every* attribute before any stack operation runs:
  byte-for-byte survivors, so nothing visible changes (30018 → 5284 on that asset,
  after which a 50% LOD target is hit exactly). It is deliberately not a stack
  operation — skipping it is never useful. The `Weld` operation is for the *lossy*
  merges (dropping normals/UVs from the comparison, or a tolerance).
- **A simplify that barely removes anything is usually attribute seams, not a
  bug.** Import splits every face corner into its own vertex, so a mesh whose
  normals or UVs differ at every corner — a scan with generated per-face normals —
  presents *every* edge as a discontinuity, and a topology-preserving collapse
  cannot cross one. Measured: a real game asset welds 369k → 108k vertices and hits
  its LOD targets with the default settings, while such a scan stalls at
  249882 → 249880 triangles until either a position weld (normals excluded) or the
  permissive flag unblocks it. `process` detects the stall and says so; the default
  weld comparing normals is *correct* for the tool's stated target of static game
  meshes, so don't "fix" it by loosening the defaults.
- **Recovering from a malformed index drops the whole triangle, never one
  corner.** An index buffer is a flat corner stream, so `continue`-ing past a
  single bad corner leaves a length that is no longer a multiple of three and
  shifts every later triangle by one — plausible-looking garbage rather than a
  visible failure, and trimming the tail afterwards doesn't undo it. Both
  `submesh::partition` and `export::build_mesh` check all three corners first and
  skip the triangle as a unit; `export` additionally consults the same check from
  its per-face material loop, since skipping in one loop and not the other
  mis-assigns every subsequent material.
- **Re-keying operation ids is a two-pass job.** `OptStack::reassign_ids` builds
  the whole old→new map before rewriting a single per-node override. Rewriting
  them as the walk consumes the id space lets an already-rewritten override
  collide with a later operation's *old* id and be rewritten twice, silently
  reattaching the user's per-object settings to the wrong operation — reachable
  with one reorder plus a preset round-trip.
- GPU struct field order must match the shader's uniform-block / vertex-input
  layouts (invariant 11) — the one shader source is
  `crates/render/src/shaders/review.glsl`; update it in lockstep with the
  `#[repr(C)]` `SceneUniforms`/`SceneVertex` structs in `scene/gpu_types.rs` (mind
  std140's 16-byte block packing). Two things catch a mistake now: `fxc /WX` in
  `build.rs` rejects a broken shader, and each struct's `size_of` assertion against
  **shdc's generated struct** rejects a layout that drifted. Re-run
  `scripts/gen-shaders.ps1` after editing the GLSL and commit what it writes.
- The scene geometry pass is **MRT** with **two** color targets: the scene
  fragment shaders in `review.glsl` write location 0 (linear scene radiance) and
  location 1 (AO-eligible diffuse ambient radiance), so every scene pipeline
  (mesh/line/uv-fill/skybox) declares *two* color formats and the offscreen pass
  binds *two* attachments (+ MSAA resolves) — keep the three in lockstep.
  Both locations alpha-blend. Overlays (zero-normal verts) write 0 to location 1 so
  they aren't AO-darkened. GTAO does not read location 1: it has its own
  single-sample mesh-only pass (`fs_gtao_gbuffer`, one `SV_Target0` output of view
  normal `xyz` + view Z `w`) into a separate G-buffer target, avoiding MSAA edge
  averaging. GTAO is horizon-based (the `gtao` program, where a structured 4×4
  spatial dither decorrelates slices) and outputs a single scalar occlusion
  (`R8Unorm`); `post` darkens the diffuse ambient (location 1) by that scalar factor
  — an additive correction over location 0 so MSAA stays correct and direct/emissive
  light is never darkened. Tone mapping + sRGB encoding happen once in `post`, not
  in the scene shader. fxc notes: use `SampleLevel` (not `Sample`) for any
  texture read inside a loop/branch (non-uniform control flow), and guard a
  possibly-negative `pow` base with `max(x, 0.0)` so `/WX` doesn't reject it.
- CPU-side vertex generation (grid, wireframe, face/vertex normal lines) lives in
  `crates/render/src/geometry/` (one file per category); `scene/resources.rs`
  (`ModelSlot` / `DerivedViews`) owns the draw list + the GPU resource cache +
  buffer upload. Derived
  line views are built-on-demand and freed-on-off by `SceneGpu::sync_line_views`
  (invariant 3) — a view's buffer exists only while its toggle is on and is rebuilt
  live when its baked length/color drifts. Add new debug views by following that
  ensure/free pattern. A `sync_*` must compare against the **borrowed** frame
  inputs and only `to_vec()` its bake key on the rebuild path, so a steady-state
  frame allocates nothing. Geometry builders that filter hidden nodes go through
  `geometry::HiddenFilter` rather than rebuilding the set-plus-length-guard by
  hand — the guard is what keeps an out-of-range index out of
  `model.triangles.node`.
- Keep `model` + camera/debug math host-agnostic so a future renderer swap only
  touches `render`.
- **IBL HDRs must stay finite.** Bright suns in an HDR exceed `f16`'s max
  (65504), so the bake's `ibl.rs` `load_equirect_from_file` clamps every channel to
  `F16_MAX` before the `Rgba16Float` upload — otherwise they become `inf`, the
  (unbounded) irradiance integral turns to `NaN`, and the model shows black
  speckles + a dead spot at the sun. The `ibl_*` programs additionally clamp each
  *sampled* radiance to `IBL_RADIANCE_CLAMP` in both convolutions (irradiance +
  prefilter) to kill fireflies, and the skybox clamps `env * intensity` to f16 max so
  the intensity multiply can't re-overflow the HDR target. (These clamps run at
  bake time now; the shipped maps are already finite.) Don't drop them.
- Environment-dropdown HDR thumbnails (`assets/thumbnails/T_HDR_*.png`) are
  `include_bytes!`-embedded by `crates/ui`, so they must exist before `cargo
  build`. They're committed, and `packaging/generate-hdr-thumbnails.ps1`
  (ImageMagick) regenerates them from `assets/textures/T_HDR_*.hdr`; the installer
  build (`build-windows-installer.ps1`) runs it automatically. Re-run it after
  adding/replacing an HDR.
- Baked IBL maps (`assets/ibl_baked/T_IBL_*.bin`) are `include_bytes!`-embedded by
  `crates/render`, so they must exist before `cargo build`. The three HDR cubes
  (env / irradiance / prefilter) are **BC6H** block-compressed (`Bc6hRgbUfloat`, 16
  bytes per 4×4 block — the bake tool encodes them with `intel_tex_2`, a bake-only
  CPU encoder); the shared BRDF LUT stays raw little-endian f16 (`Rg16Float`). The
  runtime creates BC6H textures, which are core in Direct3D 11 feature level 11_0
  (the renderer's floor), so no explicit feature request is needed.
  They're committed; `packaging/generate-ibl-bake.ps1` regenerates them by running
  the `bake_ibl` tool (`cargo run --release -p review-render --features bake --bin
  bake_ibl`, needs a real GPU; the bake runs its `ibl_*` programs on a headless
  sokol device).
  The installer build (`build-windows-installer.ps1`) invokes it, but it is
  **freshness-gated**: the script only runs the GPU bake when a baked `.bin` is
  missing or older than an input that determines its bytes (a source HDR, or
  `ibl.rs` / `review.glsl` / `bake_ibl.rs` / `rhi/bake.rs` — i.e. an IBL precompute
  constant like sizes/mips/format, or the encode path); otherwise it's a fast no-op, so a normal
  build touches no GPU. Pass `-Force` to re-bake regardless, or run it manually
  after adding/replacing an HDR or changing a precompute constant. The shipping
  binary carries the baked maps, not the raw HDRs — `T_HDR_*.hdr` are bake-tool
  inputs only. The `.bin` byte layout is mip-major with the six cube faces
  contiguous per mip (each face a row-major grid of BC6H blocks); the bake's
  `write_cube`/`compress_bc6h_face` and the runtime `Texture::cube_block_compressed`
  must stay in lockstep on it.
