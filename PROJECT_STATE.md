# Project State

Audit date: 2026-06-17

`3d-review-rs` is a Windows-first native Rust 3D model review tool. It uses
`winit` for the app shell, `wgpu` for rendering, `egui` for overlay UI, and a
vendored `ufbx` C bridge for FBX import. The current codebase is much closer to
a working MVP viewer than the stub `README.md` suggests: FBX loading, 3D review
navigation, UV layout inspection, renderer quality toggles, and Windows
packaging are all present.

## Workspace At A Glance

| Area | Current state |
| --- | --- |
| App shell | Native `winit` app in `crates/app`; on-demand redraw, drag/drop, file dialog, keyboard shortcuts, window placement persistence, startup fade. |
| Model data | Host-agnostic `review-model`; flat vertices/indices plus original topology, source stats, UV sets, material summaries, warnings field. |
| Import | FBX-only import through `review-import` and `ufbx_bridge.c`; converts source units to meters, records original unit, flattens mesh data once. |
| Rendering | `review-render`; orbit and UV cameras, offscreen HDR targets, dynamic MSAA, FXAA, IBL/PBR shading, bloom, SSAO, UV layout rendering, debug line overlays. |
| UI | `review-ui`; toolbar, status bar, panels, stats overlay, axis gizmo, startup help, semantic theme tokens. |
| Packaging | Windows resource metadata and Inno Setup installer script are present. |
| Tests | No Rust tests were found in the workspace. |
| Docs | `CLAUDE.md` is rich, but `README.md` is a one-line stub and referenced deeper docs were missing before this audit. |

## Architecture

The crate boundaries are clean and mostly match the project invariants:

- `review-model` depends only on `glam` and owns data structures such as
  `Vertex`, `ModelData`, `TopologyFace`, `ModelStats`, and `Bounds`.
- `review-import` owns the unsafe FBX/Windows startup hint code, calls the C
  bridge, converts C buffers into `ModelData`, and frees the C scene on success.
- `review-render` depends on `review-model`, not on the app crate. It owns camera
  math, GPU resources, shaders, render passes, and CPU generation for derived
  debug geometry.
- `review-ui` owns plain UI state and emits `UiOutput` intents. It does not own
  renderer resources or model internals.
- `review-app` coordinates everything: input routing, model loading, camera
  updates, egui/wgpu integration, redraw scheduling, and applying UI intents.

The normal flow is:

```text
input/file path
  -> app
  -> import
  -> ModelData
  -> render GPU resources
  -> egui scene callback
  -> egui chrome/UI intents
  -> app applies intents to renderer
```

## Implemented User-Facing Features

- FBX import via drag/drop, command-line/file-association path, double-click on
  an empty viewport, and `Ctrl+O`.
- 3D camera orbit, pan, zoom, frame, home reset, perspective/orthographic toggle,
  45 degree WASD orbit steps, and clickable/dragable axis gizmo.
- 2D UV viewport with independent pan/zoom, UV channel picker, wire layout,
  shaded fill, and per-island coloring.
- 2D Tex viewport (wgpu image draw via `TexCallback` over the scene texture pool):
  texture picker, RGB/R/G/B/A channel isolation (a shader uniform swizzle, so
  switching is instant; the `A` segment auto-hides for opaque images), mipmapped,
  pan (LMB-drag) / zoom (wheel or RMB-drag) / `F`-to-fit, black/white/grey/checker
  background fill, and a real-values stats panel (format, dimension, channels, bit
  depth, on-disk size).
- Source material color, UV checker material, and vertex-color inspection modes.
- Shaded, unlit, wireframe-only, and shaded-plus-wireframe modes.
- Grid, bounding box, face-normal lines, and vertex-normal lines.
- Stats overlay with measured draw, polygon, triangle, vertex, UV-set, unit, and
  FPS values.
- Image-based lighting, optional skybox background, bloom, SSAO, dynamic MSAA,
  and optional FXAA.
- Centralized theme tokens for UI colors, sizes, fonts, spacing, and radii.
- Window icon/resource metadata, window placement restore, and installer script.

## Important Implementation Details

