# About

3D Review is a viewer for FBX models made for games, with a built-in tool for
making them lighter to draw.

It is written in the Rust programming language. It uses `winit` for the
window, `sokol_gfx` to talk to the graphics card (through Direct3D 11 on
Windows and Metal on macOS), `egui` for the buttons and panels, and `ufbx` to
read FBX files. There is no web browser hidden inside it.

## Open source components

The viewer is built with the help of these open source projects:

| Component | What it does |
| --- | --- |
| [ufbx](https://github.com/ufbx/ufbx) | Reads FBX files |
| [ufbx_write](https://github.com/ufbx/ufbx-write) | Writes FBX files, for the Opt export |
| [meshoptimizer](https://github.com/zeux/meshoptimizer) | The operations in the Opt workspace |
| [psd_sdk](https://github.com/MolecularMatters/psd_sdk) | Reads Photoshop PSD files |
| [sokol](https://github.com/floooh/sokol) | Talks to the graphics card |
| [egui](https://github.com/emilk/egui) | Draws the buttons and panels |

The bundled typefaces are Inter and JetBrains Mono.

## Reporting a problem

The version number and the graphics system this build is using are shown at
the bottom of this window. If something goes wrong, please quote both, along
with the exact text of any error message. That is what lets us find and fix the
problem.
