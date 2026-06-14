# CLAUDE.md — 3D Review (Rust)

A Windows-first **native** 3D model-audit viewer (think F3D / Autodesk FBX
Review). Drag-drop an **FBX**, inspect game assets, switch through debug views.
Built on `winit` (window/event loop) + `wgpu` (GPU) + `egui` (overlay UI) +
vendored `ufbx` (FBX parsing via a C bridge). Pure-Rust, no web/Electron layer.
**Target priority: Windows.** This is the native alternate to the Three.js +
Electron viewer in `D:\Dev\3d-review` (see that repo's `CLAUDE.md`).

Deeper docs: `PROJECT_STRUCTURE.md` (crate map, data flow, ownership),
`MVP_PLAN.md` (goals + acceptance), `TODO.md` (running notes).

## 1. Invariants — the rules an agent will break if not told

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
   *(Gap: today the line buffers are rebuilt only on model change and kept
   regardless of toggle — converge toward this rule, don't extend the gap.)*
4. **Heavy derived views are GPU compute and capability-gated.** Compute-based
   views (normals/tangents/overdraw, post-MVP) read buffers already on the GPU
   and write transient storage buffers. Gate them on `wgpu` adapter features and
   disable them in the UI when the active adapter can't run them — never crash.
5. **Faithful stats.** The Model Stats panel reads source DCC counts carried
   through import in `ModelStats` (original polygon/vertex counts), **never**
   post-triangulation render counts. Every stat shown must be a real measured
   value. *(Gap: `stats_grid` currently shows placeholder `FPS=0`,
   `Colors=yes`, `WebGPU=yes` — these must be replaced with real values, not
   copied as a pattern. The renderer is native `wgpu` (DX12/Vulkan/Metal), not
   WebGPU; report the actual adapter backend.)*
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
   not `dark_grey_6`. *(Gap: pixel sizes are centralized but colors are repeated
   inline `Color32::from_rgb(...)` — add the token first, then reference it.)*

### Rust-specific invariants

9. **All `unsafe` and all C/FFI lives in `crates/import`.** No `unsafe` leaks
   into `model` / `render` / `ui` / `app`. Before `slice::from_raw_parts`,
   null-check the pointer and treat len 0 as empty (`checked_slice`). Free the
   C scene on **both** success and error paths (no leak).
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
            applies UiOutput back to Renderer. -> src/main.rs
  model/    review-model: host-agnostic data only (glam dep only).
            Vertex, Bounds, MaterialInfo, TopologyFace, ModelStats, ModelData,
            recompute_bounds, demo_cube_model. -> src/lib.rs
  import/   review-import: load_model/load_fbx, ImportError, the unsafe FFI
            (repr(C) mirror structs, checked_slice, model_from_bridge_scene),
            the vendored ufbx C + bridge, build.rs (cc, cfg(has_ufbx)).
            -> src/lib.rs, src/ufbx_bridge.c/.h, build.rs
  render/   review-render: ShadingMode, CameraProjection, SceneDebugOptions,
            RendererConfig, OrbitCamera (framing/orbit/pan/zoom/ortho+persp),
            CameraTransition (0.3s ease-in-out cubic), Renderer, SceneCallback +
            inline WGSL shader + GPU buffer builders. -> src/lib.rs, src/scene.rs
  ui/       review-ui: egui toolbar, option panels, axis gizmo, stats overlay,
            status bar, icon loading; UiOutput intents. -> src/lib.rs
third_party/ufbx/   vendored ufbx.c / ufbx.h (compiled only if present)
assets/icons/       PNG toolbar/gizmo icons (include_bytes!)
assets/test_models/ local FBX fixtures for manual checks
```

Data flow: input/file-drop → `app` → `import` (FBX→`ModelData`) → `model`
(shared) → `render` (camera + GPU buffers) → `ui` (overlays + scene callback) →
`app` (applies UI intents, requests redraw). See `PROJECT_STRUCTURE.md`.

## 3. Build & run

Use the **x64 Native Tools Command Prompt for VS 2022** (MSVC `cl` must be on
PATH so `cc` can compile `ufbx.c`).

- `cargo run -p review-app` — launch the viewer.
- `cargo check --workspace` — fast type check.
- `cargo clippy --workspace --all-targets -- -D warnings` — lint; **run before
  claiming done.**
- `cargo fmt --all` — format.
- `cargo test --workspace` — tests (currently none; add alongside changes).
- `cargo build --release` — release binary.

Pinned (workspace deps): `winit 0.30`, `wgpu 24`, `egui`/`egui-winit`/
`egui-wgpu 0.31`, `glam 0.30`, `bytemuck 1`, `thiserror 2`, `rfd 0.15`,
`image 0.25` (png), `pollster 0.4`, `cc 1` (build dep). Edition 2021.

> **MSRV note:** `Cargo.toml` declares `rust-version = "1.80"`, but the FFI uses
> `unsafe extern "C" { … }`, stabilized in **1.82**. Treat 1.82 as the real
> floor (bump the manifest or drop the `unsafe extern` form).

`build.rs` compiles `ufbx.c` + the bridge with `cc` **only when**
`third_party/ufbx/ufbx.{c,h}` exist, and sets `cfg(has_ufbx)`; without them the
workspace still builds and FBX import returns a clear error.

## 4. Locked decisions — DO NOT RE-LITIGATE

- Native stack: direct `winit` + `wgpu` (not `eframe`); `egui` is an overlay
  drawn via an `egui_wgpu` paint callback. Prefer DX12 on Windows
  (DX12|Vulkan|Metal requested).
- Internal format is the own `ModelData` superset (flat parallel buffers +
  original face topology + source stats). FBX is the only MVP import format,
  parsed by vendored `ufbx` through a single C bridge — don't round-trip through
  glTF (drops quad topology, changes vertex counts).
- Shading is one inline WGSL shader covering shaded / unlit / wireframe /
  uv-checker paths. `ui` MVP only needs `3D` mode; `UV` / `Tex` are placeholders.
- Crate boundaries are load-bearing (invariants 2, 9, 10) — keep them.

## 5. Current state

MVP: native window + wgpu viewport + egui chrome; FBX import via ufbx; orbit/
pan/zoom + frame-on-`F`; grid with axes; shaded / unlit / wireframe + face- and
vertex-normal debug overlays; orthographic/perspective toggle; animated axis
gizmo (orbit + snap-to-axis). Drag an FBX in, or double-click the empty viewport
to open the file picker.

Known gaps (see invariants 3, 5, 8 and MSRV note): debug line buffers aren't yet
freed on view-off and the normal length/color sliders don't update live (the
line geometry is baked only on model load); stats panel has placeholders; UI
colors are inline rather than tokenized; no tests yet though `cargo test` is an
acceptance criterion. UV / Texture panes, texture loading/KTX2, IBL/tone
mapping, and additional formats (glTF/OBJ) are post-MVP (`TODO.md`).

## 6. Gotchas

- Real GPU required for render checks; reserve CI for `check` / `clippy` /
  import unit tests.
- GPU struct field order must match the WGSL `Uniforms`/vertex layouts
  (invariant 11) — update the shader in `scene.rs` if you change them.
- `SceneCallback` reads some options live in `paint()` (grid, shading,
  uv-checker) but bakes normal-line geometry in `update_model()` — keep "live vs
  baked" explicit when adding options so sliders actually take effect.
- Keep `model` + camera/debug math host-agnostic so a future renderer swap only
  touches `render`.
