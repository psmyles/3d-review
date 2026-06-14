# MVP_PLAN.md

## Rust + wgpu + egui + ufbx MVP Plan

### Summary
Build a new Windows-first native Rust model viewer in `D:\Dev\3d-review-rs`. The MVP proves the full application loop: open a native window, initialize `wgpu`, render a 3D viewport, draw an `egui` UI overlay similar to the reference screenshot, import FBX through vendored `ufbx`, show model stats, and support shaded, wireframe, face-normal, and vertex-normal debug views.

### Environment
Use **x64 Native Tools Command Prompt for VS 2022**.

Verify before building:

```powershell
rustc -vV
cargo --version
cl
git --version
```

Expected:
- Rust host: `x86_64-pc-windows-msvc`
- `cl` reports `for x64`

No setup scripts should install system dependencies automatically.

### Project Setup
Create a new Cargo workspace at:

```text
D:\Dev\3d-review-rs
```

Workspace layout:

```text
crates/
  app/       # executable, winit event loop, app state
  render/    # wgpu renderer, camera, pipelines, shaders
  ui/        # egui layout, style, controls
  import/    # ufbx importer and FFI boundary
  model/     # shared mesh/material/stats types
third_party/
  ufbx/      # pinned ufbx.c / ufbx.h
assets/
  shaders/
  icons/
  test_models/
```

Initial dependencies:
- `winit`
- `wgpu`
- `egui`
- `egui-winit`
- `egui-wgpu`
- `glam`
- `bytemuck`
- `pollster`
- `anyhow`
- `thiserror`
- `tracing`
- `tracing-subscriber`
- `rfd`
- `cc` as a build dependency for compiling `ufbx.c`

### MVP Implementation
- App shell:
  - Use direct `winit` + `wgpu`, not `eframe`.
  - Prefer DX12 on Windows.
  - Use redraw-on-demand where possible, with continuous redraw only while camera motion or UI interaction is active.
  - Render order: 3D scene, debug overlays, egui UI.

- UI:
  - Implement top toolbar, centered `3D / UV / Tex` segmented control, left option panels, stats overlay, and bottom status bar.
  - Use fixed `egui::Area`s for overlay panels.
  - Match the compact dark visual style from the screenshot.
  - MVP only needs `3D` mode to function; `UV` and `Tex` can be disabled placeholders.

- Import:
  - Vendor `ufbx.c` and `ufbx.h`.
  - Compile `ufbx.c` with the `cc` crate.
  - Keep the Rust FFI wrapper small and isolated.
  - Import FBX into flat `ModelData`: positions, normals, UVs, colors, tangents, indices, original face topology, tri-to-face map, bounds, source stats, materials, and warnings.

- Rendering:
  - Upload `ModelData` into GPU vertex/index buffers.
  - Implement orbit/pan/zoom camera with frame-selection.
  - Render grid with colored X/Z axes.
  - Implement a simple shaded material first: base color, normal, roughness/metallic constants, directional or hemisphere lighting.
  - Add derived-on-demand buffers for quad/n-gon wireframe, face normals, and vertex normals.
  - Build/free debug buffers when toggles change or models unload.

### Acceptance Tests
- `cargo check --workspace`
- `cargo clippy --workspace --all-targets`
- `cargo test --workspace`
- `cargo build --release`

Manual MVP checks:
- App launches to an empty viewport.
- egui chrome renders over the viewport.
- File-open loads a small FBX.
- Camera frames the model.
- Stats show polygons, triangles, vertices, UV sets, draw count, and warnings.
- Shaded, wireframe, face normals, and vertex normals toggle correctly.
- Repeated load/unload and debug toggles do not duplicate overlays or crash.

### Assumptions
- Windows is the priority platform.
- Rust is preferred over C++.
- `wgpu + egui` is the chosen stack.
- FBX is the only MVP import format.
- The MVP prioritizes viewer architecture and debug correctness over final PBR polish.
- Full texture loading, IBL, KTX2/Basis, glTF, OBJ, UV view, and texture inspector are post-MVP.
