# Project State

Audit date: 2026-06-25

`3d-review-rs` is a Windows-first native Rust 3D model review tool. It uses
`winit` for the app shell, `wgpu` for rendering, `egui` for overlay UI, and a
vendored `ufbx` C bridge for FBX import. The codebase is a working MVP viewer
plus a delivered materials/textures pipeline and a modern linear-HDR renderer:
FBX loading, 3D review navigation, UV layout inspection, a Tex viewport, an
editable per-material PBR table with texture slots, and Windows packaging are all
present.

## Workspace At A Glance

| Area | Current state |
| --- | --- |
| App shell | Native `winit` app in `crates/app`; on-demand redraw, drag/drop, file dialog, keyboard shortcuts, window placement persistence, startup black-fill. Owns the scene-wide texture pool (off-thread decode + disk auto-reload) and applies material edit intents. |
| Model data | Host-agnostic `review-model`; flat vertices/indices plus original topology, source stats, UV sets, material import defaults, warnings field, grouped per-triangle `TriangleData`, and a per-mesh-part triangle BVH. |
| Import | FBX-only import through `review-import` and `ufbx_bridge.c`; converts source units to meters, records original unit, flattens mesh data once, carries per-material import defaults. |
| Rendering | `review-render`; orbit and UV cameras, fully linear-HDR offscreen MRT targets, Reversed-Z scene depth, dynamic MSAA, FXAA, baked IBL/PBR shading, bloom, GTAO ambient occlusion, selectable tone-mapping operators, an editable material table (group 3) with a path-keyed mipped texture cache, UV layout rendering, debug line overlays, and a standalone Tex image callback. |
| UI | `review-ui`; native egui windowing — toolbar/status-bar bands, dockable Outliner/Inspector side panels, per-tool option `Window`s, stats overlay, axis gizmo, bounding-box dimension labels, startup help, semantic theme tokens. |
| Packaging | Windows resource metadata, exe icon, file-association registration, and Inno Setup installer (with freshness-gated IBL bake + HDR thumbnail generation). |
| Tests | Unit tests across `model` / `import` / `render`, plus the `scene_shader_validates` naga shader-validation check; run headless in CI. GPU render checks stay manual. |
| Docs | `CLAUDE.md` is rich; this file + `RENDERING_PIPELINE.md` cover architecture and pipeline. `README.md` is still a one-line stub. |

## Architecture

The crate boundaries are clean and match the project invariants:

- `review-model` depends only on `glam` and owns data structures such as
  `Vertex`, `ModelData`, `TopologyFace`, `ModelStats`, `Bounds`, `TriangleData`,
  and the triangle `bvh`.
- `review-import` owns the unsafe FBX/Windows startup code, calls the C bridge,
  converts C buffers into `ModelData`, and frees the C scene on every path.
- `review-render` depends on `review-model`, not on the app crate. It owns camera
  math, GPU resources, shaders, render passes, the material/texture table, and
  CPU generation for derived debug geometry.
- `review-ui` owns plain UI state and emits `UiOutput` intents (including
  `MaterialEdit`). It does not own renderer resources or model internals.
- `review-app` coordinates everything: input routing, model loading, the
  scene-wide texture pool, material edit application, camera updates, egui/wgpu
  integration, and redraw scheduling.

The normal flow is:

```text
input/file path
  -> app
  -> import
  -> ModelData
  -> render GPU resources
  -> egui scene callback
  -> egui chrome/UI intents (incl. material edits)
  -> app applies intents to renderer
```

## Implemented User-Facing Features

- FBX import via drag/drop, command-line/file-association path, double-click on
  an empty viewport, and `Ctrl+O`.
- 3D camera orbit, pan, zoom, frame, home reset, perspective/orthographic toggle,
  45 degree WASD orbit steps, and a clickable/draggable axis gizmo.
- Outliner mesh-part selection and an Inspector that edits per-material PBR
  parameters: base color, metallic, roughness/smoothness workflow, emissive,
  alpha mode (Opaque/Blend/Clip) + cutoff, and texture slot channel routing.
