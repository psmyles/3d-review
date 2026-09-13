# 3D Review

3D Review is a viewer for 3D models made for game development workflows. You drop an FBX file on
the window, turn the model around, and look closely at how it was built: its
shape, its texture layout, its materials and textures, its skeleton and its
animations. When you are happy with what you see, you can also make the model
lighter for a game to draw and save it back out.

If you have used tools like Autodesk FBX Review or F3D, this will feel
familiar. If you are new to 3D, that is fine too: the manual explains things
as it goes.

It runs on **Windows and macOS** from one shared codebase. It is written in
Rust, using `winit` for the window, `sokol_gfx` to talk to the rendering backend
(Direct3D 11 on Windows, Metal on macOS), `egui` for the buttons and panels,
and `ufbx` to read FBX files. It is built to be quick: quick to start, quick to open a file, and quick to show you
the result of a change.

**[Read the manual](https://psmyles.github.io/3d-review/)** - or press `F1`
in the app, which opens the same pages right next to your model.

---

## What it does

|                                                                      |                                                                                                                                                                                                                                              |
| -------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **[Getting a model in](docs/book/src/en/getting-a-model-in.md)**     | Drag and drop, a file dialog, a double-click, or the command line. The model appears as soon as it can be drawn, while the rest loads in the background. FBX only for now.                                                                   |
| **[Four workspaces](docs/book/src/en/workspaces.md)**                | 3D for the model, UV for the texture layout, Tex for the images, Opt for making it lighter.                                                                                                                                                  |
| **[The 3D viewport](docs/book/src/en/viewport.md)**                  | Orbit, slide, zoom and fly the camera, in a natural or a flat technical view, with framing that follows what you select.                                                                                                                     |
| **[Shading and review modes](docs/book/src/en/shading.md)**          | Wireframe, unlit or fully lit; the file's own materials, plain grey clay or one color per part; a checkerboard; vertex colors; a bone-weight heat map; and a view of each shading ingredient on its own, for tracking down a bad normal map. |
| **[Debug overlays](docs/book/src/en/overlays.md)**                   | Face and vertex normals, a bounding box with its size on the edges, texture seams, the pivot and the skeleton. Every one of them moves with an animated model.                                                                               |
| **[Rendering quality](docs/book/src/en/rendering-quality.md)**       | Lighting from real photographed environments, soft shadows in corners that clean themselves up while the view is still, five tone-mapping recipes, and edge smoothing.                                                                       |
| **[Outliner and Inspector](docs/book/src/en/outliner-inspector.md)** | The list of parts, materials and animations on the left; facts about the selection and a live material editor on the right.                                                                                                                  |
| **[Materials and textures](docs/book/src/en/materials.md)**          | Seven material slots with per-channel routing, a shared texture pool, and PNG, JPEG, TGA, TIFF, PSD, BMP, GIF, HDR and PNM read directly. A texture reloads by itself when you save it.                                                      |
| **[Skinning and animation](docs/book/src/en/animation.md)**          | Bone-driven bending done on the GPU, whole-part animation and blend shapes, every animation in the file as a clip, with playback controls.                                                                                                   |
| **[Model statistics](docs/book/src/en/stats.md)**                    | Only real measured numbers: the artist's own point count next to what a game really has to store, and the gap between them.                                                                                                                  |
| **[Opt](docs/book/src/en/opt/index.md)**                             | A list of [meshoptimizer](https://meshoptimizer.org/) operations you can reorder, a soft-shadow bake, per-part exclusions, a side-by-side or ghosted comparison, and an FBX export that keeps everything you did not change.                 |

Texture files that a game engine has already compressed for itself (KTX2,
DDS) are deliberately **not** supported: this tool is for looking at the
original artwork.

## Keyboard and mouse

The full list is in [the manual](docs/book/src/en/keyboard.md). The ones worth
knowing before you start:

| Key             | What it does                                                   |
| --------------- | -------------------------------------------------------------- |
| `F`             | Frame the model, or the selected part                          |
| `1` / `2` / `3` | Wireframe only / Unlit / Shaded                                |
| `` ` ``         | Show or hide the wireframe on top of the model                 |
| `F1`            | Open the manual, at the page for whatever is under the pointer |
| Primary + `O`   | Open a model                                                   |

File commands use `Ctrl` on Windows and `Cmd` on macOS.

---

## Building

This section is for people who want to build the viewer from its source code.
If you just want to use it, download a release and skip ahead.

**Windows:** build from an **x64 Native Tools Command Prompt for VS 2022**, so
the Microsoft C compiler (`cl`) is available and the bundled C code can
compile.

**macOS:** the Xcode Command Line Tools are enough for a normal build. The
full Metal toolchain (`xcodebuild -downloadComponent MetalToolchain`) is only
needed if you want to _edit_ shaders.

```
cargo run -p review-app            # launch the viewer
cargo check --workspace            # fast type check
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo test --workspace             # every crate's tests
cargo build --release
```

`scripts/check.ps1` (or `check.sh`) runs every check in one go: formatting,
lints, tests and the shader freshness check. It sets
`REVIEW_REQUIRE_FIXTURES=1`, so a test that cannot find its sample file or its
bundled library fails instead of quietly skipping. Pass `-AllowSkips` /
`--allow-skips` on a checkout that genuinely lacks them.

You need Rust 1.88 or newer, edition 2024.

On macOS, `scripts/dev-app.sh [--debug] [model.fbx]` wraps the built binary in
an unsigned `.app` and launches it. That is the only way to try the menu bar,
the Dock and opening from Finder, because a bare executable gets none of them.

### Bundled code

These libraries are built from source by the build scripts, so you do not need
any prebuilt libraries.

| Tree                        | What it is                                                                                    |
| --------------------------- | --------------------------------------------------------------------------------------------- |
| `third_party/ufbx`          | Reads FBX files                                                                               |
| `third_party/meshoptimizer` | The Opt operations                                                                            |
| `third_party/ufbx-write`    | Writes FBX files for the Opt export (with a documented set of patches - see its `NOTICE.txt`) |
| `crates/psd/vendor/Psd`     | Reads Photoshop PSD files                                                                     |
| `vendor/sokol-rust`         | Talks to the GPU                                                                              |

The first three are only compiled if they are present. The workspace still
builds without them; the features that need them simply say they are
unavailable.

### Shaders

There is one shader source file, `crates/render/src/shaders/review.glsl`, holding all
of them. `scripts/gen-shaders.sh` (or `.ps1`) turns it into the
checked-in per-platform sources, and the build script compiles this machine's
half to committed bytecode (`fxc` on Windows, `xcrun metal` on macOS), but only
when one is out of date and the compiler is available. Nothing compiles a
shader while the viewer runs, and a broken shader is a build error.

Because both platforms commit their bytecode, a shader edit made on one system
leaves the other system's bytecode stale until that machine rebuilds. A
manifest records a hash of each generated source and of the bytecode built
from it, and the packaging scripts refuse to build a release on a mismatch.

### Text and the manual

Every piece of text a user can read is a message in
`crates/localization/locales/en/*.ftl`, reached from code through a key generated at
build time. A key that does not exist is a compile error; one that nothing
uses is a warning; and a plain string written at a call site fails
`cargo test`. English is the only language so far. Adding another is a new
directory beside `en`, and the build script reports what is not translated
yet.

The manual is the mdBook under `docs/book/`. The same pages are published to
GitHub Pages by CI **and** compiled into the viewer, which is what the Help
window and every panel's `?` show. Adding a page means adding a `.md` file
and a line in `SUMMARY.md`; naming it from code is then checked by the
compiler.

```
cargo install mdbook --locked
mdbook serve docs/book              # preview the site at localhost:3000
```

### Baked assets and packaging

The lighting maps and the environment preview pictures are committed and
built into the binary, so they must exist before `cargo build`. The scripts
under `packaging/` regenerate them, and the Windows installer build runs them
for you.

`cargo run --release -p review-render --features bake --bin bake_ibl` rebuilds
the lighting maps. It needs a real GPU.

- `packaging/build-windows-installer.ps1` builds the Windows installer.
- `scripts/build-mac.sh` builds, signs, notarizes and packages the macOS
  `.dmg`. It is run by hand on the development Mac, so the signing certificate
  never leaves that machine.
- `scripts/gate.ps1` and `scripts/gate.sh` measure the startup time, frame
  time and memory of two builds against each other, to catch a slowdown before
  it ships. There is no cross-platform comparison: a Mac and a PC are two
  machines, not two builds.

Optional Tracy profiling is compiled in but stays off unless you ask for it:
pass `--tracy` to start the client. A normal run opens no network socket and
costs essentially nothing.

## Architecture

The code is split into eleven crates (Rust's word for a library or program),
and the lines between them matter.

| Crate                | Role                                                                                                                                                                                                 |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `app`                | The event loop, input handling, graphics startup, the order things are drawn in each frame, model loading, the texture pool, the animation clock, undo, and the Opt worker. Contains no unsafe code. |
| `model`              | The mesh, scene, skinning, morph and animation data, with no knowledge of the GPU or the UI. Depends only on `glam`.                                                                                 |
| `import`             | FBX loading through the bundled ufbx and a single C bridge.                                                                                                                                          |
| `render`             | Cameras, settings, materials, geometry building, the shaders, and the graphics layer over `sokol_gfx`. The only platform-specific graphics code is a small device and swapchain file per OS.         |
| `ui`                 | The egui interface, built on egui's own windows and stock widgets. Owns no graphics state and sends requests instead of changing the renderer itself.                                                |
| `optimize`           | The meshoptimizer operations, the operation list, presets, and FBX export.                                                                                                                           |
| `localization`       | The message catalogs and the runtime that looks them up.                                                                                                                                             |
| `localization-build` | The build-time half: turns the English catalog into typed keys.                                                                                                                                      |
| `psd`                | Photoshop PSD decoding over a bundled psd_sdk bridge, with a safe API.                                                                                                                               |
| `shell-macos`        | The two macOS shell pieces: the Finder open hook and the menu bar. Does nothing on other platforms.                                                                                                  |
| `prof`               | Guarded Tracy helpers shared by every instrumented crate.                                                                                                                                            |

Data flows one way: input and dropped files reach `app`, which drives `import`
into `model`, which `render` turns into graphics resources; `app` draws the
scene and then `ui` paints its panels on top; requests from the UI come back
to `app`, which is the only place that acts on them.

For the file-by-file map, the rules those boundaries enforce, and the reasoning
behind the stack:

- **[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)** - data flow, crate map,
  invariants, settled decisions, and the platform decision index the source
  comments cite by number.
- **[docs/GOTCHAS.md](docs/GOTCHAS.md)** - the traps that have already cost
  someone a day: rest pose versus bind pose, GPU struct layout, the
  multi-target scene pass, the FBX libraries' sharp edges, and the baked
  assets.
