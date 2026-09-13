# 3D Review

A native 3D model **audit** viewer for game assets, in the spirit of F3D or
Autodesk FBX Review, aimed at the questions a technical artist actually asks of a
mesh. Drop an FBX on the window and inspect its geometry, UVs, materials,
textures, skeleton and animation, then optimize it and write it back out.

It runs on **Windows and macOS from one shared codebase**. Written in Rust on
`winit` for the window, `sokol_gfx` for the GPU work (Direct3D 11 on Windows,
Metal on macOS), `egui` for the overlay UI, and vendored `ufbx` for FBX reading.
No web layer, no Electron, no wgpu. It is designed to be fast: fast to start, fast
to read and display a file, and fast to preview and process optimizations.

** [Read the manual](https://psmyles.github.io/3d-review/)** - or press `F1` in
the app, which opens the same pages beside your model.

---

## What it does

|                                                                      |                                                                                                                                                                                                                            |
| -------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **[Getting a model in](docs/book/src/en/getting-a-model-in.md)**     | Drag and drop, a file dialog, a double-click, or the command line. Import runs on a worker and the mesh appears as soon as it is drawable. FBX only for now.                                                               |
| **[Four workspaces](docs/book/src/en/workspaces.md)**                | 3D for the scene, UV for the layout, Tex for the images, Opt for optimization.                                                                                                                                             |
| **[The 3D viewport](docs/book/src/en/viewport.md)**                  | Orbit, pan, zoom and a flycam, perspective or orthographic, with framing that follows the selection.                                                                                                                       |
| **[Shading and review modes](docs/book/src/en/shading.md)**          | Wireframe, unlit or PBR shaded; source, standard or per-part materials; a UV checker; vertex colors; a skin-weight heat map; and eleven single-buffer views for pinning down a bad normal map.                             |
| **[Debug overlays](docs/book/src/en/overlays.md)**                   | Face and vertex normals, bounding box with live dimensions, UV seams, pivot, skeleton. Every one of them deforms with an animated mesh.                                                                                    |
| **[Rendering quality](docs/book/src/en/rendering-quality.md)**       | Baked HDR image-based lighting, screen-space ambient occlusion that converges while the view is still, five tone-mapping operators, and scene MSAA.                                                                        |
| **[Outliner and Inspector](docs/book/src/en/outliner-inspector.md)** | The node hierarchy, the material list and the animation clips on the left; read-only stats and the live material editor on the right.                                                                                      |
| **[Materials and textures](docs/book/src/en/materials.md)**          | Seven PBR slots with per-channel routing, a scene-wide texture pool, and PNG/JPEG/TGA/TIFF/PSD/BMP/GIF/HDR/PNM decoded in-process. A bound texture reloads when you save it.                                               |
| **[Skinning and animation](docs/book/src/en/animation.md)**          | GPU linear-blend skinning, rigid node animation and blend shapes, every FBX animation stack as a clip, with a playback transport.                                                                                          |
| **[Model statistics](docs/book/src/en/stats.md)**                    | Measured counts only - the DCC's own vertex count beside the engine cost, and the split overhead between them.                                                                                                             |
| **[Opt](docs/book/src/en/opt/index.md)**                             | A re-orderable stack of [meshoptimizer](https://meshoptimizer.org/) operations, an AO bake, per-object exclusions, a side-by-side or ghosted comparison, and an FBX export that keeps everything the stack did not change. |

Engine-cooked compressed texture formats (KTX2, DDS) are deliberately **not**
supported: this tool is for reviewing _source_ art.

## Keyboard and mouse

The full binding list is in [the manual](docs/book/src/en/keyboard.md). The ones
worth knowing before you start:

| Key             | Action                                                         |
| --------------- | -------------------------------------------------------------- |
| `F`             | Frame the model, or the selection                              |
| `1` / `2` / `3` | Wireframe only / Unlit / Shaded                                |
| `` ` ``         | Toggle the wireframe overlay                                   |
| `F1`            | Open the manual, at the page for whatever is under the pointer |
| Primary + `O`   | Open a model                                                   |

File commands use `Ctrl` on Windows and `Cmd` on macOS.

---

## Building

**Windows:** build from an **x64 Native Tools Command Prompt for VS 2022**, so
MSVC `cl` is on `PATH` and the vendored C can compile.

**macOS:** the Xcode Command Line Tools are enough for a normal build. The full
Metal toolchain (`xcodebuild -downloadComponent MetalToolchain`) is only needed to
_edit_ shaders; check it with `xcrun -sdk macosx metal --version`, because the
Command Line Tools ship a stub that looks present and then fails.

```
cargo run -p review-app            # launch the viewer
cargo check --workspace            # fast type check
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo test --workspace             # every crate's tests
cargo build --release
```

`scripts/check.ps1` (or `check.sh`) runs the gate - format, clippy, tests and the
shader-bytecode freshness check - in one command. It sets
`REVIEW_REQUIRE_FIXTURES=1`, so a suite that cannot find its fixture or its
vendored dependency fails instead of skipping: pass `-AllowSkips` /
`--allow-skips` on a checkout that genuinely lacks them.

Rust 1.88 or newer, edition 2024.

On macOS, `scripts/dev-app.sh [--debug] [model.fbx]` wraps the built binary in an
unsigned `.app` and launches it. That is the only way to exercise the menu bar,
the Dock and the Finder open path, since a bare executable gets none of them.

### Vendored code

Compiled from source by the build scripts; no prebuilt libraries are needed.

| Tree                        | What it is                                                                      |
| --------------------------- | ------------------------------------------------------------------------------- |
| `third_party/ufbx`          | FBX reading                                                                     |
| `third_party/meshoptimizer` | The Opt operations                                                              |
| `third_party/ufbx-write`    | The Opt FBX export (carried with a documented patch set - see its `NOTICE.txt`) |
| `crates/psd/vendor/Psd`     | PSD decoding                                                                    |
| `vendor/sokol-rust`         | The graphics API bindings                                                       |

The first three are compiled only if present, and the workspace still builds
without them; the features that depend on them report themselves unavailable.

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

### Text and the manual

Every user-visible string is a Fluent message in `crates/l10n/locales/en/*.ftl`,
reached from code through a key generated at build time. A key that does not exist
is a compile error; one that nothing uses is a warning; and a literal written at a
call site fails `cargo test`. English is the only locale so far - adding another
is a new directory beside `en`, and the build script reports what it has not
translated yet.

The manual is the mdBook under `docs/book/`. The same markdown is published to
GitHub Pages by CI **and** compiled into the viewer, which is what the Help window
and every panel's `?` show. Adding a page means adding a `.md` file and a line in
`SUMMARY.md`; naming it from code is then checked by the compiler.

```
cargo install mdbook --locked
mdbook serve docs/book              # preview the site at localhost:3000
```

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

Eleven crates, with boundaries that carry real weight.

| Crate         | Role                                                                                                                                                                           |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `app`         | Event loop, input routing, GPU startup, per-frame draw order, model loading, texture pool, animation clock, undo, the Opt worker. Contains no unsafe code.                     |
| `model`       | Mesh, scene, skinning, morph and animation data, with no knowledge of the GPU or UI. Depends only on `glam`.                                                                   |
| `import`      | FBX loading through vendored ufbx and a single C bridge.                                                                                                                       |
| `render`      | Cameras, configuration, materials, geometry building, the shaders, and the GPU layer over `sokol_gfx`. The only platform GPU code is a small device and swapchain file per OS. |
| `ui`          | The egui chrome, built on egui's own windows and stock widgets. Owns no GPU state and sends intents instead of changing the renderer itself.                                   |
| `optimize`    | meshoptimizer operations, the operation stack, presets, and FBX export.                                                                                                        |
| `l10n`        | The message catalogs and the runtime that resolves them.                                                                                                                       |
| `l10n-build`  | The build-time half: turns the English catalog into typed keys.                                                                                                                |
| `psd`         | PSD decoding over a vendored psd_sdk bridge, with a safe API.                                                                                                                  |
| `shell-macos` | The two macOS shell pieces: the Finder open hook and the menu bar. No-ops on other platforms.                                                                                  |
| `prof`        | Guarded Tracy helpers shared by every instrumented crate.                                                                                                                      |

Data flows one way: input and file drops reach `app`, which drives `import` into
`model`, which `render` turns into GPU resources; `app` draws the scene and then
`ui` paints its chrome on top; UI intents come back to `app`, which is the only
place that applies them.

For the file-by-file crate map, the invariants those boundaries enforce, and the
reasoning behind the stack:

- **[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)** - data flow, crate map,
  invariants, settled decisions, and the platform decision index the source
  comments cite by number.
- **[docs/GOTCHAS.md](docs/GOTCHAS.md)** - the traps that have already cost
  someone a day: rest pose versus bind pose, GPU struct layout, the multi-target
  scene pass, the FBX libraries' sharp edges, and the baked assets.
