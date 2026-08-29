# CLAUDE.md — 3D Review (Rust)

A Windows-first **native** 3D model-audit viewer (think F3D / Autodesk FBX
Review). Drag-drop an **FBX**, inspect game assets, switch through debug views.
Built on `winit` (window/event loop) + **Direct3D 11** (GPU, native via the
`windows` crate) + `egui` (overlay UI, via `egui-directx11`) + vendored `ufbx`
(FBX parsing via a C bridge). Pure-Rust, no web/Electron layer.
**Target priority: Windows.**

Deeper docs: the crate map + data flow live in §2 below; `PROJECT_STATE.md`
(architecture, status, risk register), `RENDERING_PIPELINE.md` (render-pass
detail), `materials and textures plan.md` (the active materials/textures
roadmap), and `TODO.md` (running notes).

## 1. Invariants — the rules an agent will break if not told

> **Lean on the existing system; don't hand-roll your own.** Before building any
> mechanism, reach for what's already there — first the platform/framework
> primitive (e.g. egui's native `Window`/`SidePanel`/`Grid`/`Slider`, winit
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
   `render/src/scene/d3d.rs` (build-on-demand, free-on-off, live param rebuild).
4. **Heavy derived views are GPU compute and capability-gated.** Compute-based
   views (normals/tangents/overdraw, post-MVP) read buffers already on the GPU
   and write transient storage buffers. Gate them on Direct3D 11 feature/format
   support (`CheckFeatureSupport` / `CheckFormatSupport` /
   `CheckMultisampleQualityLevels`) and disable them in the UI when the active
   device can't run them — never crash.
5. **Faithful stats.** The Model Stats panel reads source DCC counts carried
   through import in `ModelStats` (original polygon/vertex counts), **never**
   post-triangulation render counts. Every stat shown must be a real measured
   value. `stats_grid` shows only measured values (Draws/Polys/Tris/Verts/UV
   Sets/FPS); `fps` is fed from the `app` render loop. Don't reintroduce
   placeholder rows.
6. **The redraw loop lives in `app`.** Redraw is driven by `winit` events and
   active camera animation (redraw-on-demand), never by UI state mutation.
   Continuous redraw only while a camera transition or interaction is live.
7. **One C extraction, one funnel.** The `ufbx_scene → flat arrays` walk is
   written once in `crates/import/src/ufbx_bridge.c` and is the *only* path FBX
   import takes. Any future caller of that C core must produce identical
   `ModelData`. Keep the two-pass count→fill discipline and free every buffer on
   every error path.
8. **No hardcoded visual values in UI components.** Every color / font size /
   border width / radius / spacing / opacity in `crates/ui` must come from a
   central semantic theme (a `theme` module), not inline literals. The only
   exception is a value computed at runtime from state (e.g. a per-axis gizmo
   color). Token names are **semantic** (`panel_bg`, `selection`, `gizmo_ball`),
   not `dark_grey_6`. Tokens live in `crates/ui/src/theme.rs` (`color`, `size`,
   `font` submodules + `apply_visuals`); add the token there first, then
   reference it.

### Rust-specific invariants

9. **All `unsafe` and all C/FFI lives in `crates/import`, `crates/psd` and
   `crates/optimize`** —
   *plus* the scoped Direct3D 11 sites below. No `unsafe` leaks into `model` / `ui`,
   nor into `render`'s geometry / material / camera modules. Before
   `slice::from_raw_parts`, null-check the pointer and treat len 0 as empty
   (`checked_slice`). Free the C scene on **both** success and error paths (no leak).
   **Sanctioned exception — psd_sdk FFI** (`crates/psd`, `review-psd`): the C-ABI
   bridge to the vendored psd_sdk C++ that decodes a PSD's merged composite (source
   art), so the viewer reads layered PSDs without a bundled ImageMagick. It links a
   **prebuilt** static lib + committed bindgen output (no `cc`/`bindgen`/libclang at
   build time — see that crate's `vendor/NOTICE.txt`); the `unsafe` is confined to
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
   **Sanctioned exception — Direct3D 11 / DXGI COM** (via the `windows` crate) is
   `unsafe` and pervasive in the renderer. It is confined to **one** place:
   `crates/render/src/rhi/` (the GPU-plumbing module — device, swapchain,
   pipelines, buffers, offscreen targets, textures, samplers, the GPU profiler, and
   the bake-only `rhi::bake`; shared by the runtime and the `bake_ibl` binary).
   `crates/app` is **fully safe** (`#![forbid(unsafe_code)]` — as are `model` and
   `ui`): its device/swapchain bootstrap goes through the safe `Gpu::new` wrapper,
   including the first-frame black clear+present that replaced the old GDI
   `startup_paint` hack. Every `unsafe` carries a `// SAFETY:` rationale and
   touches only GPU plumbing — never model geometry, camera math, or material
   logic, which stay safe.
10. **`crates/model` is host-agnostic.** It depends only on `glam` — no `windows`/
    D3D, `egui`, `winit`, or importer types. This is what kept the renderer
    swappable (wgpu → D3D11); don't add rendering/UI deps to `model`.
11. **GPU structs are `#[repr(C)]` + `bytemuck` `Pod`/`Zeroable`.** Match the HLSL
    `cbuffer` / vertex-input layout exactly (16-byte cbuffer packing) — don't
    reorder fields without updating the shader.

## 2. Where things live

```
crates/
  app/      review-app: winit ApplicationHandler, event loop, input routing
            (LMB orbit / RMB pan / wheel zoom / F frame / drag-drop /
            double-click-open), egui_winit + egui-directx11 wiring, the D3D11
            device/swapchain bootstrap (in `resumed`, via the safe `Gpu::new` —
            `app` has zero `unsafe`; incl. the first-frame black clear+present
            that replaced the GDI startup hack) + per-frame draw order (scene →
            composite → egui chrome on top → Present), redraw timing, applies
            UiOutput back to Renderer. -> src/main.rs;
            scene texture pool + off-thread decode + disk-auto-reload
            (an `impl App` block) -> src/texture_manager.rs;
            window position/size restore via %APPDATA% -> src/window_state.rs;
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
            demo_cube_model -> src/lib.rs; per-mesh-part triangle BVH (occlusion
            for the dimension labels) -> src/bvh.rs
  import/   review-import: load_model/load_fbx, ImportError, the unsafe FFI
            (repr(C) mirror structs, checked_slice, model_from_bridge_scene),
            the vendored ufbx C + bridge, build.rs (cc, cfg(has_ufbx)).
            -> src/lib.rs, src/ufbx_bridge.c/.h, build.rs
  optimize/ review-optimize: mesh optimization for the Opt workspace, over
            vendored meshoptimizer v1.2 (invariant 9's third FFI site). Depends on
            `review-model` only — hands back plain `ModelData`, never touches GPU
            or UI types. Layers, bottom up: src/ffi.rs (raw `extern "C"` decls) +
            src/meshopt.rs (the checked safe wrappers; together the entire `unsafe`
            surface) -> src/submesh.rs (splits a `ModelData` into per-(node,
            material) pieces — the unit that can survive a simplify, since
            meshoptimizer returns a new index buffer with no triangle
            correspondence) -> src/ops.rs (one function per operation) ->
            src/process.rs (walks the stack, fans out the LOD chain, reassembles a
            `ModelData` per level + measures it). src/stack.rs is the serializable
            operation stack the UI edits; src/preset.rs is its versioned JSON
            envelope. Processed meshes are pure triangles and carry **no** face
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
  psd/      review-psd: safe `decode_psd` over a C-ABI bridge to psd_sdk (C++),
            returning a PSD's merged composite as RGBA8 (invariant 9's third FFI
            site). Links a PREBUILT MSVC static lib (`vendor/fire_psd.lib`) +
            committed bindgen output (`src/bindings.rs`); the link-only build.rs runs
            no cc/bindgen. -> src/lib.rs, src/bindings.rs, build.rs, vendor/
  render/   review-render: per-view config/option types (ShadingMode,
            VertexColorMode, ActiveMaterial, CameraProjection, AntiAliasing,
            EnvironmentSettings, GtaoSettings, TonemapSettings, SceneDebugOptions,
            RendererConfig) -> src/config.rs; OrbitCamera (framing/orbit/pan/zoom/
            ortho+persp, Reversed-Z infinite perspective), UvCamera (2D UV
            viewport), CameraTransition (0.3s ease-in-out cubic), Renderer ->
            src/lib.rs.
            The Direct3D 11 GPU layer:
              * src/rhi/ — the SOLE home for D3D11/DXGI COM (`windows` crate,
                invariant 9): mod.rs (`Gpu` = device + immediate context + swapchain
                + backbuffer + the scene/color/backbuffer pass helpers + MSAA
                capability query), pipeline.rs (`Pipeline` = VS+PS+input-layout +
                raster/depth/blend state bundled from DXBC), buffer.rs (immutable
                vertex/index + dynamic `Map(WRITE_DISCARD)` cbuffer), target.rs
                (offscreen MSAA color + depth + `ResolveSubresource`), texture.rs
                (BC6H cube / immutable 2D / mipped RGBA8), sampler.rs, and bake.rs
                (offline-only headless device + cube/2D render targets + readback,
                `bake` feature).
              * src/scene/ — d3d.rs (`SceneGpu`: the whole scene renderer — builds
                the pipelines/targets/IBL/material table, draws the 2-MRT offscreen
                scene pass + GTAO + composite-to-backbuffer, plus the UV viewport,
                derived line views and selection flash, with all the `sync_*`
                build-on-demand caches incl. `sync_line_views`) and gpu_types.rs (the
                #[repr(C)] scene/post/GTAO uniforms + SceneVertex, kept in HLSL
                lockstep). The `--tracy`-gated GPU timestamp profiler
                (`ID3D11Query` → Tracy GPU context) lives in rhi/gpu_profiler.rs.
            CPU vertex generation -> src/geometry/ (vertex/grid/mesh/select/
            debug_lines/uv); editable per-material table (cbuffer `b1` + `t5..t11` +
            aniso sampler) + path-keyed texture cache -> src/material/ (state / mode /
            d3d); source-texture decode (magic-byte dispatch: PSD via `review-psd`,
            JPEG via zune's fast path, PNG/TGA/TIFF/HDR/BMP/GIF/PNM via the `image`
            crate — no ImageMagick) + filename channel auto-detect -> src/texture.rs;
            the Tex viewport's own minimal image draw (`TexGpu` — a fullscreen-
            triangle pipeline + path-keyed mipped cache + channel-select/placement
            uniform, deliberately outside the scene MRT/tonemap path so the displayed
            texel equals the stored texel) -> src/tex_d3d.rs.
            Shaders are hand-written HLSL in src/hlsl/*.hlsl, compiled offline to
            committed DXBC blobs by build.rs (`fxc`, freshness-gated, `/WX`); the
            runtime `include_bytes!`s the DXBC (no runtime shader compilation):
            scene.hlsl (mesh / line / skybox / selection / uv-fill / gtao-gbuffer),
            post.hlsl (composite: AO-darkened ambient + tone map + sRGB), gtao.hlsl
            (horizon occlusion + bilateral blur), tex.hlsl (Tex viewport image /
            checker), ibl.hlsl (the bake-only IBL precompute).
            Per-model GPU state (mesh buffers, derived views, selection/visibility
            draw lists, each with its bake key) lives in a `ModelSlot`; `SceneGpu`
            holds an **active/idle pair** so the Opt workspace can keep the source
            and processed meshes both resident and alternate between them within a
            frame for one `mem::swap` — a single slot would rebuild both meshes on
            every alternation. `render`/`render_uv` claim the source slot
            explicitly. `render_opt` draws the split (each half through the *same*
            targets sized to half the backbuffer, composited into its own half via
            a viewport rect — D3D11 clips to the viewport and the composite's UVs
            come from its vertex attribute, so no scissor and no shader change) or
            the overlay (one mesh solid, the other a ghost: the x-ray reuses the
            selection-flash fill, the wireframe ghost the line pipeline).
            Scene depth is `Depth32Float`, Reversed-Z. The offscreen scene pass is
            **2-MRT** linear-HDR (`R16G16B16A16_FLOAT`): location 0 = linear scene
            radiance, location 1 = AO-eligible diffuse-ambient radiance; GTAO has a
            *separate single-sample* view-normal/Z G-buffer (its own mesh-only pass) +
            raw/blurred `R8` occlusion, and the composite darkens the ambient by the
            scalar AO factor. HDR image-based lighting: at runtime the env cube +
            irradiance + prefilter + shared BRDF LUT are **loaded**
            (`IblD3d::from_baked`, a pure D3D11 upload — no startup precompute) from
            offline-baked assets (`assets/ibl_baked/`) for the PBR shaded path +
            skybox. The three HDR cubes ship **BC6H** block-compressed
            (`Bc6hRgbUfloat`, ~8× smaller than `Rgba16Float`, GPU-native so no
            decode); the shared BRDF LUT stays `Rg16Float`. The precompute that bakes
            + BC6H-encodes them (the `ibl.hlsl` passes run on a headless D3D11 device
            via `rhi::bake`, then `intel_tex_2` encodes BC6H) compiles only into the
            offline `bake_ibl` tool (render's `bake` feature, src/bin/bake_ibl.rs) ->
            src/ibl.rs. The model wireframe is a plain LineList drawn in the scene
            pass via the line pipeline (depth-tested against the mesh so hidden-face
            edges are occluded; fixed 1px hardware width) -> src/scene/d3d.rs +
            src/geometry/ (`wireframe_lines`).
  ui/       review-ui: egui chrome built on egui's **native windowing**, not a
            hand-rolled layout system. Option tools are native `egui::Window`s
            (collapsible/closable, non-resizable, multi-open via
            `UiState::panels_open`); the Outliner (left) + Inspector (right) are
            dockable, resizable `egui::SidePanel`s; the toolbar + status bar are
            `egui::TopBottomPanel` bands (interiors still hand-laid via
            `scope_builder` — the one remaining rework step). The 3D/UV scene + the
            composite are drawn by `app` (Direct3D 11) *before* egui, which paints
            its chrome on top with a transparent central viewport; the Tex viewport
            (`texture_view.rs`) handles only interaction (pan/zoom/fit, background
            fill, channel pick) and hands the image to `review_render`'s `TexGpu`
            (a D3D11 draw, also issued by `app`) — `ui` owns no GPU state. Plus axis
            gizmo, stats overlay, bounding-box dimension labels, startup help
            overlay; emits UiOutput intents. Thin root re-exports; modules: theme/
            state/assets/widgets/overlay/toolbar/status_bar/stats/texture_view/gizmo/
            dimensions/help
            + panels/ (mod.rs = width-pinning dispatch; one file per tool:
            anti_aliasing, bounding_box, environment, normals, gtao, tonemap,
            uv_checker, vertex_colors, wireframe, material_mode; plus inspector +
            outliner for the side panels, and opt_stack + opt_inspector for the Opt
            workspace). -> src/lib.rs + src/*.rs
            The Opt workspace's own state (the `OptStack` the chrome edits, the
            comparison-view settings, and the last run's measured figures) ->
            src/opt_state.rs. Ownership follows the convention already used for
            selection and the hidden-mesh set: `UiState` owns it, the chrome edits
            it in place, and bumping `stack_revision` is what tells `app` to
            reprocess and what undo compares; only the file-dialog actions (export,
            preset load/save) travel as an `OptIntent`. In Opt the left side panel
            splits horizontally — the scene tree on top, the operation stack in a
            resizable `TopBottomPanel::show_inside` band below — and the Inspector
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
(shared) → `render` (camera + D3D11 GPU resources) → `app` draws the scene +
composite, then `ui` paints the egui chrome on top → `app` (applies UI intents,
requests redraw). See §2 above + `PROJECT_STATE.md`. The Opt workspace branches
off that at `model`: `optimize` turns a `ModelData` plus an operation stack into
one `ModelData` per LOD level on a worker thread, and those go back through the
same `render` path as the source.

> **Model revisions must be unique across *every* mesh the renderer is handed**,
> not merely increasing within one. `SceneGpu` keys its mesh-buffer cache on the
> revision alone, so two different meshes sharing a number leaves one of them
> stale on screen. `app` issues them all from one counter
> (`App::next_model_revision`) — the source mesh and each processed Opt level.

## 3. Build & run

Use the **x64 Native Tools Command Prompt for VS 2022** (MSVC `cl` must be on
PATH so `cc` can compile `ufbx.c`).

- `cargo run -p review-app` — launch the viewer.
- `cargo check --workspace` — fast type check.
- `cargo clippy --workspace --all-targets -- -D warnings` — lint; **run before
  claiming done.**
- `cargo fmt --all` — format.
- `cargo test --workspace` — unit tests (model / import / render); add alongside
  changes. (HLSL is validated by `fxc /WX` in render's `build.rs`, not a test.)
- `cargo build --release` — release binary.
- `cargo run -p review-render --features bake --bin bake_ibl` — re-bake the IBL
  maps (needs a real GPU; outputs committed under `assets/ibl_baked/`).

Pinned (workspace deps): `winit 0.30`, `windows 0.62` (Direct3D 11/DXGI), `egui`/
`egui-winit 0.33`, `egui-directx11 0.12`, `glam 0.30`, `bytemuck 1`,
`thiserror 2`, `rfd 0.15`, `image 0.25` (png + hdr + tga/tiff/jpeg/pnm), `half 2`,
`zune-image`/`zune-core 0.5` (JPEG-only + simd; the texture fast path), `cc 1`
(build dep), `tracy-client 0.18`. `intel_tex_2` is a `bake`-only dep (CPU
BC6H encoder). Edition 2024.

> **MSRV:** `rust-version = "1.88"` — the floor required by `egui 0.33`.
> The FFI's `unsafe extern "C" { … }` blocks are the idiomatic (and, under
> Edition 2024, required) form. Bump the floor only when adopting a feature
> that needs a higher version.

`build.rs` (import) compiles `ufbx.c` + the bridge with `cc` **only when**
`third_party/ufbx/ufbx.{c,h}` exist, and sets `cfg(has_ufbx)`; without them the
workspace still builds and FBX import returns a clear error. `build.rs` (render)
compiles the HLSL in `src/hlsl/*.hlsl` to committed DXBC with `fxc`, but only when
a blob is stale and `fxc` is present (a no-fxc CI box uses the committed blobs).

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
- Native stack: direct `winit` + **native Direct3D 11** (via the `windows` crate),
  not `eframe`/wgpu. `egui` is an overlay rendered by `egui-directx11` on the same
  D3D11 device. The renderer was migrated off `wgpu` to shrink the shipped binary,
  startup time, and baseline RAM (one `D3D11CreateDevice` instead of probing every
  DX12 adapter); the workspace carries **zero** wgpu/naga/pollster/egui-wgpu, in
  the runtime *and* the offline bake tool. This supersedes the original
  "direct wgpu" decision — **do not reintroduce wgpu.** All D3D11 COM lives in
  `render/src/rhi/` + `app`'s swapchain bootstrap (invariant 9).
- UI chrome uses egui's **native windowing** (`Window` / `SidePanel` /
  `TopBottomPanel`) and stock widgets (`Grid` / `Slider` / `DragValue` /
  `ComboBox`), styled from egui's default `Visuals::dark()` plus a few theme-token
  overrides. The old hand-rolled `egui::Area` + pixel-rect panel system is gone —
  do **not** bring it back (see the lead-in to §1). Styling exceptions: keep the
  bundled Inter (proportional) + JetBrains Mono fonts; numeric value boxes
  (`DragValue`) render in monospace, everything else proportional.
- Internal format is the own `ModelData` superset (flat parallel buffers +
  original face topology + source stats). FBX is the only MVP import format,
  parsed by vendored `ufbx` through a single C bridge — don't round-trip through
  glTF (drops quad topology, changes vertex counts).
- Shading is one HLSL scene shader (`hlsl/scene.hlsl`) covering shaded / unlit /
  wireframe / uv-checker / vertex-color paths; tone mapping + sRGB encoding live in
  the post shader (`hlsl/post.hlsl`), and the model wireframe is a depth-tested
  line-list draw in the scene pass. HLSL is compiled offline to committed DXBC by
  `build.rs` (`fxc`); the runtime does no shader compilation. `3D`, `UV` and `Tex`
  viewports are all implemented; the `Tex` image is drawn by its own minimal
  fullscreen-triangle pipeline (`TexGpu`, outside the scene MRT/tonemap path) so
  channel isolation is a uniform swizzle and the displayed texel equals the stored
  texel.
- Crate boundaries are load-bearing (invariants 2, 9, 10) — keep them.

## 5. Current state

MVP: native window + Direct3D 11 viewport + egui chrome; FBX import via ufbx (drag-drop,
`Ctrl+O`, double-click empty viewport, command-line/file-association path);
orbit/pan/zoom + frame-on-`F` + home reset + 45° WASD orbit steps; grid with
axes; shaded / unlit / wireframe / shaded+wireframe, source-color / UV-checker /
vertex-color materials, plus bounding-box, face- and vertex-normal debug
overlays; orthographic/perspective toggle; animated axis gizmo (orbit +
snap-to-axis); a 2D UV viewport (independent pan/zoom, UV channel picker, wire
layout, shaded fill, per-island coloring); a 2D Tex viewport (a Direct3D 11 image
draw via `TexGpu` over the scene texture pool: texture picker, RGB/R/G/B/A channel
isolation — a shader uniform swizzle, so switching is instant; the `A` segment
auto-hides for opaque images — mipmapped, pan (LMB-drag) / zoom (wheel or
RMB-drag) / `F`-to-fit, black/white/grey/checker
background fill, and a real-values stats panel). Windows packaging (exe icon/resource
metadata + Inno Setup installer) is present.

**Opt workspace** (mesh optimization, static meshes only): a fourth mode sharing
every 3D control. The user builds a re-orderable stack of meshoptimizer
operations — weld, filter degenerate/duplicate triangles, prune components,
generate a LOD chain (standard / attribute-preserving / sloppy simplifiers with
the option flags), and the vertex-cache / overdraw / vertex-fetch reorders — with
per-object exclusions, and saves it as a JSON preset. Every edit reprocesses on a
worker (latest-request-wins, a notice only if it runs long) and the result is in
the viewport as soon as it lands, undoable through the same snapshot stack as
everything else. Comparison is a vertical split (optionally camera-synced) or a
single view with one mesh ghosted over the other and an `X` A/B swap; a second
stats card carries the processed mesh's measured counts, their change against the
source, and the ACMR / ATVR / overdraw / overfetch figures meshoptimizer measured
— which is what makes the reorder operations, invisible in the viewport, worth
having. An explicit export writes the chain to FBX via vendored ufbx_write.
Startup is untouched: nothing Opt-specific is built until the workspace is first
opened, and a stack with nothing enabled schedules no run.

The `ui` crate was migrated off the old hand-rolled `egui::Area` + pixel-rect
layout system onto egui's **native windowing**: option tools are native
`egui::Window`s (collapsible/closable, non-resizable, multi-open via
`UiState::panels_open`), the Outliner/Inspector are dockable `egui::SidePanel`s,
the toolbar/status bar are `TopBottomPanel` bands, and panel bodies use stock
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
time in `scene.hlsl` via `projection_params.y`, so it never rebuilds the IBL maps);
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
happen in `post.hlsl`); location 1 is the AO-eligible diffuse ambient radiance (IBL
diffuse + analytic fill only), which post darkens by the scalar GTAO factor. Scene
depth is `Depth32Float` cleared to 0 with `GreaterEqual` and infinite reversed
perspective (`perspective_infinite_reverse_rh`); egui-directx11 then draws the
chrome on top of the backbuffer with no depth. The model wireframe is a plain
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
- **ufbx_write's `ufbxw_view_*` buffers are borrowed, not copied** — the pointer
  must stay valid until the save completes. `export_bridge.c` uses `ufbxw_copy_*`
  throughout so nothing is borrowed past the call; reusing one scratch buffer
  across meshes with a `view` was an access violation. Related: FBX *declares* its
  unit rather than fixing one, conventionally centimeters, while import normalizes
  every file to meters — so the writer sets `UnitScaleFactor = 100`. Without it the
  geometry reads back exactly 100× too small, which no error reports.
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
- GPU struct field order must match the HLSL `cbuffer`/vertex-input layouts
  (invariant 11) — the shaders live in `crates/render/src/hlsl/*.hlsl` (compiled to
  DXBC by `build.rs`, `include_bytes!`'d at runtime); update `scene.hlsl` in
  lockstep with the `#[repr(C)]` `SceneUniforms`/`SceneVertex` structs in
  `scene/gpu_types.rs` (mind HLSL's 16-byte cbuffer packing). `fxc /WX` in
  `build.rs` catches a broken shader at build time.
- The scene geometry pass is **MRT** with **two** color targets: `scene.hlsl`'s
  `FragOutput` writes `SV_Target0` (linear scene radiance) and `SV_Target1`
  (AO-eligible diffuse ambient radiance), so every scene pipeline
  (mesh/line/uv-fill/skybox) renders into *two* RTVs and the offscreen pass binds
  *two* attachments (+ MSAA resolves) — keep them in lockstep with `FragOutput`.
  Both locations alpha-blend. Overlays (zero-normal verts) write 0 to location 1 so
  they aren't AO-darkened. GTAO does not read location 1: it has its own
  single-sample mesh-only pass (`fs_gtao_gbuffer`, one `SV_Target0` output of view
  normal `xyz` + view Z `w`) into a separate G-buffer target, avoiding MSAA edge
  averaging. GTAO is horizon-based (`gtao.hlsl`, a structured 4×4 spatial dither
  decorrelates slices) and outputs a single scalar occlusion (`R8Unorm`);
  `post.hlsl` darkens the diffuse ambient (location 1) by that scalar factor — an
  additive correction over location 0 so MSAA stays correct and direct/emissive
  light is never darkened. Tone mapping + sRGB encoding happen once in `post.hlsl`,
  not in the scene shader. fxc notes: use `SampleLevel` (not `Sample`) for any
  texture read inside a loop/branch (non-uniform control flow), and guard a
  possibly-negative `pow` base with `max(x, 0.0)` so `/WX` doesn't reject it.
- CPU-side vertex generation (grid, wireframe, face/vertex normal lines) lives in
  `crates/render/src/geometry/` (one file per category); `scene/d3d.rs`
  (`SceneGpu`) owns the draw list + the GPU resource cache + buffer upload. Derived
  line views are built-on-demand and freed-on-off by `SceneGpu::sync_line_views`
  (invariant 3) — a view's buffer exists only while its toggle is on and is rebuilt
  live when its baked length/color drifts. Add new debug views by following that
  ensure/free pattern.
- Keep `model` + camera/debug math host-agnostic so a future renderer swap only
  touches `render`.
- **IBL HDRs must stay finite.** Bright suns in an HDR exceed `f16`'s max
  (65504), so the bake's `ibl.rs` `load_equirect_from_file` clamps every channel to
  `F16_MAX` before the `Rgba16Float` upload — otherwise they become `inf`, the
  (unbounded) irradiance integral turns to `NaN`, and the model shows black
  speckles + a dead spot at the sun. `ibl.hlsl` additionally clamps each *sampled*
  radiance to `IBL_RADIANCE_CLAMP` in both convolutions (irradiance + prefilter) to
  kill fireflies, and `scene.hlsl`'s skybox clamps `env * intensity` to f16 max so
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
  the `bake_ibl` tool (`cargo run -p review-render --features bake --bin bake_ibl`,
  needs a real GPU; the bake runs its `ibl.hlsl` passes on a headless D3D11 device).
  The installer build (`build-windows-installer.ps1`) invokes it, but it is
  **freshness-gated**: the script only runs the GPU bake when a baked `.bin` is
  missing or older than an input that determines its bytes (a source HDR, or
  `ibl.rs` / `ibl.hlsl` / `bake_ibl.rs` — i.e. an IBL precompute constant like
  sizes/mips/format, or the encode path); otherwise it's a fast no-op, so a normal
  build touches no GPU. Pass `-Force` to re-bake regardless, or run it manually
  after adding/replacing an HDR or changing a precompute constant. The shipping
  binary carries the baked maps, not the raw HDRs — `T_HDR_*.hdr` are bake-tool
  inputs only. The `.bin` byte layout is mip-major with the six cube faces
  contiguous per mip (each face a row-major grid of BC6H blocks); the bake's
  `write_cube`/`compress_bc6h_face` and the runtime `Texture::cube_block_compressed`
  must stay in lockstep on it.