- 2D UV viewport with independent pan/zoom, UV channel picker, wire layout,
  shaded fill, and per-island coloring.
- 2D Tex viewport (wgpu image draw via `TexCallback` over the scene texture pool):
  texture picker, RGB/R/G/B/A channel isolation (a shader uniform swizzle, so
  switching is instant; the `A` segment auto-hides for opaque images), mipmapped,
  pan (LMB-drag) / zoom (wheel or RMB-drag) / `F`-to-fit, black/white/grey/checker
  background fill, and a real-values stats panel.
- Source material color, UV checker material, and vertex-color inspection modes.
- Shaded, unlit, wireframe-only, and shaded-plus-wireframe modes (the wireframe
  is a depth-tested line-list drawn inside the scene pass).
- Grid, bounding box with dimension labels (BVH-occluded), face-normal lines, and
  vertex-normal lines.
- Stats overlay with measured draw, polygon, triangle, vertex, UV-set, unit, and
  FPS values.
- Baked HDR image-based lighting (six environments with preview thumbnails +
  live 0–360° yaw), optional skybox, bloom, GTAO ambient occlusion, selectable
  tone-mapping operators (Khronos PBR Neutral / Linear / Reinhard / ACES / AgX),
  dynamic MSAA (Off/2×/4×/8×/16×), and optional FXAA.
