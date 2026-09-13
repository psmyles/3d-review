# Windows and macOS

One codebase, one shell, one set of shaders. The only platform-specific code is a
small device and swapchain layer per operating system plus two macOS shell
pieces, and the viewport output matches between the two on the models checked.

What actually differs:

- **Antialiasing levels.** Windows GPUs typically offer 1x through 16x; Apple
  silicon offers 1x, 2x and 4x. The menu lists what the current GPU reports.
- **macOS shell.** A menu bar (App, and File with New and Open, and Window) and
  Finder open support, so a double-click, `open`, or a drop on the Dock icon
  reaches the viewer.
- **Packaging.** Windows ships an installer with the `.fbx` file association;
  macOS ships a signed `.dmg` with a hardened runtime.
- **Settings location.** Window placement and preferences live in
  `%APPDATA%\3D Review` on Windows and
  `~/Library/Application Support/3D Review` on macOS.

macOS builds are Apple silicon only.
