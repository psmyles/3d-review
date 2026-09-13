# Windows and macOS

3D Review is the same program on both systems, and the picture it draws is
the same on both for the models we have checked. Only a few small things
differ:

- **Anti-aliasing levels.** A Windows GPU usually offers 1x up to
  16x. A Mac with Apple silicon offers 1x, 2x and 4x. The menu only lists what
  your card can actually do.
- **The Mac menu bar.** On macOS there is a menu bar (the app menu, File with
  New and Open, and Window), and the app works with Finder, so double-clicking
  a file, using `open`, or dropping a file on the Dock icon all open it.
- **How it is installed.** Windows gets an installer that also teaches the
  system that `.fbx` files belong to 3D Review. macOS gets a signed `.dmg`.
- **Where settings are kept.** Window position and preferences are saved in
  `%APPDATA%\3D Review` on Windows and
  `~/Library/Application Support/3D Review` on macOS.

The Mac version runs on Apple silicon only.
