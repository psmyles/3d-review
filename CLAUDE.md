# CLAUDE.md — 3D Review (Rust)

A Windows-first **native** 3D model-audit viewer (think F3D / Autodesk FBX
Review). Drag-drop an **FBX**, inspect game assets, switch through debug views.
Built on `winit` (window/event loop) + `wgpu` (GPU) + `egui` (overlay UI) +
vendored `ufbx` (FBX parsing via a C bridge). Pure-Rust, no web/Electron layer.
**Target priority: Windows.**

Deeper docs: the crate map + data flow live in §2 below; `PROJECT_STATE.md`
(architecture, status, risk register), `RENDERING_PIPELINE.md` (render-pass
detail), `materials and textures plan.md` (the active materials/textures
roadmap), and `TODO.md` (running notes).

## 1. Invariants — the rules an agent will break if not told

> **Lean on the existing system; don't hand-roll your own.** Before building any
> mechanism, reach for what's already there — first the platform/framework
> primitive (e.g. egui's native `Window`/`SidePanel`/`Grid`/`Slider`, winit/wgpu
> facilities), then an existing helper in this codebase (a `theme` token, a
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
   Implemented for the line views by `SceneResources::sync_line_views` in
   `render/src/scene/resources.rs` (build-on-demand, free-on-off, live param
   rebuild).
4. **Heavy derived views are GPU compute and capability-gated.** Compute-based
   views (normals/tangents/overdraw, post-MVP) read buffers already on the GPU
   and write transient storage buffers. Gate them on `wgpu` adapter features and
   disable them in the UI when the active adapter can't run them — never crash.
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

9. **All `unsafe` and all C/FFI lives in `crates/import`.** No `unsafe` leaks
   into `model` / `render` / `ui` / `app`. Before `slice::from_raw_parts`,
   null-check the pointer and treat len 0 as empty (`checked_slice`). Free the
   C scene on **both** success and error paths (no leak).
   **One sanctioned exception:** `crates/app/src/startup_paint.rs` holds a small
   Windows-only `unsafe` GDI block (`GetDC`/`FillRect`/`ReleaseDC`) that fills
   the new window black before the first wgpu present, killing the white startup
   flash. It has to run on the live winit `Window` the instant it's created in
   `resumed` (before surface setup), touches no model/render state, and releases
   its DC in the same call — so the boundary this invariant protects still holds.
   Do not grow this exception: any *other* new `unsafe`/FFI still belongs in
   `import`.
10. **`crates/model` is host-agnostic.** It depends only on `glam` — no `wgpu`,
    `egui`, `winit`, or importer types. This is what lets the renderer be
    swapped later; don't add rendering/UI deps to `model`.
11. **GPU structs are `#[repr(C)]` + `bytemuck` `Pod`/`Zeroable`.** Match the
    WGSL layout exactly; don't reorder fields without updating the shader.

## 2. Where things live

```
crates/
  app/      review-app: winit ApplicationHandler, event loop, input routing
            (LMB orbit / RMB pan / wheel zoom / F frame / drag-drop /
            double-click-open), egui_winit + egui_wgpu wiring, redraw timing,
            applies UiOutput back to Renderer. -> src/main.rs;
            scene texture pool + off-thread decode + disk-auto-reload
            (an `impl App` block) -> src/texture_manager.rs;
            window position/size restore via %APPDATA% -> src/window_state.rs;
            startup black-fill (invariant 9 exception) -> src/startup_paint.rs
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
  render/   review-render: per-view config/option types (ShadingMode,
            VertexColorMode, ActiveMaterial, CameraProjection, AntiAliasing,
            EnvironmentSettings, Bloom/Ssao/Tonemap settings, SceneDebugOptions,
            RendererConfig) -> src/config.rs; OrbitCamera (framing/orbit/pan/zoom/
            ortho+persp, Reversed-Z infinite perspective), UvCamera (2D UV
            viewport), CameraTransition (0.3s ease-in-out cubic), Renderer ->
            src/lib.rs. SceneCallback (+ new_uv) + the GPU resource cache split
            into src/scene/: callback.rs (inputs + prepare/paint + the draw list),
            resources.rs (SceneResources + sync/update/encode incl.
            sync_line_views), pipelines.rs (pipeline builders + the 3-MRT
            `scene_color_targets` factory), buffers.rs (buffer/texture/bind-group
            builders), gpu_types.rs (the #[repr(C)] SceneUniforms/SceneVertex +
            the scene.wgsl include + naga validation test). CPU vertex generation
            -> src/geometry/ (vertex/grid/mesh/select/debug_lines/uv); editable
            material table (group 3) + path-keyed texture cache -> src/material/
            (state/table/upload); texture decode + filename channel auto-detect ->
            src/texture.rs; scene shader -> src/scene.wgsl.
            SCENE_DEPTH_FORMAT (Depth32Float,
            Reversed-Z) is separate from EGUI_DEPTH_FORMAT (Depth24Plus).
            Offscreen linear-HDR targets (linear scene radiance + linear-HDR bloom
            MRT + AO-eligible diffuse-ambient MRT, all
            Rgba16Float; separate single-sample GTAO normal/view-Z G-buffer) +
            composite/tone-map/FXAA seam -> src/targets.rs, src/post.rs(+post.wgsl).
            HDR image-based lighting: at runtime the env cube + irradiance +
            prefilter + shared BRDF LUT are **loaded** from offline-baked assets
            (`assets/ibl_baked/`) via `IblResources::from_baked` — a pure upload,
            no startup precompute — for the PBR shaded path + skybox. The three HDR
            cubes ship **BC6H** block-compressed (`Bc6hRgbUfloat`, ~8× smaller than
            `Rgba16Float`, GPU-native so no decode); the shared BRDF LUT stays
            `Rg16Float`. The precompute that bakes + BC6H-encodes them (+
            `ibl.wgsl`) compiles only into the offline `bake_ibl` tool (render's
            `bake` feature, src/bin/bake_ibl.rs) -> src/ibl.rs, src/ibl.wgsl. Bloom
            (bright-pass + separable blur, half-res) -> src/bloom.rs, src/bloom.wgsl.
            GTAO (horizon-based occlusion with a structured 4x4 spatial dither + 5x5
            bilateral blur over the single-sample G-buffer; post darkens the diffuse
            ambient by a scalar AO factor) -> src/gtao.rs, src/gtao.wgsl. The model wireframe is a plain
            LineList drawn in the scene pass via `line_pipeline` (depth-tested
            against the mesh so hidden-face edges are occluded; no thickness
            control) -> src/scene/ + src/geometry/ (`wireframe_lines`). The Tex
            viewport's image draw is its own standalone egui paint callback
            (`TexCallback`) — a minimal fullscreen-triangle pipeline with a path-keyed
            mipped-texture cache + a channel-select/placement uniform, deliberately
            outside the scene MRT/tonemap path so the displayed texel equals the
            stored texel -> src/tex.rs, src/tex.wgsl
  ui/       review-ui: egui chrome built on egui's **native windowing**, not a
            hand-rolled layout system. Option tools are native `egui::Window`s
            (collapsible/closable, non-resizable, multi-open via
            `UiState::panels_open`); the Outliner (left) + Inspector (right) are
            dockable, resizable `egui::SidePanel`s; the toolbar + status bar are
            `egui::TopBottomPanel` bands (interiors still hand-laid via
            `scope_builder` — the one remaining rework step). The 3D/UV scene
            paints on the background layer behind the chrome; the Tex viewport
            (`texture_view.rs`) handles only interaction (pan/zoom/fit, background
            fill, channel pick) and hands the image to `review_render`'s
            `TexCallback` to draw — it owns no GPU state. Plus axis gizmo,
            stats overlay, bounding-box dimension labels, startup help overlay;
            emits UiOutput intents. Thin root re-exports; modules: theme/state/
            assets/widgets/overlay/toolbar/status_bar/stats/texture_view/gizmo/
            dimensions/help
            + panels/ (mod.rs = width-pinning dispatch; one file per tool:
            anti_aliasing, bloom, bounding_box, environment, normals, gtao,
            tonemap, uv_checker, vertex_colors, wireframe; plus inspector +
            outliner for the side panels). -> src/lib.rs + src/*.rs
third_party/ufbx/   vendored ufbx.c / ufbx.h (compiled only if present)
assets/icons/       PNG toolbar/gizmo icons (include_bytes!)
assets/test_models/ local FBX fixtures for manual checks
```

Data flow: input/file-drop → `app` → `import` (FBX→`ModelData`) → `model`
(shared) → `render` (camera + GPU buffers) → `ui` (overlays + scene callback) →
`app` (applies UI intents, requests redraw). See §2 above + `PROJECT_STATE.md`.

## 3. Build & run

Use the **x64 Native Tools Command Prompt for VS 2022** (MSVC `cl` must be on
PATH so `cc` can compile `ufbx.c`).

- `cargo run -p review-app` — launch the viewer.
- `cargo check --workspace` — fast type check.
- `cargo clippy --workspace --all-targets -- -D warnings` — lint; **run before
  claiming done.**
- `cargo fmt --all` — format.
- `cargo test --workspace` — unit tests (model / import / render, incl. the
  `scene_shader_validates` naga shader-validation test); add alongside changes.
- `cargo build --release` — release binary.

Pinned (workspace deps): `winit 0.30`, `wgpu 24`, `egui`/`egui-winit`/
`egui-wgpu 0.31`, `glam 0.30`, `bytemuck 1`, `thiserror 2`, `rfd 0.15`,
`image 0.25` (png + hdr), `half 2`, `pollster 0.4`, `cc 1` (build dep). Edition 2024.

> **MSRV:** `rust-version = "1.85"` — the floor required by Edition 2024.
> The FFI's `unsafe extern "C" { … }` blocks are the idiomatic (and, under
> Edition 2024, required) form. Bump the floor only when adopting a feature
> that needs a higher version.

