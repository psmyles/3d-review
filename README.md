# 3D Review

A native, Windows-first 3D model **audit** viewer for game assets — think F3D or
Autodesk FBX Review, aimed at the questions a technical artist actually asks of a
mesh. Drop an FBX on the window and inspect its geometry, UVs, materials,
textures, skeleton and animation, then optimize it and write it back out.

Written in Rust on `winit` + native **Direct3D 11** (via the `windows` crate),
with `egui` as the overlay UI and vendored `ufbx` for FBX parsing. No web layer,
no Electron, no wgpu. A macOS port is planned (see `mac-port-plan.md`).

---

## Contents

- [Getting a model in](#getting-a-model-in)
- [Workspaces](#workspaces)
- [The 3D viewport](#the-3d-viewport)
- [Shading and review modes](#shading-and-review-modes)
- [Debug overlays](#debug-overlays)
- [Rendering quality](#rendering-quality)
- [Outliner and Inspector](#outliner-and-inspector)
- [Materials and textures](#materials-and-textures)
- [UV workspace](#uv-workspace)
- [Texture workspace](#texture-workspace)
- [Skinning and animation](#skinning-and-animation)
- [Model statistics](#model-statistics)
- [Opt: the optimization stack](#opt-the-optimization-stack)
- [Undo, presets and session state](#undo-presets-and-session-state)
- [Keyboard and mouse](#keyboard-and-mouse)
- [Building](#building)
- [Architecture](#architecture)

---

## Getting a model in

FBX is the only import format. It is parsed by vendored `ufbx` through a single C
bridge, so quad topology and the file's own vertex counts survive intact rather
than being flattened by a glTF round-trip.

Four ways in, all landing on the same loader:

- Drag and drop a file onto the window.
- `Ctrl+O` for a file dialog.
- Double-click the empty viewport.
- Pass a path on the command line, or open an `.fbx` through the Windows file
  association the installer registers.

Import runs on a worker thread with a progress toast. A newer request supersedes
an in-flight one, so opening two files in a row never shows the wrong result.
`Ctrl+N` returns the viewer to its launch state.

## Workspaces

Four modes, switched from the segmented control in the middle of the toolbar. The
toolbar swaps its tool groups to match the active workspace.

| Workspace | What it is |
| --- | --- |
| **3D** | The scene viewport: shading modes, debug overlays, materials, animation. |
| **UV** | A 2D UV-layout viewer with its own pan/zoom camera. |
| **Tex** | A 2D image viewer over the scene texture pool. |
| **Opt** | Mesh optimization with side-by-side or ghosted comparison against the source. |

## The 3D viewport

Orbit camera with framing, panning and zooming, in perspective or orthographic
projection. `F` frames the model, or the selection when one is active; pressing it
again alternates back to the whole model. `R` returns to the home framing. `WASD`
orbit in animated 45° steps. Camera moves ease over 0.3 s rather than snapping.

Display toggles that live in the toolbar's right-hand group: the floor grid with
axes, the animated axis gizmo (click an axis to snap the camera to it), the object
pivot marker, the bounding box, and the skeleton overlay for rigged models.

The viewport background is a status-bar button: black, 25/50/75% grey, white, or a
gradient.

## Shading and review modes

**Shading mode** is a three-way radio — exactly one is active:

- **Wireframe only** — edges alone.
- **Unlit** — flat surface color, no lighting.
- **Shaded** — environment-lit PBR, the default.

Two independent toggles bookend it and combine with any of the three:

- **Show Wireframe** — the edge overlay drawn on top of the filled surface. It is
  a real depth-tested line-list draw in the scene pass, so edges on hidden faces
  are correctly occluded and the scene's MSAA antialiases them. Edge color is
  configurable.
- **Backface Rendering** — off culls back faces (the default, which is how you
  find inverted normals); on draws the mesh double-sided.

**Active material** is a second radio, choosing what the filled faces show. It
applies in every shading mode.

| Mode | What it shows |
| --- | --- |
| **Source Material** | The imported materials plus any Inspector edits. Re-clicking the button cycles **Source → Standard → Unique**: a uniform matte mid-grey for every part, or a distinct randomized hue per mesh part so the pieces read apart. The replacement is renderer-side, so the imported materials are never touched. |
| **UV Checker** | A greyscale or color checker at an adjustable tiling density, on any UV channel of a multi-set model — the fast read on stretching, mirroring and texel density. |
| **Vertex Colors** | The mesh's vertex-color attribute as RGB, alpha-as-greyscale, or RGB with alpha driving opacity. |
| **Buffers** | One shading input at a time, drawn flat. See below. |
| **Skin Weights** | A blue-green-red heat map of how strongly the bones selected in the Outliner influence each vertex. Flat, unlit and untonemapped, so the displayed color *is* the weight. Offered only for a skinned model. |

### Buffer inspection

The **Buffers** view renders a single material or geometry buffer straight to the
screen, bypassing lighting and tone mapping so the pixel you see is the value
itself. Re-clicking the toolbar button cycles them; the options panel picks one
directly.

Base Color · Normal (World) · Normal Map (Tangent) · Geometric Normal · Tangent ·
Roughness (or Smoothness, following the material's workflow) · Metallic · Ambient
Occlusion · Emission · Opacity · UV

Both the final shading normal and the raw authored normal map are offered, next to
the geometric normal and the tangent basis, which is what lets a misbehaving normal
map be pinned down — handedness, green-channel convention, or missing tangents.
Color buffers are sRGB-encoded for display; the rest are written raw, so a 0.5
scalar reads as mid-grey.

## Debug overlays

Independent toggles, each with its own options panel. Every derived buffer is
built when its view turns on and freed when it turns off, so the steady-state
shaded view carries none of them.

- **Face normals** — one line per face, adjustable length and color.
- **Vertex normals** — one line per vertex, adjustable length and color.
- **Bounding box** — with live dimension labels that occlude correctly against the
  mesh. Scope is selectable: all meshes, only the selection, or only the visible
  meshes. Edge color is configurable.
- **Pivot marker** — the object's origin.
- **Skeleton** — octahedral bones with joint markers for a rigged model, with an
  adjustable size multiplier and bone color. The selected bone highlights in the
  viewport selection color.
- **Axis gizmo** and **grid**.

Every overlay derived from the mesh deforms with it, so overlays stay attached to
a skinned, animated model rather than sitting on the bind pose.

Right-clicking any tool button opens that tool's options panel. Panels are native,
collapsible, closable windows and several can be open at once.

## Rendering quality

The renderer is fully linear-HDR with reversed-Z depth, rendering into offscreen
multi-render-target buffers that a fullscreen composite pass resolves. The
status-bar group on the right controls it — left-click toggles or cycles,
right-click opens the options.

- **Image-based lighting** — six baked HDR environments, each with a preview
  thumbnail in the dropdown. Optional skybox background, an intensity multiplier,
  and a live 0–360° environment rotation that applies at sample time and never
  rebuilds the maps. The irradiance, prefilter and BRDF maps are baked offline and
  ship BC6H-compressed, so startup does no precompute.
- **Ambient Occlusion** — ground-truth-style horizon occlusion with a bilateral
  blur, over its own single-sample normal/depth G-buffer. Knobs are Radius,
  Intensity, Thickness and Quality (Low/Medium/High). It darkens only the
  AO-eligible diffuse ambient term, so direct and emissive light are never dimmed.
  On by default.
- **Tone mapping** — Khronos PBR Neutral, Linear, Reinhard, ACES or AgX, applied
  to the linear radiance before sRGB encoding. Toggling it off is a linear
  pass-through. On by default.
- **Anti-aliasing** — scene MSAA at 2×, 4×, 8× or 16×. Levels the active adapter
  cannot render are omitted from the menu rather than offered and failing.
- **Viewport background** — black, three greys, white, or a gradient.

## Outliner and Inspector

The **Outliner** is a dockable, resizable left panel with two or three tabs.

- **Scene** — the node hierarchy, indented and collapsible with parent guide
  lines, or a flat list. A search box filters both tabs and lists matches flat, so
  a hit is never buried in a folded branch. Node-kind filter glyphs narrow what is
  listed. Clicking a row selects it, which drives the viewport highlight (with a
  selection flash), the Inspector, and the selection-scoped bounding box and
  framing. Arrow keys walk the rows. Mesh rows carry a visibility eye;
  Ctrl-clicking the eye isolates that mesh instead of toggling it. On bone rows,
  Ctrl adds to the selection and Shift takes a range, which is what feeds the skin
  weight heat map.
- **Materials** — the deduplicated material list. Selecting one highlights its
  faces in the viewport and opens it in the Inspector.
- **Animations** — the clip list. Present only for a file that carries animation,
  and never in the Opt workspace.

The **Inspector** is the right panel. For a selected node it shows read-only
stats; for a multi-bone selection, a summary. For a material it shows three
collapsible sections, described next.

## Materials and textures

Materials are editable live. The Inspector's **Material** section carries the
shader type, transparency mode (Opaque / Blend / Clip), base color, roughness,
metallic and emissive. Roughness follows a per-material **workflow** setting:
native metallic-roughness, or Unity-style smoothness, where the slider and any
bound map read as smoothness and the shader inverts them.

**Texture mapping** binds a pooled image plus a channel to each of the seven PBR
slots: base color, normal, roughness, metallic, ambient occlusion, emissive and
opacity. Scalar slots pick a single channel, so an ORM-packed map feeds three
slots from one file. Channel routing is auto-detected from the filename on import
and can be overridden.

Textures live in a **scene-wide pool**, decoded once and shared. The **Texture
files** section lists the pool with thumbnails and a remove action. Source formats
are decoded natively with no ImageMagick dependency: PNG, JPEG, TGA, TIFF, PSD
(layered files, via a vendored psd_sdk bridge that reads the merged composite),
BMP, GIF, HDR and PNM. A bound texture is watched on disk and re-decoded when it
changes, so a save in Photoshop or Substance shows up in the viewport.

Engine-cooked compressed formats (KTX2, DDS) are deliberately not supported — this
tool is for reviewing *source* art.

## UV workspace

An independent 2D viewport for the UV layout, with its own pan, zoom and fit. The
UV edges draw in every mode; the shading radio picks what fills them.

- **Wire** — the layout alone.
- **Shaded** — solid islands.
- **Islands** — a unique color per island, so overlaps and stray shells stand out.

A UV-set picker in the toolbar switches channels on a multi-set model.

## Texture workspace

A 2D image viewer over the scene texture pool, drawn through its own minimal GPU
pipeline outside the scene's HDR and tone-mapping path — so the displayed texel
equals the stored texel.

- Texture picker for the pool.
- **RGB / R / G / B / A** channel isolation, a shader swizzle, so switching is
  instant. The alpha segment auto-hides for an opaque image.
- Mipmapped, with pan (left-drag), zoom (wheel or right-drag), and `F` to fit.
- Background fill: black, white, grey or checker.
- A stats panel reporting the file's real properties: format, pixel dimensions,
  channel layout, bit depth and on-disk size.

## Skinning and animation

A skinned mesh rests in the file's *default* pose, skinned onto the skeleton as
drawn, not the bind pose its buffers hold. Deformation is GPU linear-blend
skinning over each vertex's exact influence run, with weights normalized the way
ufbx does, alongside rigid node animation and blend shapes through the same vertex
shader stage.

Every FBX animation stack imports as a clip, baked at the file's own frame rate
with no key reduction. Frame counts come from the stack's time range, not from key
counts. Clicking a clip in the Outliner selects it, paused on its first frame, and
raises the playback transport at the bottom of the viewport: go to start, step
back, play/pause, step forward, a scrubber, a `frame / total   seconds` readout,
looping (on by default) and a speed control. `Space` plays and pauses; `,` and `.`
step single frames.

While a clip is selected, `F` and the bounding box use that clip's motion
envelope, measured once at import, so framing covers the whole animation rather
than one frame of it. Dual-quaternion skins are evaluated as linear blends and the
Inspector says so. Playback is view state and is never captured by undo.

## Model statistics

A toggleable panel of measured counts. Every figure is a real measured value —
there are no placeholder rows, and the source DCC counts carried through import
are reported rather than post-triangulation render counts.

Draws · Polys · Tris · Verts · GPU Verts · Vtx Splits · UV Sets · Bones · Clips ·
Unit · FPS

`Verts` is the file's own vertex count. `GPU Verts` is the engine cost: unique
vertices per draw group. The stats can be scoped to all meshes, the selection, or
only the visible meshes, and copied to the clipboard.

---

## Opt: the optimization stack

A fourth workspace that turns the viewer into a mesh optimizer, built on vendored
**meshoptimizer**. It shares every 3D control, so the processed mesh can be
inspected with the same shading modes, buffer views and overlays as the source.
Static meshes only.

Nothing Opt-specific is built until the workspace is first opened, and a stack
with nothing enabled schedules no run — a session that never opens Opt pays
nothing for it.

### How a run works

Every run begins with a **lossless index pass**, and this is what makes any of it
work. FBX import splits each face corner into its own vertex, so a mesh reaches
the optimizer with no shared vertices at all — a real 10,006-triangle asset
arrives as 30,018 vertices. Every meshoptimizer operation works through the index
buffer, so on that mesh they would all be no-ops. The index pass merges vertices
identical in every attribute, byte for byte, before any operation runs, and
nothing visible changes. On that asset it is 30,018 → 5,284, after which a 50% LOD
target is hit exactly.

The run's baseline is measured *after* that pass, so the figures credit your
operations rather than the free win any engine cooker also gets. That baseline
also equals the stats panel's `GPU Verts`.

### Operations

The stack is ordered and re-orderable; operations apply top to bottom, and each
can be enabled or disabled without deleting it.

| Operation | What it does |
| --- | --- |
| **Weld Vertices** | Widens what counts as a match beyond the exact-attribute merge every run already does — dropping normals, UVs or colors from the comparison, or allowing a per-component tolerance. This one *does* change the mesh. |
| **Filter Triangles** | Removes degenerate triangles (two corners at one position) and exact duplicates. Opposite-winding duplicates are kept, for double-sided geometry. |
| **Prune Components** | Removes disconnected pieces below an error threshold — stray shells and orphaned faces. |
| **Reduce** | Simplifies the mesh **in place**. The output is what every later operation sees, what LOD levels start from, and what the export writes in the source mesh's place. |
| **Generate LODs** | Fans out into a LOD chain. Operations above it run once on the base mesh; operations below it run on every generated level. At most one per stack. Default chain is three levels at 50%, 25% and 12.5%. |
| **Bake AO to Vertex Colors** | Raycast ambient occlusion baked into the vertex-color set. Changes no geometry. |
| **Optimize Vertex Cache** | Reorders triangles for the GPU's post-transform vertex cache. Watch ACMR and ATVR. |
| **Optimize Overdraw** | Reorders triangles front-to-back within cache-friendly clusters, so the GPU shades fewer hidden pixels. |
| **Optimize Vertex Fetch** | Reorders vertices into index-buffer read order and drops anything unreferenced. |

### Simplification settings

**Reduce** and **Generate LODs** share one simplifier configuration.

- **Algorithm** — *Standard* (topology-preserving, position error only), *Preserve
  Attributes* (additionally penalizes normal/UV/color drift, with per-attribute
  weights), or *Sloppy* (ignores topology; much faster and hits the target far
  more reliably, but can close holes and merge nearby shells — best for the
  smallest levels).
- **Target** — a triangle ratio plus an error limit the simplifier may not exceed.
  It stops short of the ratio rather than exceeding the error.
- **Flags** — lock border, absolute error units, prune disconnected parts,
  regularize (and a lighter variant), and permissive collapses across attribute
  seams.

> A simplify that barely removes anything is usually attribute seams, not a bug.
> A mesh whose normals or UVs differ at every corner presents every edge as a
> discontinuity, and a topology-preserving collapse cannot cross one. The run
> detects the stall and says so. A position-only weld or the permissive flag
> unblocks it.

### AO baking

Deterministic CPU cosine-hemisphere raycasts against a triangle BVH, parallel
across threads.

- **Quality** — 32, 64, 128 or 512 rays per vertex.
- **Max distance** — how far a surface can be and still occlude, in world meters;
  zero is unlimited whole-scene occlusion.
- **Intensity** — a power on visibility, matching the viewport AO panel.
- **Target** — alpha, RGB greyscale, multiplied into RGB, or a single R/G/B
  channel. Optional sRGB encoding for the RGB-family targets; alpha is always
  linear.

Two rules make it usable on real game FBXs. Objects excluded from the stack still
**occlude** but are not written, while Outliner-**hidden** nodes neither occlude
nor bake — hide the collision shell first. And occluders partition by the
`_LOD<n>` name suffix: game assets carry their whole LOD chain as co-located
siblings, and raycasting one LOD against another's near-coincident surfaces shreds
the result. Each LOD bakes only against its own group plus every suffix-less node,
so an entire visible chain bakes correctly in one run.

Adding the operation auto-switches the viewport to the matching vertex-color mode,
and the exporter already writes vertex colors, so nothing else is needed.

### Per-object overrides

Any object can be **excluded** from the stack, passing through untouched at full
detail in every output level — the way to keep a hero prop or a collision shell
out of the optimization. Individual operations can also be given replacement
settings on a per-object basis; operations not overridden use the global ones.

### Comparison

Every edit reprocesses on a worker thread. One run is ever in flight: edits
arriving mid-run mark it dirty and it respawns once with the latest stack, so
dragging a slider coalesces without a debounce timer, and a superseded result is
dropped rather than shown. A notice appears only if a run takes a while.

The status bar's centred group — sitting over the split's divider, because it
describes the viewport as a whole — carries the comparison controls.

- **Split** — source left, processed right, optionally camera-synced so both views
  move together.
- **Overlay** — both in one space, one mesh shaded and the other a ghost, either
  translucent x-ray or wireframe. `X` swaps which is solid, which is the most
  reliable way to spot where a simplification moved the silhouette. A legend names
  which is which.
- **LOD picker** — which level of the chain is displayed. Once you pick a level,
  later runs keep showing it.

A second stats card reports the processed mesh's measured counts alongside the
metrics meshoptimizer measured — **ACMR**, **ATVR**, **overdraw**, **overfetch**
and the largest simplification error — each with its change against the source
tinted green or red. Every figure there is one where lower is better. That card is
up as soon as the workspace opens: a run with nothing enabled produces no mesh but
still measures the source, so the baseline is readable before anything is added.
This is what makes the three reorder operations, invisible in the viewport, worth
having.

### Export

An explicit export writes the chain to FBX through vendored `ufbx_write`.

- **Packaging** — one file holding `MeshName_LOD0` … `MeshName_LODn` as sibling
  nodes (the naming convention most engines auto-detect), or one file per level.
- **Hierarchy** — rebuild the original node hierarchy so the export round-trips
  like the source asset, or emit flat world-baked meshes with identity transforms.
- **Format** — binary or ASCII FBX.

Output is always triangulated, carries the source materials (untextured) and
vertex colors, and declares a unit scale of 100, since import normalizes every
file to meters while FBX's conventional unit is centimeters.

## Undo, presets and session state

`Ctrl+Z` undoes and `Ctrl+Y` (or `Ctrl+Shift+Z`) redoes. Undo covers **document
edits**: Outliner selection, mesh visibility, material parameters, texture slot
bindings, the texture pool, and the Opt operation stack. A continuous slider or
color-picker drag coalesces into a single step. **View** state — camera, grid,
shading mode, AA, AO, tone mapping, environment, the UV and Tex viewports, and
animation playback — is deliberately not captured.

An Opt stack saves and loads as a versioned JSON **preset**, carrying its
operations, per-object overrides and export settings.

Window position and size are restored across sessions, validated against the
monitor geometry actually present at launch.

## Keyboard and mouse

A cheat-sheet of these is shown on launch and dismissed by any key or click.

**Mouse (3D)** — left-drag orbits, right-drag pans, middle-drag pans,
Alt+right-drag zooms, wheel zooms. Double-clicking the empty viewport opens a
file.

**Mouse (UV)** — left-drag pans, right-drag zooms, wheel zooms.

**Mouse (Tex)** — left-drag pans, right-drag zooms, wheel zooms.

| Key | Action |
| --- | --- |
| `` ` `` | Toggle the wireframe overlay |
| `1` / `2` / `3` | Wireframe only / Unlit / Shaded |
| `I` | Toggle the stats display |
| `G` | Toggle the grid |
| `F` | Frame the model, or the selection |
| `R` | Reset the camera (refits the image in UV and Tex) |
| `W` `A` `S` `D` | Orbit the camera in 45° steps |
| `Space` | Play / pause the selected clip |
| `,` / `.` | Previous / next frame of the selected clip |
| `X` | Swap source and processed in the Opt overlay |
| `Esc` | Clear the selection |
| `Ctrl+N` | Reset to the start state |
| `Ctrl+O` | Open a model |
| `Ctrl+Z` / `Ctrl+Y` / `Ctrl+Shift+Z` | Undo / redo / redo |

Right-click any toolbar or status-bar tool button to open its options panel.

## Building

Windows, from an **x64 Native Tools Command Prompt for VS 2022** — MSVC `cl` must
be on `PATH` so the vendored C can compile.

```
cargo run -p review-app            # launch the viewer
cargo check --workspace            # fast type check
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo test --workspace             # model / import / render / optimize tests
cargo build --release
```

Rust 1.88 or newer, edition 2024.

Three vendored C/C++ trees are compiled only if present, and the workspace builds
without any of them — the dependent features simply report themselves
unavailable:

- `third_party/ufbx` — FBX reading.
- `third_party/meshoptimizer` — the Opt operations.
- `third_party/ufbx-write` — the Opt FBX export.

HLSL shaders are compiled offline to committed DXBC blobs by the build script
using `fxc`, freshness-gated and with warnings as errors, so the runtime compiles
no shaders and a machine without `fxc` builds from the committed blobs. The baked
IBL maps and HDR thumbnails are likewise committed; the scripts in `packaging/`
regenerate them, and the installer build invokes them automatically.

`cargo run -p review-render --features bake --bin bake_ibl` re-bakes the IBL maps.
It needs a real GPU.

Optional Tracy profiling is compiled in but runtime-gated: pass `--tracy` to start
the client. A normal run opens no socket and pays essentially nothing.

## Architecture

Eight crates, with load-bearing boundaries.

| Crate | Role |
| --- | --- |
| `app` | Event loop, input routing, the D3D11 bootstrap, per-frame draw order, model loading, texture pool, animation clock, undo, the Opt worker. Fully safe code. |
| `model` | Host-agnostic mesh, scene, skinning, morph and animation data. Depends only on `glam`. |
| `import` | FBX loading through vendored ufbx and a single C bridge. |
| `render` | Cameras, configuration, materials, geometry generation, and the D3D11 GPU layer. All COM confined to one module. |
| `ui` | The egui chrome, built on egui's native windowing and stock widgets. Owns no GPU state and emits intents rather than mutating the renderer. |
| `optimize` | meshoptimizer operations, the operation stack, presets, and FBX export. |
| `psd` | Safe PSD composite decoding over a prebuilt psd_sdk bridge. |
| `prof` | Guarded Tracy helpers shared by every instrumented crate. |

Data flows one way: input and file drops reach `app`, which drives `import` into
`model`, which `render` turns into GPU resources; `app` draws the scene and then
`ui` paints its chrome on top; UI intents come back to `app`, which is the only
coordinator that applies them.

Deeper documentation lives in `CLAUDE.md` (invariants and crate map),
`docs/PROJECT_STATE.md` (architecture and risk register),
`docs/RENDERING_PIPELINE.md` (render-pass detail), and `mac-port-plan.md` (the
planned macOS port, which moves the GPU layer to `sokol_gfx` on both platforms).

## License

The workspace manifest declares `MIT OR Apache-2.0`. Vendored third-party code
carries its own license and notice files under `third_party/` and
`crates/psd/vendor/`.
