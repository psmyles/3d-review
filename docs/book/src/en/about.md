# About

3D Review is a native FBX audit viewer and mesh optimizer for game assets.

It is written in Rust on `winit` for the window, `sokol_gfx` for the GPU work
(Direct3D 11 on Windows, Metal on macOS), `egui` for the overlay interface, and
vendored `ufbx` for FBX reading. There is no web layer and no Electron.

## Open source components

The viewer builds several third-party libraries from vendored source:

| Component | What it does |
| --- | --- |
| [ufbx](https://github.com/ufbx/ufbx) | FBX reading |
| [ufbx_write](https://github.com/ufbx/ufbx-write) | FBX writing, for the Opt export |
| [meshoptimizer](https://github.com/zeux/meshoptimizer) | The Opt operations |
| [psd_sdk](https://github.com/MolecularMatters/psd_sdk) | PSD decoding |
| [sokol](https://github.com/floooh/sokol) | The graphics API layer |
| [egui](https://github.com/emilk/egui) | The user interface |

The bundled typefaces are Inter and JetBrains Mono.

## Reporting a problem

The version and graphics backend this build is running are shown at the bottom of
this window. Quoting both, along with the exact text of any error message, is
what makes a report actionable.