- The importer preserves original polygon topology in `faces` while also
  generating triangulated render indices. Wireframe and UV layout views use
  original face loops instead of showing triangulation diagonals.
- Imported positions are normalized to meters, while the original source unit is
  retained for display in the stats overlay.
- Material base color and smoothness are baked per vertex. This keeps rendering
  simple, but it means the renderer currently does not have a true material
  table, texture slots, or per-material draw path.
- Derived debug buffers are built on demand and freed when inactive. This is
  implemented for wireframe, bounding box, normal lines, and UV view buffers.
- IBL maps are precomputed on the GPU when the environment map changes and reused
  afterward.
- The app redraws on demand. Continuous redraw is paced to the monitor refresh
  only while camera animation, pointer interaction, egui repaint, or startup fade
  needs it.

## Startup Performance

Startup was cut from ~1150ms to ~350ms (resumed → first frame submitted, release,
RTX 4080 / DX12). The remaining budget is dominated by one intrinsic cost, below.
Instrumentation is always on (`startup_timing.rs` laps + the `SceneResources::
new_core` breakdown); `crates/app`'s `--features startup-trace` additionally turns
egui-wgpu's + wgpu's `profiling::scope!`s into logged spans for a finer split.
Re-measure with `RUST_LOG=3d_review=info,review_render=info` (narrow it — `info`
alone pulls in wgpu's per-pipeline shader dumps, which inflate the timings).

Landed:
- **IBL baked offline** — runtime IBL is a pure upload (`from_baked`), no startup
  precompute (was ~270ms).
- **Deferred first frame** — `SceneResources::new_core` builds only what the
  grid-only first frame needs; the heavy scene pipelines / GTAO / real IBL warm up
  over the next few frames (`advance_build` + the app `warmup_frames` budget).
- **Cheap line shader** — the one scene pipeline on the first-frame critical path
  (the grid's `line_pipeline`) uses a dedicated flat-color `fs_line` entry instead
  of the PBR `fs_main`. Measured A/B: its compile dropped 34.7ms → 7.4ms, taking
  first_frame ~90ms → ~64ms and total ~388ms → ~352ms. Output is byte-identical to
  `fs_main`'s zero-normal overlay branch.

The floor — do NOT re-investigate without new evidence (fully detailed at the
`set_window` call site in `crates/app/src/main.rs`): the first `set_window`
(~265ms) is **intrinsic DX12 multi-adapter probing**, not our overhead. wgpu's
`enumerate_adapters` creates an `ID3D12Device` per adapter to read its features
(~197ms for 4 adapters, incl. the cold NVIDIA driver-DLL load), then
`request_device` (~47ms) + egui `Renderer::new` (~13ms) + swapchain (~4ms). Phase
D ruled out every workaround: driver pre-warm can't beat the cold-load timing,
bypassing egui to call `request_adapter` ourselves pays the same per-adapter
probing, and it is unfixed upstream through wgpu 29 / egui-wgpu 0.34 (wgpu #3332).

## What Is Good

- The crate split is strong. `model` is renderer/UI agnostic, `render` avoids
  app ownership, and `app` is the coordinator.
- The rendering pipeline already has a useful offscreen seam. This makes bloom,
  SSAO, FXAA, and future post effects possible without drawing the scene directly
  into egui's framebuffer.
- Capability gating is present for MSAA levels, IBL, and SSAO, which should
  prevent unsupported adapter features from crashing the UI path.
- Resource lifetimes are thoughtful. Steady-state frames do not rebuild targets,
  pipelines, IBL maps, mesh buffers, or derived debug buffers unless an input
  actually changes.
- The importer has a clear count-then-fill structure, null checks before slices,
  and a single funnel from FBX to `ModelData`.
- The camera code handles framing, safe areas for chrome, near/far precision, UV
  pan/zoom, and animation with more care than a typical MVP.
- The UI implementation is modular and theme-driven. Most controls share common
  widgets and semantic tokens instead of scattering visual literals.
- Packaging is not an afterthought: product metadata, exe icon resources, file
  association registration, and installer generation are all represented.

## What Can Be Better

- Add tests. There are currently no `#[test]` or `cfg(test)` blocks. Good first
  targets are `ModelData` UV helpers, bounds recomputation, camera framing,
  import conversion with fixture FBX files, UV island detection, and shader/host
  layout consistency.
- Expand docs. `README.md` is only a title, and `CLAUDE.md` referenced deeper
  docs that did not exist. This file and `RENDERING_PIPELINE.md` are a start, but
  the public README still needs build/run/use instructions.
- Surface import errors and model warnings in the UI. Today failed loads are
  logged with `tracing::warn!`, but the viewer does not show an in-app error.
  `ModelWarning` exists in the data model, but warning generation/display is not
  active yet.
- Texture mode is implemented: the `Tex` tab draws the selected pooled texture
  through `review-render`'s standalone `TexCallback` (`render/src/tex.rs` +
  `tex.wgsl`), while `ui/src/texture_view.rs` owns only interaction (pan/zoom/fit,
  background fill, channel pick) — texture picker, RGB/R/G/B/A channel isolation
  (a shader uniform swizzle; the `A` segment auto-hides for opaque images),
  mipmapped, pan/zoom, black/white/grey/checker background fill, and a real-values
  stats panel. The callback is its own minimal fullscreen-triangle pipeline,
  outside the scene MRT/tonemap path. The per-path GPU cache is an LRU bounded to
  16 textures (a 17th upload evicts the least-recently-viewed). Future work:
  DDS/KTX2 (compressed) source support and a per-mip view.
- Decide what `LoadOptions::triangulate` means. It is passed through Rust/C, but
  the C bridge currently always triangulates render indices and ignores the
  option value.
- Move toward a real material/texture pipeline. Current rendering bakes material
  base color and smoothness into vertices; there is no texture loading, normal
  map support, metallic/roughness maps, alpha mode handling, or per-material draw
  grouping.
- Add validation and diagnostics for large scenes. High MSAA plus three
  `Rgba16Float` color targets can consume a lot of VRAM, and CPU-derived
  overlays can be expensive on dense meshes.
- Consider an explicit render graph once more effects arrive. `targets.rs` is a
  good seed, but the pass order and resource dependencies are still hand-wired in
  the `scene/` module (`scene/callback.rs` orchestration + `scene/resources.rs`).
- Add CI or at least documented local acceptance commands. `CLAUDE.md` says to
  run check/clippy/test, but the repo does not yet encode that policy.

## Current Risk Register

| Risk | Why it matters | Suggested response |
| --- | --- | --- |
| No automated tests | Renderer and importer changes can regress silently. | Add focused unit tests and fixture-based importer tests first. |
| Minimal public docs | New contributors/users cannot build or understand the app from `README.md`. | Expand README and link these audit docs. |
| Partial HDR color pipeline | Bloom/SSAO are useful, but final color is still tone-mapped in the scene shader before post. | Migrate to a fully linear HDR scene color and move tone mapping to post. |
| Texture viewer source-format coverage | The Tex viewer decodes PNG/JPG/TGA/etc. but not compressed (DDS/KTX2) game textures. | Add compressed-format decode so packed game assets can be inspected too. |
| Import errors are invisible | A bad file appears to do nothing unless logs are visible. | Add status/toast/modal error reporting. |
| GPU memory pressure at high MSAA | Three HDR MRTs plus resolves/depth scale quickly. | Show memory-aware limits or defaults, and profile common resolutions. |
| IBL precompute submission burst | Environment changes submit many small GPU passes. | Batch precompute passes into fewer encoders/submissions if stalls appear. |

## Recommended Next Steps

1. Write the public `README.md`: purpose, screenshots later, prerequisites,
   build/run commands, controls, supported formats, packaging.
2. Add a small test suite around model helpers, camera math, UV island logic, and
   importer fixture loading.
3. Add in-app load failure and warning display.
4. Extend the Tex viewer to compressed (DDS/KTX2) source textures.
5. Start the full linear-HDR migration: scene color stays linear, tone mapping
   and output encoding move fully into post.
6. Introduce lightweight renderer diagnostics: adapter name, active target size,
   MSAA, pass toggles, and perhaps GPU timing behind a debug flag.

