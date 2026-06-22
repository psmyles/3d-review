# Materials & Textures — Outliner, Inspector, Material Table & Texture Slots

## Context

3D Review is a Windows-first native FBX viewer/auditor for game developers. Its
rendering quality is strong, but the **materials layer is essentially absent**:
import flattens every FBX node into one buffer, per-face material assignment is
read then discarded, and material base color + smoothness are *baked into each
vertex* and drawn in a **single draw call**. There is no material table, no
texture support, no selection, and no way to inspect or edit anything per-mesh or
per-material.

This plan builds the foundation of the Materials & Textures system: a
**scene-graph Outliner** + **material/texture Inspector**, backed by a **real
material table with per-material draws** and **7 texture slots** that accept
**source image formats** (PSD/TGA/TIFF/PNG/JPG). It is the prerequisite for every
later materials feature (per-channel views, mesh-audit overlays, etc.).

### Confirmed product decisions
- **Outliner = full scene-graph hierarchy** (nested node tree, collapsible),
  listing meshes *and* materials; selecting drives the Inspector.
- **Selection = highlight/outline + a solo (isolate) toggle.**
- **Edits are live-preview only** this round (discarded on reload; no export).
- **Texture sourcing = all three:** manual (browse/drag-drop), auto-resolve FBX
  *referenced* files on disk, and *embedded* textures extracted via ufbx.
- **Slots = Base Color, Normal, Roughness, Metallic, AO, Emissive, Opacity**
  (Opacity adds alpha-blend/alpha-clip). Inspector also edits scalar/color params.
  The shader must activate metallic + normal mapping (both currently off).
- **Packed textures supported:** one image can feed several properties via
  per-property **channel routing** (e.g. an ORM map → AO=R, Roughness=G,
  Metallic=B). A loaded texture is decoded/uploaded **once** (deduped by path) and
  referenced by multiple property bindings; channels are **auto-detected from the
  filename suffix** (`_ORM`/`_RMA`/`_MRAO`/…) with manual override.
- **Formats = PSD, TGA, TIFF, PNG, JPG** (source, not DDS/KTX). **Textures
  auto-reload when the source file changes on disk.**
- **Decoding uses prebuilt systems only — never a custom decoder:** the Rust
  `image` crate for what it covers, and a **bundled ImageMagick `magick.exe` CLI**
  (shipped in the installer) for the rest (PSD, multi-layer TIFF, …).
- **Also finish the `Tex` viewport tab** (full-screen viewer, R/G/B/A isolation,
  format/resolution/mip readout).

### Two load-bearing architectural decisions
1. **Editable material state lives renderer-side, not in `ModelData`.** Geometry
   stays `model`-owned and host-agnostic (invariants 1, 10). The renderer owns an
   editable `MaterialTable` seeded from immutable import descriptors; the UI shows
   an app→UI **snapshot** and emits **edit/assign intents** via an expanded
   `UiOutput` that `app` applies to the `Renderer` (invariant 2 — same flow as
   environment/bloom settings).