- Centralized theme tokens for UI colors, sizes, fonts, spacing, and radii.
- Window icon/resource metadata, window placement restore, and installer script.
- **Skinned meshes and animation clips.** A skinned FBX rests in its file default
  pose with the mesh skinned onto the skeleton (GPU linear-blend skinning over
  exact per-vertex influence runs, normalised as ufbx does); rigidly animated
  hierarchies and blend shapes deform too. Every animation stack imports as a
  clip (baked by ufbx at the file's frame rate, no key reduction) and is listed
  in a third Outliner tab, `Animations (N)`, present only for animated files and
  never in Opt. Selecting a clip lands paused on its first frame and brings up a
  bottom-centre transport: go-to-start, step back, play/pause, step forward, a
  frame scrubber, `frame / total   seconds`, loop (on by default) and speed
  (0.25–2×); `Space` and `,` / `.` mirror it. Framing and the bounding box use
  the selected clip's motion envelope, measured once at import.

## Important Implementation Details

- The importer preserves original polygon topology in `faces` while also
  generating triangulated render indices. Wireframe and UV layout views use
  original face loops instead of showing triangulation diagonals. Per-triangle
  face/material/node arrays are grouped in `TriangleData` with a lockstep
  `validate` guard.
- Imported positions are normalized to meters, while the original source unit is
  retained for display in the stats overlay.
- Vertices stay world-baked in the *bind* pose (invariant 1); every deformation
  is a delta applied on the GPU. `SceneVertex` carries a 16-byte `deform` lane
  (influence run + blend-shape run) into per-model structured buffers built with
  the mesh; a pose is a palette of `float3x4` deltas — one per scene node
  (`world × inverse(rest world)`, what rigid geometry and the skeleton overlay
  reference) then one per skin cluster (`bone_world × geometry_to_bone ×
  inverse(mesh geometry_to_world)`) — plus per-shape morph weights, uploaded only
  when the pose revision moves. `review_model::anim` is the single definition of
  the pose (parents-first recomposition from each node's rest local transform
  with the clip's keys substituted per channel, ufbx's in-between blend-shape
  rule) and the CPU reference the shader, the clip envelopes and the tests share.
  The importer loads with ufbx's helper-node inherit-mode handling so every node
  is a plain `parent × local` product. Dual-quaternion skins render as linear and
  the Inspector says so; the Opt workspace draws the bind pose (its processed
  meshes carry no skin) and hides the clip UI; dimension-label occlusion and the
  AO bake still use the bind-pose buffers.
- There is now a real material/texture pipeline. The editable `MaterialState`
  table (bind group 3) carries per-material base color, metallic, roughness,
  emissive, alpha mode + cutoff, and a roughness/smoothness workflow, seeded from
  import defaults and edited via `MaterialEdit` intents. It has seven texture
  slots fed by a path-keyed GPU texture cache (mipmapped + 16× anisotropic),
  per-material draw ranges over the reordered index buffer, and a neutral
  fallback for unmaterialed triangles.
- Textures are decoded off-thread into an app-owned scene-wide pool with disk
  auto-reload; the per-path GPU caches are bounded (the Tex viewer's standalone
  cache is an LRU of 16).
- The renderer is fully linear-HDR with Reversed-Z scene depth. The scene
  geometry pass is MRT with three `Rgba16Float` color targets (location 0 linear
  radiance, location 1 linear-HDR bloom source, location 2 AO-eligible diffuse
  ambient); tone mapping + sRGB encoding happen once in `post.wgsl`. Scene depth
  is `Depth32Float` cleared to 0 with `GreaterEqual` and infinite reversed
  perspective.
- GTAO (horizon-based) runs over a separate single-sample view-normal/view-Z
  G-buffer (its own mesh-only pass, not the MSAA MRT), with a 4×4 structured
  dither + 5×5 bilateral blur; post darkens only the diffuse ambient (location 2)
  by the scalar AO factor, so direct + emissive light are never darkened.
- IBL is baked offline. At runtime the env / irradiance / prefilter cubes (BC6H,
  `Bc6hRgbUfloat`) + the shared BRDF LUT (`Rg16Float`) are a pure upload via
  `IblResources::from_baked` — no startup precompute. The device hard-requests
  `TEXTURE_COMPRESSION_BC`.
- Derived debug buffers are built on demand and freed when inactive
  (`SceneResources::sync_line_views`). The steady-state shaded view holds zero
  derived buffers.
- The app redraws on demand. Continuous redraw is paced to the monitor refresh
  only while camera animation, pointer interaction, egui repaint, or startup
  warmup needs it.

## Startup Performance

Startup was cut from ~1150ms to ~350ms (resumed → first frame submitted, release,
RTX 4080 / DX12). The remaining budget is dominated by one intrinsic cost, below.
Startup is instrumented with **Tracy** zones: `resumed` opens a sequence of named
phase zones ("Window Create" → "Renderer Init" → "Set Window (adapter/device/
surface)" → "Shell Init" → "Initial Model Load" → "First Frame"), and
`SceneResources::new_core` opens the first-frame GPU-build sub-zones ("Shader
Module" / "IBL Upload" / "Line Pipeline" / "Post Pass"). Tracy is compiled into
every build (incl. release) but is runtime-gated: it only runs when launched with
`--tracy`, so a default launch starts no client, opens no socket, and pays ~one
atomic load per zone/alloc. Re-measure by running a Tracy server, then
`cargo run --release -p review-app -- --tracy <model.fbx>`. See `Tracy profiling.md`
for the full CPU/GPU/memory instrumentation.

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
- The renderer is a clean, fully linear-HDR offscreen seam. Bloom, GTAO, FXAA,
  and tone mapping compose in post without drawing the scene directly into egui's
  framebuffer.
- Capability gating is present for MSAA levels, IBL, and AO, which keeps
  unsupported adapter features from crashing the UI path.
- Resource lifetimes are thoughtful. Steady-state frames do not rebuild targets,
  pipelines, IBL maps, mesh buffers, the material table, or derived debug buffers
  unless an input actually changes.
- The importer has a clear count-then-fill structure, null checks before slices,
  and a single funnel from FBX to `ModelData`.
- The material/texture pipeline reuses one editable table, a path-keyed shared
  GPU texture cache, and a single set of `MaterialEdit` intents rather than
  per-slot ad-hoc state.
- The camera code handles framing, safe areas for chrome, near/far precision, UV
  pan/zoom, and animation with more care than a typical MVP.
- The UI runs on egui's native windowing with shared widgets and semantic theme
  tokens instead of scattered visual literals or a hand-rolled layout system.
- Packaging is not an afterthought: product metadata, exe icon resources, file
  association registration, freshness-gated IBL bake, and installer generation are
  all represented.
- A real test suite now exists: model / import / render unit tests plus the naga
  shader-validation check, run headless in CI.

## What Can Be Better

- Write the public `README.md`. It is still a one-line stub; it needs purpose,
  prerequisites, build/run commands, controls, supported formats, and packaging.
- Surface import errors and model warnings in the UI. Today failed loads emit a
  toast + a Tracy message (`prof::msg`) but warnings aren't shown in-app.
  `ModelWarning` exists in the data model, but warning generation/display is not
  active yet.
- Finish opacity polish (TODO Phase 7). The scene pass is single-forward
  alpha-blended MRT, so translucent materials can blend out of order; the planned
  fix is opaque-first then translucent back-to-front by per-range centroid.
- Decide what `LoadOptions::triangulate` means. It is passed through Rust/C, but
  the C bridge currently always triangulates render indices and ignores the value.
- Add GPU-buffer visualization and additional debug views (overdraw, UV
  stretching) — listed in `TODO.md` as post-MVP.
- Add validation and diagnostics for large scenes. High MSAA plus three
  `Rgba16Float` color targets can consume a lot of VRAM, and CPU-derived overlays
  can be expensive on dense meshes.
- Consider an explicit render graph once more effects arrive. `targets.rs` is a
  good seed, but pass order and resource dependencies are still hand-wired in the
  `scene/` module (`scene/callback.rs` orchestration + `scene/resources.rs`).
- Extend the test suite toward camera framing, UV island logic, and
  fixture-based importer loading.

## Current Risk Register

| Risk | Why it matters | Suggested response |
| --- | --- | --- |
| Minimal public docs | New contributors/users cannot build or understand the app from `README.md`. | Expand README and link these audit docs. |
| Import errors are invisible | A bad file appears to do nothing unless logs are visible. | Add status/toast/modal error reporting. |
| Out-of-order translucency | Single-forward alpha-blended MRT blends overlapping translucent materials wrong. | Land TODO Phase 7 opaque-first + back-to-front-by-centroid ordering. |
| Texture viewer source-format coverage | The Tex viewer decodes PNG/JPG/TGA/etc. but intentionally not compressed (DDS/KTX2) cooked textures. | Out of scope by decision (artists test source assets); revisit only if requirements change. |
| GPU memory pressure at high MSAA | Three HDR MRTs plus the GTAO G-buffer, resolves, and depth scale quickly. | Show memory-aware limits or defaults, and profile common resolutions. |
| Hand-wired pass order | Adding effects keeps growing the `scene/` orchestration by hand. | Introduce an explicit render graph if effect count grows further. |
| Helper nodes from inherit-mode handling | Files using 3ds Max-style segment-scale compensation gain ufbx scale-helper nodes in the Outliner, and their node indices shift versus a `PRESERVE` load. | Accepted for exact pose recomposition; surface helpers with their own kind if users find them confusing. |
| Opt shows the bind pose | A skinned model looks different in Opt than in 3D (rest pose) because processed meshes carry no skin. | Carry skin/morph data through `optimize` in a follow-up. |

## Recommended Next Steps

1. Write the public `README.md`: purpose, prerequisites, build/run commands,
   controls, supported formats, packaging.
2. Add in-app load failure and warning display.
3. Land TODO Phase 7 opacity ordering to close the materials/textures milestone.
4. Add the next debug views from `TODO.md` (GPU-buffer visualization, overdraw,
   UV stretching).
5. Extend tests toward camera math, UV island logic, and importer fixtures.
6. Introduce lightweight renderer diagnostics: adapter name, active target size,
   MSAA, pass toggles, and perhaps GPU timing behind a debug flag.
</content>
</invoke>
