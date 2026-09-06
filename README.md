# 3D Review

A native 3D model **audit** viewer for game assets, in the spirit of F3D or
Autodesk FBX Review, aimed at the questions a technical artist actually asks of a
mesh. Drop an FBX on the window and inspect its geometry, UVs, materials,
textures, skeleton and animation, then optimize it and write it back out.

It runs on **Windows and macOS from one shared codebase**. Written in Rust on
`winit` for the window, `sokol_gfx` for the GPU work (Direct3D 11 on Windows,
Metal on macOS), `egui` for the overlay UI, and vendored `ufbx` for FBX reading.
No web layer, no Electron, no wgpu. Is is designed to be fast: fast to start, fast to read and display a file, and fast to preview and process optimizations.

---

## Contents

- [3D Review](#3d-review)
  - [Contents](#contents)
  - [Getting a model in](#getting-a-model-in)
  - [Workspaces](#workspaces)
  - [The 3D viewport](#the-3d-viewport)
  - [Shading and review modes](#shading-and-review-modes)
    - [Buffer inspection](#buffer-inspection)
  - [Debug overlays](#debug-overlays)
  - [Rendering quality](#rendering-quality)
  - [Outliner and Inspector](#outliner-and-inspector)
  - [Materials and textures](#materials-and-textures)
  - [UV workspace](#uv-workspace)
  - [Texture workspace](#texture-workspace)
  - [Skinning and animation](#skinning-and-animation)
  - [Model statistics](#model-statistics)
  - [Opt: the optimization stack](#opt-the-optimization-stack)
    - [How a run works](#how-a-run-works)
    - [Operations](#operations)
    - [Simplification settings](#simplification-settings)
    - [AO baking](#ao-baking)
    - [Per-object overrides](#per-object-overrides)
    - [Comparison](#comparison)
    - [Export](#export)
  - [Undo, presets and session state](#undo-presets-and-session-state)
  - [Keyboard and mouse](#keyboard-and-mouse)
  - [Windows and macOS](#windows-and-macos)
  - [Building](#building)
    - [Shaders](#shaders)
    - [Baked assets and packaging](#baked-assets-and-packaging)
  - [Architecture](#architecture)
  - [License](#license)

---

## Getting a model in

FBX is the only import format for now. It is read by vendored `ufbx` through a single C
bridge, so quad topology and the file's own vertex counts survive intact instead
of being flattened by a glTF round-trip.

There are multiple ways to open a file in 3D Review:

- Drag and drop a file onto the window.
- `Ctrl+O` (`Cmd+O` on macOS) for a file dialog.
- Double-click the empty viewport.
- Pass a path on the command line, or open an `.fbx` from the desktop: the
  Windows installer registers the file type, and on macOS the app bundle offers
  itself for `.fbx` in Finder's Open With.

Import runs on a background worker thread with a progress toast. A newer request replaces one
still in progress, so opening two files in a row never shows the wrong result.
`Ctrl/Cmd+N` returns the viewer to its launch state.

## Workspaces

Four modes, switched from the segmented control in the middle of the toolbar. The
toolbar swaps its tool groups to match the active workspace.

| Workspace | What it is                                                                       |
| --------- | -------------------------------------------------------------------------------- |
| **3D**    | The scene viewport: shading modes, debug overlays, materials, animation.         |
| **UV**    | A 2D UV-layout viewer with its own pan and zoom camera.                          |
| **Tex**   | A 2D image viewer over the scene texture pool.                                   |
| **Opt**   | Mesh optimization, with a side-by-side or ghosted comparison against the source. |

## The 3D viewport

An orbit camera with framing, panning and zooming, in perspective or orthographic
projection. `F` frames the model, or the selection when one is active; pressing it
again goes back to the whole model. `R` returns to the home framing. `WASD` orbit
in animated 45 degree steps. Camera moves ease over 0.3 s rather than snapping.

Display toggles live in the toolbar's right-hand group: the floor grid with axes,
the axis gizmo (click an axis to snap the camera to it), the object pivot
marker, the bounding box, and the skeleton overlay for rigged models.

The viewport background is a status-bar button: black, 25/50/75% grey, white, or a
gradient.

## Shading and review modes

**Shading mode** is a three-way choice, exactly one active at a time:

- **Wireframe only** - edges alone.
- **Unlit** - flat surface color, no lighting.
- **Shaded** - environment-lit PBR, the default.

Two independent toggles sit either side of it and combine with all three:

- **Show Wireframe** - the edge overlay drawn on top of the filled surface. It is
  a real depth-tested line draw in the scene pass, so edges on hidden faces are
  correctly hidden and the scene's antialiasing smooths them. Edge color is
  configurable.
- **Backface Rendering** - off culls back faces (the default, which is how you
  find inverted normals); on draws the mesh double-sided.

**Active material** is a second choice, deciding what the filled faces show. It
applies in every shading mode.

| Mode                | What it shows                                                                                                                                                                                                                                                                                                  |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Source Material** | The imported materials plus any Inspector edits. Clicking the button again cycles **Source, Standard, Unique**: a plain matte mid-grey for every part, or a different random hue per mesh part so the pieces read apart. The replacement happens in the renderer, so the imported materials are never touched. |
| **UV Checker**      | A greyscale or color checker at an adjustable tiling density, on any UV channel of a multi-set model. The fast read on stretching, mirroring and texel density.                                                                                                                                                |
| **Vertex Colors**   | The mesh's vertex-color attribute as RGB, alpha as greyscale, or RGB with alpha driving opacity.                                                                                                                                                                                                               |
| **Buffers**         | One shading input at a time, drawn flat. See below.                                                                                                                                                                                                                                                            |
| **Skin Weights**    | A blue-green-red heat map of how strongly the bones selected in the Outliner pull on each vertex. Flat, unlit and untonemapped, so the color you see _is_ the weight. Offered only for a skinned model.                                                                                                        |

### Buffer inspection

The **Buffers** view draws a single material or geometry input straight to the
screen, skipping lighting and tone mapping so the pixel you see is the value
itself. Clicking the toolbar button again cycles through them; the options panel
picks one directly.

Base Color, Normal (World), Normal Map (Tangent), Geometric Normal, Tangent,
Roughness (or Smoothness, following the material's workflow), Metallic, Ambient
Occlusion, Emission, Opacity, UV.

Both the final shading normal and the raw authored normal map are offered, next to
the geometric normal and the tangent basis, which is what lets a misbehaving
normal map be pinned down: handedness, green-channel convention, or missing
tangents. Color buffers are sRGB-encoded for display; the rest are written raw, so
a 0.5 scalar reads as mid-grey.

## Debug overlays

Independent toggles, each with its own options panel. Every extra buffer an
overlay needs is built when the overlay turns on and freed when it turns off, so
the plain shaded view carries none of them.

- **Face normals** - one line per face, adjustable length and color.
- **Vertex normals** - one line per vertex, adjustable length and color.
- **Bounding box** - with live dimension labels that are correctly hidden behind
  the mesh. Scope is selectable: all meshes, only the selection, or only the
  visible meshes. Edge color is configurable.
- **Pivot marker** - the object's origin.
- **Skeleton** - octahedral bones with joint markers for a rigged model, with an
  adjustable size multiplier and bone color. The selected bone highlights in the
  viewport selection color.
- **Axis gizmo** and **grid**.

Every overlay built from the mesh deforms with it, so overlays stay attached to a
skinned, animated model instead of sitting on the bind pose.

Right-clicking any tool button opens that tool's options panel. Panels are native
windows that can be collapsed and closed, and several can be open at once.

## Rendering quality

The renderer works in linear HDR with reversed-Z depth, drawing into offscreen
buffers that a fullscreen composite pass resolves. The status-bar group on the
right controls it: left-click toggles or cycles, right-click opens the options.

- **Image-based lighting** - six baked HDR environments, each with a preview
  thumbnail in the dropdown. Optional skybox background, an intensity multiplier,
  and a live 0-360 degree environment rotation that is applied while sampling and
  never rebuilds the maps. The lighting maps are baked ahead of time and ship
  block-compressed, so startup does no precompute.
- **Ambient Occlusion** - horizon-based occlusion with a smoothing blur, over its
  own normal and depth buffer. Knobs are Radius, Intensity, Thickness and Quality
  (Low/Medium/High). It darkens only the ambient light, so direct and emissive
  light are never dimmed. On by default.
- **Tone mapping** - Khronos PBR Neutral, Linear, Reinhard, ACES or AgX, applied
  to the linear radiance before sRGB encoding. Turning it off is a straight
  pass-through. On by default.
- **Anti-aliasing** - scene MSAA at 2x, 4x, 8x or 16x. Levels the current GPU
  cannot render are left out of the menu rather than offered and failing, so Apple
  silicon shows up to 4x and a Windows GPU usually shows all of them.
- **Viewport background** - black, three greys, white, or a gradient.

## Outliner and Inspector

The **Outliner** is a dockable, resizable left panel with two or three tabs.

- **Scene** - the node hierarchy, indented and collapsible with parent guide
  lines, or a flat list. A search box filters both tabs and lists matches flat, so
  a hit is never buried in a folded branch. Node-kind filter glyphs narrow what is
  listed. Clicking a row selects it, which drives the viewport highlight (with a
  selection flash), the Inspector, and the selection-scoped bounding box and
  framing. Arrow keys walk the rows. Mesh rows carry a visibility eye;
  Ctrl-clicking the eye isolates that mesh instead of toggling it. On bone rows,
  Ctrl adds to the selection and Shift takes a range, which is what feeds the skin
  weight heat map.
- **Materials** - the material list with duplicates merged. Selecting one
  highlights its faces in the viewport and opens it in the Inspector.
- **Animations** - the clip list. Present only for a file that carries animation,
  and never in the Opt workspace.

The **Inspector** is the right panel. For a selected node it shows read-only
stats; for a multi-bone selection, a summary. For a material it shows three
collapsible sections, described next.

## Materials and textures

Materials can be edited live. The Inspector's **Material** section carries the
shader type, transparency mode (Opaque / Blend / Clip), base color, roughness,
metallic and emissive. Roughness follows a per-material **workflow** setting:
native metallic-roughness, or Unity-style smoothness, where the slider and any
bound map read as smoothness and the shader flips them.

**Texture mapping** binds a pooled image plus a channel to each of the seven PBR
slots: base color, normal, roughness, metallic, ambient occlusion, emissive and
opacity. Single-value slots pick one channel, so an ORM-packed map can feed three
slots from one file. Channel routing is guessed from the filename on import and
can be overridden.

Textures live in a **scene-wide pool**, decoded once and shared. The **Texture
files** section lists the pool with thumbnails and a remove action. Source formats
are decoded directly with no ImageMagick dependency: PNG, JPEG, TGA, TIFF, PSD
(layered files, through a vendored psd_sdk bridge that reads the flattened
composite), BMP, GIF, HDR and PNM. A bound texture is watched on disk and decoded
again when it changes, so a save in Photoshop or Substance shows up in the
viewport.

Engine-cooked compressed formats (KTX2, DDS) are deliberately not supported. This
tool is for reviewing _source_ art.

## UV workspace

A separate 2D viewport for the UV layout, with its own pan, zoom and fit. The UV
edges draw in every mode; the shading choice picks what fills them.

- **Wire** - the layout alone.
- **Shaded** - solid islands.
- **Islands** - a different color per island, so overlaps and stray shells stand
  out.

A UV-set picker in the toolbar switches channels on a multi-set model.

## Texture workspace

A 2D image viewer over the scene texture pool, drawn through its own small GPU
path outside the scene's HDR and tone-mapping work, so the pixel on screen equals
the pixel in the file.

- Texture picker for the pool.
- **RGB / R / G / B / A** channel isolation. It is a shader swizzle, so switching
  is instant. The alpha segment hides itself for an opaque image.
- Mipmapped, with pan (left-drag), zoom (wheel or right-drag), and `F` to fit.
- Background fill: black, white, grey or checker.
- A stats panel reporting the file's real properties: format, pixel dimensions,
  channel layout, bit depth and size on disk.

## Skinning and animation

A skinned mesh rests in the file's _default_ pose, skinned onto the skeleton as
drawn, not the bind pose its buffers hold. Deformation is GPU linear-blend
skinning over each vertex's exact list of influences, with weights normalized the
way ufbx does, alongside rigid node animation and blend shapes through the same
vertex shader stage.

Every FBX animation stack imports as a clip, baked at the file's own frame rate
with no key reduction. Frame counts come from the stack's time range, not from key
counts. Clicking a clip in the Outliner selects it, paused on its first frame, and
raises the playback transport at the bottom of the viewport: go to start, step
back, play/pause, step forward, a scrubber, a `frame / total   seconds` readout,
looping (on by default) and a speed control. `Space` plays and pauses; `,` and `.`
step single frames.

While a clip is selected, `F` and the bounding box use that clip's full range of
motion, measured once at import, so framing covers the whole animation rather than
one frame of it. Dual-quaternion skins are evaluated as linear blends and the
Inspector says so. Playback is view state and is never captured by undo.

## Model statistics

A panel of measured counts that can be toggled on and off. Every figure is a real
measured value: there are no placeholder rows, and the counts the DCC package
recorded are reported rather than post-triangulation render counts.

Draws, Polys, Tris, Verts, GPU Verts, Vtx Splits, UV Sets, Bones, Clips, Unit,
FPS.

`Verts` is the file's own vertex count. `GPU Verts` is the engine cost: unique
vertices per draw group. The stats can be scoped to all meshes, the selection, or
only the visible meshes, and copied to the clipboard.

---

## Opt: the optimization stack

A fourth workspace that turns the viewer into a mesh optimizer, built on [**meshoptimizer**](https://meshoptimizer.org/). It shares every 3D control, so the processed mesh can be
inspected with the same shading modes, buffer views and overlays as the source.
Static meshes only for now.

Nothing for Opt is built until the workspace is first opened, and a stack with
nothing enabled schedules no work, so a session that never opens Opt pays nothing
for it.

### How a run works

Every run starts by **merging identical vertices**, and this is what makes the
rest work at all. FBX import splits each face corner into its own vertex, so a
mesh reaches the optimizer with no shared vertices at all: a real 10,006-triangle
asset arrives as 30,018 vertices. Every meshoptimizer operation works through the
index buffer, so on that mesh they would all do nothing. The merge pass joins
vertices that are identical in every attribute, byte for byte, before any
operation runs, and nothing visible changes. On that asset it is 30,018 down to
5,284, after which a 50% LOD target is hit exactly.

The run's baseline is measured _after_ that pass, so the figures credit your
operations rather than the free win any engine cooker also gets. That baseline
also matches the stats panel's `GPU Verts`.

### Operations

The stack is ordered and can be reordered; operations apply top to bottom, and
each can be switched off without deleting it.

| Operation                    | What it does                                                                                                                                                                                                |
| ---------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Weld Vertices**            | Widens what counts as a match beyond the exact merge every run already does, by dropping normals, UVs or colors from the comparison, or allowing a small tolerance. This one _does_ change the mesh.        |
| **Filter Triangles**         | Removes broken triangles (two corners at one position) and exact duplicates. Duplicates wound the other way are kept, for double-sided geometry.                                                            |
| **Prune Components**         | Removes disconnected pieces below a size threshold: stray shells and orphaned faces.                                                                                                                        |
| **Reduce**                   | Simplifies the mesh **in place**. Its output is what every later operation sees, what LOD levels start from, and what the export writes in the source mesh's place.                                         |
| **Generate LODs**            | Fans out into a LOD chain. Operations above it run once on the base mesh; operations below it run on every generated level. At most one per stack. The default chain is three levels at 50%, 25% and 12.5%. |
| **Bake AO to Vertex Colors** | Raycast ambient occlusion baked into the vertex-color set. Changes no geometry.                                                                                                                             |
| **Optimize Vertex Cache**    | Reorders triangles for the GPU's vertex cache. Watch ACMR and ATVR.                                                                                                                                         |
| **Optimize Overdraw**        | Reorders triangles front to back within cache-friendly clusters, so the GPU shades fewer hidden pixels.                                                                                                     |
| **Optimize Vertex Fetch**    | Reorders vertices into the order the index buffer reads them, and drops anything unreferenced.                                                                                                              |

### Simplification settings

**Reduce** and **Generate LODs** share one simplifier setup.

- **Algorithm** - _Standard_ (keeps topology, position error only), _Preserve
  Attributes_ (also penalizes normal, UV and color drift, with per-attribute
  weights), or _Sloppy_ (ignores topology; much faster and hits the target far
  more reliably, but can close holes and merge nearby shells, so it suits the
  smallest levels).
- **Target** - a triangle ratio plus an error limit the simplifier may not exceed.
  It stops short of the ratio rather than going past the error.
- **Flags** - lock the border, absolute error units, prune disconnected parts,
  regularize (and a lighter variant), and permissive collapses across attribute
  seams.

> A simplify that barely removes anything is usually attribute seams, not a bug.
> A mesh whose normals or UVs differ at every corner presents every edge as a
> break, and a topology-preserving collapse cannot cross one. The run notices the
> stall and says so. A position-only weld or the permissive flag unblocks it.

### AO baking

Repeatable CPU raycasts over a hemisphere at each vertex, against a triangle tree,
spread across threads.

- **Quality** - 32, 64, 128 or 512 rays per vertex.
- **Max distance** - how far a surface can be and still block light, in world
  meters; zero means the whole scene.
- **Intensity** - a power curve on visibility, matching the viewport AO panel.
- **Target** - alpha, RGB greyscale, multiplied into RGB, or a single R/G/B
  channel. Optional sRGB encoding for the RGB targets; alpha is always linear.

Two rules make it usable on real game FBXs. Objects excluded from the stack still
**block light** but are not written to, while nodes **hidden** in the Outliner
neither block light nor get baked, so hide the collision shell first. And blockers
are grouped by the `_LOD<n>` name suffix: game assets carry their whole LOD chain
as siblings in one file, and casting rays from one LOD onto another's almost
identical surfaces ruins the result. Each LOD bakes only against its own group
plus every node with no suffix, so a whole visible chain bakes correctly in one
run.

Adding the operation switches the viewport to the matching vertex-color mode, and
the exporter already writes vertex colors, so nothing else is needed.

### Per-object overrides

Any object can be **excluded** from the stack, passing through untouched at full
detail in every output level. That is the way to keep a hero prop or a collision
shell out of the optimization. Individual operations can also be given replacement
settings for one object; operations without an override use the global settings.

### Comparison

Every edit reprocesses on a worker thread. Only one run is ever in flight: edits
arriving mid-run mark it out of date and it restarts once with the latest stack,
so dragging a slider settles on its own without a timer, and a result that has
been overtaken is dropped rather than shown. A notice appears only if a run takes
a while.

The centre group in the status bar, sitting over the split's divider because it
describes the viewport as a whole, carries the comparison controls.

- **Split** - source left, processed right, with an option to link both cameras so
  they move together.
- **Overlay** - both in one space, one mesh shaded and the other a ghost, either
  see-through or wireframe. `X` swaps which one is solid, which is the most
  reliable way to spot where a simplification moved the silhouette. A legend names
  which is which.
- **LOD picker** - which level of the chain is shown. Once you pick a level, later
  runs keep showing it.

A second stats card reports the processed mesh's measured counts alongside the
figures meshoptimizer measured: **ACMR**, **ATVR**, **overdraw**, **overfetch**
and the largest simplification error, each with its change against the source
tinted green or red. Every figure there is one where lower is better. That card is
up as soon as the workspace opens: a run with nothing enabled produces no mesh but
still measures the source, so the baseline is readable before anything is added.
This is what makes the three reorder operations, which change nothing you can see
in the viewport, worth having.

### Export

An explicit export writes the chain to FBX through vendored `ufbx_write`.

- **Packaging** - one file holding `MeshName_LOD0` through `MeshName_LODn` as
  sibling nodes (the naming most engines detect automatically), or one file per
  level.
- **Hierarchy** - rebuild the original node hierarchy so the export round-trips
  like the source asset, or write flat meshes with identity transforms.
- **Format** - binary or ASCII FBX.

Output is always triangulated, carries the source materials (without textures) and
vertex colors, and declares a unit scale of 100, since import converts every file
to meters while FBX conventionally stores centimeters.

## Undo, presets and session state

`Ctrl+Z` undoes and `Ctrl+Y` (or `Ctrl+Shift+Z`) redoes, with `Cmd` in place of
`Ctrl` on macOS. Undo covers **document edits**: Outliner selection, mesh
visibility, material parameters, texture slot bindings, the texture pool, and the
Opt operation stack. A continuous slider or color-picker drag collapses into a
single step. **View** state (camera, grid, shading mode, antialiasing, ambient
occlusion, tone mapping, environment, the UV and Tex viewports, and animation
playback) is deliberately left out.

An Opt stack saves and loads as a versioned JSON **preset**, carrying its
operations, per-object overrides and export settings.

Window position and size are restored between sessions, checked against the
monitors actually present at launch. The settings file lives in the usual place
for each OS: `%APPDATA%\3D Review` on Windows, `~/Library/Application Support/3D
Review` on macOS.

## Keyboard and mouse

A cheat-sheet of these is shown on launch and dismissed by any key or click.

**Mouse (3D)** - left-drag orbits, right-drag pans, middle-drag pans,
Alt+right-drag zooms, wheel zooms. Double-clicking the empty viewport opens a
file.

**Mouse (UV)** - left-drag pans, right-drag zooms, wheel zooms.

**Mouse (Tex)** - left-drag pans, right-drag zooms, wheel zooms.

Trackpad pinch zooms in all three viewports.

File commands use the platform's own modifier: `Ctrl` on Windows, `Cmd` on macOS.
The on-screen labels follow the platform.

| Key                                  | Action                                            |
| ------------------------------------ | ------------------------------------------------- |
| `` ` ``                              | Toggle the wireframe overlay                      |
| `1` / `2` / `3`                      | Wireframe only / Unlit / Shaded                   |
| `I`                                  | Toggle the stats display                          |
| `G`                                  | Toggle the grid                                   |
| `F`                                  | Frame the model, or the selection                 |
| `R`                                  | Reset the camera (refits the image in UV and Tex) |
| `W` `A` `S` `D`                      | Orbit the camera in 45 degree steps               |
| `Space`                              | Play / pause the selected clip                    |
| `,` / `.`                            | Previous / next frame of the selected clip        |
| `X`                                  | Swap source and processed in the Opt overlay      |
| `Esc`                                | Clear the selection                               |
| `Ctrl+N`                             | Reset to the start state                          |
| `Ctrl+O`                             | Open a model                                      |
| `Ctrl+Z` / `Ctrl+Y` / `Ctrl+Shift+Z` | Undo / redo / redo                                |

Right-click any toolbar or status-bar tool button to open its options panel.

## Windows and macOS

One codebase, one shell, one set of shaders. The only platform-specific code is a
small device and swapchain layer per OS (Direct3D 11 and Metal) plus two macOS
shell pieces, and the viewport output matches between the two on the models
checked.

What actually differs:

- **Antialiasing levels.** Windows GPUs typically offer 1x through 16x; Apple
  silicon offers 1x, 2x and 4x. The menu lists what the current GPU reports.
- **macOS shell.** A menu bar (App / File with New and Open / Window) and Finder
  open support, so a double-click, `open`, or a drop on the Dock icon reaches the
  viewer. On this OS those arrivals are system events rather than command-line
  arguments, so they need their own handling.
- **Packaging.** Windows ships an installer with the `.fbx` file association;
  macOS ships a signed `.dmg` with a hardened runtime.
- **Lighting bake bytes.** A fresh bake of the lighting maps reproduces the
  committed files exactly on Windows, and only to within tiny floating-point
  differences on Metal, so the Windows bake is the one set of files both platforms
  ship. The macOS packaging script deliberately does not re-bake.

macOS builds are Apple silicon only.

## Building

**Windows:** build from an **x64 Native Tools Command Prompt for VS 2022**, so
MSVC `cl` is on `PATH` and the vendored C can compile.

**macOS:** the Xcode Command Line Tools are enough for a normal build. The full
Metal toolchain (`xcodebuild -downloadComponent MetalToolchain`) is only needed to
_edit_ shaders; check it by running `xcrun -sdk macosx metal --version`, because
the Command Line Tools ship a stub that looks present and then fails.

```
cargo run -p review-app            # launch the viewer
cargo check --workspace            # fast type check
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo test --workspace             # model / import / render / optimize tests
cargo build --release
```

Rust 1.88 or newer, edition 2024.

On macOS, `scripts/dev-app.sh [--debug] [model.fbx]` wraps the built binary in an
unsigned `.app` and launches it. That is the only way to exercise the menu bar,
the Dock and the Finder open path, since a bare executable gets none of them.

Vendored C and C++ code is compiled from source by the build scripts and needs no
prebuilt libraries:

- `third_party/ufbx` - FBX reading.
- `third_party/meshoptimizer` - the Opt operations.
- `third_party/ufbx-write` - the Opt FBX export.
- `crates/psd/vendor/Psd` - PSD decoding.
- `vendor/sokol-rust` - the graphics API bindings.

The first three are compiled only if present, and the workspace still builds
without them; the features that depend on them simply report themselves
unavailable.

### Shaders

There is one shader source, `crates/render/src/shaders/review.glsl`, holding all
fifteen programs. `scripts/gen-shaders.sh` (or `.ps1`) turns it into the
checked-in per-backend HLSL and Metal sources, and the build script compiles this
machine's half to committed bytecode (`fxc` on Windows, `xcrun metal` on macOS),
only when a blob is out of date and the compiler is available. Nothing compiles a
shader at run time, and a broken shader is a build error.

Because both platforms commit bytecode, a shader edit made on one OS ships stale
bytecode for the other until that machine rebuilds. A manifest records the hash of
each generated source and the blob built from it, and the packaging checks fail a
release build on a mismatch.

### Baked assets and packaging

The baked lighting maps and the HDR preview thumbnails are committed and built
into the binary, so they must exist before `cargo build`. The scripts under
`packaging/` regenerate them, and the Windows installer build runs them
automatically.

`cargo run --release -p review-render --features bake --bin bake_ibl` re-bakes the
lighting maps. It needs a real GPU.

- `packaging/build-windows-installer.ps1` builds the Windows installer.
- `scripts/build-mac.sh` builds, signs, notarizes and packages the macOS `.dmg`.
  It runs by hand on the dev Mac so the signing certificate never leaves that
  machine.
- `scripts/gate.ps1` and `scripts/gate.sh` measure startup time, frame time and
  memory of two builds against each other, to catch a performance regression
  before it ships. There is no cross-platform comparison: a Mac and a PC are two
  machines, not two builds.

Optional Tracy profiling is compiled in but off unless asked for: pass `--tracy`
to start the client. A normal run opens no socket and costs essentially nothing.

## Architecture

Nine crates, with boundaries that carry real weight.

| Crate         | Role                                                                                                                                                                           |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `app`         | Event loop, input routing, GPU startup, per-frame draw order, model loading, texture pool, animation clock, undo, the Opt worker. Contains no unsafe code.                     |
| `model`       | Mesh, scene, skinning, morph and animation data, with no knowledge of the GPU or UI. Depends only on `glam`.                                                                   |
| `import`      | FBX loading through vendored ufbx and a single C bridge.                                                                                                                       |
| `render`      | Cameras, configuration, materials, geometry building, the shaders, and the GPU layer over `sokol_gfx`. The only platform GPU code is a small device and swapchain file per OS. |
| `ui`          | The egui chrome, built on egui's own windows and stock widgets. Owns no GPU state and sends intents instead of changing the renderer itself.                                   |
| `optimize`    | meshoptimizer operations, the operation stack, presets, and FBX export.                                                                                                        |
| `psd`         | PSD decoding over a vendored psd_sdk bridge, with a safe API.                                                                                                                  |
| `shell-macos` | The two macOS shell pieces: the Finder open hook and the menu bar. No-ops on other platforms.                                                                                  |
| `prof`        | Guarded Tracy helpers shared by every instrumented crate.                                                                                                                      |

Data flows one way: input and file drops reach `app`, which drives `import` into
`model`, which `render` turns into GPU resources; `app` draws the scene and then
`ui` paints its chrome on top; UI intents come back to `app`, which is the only
place that applies them.

Deeper documentation lives in `CLAUDE.md` (the current reference: invariants,
crate map, build notes) and `docs/mac-port-plan.md` (the cross-platform work, its
decisions and its measurements). `docs/PROJECT_STATE.md` and
`docs/RENDERING_PIPELINE.md` are older audit notes from before the GPU layer moved
to `sokol_gfx` and are due a rewrite.

## License

The workspace manifest declares `MIT OR Apache-2.0`. Vendored third-party code
carries its own license and notice files under `third_party/`, `vendor/` and
`crates/psd/vendor/`.