2. **Material-segmented draws, not a per-vertex `material_id`.** Textures must be
   bound per draw regardless (no bindless baseline), so we group triangles by
   material and draw one range per material, feeding color/metallic/roughness/
   emissive from a per-material uniform + bind group (**group 3**, modeled on
   IBL's group 2). This lets us **drop baked `color`/`smoothness` from
   `SceneVertex`**, and makes highlight/solo trivial (same index ranges). We
   **reorder indices** (not vertices) into the render buffer, so invariant 1 holds
   — `ModelData::indices`/`faces`/`tri_to_face` stay authoritative for topology,
   UV, and wireframe builders.

## Patterns to reuse (don't reinvent)
- **Texture create/upload/sampler/bind-group + 1×1 fallback:** `IblResources`
  ([ibl.rs:111](crates/render/src/ibl.rs#L111)) and `create_checker_bind_group`
  ([scene.rs:1833](crates/render/src/scene.rs#L1833)).
- **Import FFI discipline (two-pass count→fill, `checked_slice`, free on every
  error path):** [ufbx_bridge.c](crates/import/src/ufbx_bridge.c) + the repr(C)
  mirrors in [import/src/lib.rs](crates/import/src/lib.rs).
- **Panel recipe** (`body(ui, &mut UiState)` → register in
  [panels/mod.rs:30](crates/ui/src/panels/mod.rs#L30) → `open_panel` from
  toolbar/status-bar right-click), all visuals from
  [theme.rs](crates/ui/src/theme.rs) (invariant 8).
- **Build-on-change / free-on-off** cadence: `sync_line_views` /
  `sync_environment` in [scene.rs](crates/render/src/scene.rs) (invariant 3).
- **File dialog:** `rfd` (already used at [main.rs:714](crates/app/src/main.rs#L714)).
- **Mesh build / single draw:** `model_mesh`
  ([geometry.rs:348](crates/render/src/geometry.rs#L348)), `record_scene`
  ([scene.rs:410](crates/render/src/scene.rs#L410)).

---

## Phase 0 — FFI: node tree + per-triangle material index

Carry hierarchy + per-triangle material into `ModelData`; **render path
untouched, app runs identically.**

- **[ufbx_bridge.c](crates/import/src/ufbx_bridge.c)/.h** (invariants 7, 9): add
  `review_import_node { name; parent; mesh_part_index; transform[16] }` array and a
  `tri_material` array parallel to `tri_to_face` — write the already-computed
  `material_slot` (currently discarded ~`:565`) into it. Populate `nodes` from
  `scene->nodes` (name/parent/`node_to_world`). Extend `review_import_free_scene`
  to free both. Transform is *display metadata only* (geometry stays world-baked).
- **[import/src/lib.rs](crates/import/src/lib.rs):** mirror `ReviewImportNode` +
  new scene fields (exact order), marshal via `checked_slice`.
- **[model/src/lib.rs](crates/model/src/lib.rs)** (glam only, invariant 10):
  `SceneNode { name, parent: Option<usize>, mesh_part: Option<usize>, transform:
  Mat4 }`; `ModelData.nodes`, `ModelData.tri_material` (len == triangle count);
  extend `MaterialInfo` with import defaults (`base_color`, `smoothness`,
  `metallic`, `emissive`) to seed the editable table. Update `demo_cube_model`.
- **✋ User-verifiable checkpoint (gate to Phase 1):** the user launches the app,
  loads an FBX, and confirms it renders **exactly as before** — this phase is pure
  plumbing, so *nothing should look different*. Backing automated proof: a passing
  import unit test asserting `nodes` is non-empty and
  `tri_material.len() == triangle_count`, plus `cargo clippy` clean. **Do not start
  Phase 1 until the user confirms "no visual regression."**

## Phase 1 — Material table + per-material draws + scalar editing

First user-visible feature; must work before textures.

- **Index ordering:** `model_mesh` also returns `Vec<MaterialDrawRange { material,
  first_index, index_count }>`, grouping triangles by `tri_material` and emitting
  indices in material order (vertices untouched). Single-range fallback when
  empty.
- **New [render/src/material.rs](crates/render/src/material.rs):**
  `#[repr(C)] MaterialUniform { base_color:[f32;4], emissive:[f32;4],
  params:[f32;4] /* metallic, roughness, slot_flags, highlight */ }` (Pod/Zeroable,
  invariant 11). `MaterialTable` = editable CPU vec + dynamic-offset uniform buffer
  + per-material bind group, seeded from `ModelData::materials`; `set_param`
  mutates + reuploads. `material_layout(device)` modeled on `ibl.rs:111` (Phase 1
  = uniform only; extended with textures in Phase 3).
- **[scene.rs](crates/render/src/scene.rs):** add `material_layout` to the shared
  `pipeline_layout` (`:696`) → `[uniform, checker, ibl, material]`; **bind group 3
  on every draw** in `record_scene` *and* `encode_ssao_gbuffer` (bind material-0/
  fallback for line/grid/skybox/UV draws). Store `MaterialTable` + ranges in
  `SceneResources`, rebuild in `update_model` (`:1056`). Replace the single mesh
  `draw_indexed` with a loop over ranges. Add a `material_revision` to
  `SceneCallback` (separate from `model_revision`) so live edits re-upload without
  a full mesh rebuild.
- **[scene.wgsl](crates/render/src/scene.wgsl)** (invariant 11, atomic edit):
  add `MaterialUniform` at `@group(3) @binding(0)`; **remove `color` (loc 3) and
  `smoothness` (loc 5)** from `VertexInput` + `SceneVertex` (+ `ATTRIBUTES` + both
  buffer placeholders + `model_mesh`), renumber `vertex_color`; source base color/
  metallic/roughness/emissive from the uniform; pass real `metallic` into
  `shade_ibl` (drop hardcoded 0). **`cargo check` immediately after.**
- **Import:** stop baking `color`/`smoothness` into vertices; values now live on
  `MaterialInfo` (Phase 0).
- **UI plumbing:** `UiOutput.material_edit: Option<MaterialEdit>` (BaseColor/
  Metallic/Roughness/Emissive); `UiState.materials_snapshot`. `apply_ui_output`
  (`:891`) → `renderer.set_material_param`, bump `material_revision`, redraw; thread
  `material_revision` through `draw_viewport_scene`.
- **✋ User-verifiable checkpoint (gate to Phase 2):** the user loads a
  multi-material FBX and **sees each material rendered with its own base color**
  (no longer one flat baked look), and dragging a temporary metallic/roughness
  control **visibly changes the surface in real time**. The Draws stat reflects N
  material ranges. `cargo clippy` clean. **Do not start Phase 2 until the user
  confirms per-material color + live scalar editing work.**

## Phase 2 — Outliner + Inspector + selection (highlight + solo)

- **Selection state (UI):** `Selection { Node(usize) | Material(usize) | None }`
  (first selection in the app) + `solo: bool`; carry selection + solo into
  `SceneCallback`.
- **Per-node ranges (render):** material ranges exist from Phase 1; add per-node
  ranges (group triangles by owning node via `tri_to_face`→face→node, or a parallel
  `tri_node` from Phase 0).
- **Render path (simplest correct):** *solo* = draw only selected ranges (draw-list
  filter, no new pipeline); *highlight* = reuse `line_pipeline` to draw selected
  ranges' edges in a `theme::color::selection` token, built-on-select / freed-on-
  deselect (invariant 3, `sync_line_views` pattern).
- **Panels — independent windows (NOT the single-`active_panel` slot):** the
  Outliner and Inspector reuse the existing draggable/collapsible window *chrome*
  (`draw_option_panel`/`option_panel` look — header, collapse-into-header, move
  around the viewport) but each has its **own** `open: bool`, `pos`, and
  `collapsed` state, drawn outside the `active_panel` dispatch. They can be open
  **simultaneously with each other and with any option panel** — i.e. they don't
  go through `open_panel`'s "only one at a time" rule. Two dedicated toolbar
  buttons toggle each window's `open` flag.
  - `panels/outliner.rs`: **two tabs at the top — `Scene` and `Materials`** (a
    simple `OutlinerTab` enum in `UiState`, styled with theme tokens; no new
    colors). **Scene tab** = the collapsible node hierarchy (nested rows from the
    `SceneNode` parent links, each leaf mesh node expandable to its mesh *parts*),
    a small per-row type glyph (node ⬢ / part ⬡). **Materials tab** = a flat,
    **deduplicated** material list (`◆ name`, count in the header). Both tabs read
    from the app→UI snapshot; clicking any row sets `Selection` (Node/part →
    `Node`, material row → `Material`) which drives the Inspector + viewport
    highlight. One window, one structure visible at a time; switching tabs is a
    cheap click. A filter box + the solo toggle live in the window header above the
    tabs.
  - `panels/inspector.rs`: material → color/metallic/roughness/emissive editors
    emitting Phase-1 intents; node/mesh → read-only stats; texture slots stubbed.
  - `state.rs`: add `outliner_window` / `inspector_window` window-state structs
    (open/pos/collapsed); factor the existing panel-chrome rendering so both the
    single option panel and these two independent windows share it. Add the
    `selection` theme token first (invariant 8).
- **✋ User-verifiable checkpoint (gate to Phase 3):** the user opens the Outliner
  and Inspector from their two toolbar buttons, **drags/collapses each window
  independently and keeps both open alongside an option panel**; switches between
  the Outliner's **Scene** and **Materials** tabs, and clicking a
  node/part/material row **highlights it in the viewport and populates the
  Inspector**; the solo toggle **isolates the selection**; deselecting clears the
  outline. The Inspector's scalar editors drive the Phase-1 live edits. `cargo
  clippy` clean. **Do not start Phase 3 until the user confirms selection,
  highlight, solo, and the independent windows behave as intended.**

## Phase 3 — Texture slots (7) + packed-channel routing + manual assignment + disk auto-reload

- **Decode (render — prebuilt systems only, no custom decoder):**
  [Cargo.toml](Cargo.toml#L26) `image` features → add `tga`,`tiff`,`jpeg`,`pnm`.
  New [render/src/texture.rs](crates/render/src/texture.rs): `decode_image(path)`
  dispatches on extension — formats `image` supports go straight through; the rest
  (PSD, multi-layer TIFF) shell out to the **bundled `magick.exe`** streaming
  **uncompressed PAM** to stdout (`magick <in> pam:-`), decoded by `image`'s `pnm`
  feature. PAM is chosen over PNG/MIFF on purpose: PNG's deflate pass is slow on
  large (4K) images and MIFF has no Rust decoder, whereas PAM is uncompressed +
  lossless (near-instant write) yet still read by a prebuilt crate. Fallback if PAM
  alpha disappoints: raw `RGBA:-` + a cheap `magick identify` for dimensions. Locate
  the bundled binary next to the exe; this is *not* FFI (no `unsafe`). Reuse the
  `create_checker_bind_group` upload pattern. `Rgba8UnormSrgb` for color/emissive;
  **`Rgba8Unorm` (linear)** for normal/roughness/metallic/AO/opacity. Decode on the
  app thread on assignment (one-shot, user-initiated) — **never** in the render
  `prepare` callback; worker-thread is a noted TODO for large PSD/TIFF.
- **Loaded-texture cache + packed-channel routing:** `MaterialTable` keeps a
  **texture cache keyed by absolute path** so a packed map is decoded/uploaded
  once and shared. Each of the 7 material properties holds a **binding**
  `{ texture: Option<path>, channel: ChannelSelect (R/G/B/A/RGB/RGBA) }`. On
  assignment, **auto-detect channels from the filename suffix** (`_ORM`→AO=R,
  Roughness=G, Metallic=B; `_RMA`, `_MRAO`, etc.) with manual override in the
  Inspector. The GPU model stays simple: still 7 texture bindings per material, but
  the *same* cached view may be bound to several slots, and per-property channel
  selectors live in the uniform — so the shader just samples its slot then swizzles.
- **Group 3 (render):** `material_layout` adds 7 `texture_2d<f32>` + 1 sampler;
  each material's bind group binds its (possibly shared) views or fallbacks. Extend
  `MaterialUniform.params` with a `slot_flags` bitfield (which slots are bound) and
  **per-property `channel_select`** fields. Per-slot 1×1 fallback (white base/AO/
  opacity, `[128,128,255]` normal, mid-grey roughness/metallic, black emissive).
  Reassigning a slot rebuilds only that material's bind group.
- **[scene.wgsl](crates/render/src/scene.wgsl)** (invariant 11): add the 7
  textures + sampler at group 3; for each scalar property sample its slot and
  **select the configured channel**, then multiply the scalar param; color/normal/
  emissive use RGB. **Normal mapping:** forward `tangent` (already on `Vertex`, not
  yet in `SceneVertex`) → build TBN → perturb normal. **Opacity:** sample →
  `out_alpha` (MRT already alpha-blends) + a material-flag-gated `discard`
  alpha-clip.
- **Disk auto-reload:** add the `notify` crate; the texture cache registers each
  loaded path with a watcher (debounced). On change, the watcher posts an event via
  a winit `EventLoopProxy` (redraw stays in `app`, invariant 6); `app` re-decodes
  that one cached entry, re-uploads, bumps `material_revision`. Referenced paths
  (Phase 4) register with the same watcher; embedded textures (Phase 5) are not
  watched.
- **Manual assignment UI:** Inspector per-slot row = thumbnail + Browse (rfd image
  filter) + Clear + drag-drop + a channel dropdown (pre-filled by auto-detect).
  `UiOutput.texture_assign{material,slot,path,channel}` / `texture_clear` →
  `apply_ui_output` decodes + uploads via `MaterialTable::set_slot`, rebuilds that
  bind group, bumps `material_revision`.
- **✋ User-verifiable checkpoint (gate to Phase 4):** the user, via the Inspector
  slots, **assigns a PNG/TGA/PSD base color** (appears on the model), **a normal
  map** (visible surface relief), and **an opacity map** (translucency + cutout);
  **assigns one `_ORM` packed map** and confirms AO/Roughness/Metallic each read the
  correct R/G/B channel (with the dropdown override working); **edits a texture file
  in an external editor and saves**, confirming the viewport **auto-updates**; and
  **Clear** reverts a slot to its fallback. `cargo clippy` clean. **Do not start
  Phase 4 until the user confirms manual assignment, packed-channel routing, and
  disk auto-reload all work.**

## Phase 4 — Auto-resolve FBX-referenced textures

- **[ufbx_bridge.c](crates/import/src/ufbx_bridge.c) (FFI):** marshal per-slot
  texture filenames (`material->pbr.*.texture->filename`/`relative_filename`) into
  extended `review_import_material`.
- **import/lib.rs + `MaterialInfo`:** `referenced: [Option<String>; 7]` (plain
  data).
- **app on load:** resolve (absolute → relative to FBX dir → `textures/` sibling),
  decode via Phase-3 `texture.rs`, auto-populate slots. Missing → fallback +
  `ModelWarning`.
- **✋ User-verifiable checkpoint (gate to Phase 5):** the user loads an FBX whose
  texture files sit beside it (or in a `textures/` sibling) and **the maps appear
  automatically with no manual assignment**; loading an FBX with a missing
  referenced map shows the **fallback + a non-fatal warning** (no crash). `cargo
  clippy` clean. **Do not start Phase 5 until the user confirms referenced textures
  auto-resolve and missing ones degrade gracefully.**

## Phase 5 — Extract embedded textures (FFI → import only)

- **[ufbx_bridge.c](crates/import/src/ufbx_bridge.c):** when
  `texture->content.size > 0`, copy the blob across (own malloc, free in
  `review_import_free_scene`).
- **import/lib.rs + `MaterialInfo`:** `embedded: [Option<Vec<u8>>; 7]` via
  `checked_slice` (owned bytes in `model` = plain data; only *extraction* is FFI,
  *decode* stays render/app).
- **app on load:** decode embedded bytes (`load_from_memory`), populate slots;
  define embedded-vs-referenced precedence.
- **Edge cases:** embedded DDS/KTX unsupported (warn/skip); large blobs may stall
  (async TODO); both-present precedence; thumbnail-only embeds.
- **✋ User-verifiable checkpoint (gate to Phase 6):** the user loads an FBX with
  **embedded** textures (no external files present) and the maps **appear on the
  model**; an FBX with both embedded and referenced maps respects the defined
  precedence. `cargo clippy` clean. **Do not start Phase 6 until the user confirms
  embedded textures load with no external files.**

## Phase 6 — Finish the `Tex` viewport tab

`WorkspaceMode::Texture` currently renders nothing
([overlay.rs:47](crates/ui/src/overlay.rs#L47)).
- **render:** `SceneCallback::new_tex(...)` (parallel to `new_uv`) drawing a
  fullscreen triangle sampling the selected material slot's existing GPU view, with
  a `channel_mask` uniform for R/G/B/A/all isolation (alpha shown greyscale).
- **UI:** replace the `Texture => return` stub; add selected-texture + channel-mask
  state; a Texture-mode toolbar to pick slot + toggle channels; an overlay showing
  resolution/format/mips (theme tokens).
- **✋ User-verifiable checkpoint (gate to Phase 7):** the user switches to the
  `Tex` tab and **sees the selected material slot's map full-screen**, toggles
  **R/G/B/A isolation** (alpha shown greyscale), and reads a correct
  **resolution/format/mips** overlay. `cargo clippy` clean. **Do not start Phase 7
  until the user confirms the Tex viewer + channel isolation + readout are
  correct.**

## Phase 7 — Opacity polish: alpha draw order (last, honestly approximate)

The scene pass is single forward + alpha-blended MRT, so translucent materials can
blend out of order.
- Split the per-material loop into **opaque first** (depth write on) then
  **translucent back-to-front** by per-range centroid (depth test on, write off);
  cutout stays opaque. One extra write-off pipeline; centroids computed once on
  load.
- **Risk:** not true OIT — interpenetrating translucency is wrong; acceptable for
  an auditor, flagged in code + UI. Done last so everything else ships without it.
- **✋ User-verifiable checkpoint (final acceptance):** the user loads a model with
  overlapping translucent materials and confirms they **blend in a sensible
  back-to-front order** (no obvious pop-through for non-interpenetrating geometry),
  with cutout materials still crisp. `cargo clippy` clean. This closes the
  Materials & Textures milestone.

---

## Verification

**Each phase ends with a ✋ user-verifiable checkpoint that gates the next phase** —
implementation pauses there for the user to confirm the described, hands-on result
before any work on the following phase begins. The checks below back those gates.

- `cargo check --workspace` → `cargo clippy --workspace --all-targets -- -D
  warnings` (before claiming done). Build from the **x64 Native Tools Command
  Prompt for VS 2022** so `cc` finds `cl` for `ufbx.c`.
- `cargo test --workspace` — add Phase-0 import marshaling tests + pure `model`
  helpers.
- **Manual GPU checks** (real GPU) against `assets/test_models`: single-material,
  multi-material, multi-node, referenced-maps-beside-it, embedded-maps. Per phase:
  N draws (1); Outliner + highlight + solo (2); every slot/format incl. normal +
  opacity (3); auto-load referenced (4) + embedded (5); Tex channel isolation +
  readout (6); translucent ordering (7).
- After each lockstep shader change (Phase 1, 3), `cargo check` immediately to
  catch `SceneVertex`/`VertexInput`/`MaterialUniform`/bind-group mismatches.

## Risks / cons (honest)

- **scene.wgsl + SceneVertex blast radius (Phase 1/3):** removing `color`/
  `smoothness`, adding `tangent`, group 3 + `MaterialUniform`, 7 textures — all
  force atomic edits across `scene.wgsl`, `SceneVertex`/`ATTRIBUTES`, both buffer
  placeholders, the pipeline layout, and every `set_bind_group` site
  (`record_scene`, `encode_ssao_gbuffer`). One mismatch = validation panic.
- **Index reordering (Phase 1):** confined to the render mesh buffer;
  `ModelData::indices`/`faces`/`tri_to_face` stay authoritative (invariant 1).
- **ImageMagick dependency:** bundling `magick.exe` adds installer size and a
  shell-out per non-native decode (temp/stdout PNG round-trip); the Inno Setup
  installer must ship the binary and the app must locate it relative to the exe
  (with a clear error if missing). Decode failures warn + fall back (mirror
  `create_checker_bind_group`).
- **Filename channel auto-detect:** suffix conventions vary by studio; auto-detect
  is a *guess* that the Inspector channel dropdown must always allow overriding.
- **File-watcher:** must debounce rapid editor saves and tolerate atomic-rename
  saves (watch the directory, re-resolve the path) to avoid missed/duplicate
  reloads.
- **Embedded edge cases (Phase 5):** DDS/KTX-in-FBX unsupported; large blobs stall
  (async TODO); precedence rules.
- **Hierarchy vs world-baked geometry:** node transforms are display metadata; the
  Outliner is informational, not a live transform editor (invariant 1).
- **Alpha sorting (Phase 7):** approximate; flagged; done last.
- **Decode cost:** large PSD/TIFF decode is synchronous on assignment v1 (brief
  stall possible); worker-thread path noted.

## Critical files

- [render/src/scene.rs](crates/render/src/scene.rs) — pipeline layout, per-material/
  per-node draw loop, `SceneVertex`, `SceneResources`, selection/solo,
  `material_revision`
- [render/src/scene.wgsl](crates/render/src/scene.wgsl) — `MaterialUniform`, group
  3, metallic, normal mapping, opacity, channel isolation (lockstep with
  `SceneVertex`)
- [import/src/ufbx_bridge.c](crates/import/src/ufbx_bridge.c) +
  [import/src/lib.rs](crates/import/src/lib.rs) — node tree, per-triangle material,
  referenced paths, embedded blobs (all FFI here)
- [model/src/lib.rs](crates/model/src/lib.rs) — `SceneNode`, `tri_material`,
  `MaterialInfo` extensions (host-agnostic)
- [ui/src/state.rs](crates/ui/src/state.rs) — `Selection`, expanded `UiOutput`,
  snapshots; new `ui/src/panels/outliner.rs` + `inspector.rs`; new
  `render/src/material.rs` (table, cache, channel routing) + `render/src/texture.rs`
  (decode dispatch incl. bundled `magick.exe`, `notify` watcher)
- [Cargo.toml](Cargo.toml) — `image` features (`tga`/`tiff`/`jpeg`/`pnm`) + `notify`
  dep;
  packaging (Inno Setup) bundles `magick.exe` beside the app exe
