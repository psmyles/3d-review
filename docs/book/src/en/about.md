# About

3D Review is a viewer for FBX models made for games, with a built-in tool for
making them lighter to draw.

It is written in the Rust programming language. It uses `winit` for the
window, `sokol_gfx` to talk to the GPU (through Direct3D 11 on
Windows and Metal on macOS), `egui` for the buttons and panels, and `ufbx` to
read FBX files. There is no web browser hidden inside it.

## Open source components

The viewer is built with the help of these open source projects:

- [ufbx](https://github.com/ufbx/ufbx) - reads FBX files.
- [ufbx_write](https://github.com/ufbx/ufbx-write) - writes FBX files, for the
  Opt export.
- [meshoptimizer](https://github.com/zeux/meshoptimizer) - the operations in
  the Opt workspace.
- [psd_sdk](https://github.com/MolecularMatters/psd_sdk) - reads Photoshop PSD
  files.
- [sokol](https://github.com/floooh/sokol) - talks to the GPU.
- [egui](https://github.com/emilk/egui) - draws the buttons and panels.

The bundled typefaces are Inter and JetBrains Mono.

## Reporting a problem

If something goes wrong, please tell us the exact text of any error message,
what you were doing at the time, and which version you are running. That is
what lets us find and fix the problem.

To find the version: on macOS, open the app menu and choose About 3D Review.
On Windows, right-click `3d-review.exe`, choose Properties, and look at the
Details tab.