`build.rs` compiles `ufbx.c` + the bridge with `cc` **only when**
`third_party/ufbx/ufbx.{c,h}` exist, and sets `cfg(has_ufbx)`; without them the
workspace still builds and FBX import returns a clear error.

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
- Native stack: direct `winit` + `wgpu` (not `eframe`); `egui` is an overlay
  drawn via an `egui_wgpu` paint callback. Prefer DX12 on Windows
  (DX12|Vulkan|Metal requested).
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
- Shading is one inline WGSL scene shader covering shaded / unlit / wireframe /
  uv-checker / vertex-color paths; tone mapping + sRGB encoding live in the post
  shader, and the model wireframe is a depth-tested line-list draw in the scene
  pass. `3D`, `UV` and `Tex` viewports are all implemented; the `Tex` image is
  drawn by its own standalone egui paint callback (`TexCallback`, a minimal
  fullscreen-triangle pipeline outside the scene MRT/tonemap path) so channel
  isolation is a uniform swizzle and the displayed texel equals the stored texel.
- Crate boundaries are load-bearing (invariants 2, 9, 10) — keep them.

## 5. Current state

MVP: native window + wgpu viewport + egui chrome; FBX import via ufbx (drag-drop,
`Ctrl+O`, double-click empty viewport, command-line/file-association path);
orbit/pan/zoom + frame-on-`F` + home reset + 45° WASD orbit steps; grid with
axes; shaded / unlit / wireframe / shaded+wireframe, source-color / UV-checker /
vertex-color materials, plus bounding-box, face- and vertex-normal debug
overlays; orthographic/perspective toggle; animated axis gizmo (orbit +
snap-to-axis); a 2D UV viewport (independent pan/zoom, UV channel picker, wire
layout, shaded fill, per-island coloring); a 2D Tex viewport (a wgpu image draw
via `TexCallback` over the scene texture pool: texture picker, RGB/R/G/B/A channel
isolation — a shader uniform swizzle, so switching is instant; the `A` segment
auto-hides for opaque images — mipmapped, pan (LMB-drag) / zoom (wheel or
RMB-drag) / `F`-to-fit, black/white/grey/checker
background fill, and a real-values stats panel). Windows packaging (exe icon/resource
metadata + Inno Setup installer) is present.

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
pass; **anti-aliasing** has dynamic scene MSAA (Off/2×/4×/8×/16×, gated on
`Depth32Float` support) + FXAA, on the status-bar AA button; **HDR image-based
lighting + PBR** is the default Shaded look — six baked HDR environments (each
with a preview thumbnail shown in the Environment dropdown), precomputed
irradiance/prefilter/BRDF-LUT maps in `ibl.rs`, an optional skybox, and a live
0–360° environment yaw rotation (applied at sample time in `scene.wgsl` via
`projection_params.y`, so it never rebuilds the IBL maps);
**bloom** (HDR glow) is off by default — a bright-pass + separable blur over the
linear-HDR bloom MRT so only bright highlights glow and overlays never do;
**Ambient Occlusion** (GTAO internally) is on by default — horizon-based
occlusion (a structured 4×4 spatial dither decorrelates slices) + 5×5 bilateral
blur over a *separate single-sample* view-normal/view-Z G-buffer (its own
mesh-only pass, not an MSAA MRT). Composed in post as an additive correction over
the full radiance: post darkens the AO-eligible diffuse ambient (location 2) by a
scalar AO factor and leaves location 0 otherwise intact, so direct + emissive
light are never darkened. UI knobs are Radius / Intensity / Thickness / Quality;
user-facing strings stay "Ambient Occlusion". **Tone mapping** is on by default — the
composite applies a selectable operator (Khronos PBR Neutral / Linear / Reinhard /
ACES / AgX) to the linear-HDR radiance before sRGB encoding; toggling it off is a
linear pass-through. The status bar's right group holds the IBL / Bloom / Ambient
Occlusion / Tonemapper / Anti-aliasing toggles (left-click toggles, right-click opens each
tool's options panel — Environment / Bloom / Ambient Occlusion / Tonemapper / Anti
Aliasing).

The renderer is now **fully linear-HDR with Reversed-Z scene depth**: scene MRT
location 0 carries linear radiance (PBR-Neutral tone mapping + `linear_to_srgb`
moved into `post.wgsl`); location 1 is the linear-HDR bloom source; location 2 is
the AO-eligible diffuse ambient radiance (IBL diffuse + analytic fill only), which
post darkens by the scalar GTAO factor. Scene depth
is `Depth32Float` cleared to 0 with `GreaterEqual` and infinite reversed
perspective (`perspective_infinite_reverse_rh`); egui's framebuffer keeps its own
`EGUI_DEPTH_FORMAT`. The model wireframe is a plain `LineList` drawn inside the
scene pass via `line_pipeline`, so it depth-tests against the mesh (Reversed-Z
`GreaterEqual`, no depth write) and edges on hidden faces are occluded, while the
scene MSAA antialiases it (the trade-off is fixed 1px hardware line width).

Known gaps: in-app load-error/warning display, GPU-buffer visualization, and
additional import formats (glTF/OBJ) are post-MVP (`TODO.md`). Importing
engine-cooked compressed textures (KTX2/DDS) is intentionally **not** a goal:
artists test *source* assets (PNG/TGA/…); KTX2/DDS are produced inside an
engine's content pipeline and never hand-authored or carried, so the viewer is
never handed one. (We do use GPU block compression internally — the baked IBL
cubes are BC6H — but that's our own offline bake, not an import path.) Tests
exist (model / import / render unit tests + the `scene_shader_validates` naga
check, run headless in CI); GPU render checks stay manual.

## 6. Gotchas

- Real GPU required for render checks; reserve CI for `check` / `clippy` /
  import unit tests.
- GPU struct field order must match the WGSL `Uniforms`/vertex layouts
  (invariant 11) — the shader lives in `crates/render/src/scene.wgsl` (loaded via
  `include_str!` in `scene/gpu_types.rs`); update it in lockstep with the
  `#[repr(C)]` `SceneUniforms`/`SceneVertex` structs there (the
  `scene_shader_validates` naga test lives beside them) if you change them.
- The scene geometry pass is **MRT** with **three** color targets: `scene.wgsl`'s
  `FragOutput` writes location 0 (linear scene radiance), location 1 (linear-HDR
  bloom source) and location 2 (AO-eligible diffuse ambient radiance), so every
  scene pipeline (mesh/line/uv-fill/skybox) must declare *three* color targets —
  built via the shared `scene_color_targets` factory in `scene/pipelines.rs` — and
  the offscreen pass *three* attachments + resolves — keep them in lockstep with
  `FragOutput`. All three locations alpha-blend now. Overlays (zero-normal verts)
  write 0 to locations 1 and 2 so they neither bloom nor get AO-darkened. GTAO does
  not read location 2's normal: it has its own single-sample mesh-only pass
  (`fs_gtao_gbuffer`, one `@location(0)` output of view normal `xyz` + view Z `w`)
  into a separate G-buffer target, avoiding MSAA edge averaging. GTAO is
  horizon-based (`gtao.wgsl`, a structured 4×4 spatial dither decorrelates slices)
  and outputs a single scalar occlusion (`R8Unorm`); `post.wgsl` darkens the
  diffuse ambient (location 2) by that scalar factor — an additive correction over
  location 0 so bloom/MSAA stay correct and direct/emissive light are never
  darkened. Tone mapping +
  sRGB encoding happen once in `post.wgsl`, not in the scene shader. naga's WGSL
  rejects `_` digit separators in numeric literals (e.g. `0.227_027`) — write
  float constants without them, and use `textureSampleLevel` (not `textureSample`)
  anywhere a texture is read inside a loop/branch (non-uniform control flow).
- CPU-side vertex generation (grid, wireframe, face/vertex normal lines) lives in
  `crates/render/src/geometry/` (one file per category); `scene/callback.rs` owns
  the callback + draw list, `scene/resources.rs` the GPU resource cache + buffer
  upload. Derived line views are built-on-demand and freed-on-off by
  `SceneResources::sync_line_views` (invariant 3) — a view's buffer exists only
  while its toggle is on and is rebuilt live when its baked length/color drifts.
  Add new debug views by following that ensure/free pattern.
- Keep `model` + camera/debug math host-agnostic so a future renderer swap only
  touches `render`.
- **IBL HDRs must stay finite.** Bright suns in an HDR exceed `f16`'s max
  (65504), so `ibl.rs` `decode_hdr` clamps every channel to `F16_MAX` before the
  `Rgba16Float` upload — otherwise they become `inf`, the (unbounded) irradiance
  integral turns to `NaN`, and the model shows black speckles + a dead spot at
  the sun. `ibl.wgsl` additionally clamps each *sampled* radiance to
  `IBL_RADIANCE_CLAMP` in both convolutions (irradiance + prefilter) to kill
  fireflies, and `scene.wgsl`'s skybox clamps `env * intensity` to f16 max so the
  intensity multiply can't re-overflow the HDR target. Don't drop these clamps.
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
  dep); the shared BRDF LUT stays raw little-endian f16 (`Rg16Float`). Because the
  runtime creates BC6H textures, the device hard-requests `TEXTURE_COMPRESSION_BC`
  (`crates/app/src/main.rs`) — universal on the desktop DX12/Vulkan/Metal targets.
  They're committed; `packaging/generate-ibl-bake.ps1` regenerates them by running
  the `bake_ibl` tool (`cargo run -p review-render --features bake --bin bake_ibl`,
  needs a real GPU). The installer build (`build-windows-installer.ps1`) invokes
  it, but it is **freshness-gated**: the script only runs the GPU bake when a
  baked `.bin` is missing or older than an input that determines its bytes (a
  source HDR, or `ibl.rs` / `ibl.wgsl` / `bake_ibl.rs` — i.e. an IBL precompute
  constant like sizes/mips/format, or the encode path); otherwise it's a fast
  no-op, so a normal build touches no GPU. Pass `-Force` to re-bake regardless,
  or run it manually after adding/replacing an HDR or changing a precompute
  constant. The
  shipping binary carries the baked maps, not the raw HDRs — `T_HDR_*.hdr` are
  bake-tool inputs only. The `.bin` byte layout is mip-major with the six cube
  faces contiguous per mip (each face a row-major grid of BC6H blocks);
  `compress_bc6h_face`/readback and the runtime `upload_cube` must stay in lockstep
  on it.
