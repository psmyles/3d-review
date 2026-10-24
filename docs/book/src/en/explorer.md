# Thumbnails and previews in File Explorer and Finder

3D Review can show `.fbx` files in File Explorer on Windows and in Finder on
macOS the way the viewer shows them, with the same lighting and materials. The
Windows half comes first below; the Mac half is in [On macOS](#on-macos).

- **Thumbnails.** In any icon view, each `.fbx` file shows a picture of its
  model from the viewer's default three-quarter angle, on a see-through
  background, so it sits on Explorer's own background in light and dark mode.
- **The preview pane.** Turn on Explorer's preview pane (**View > Preview pane**,
  or **Alt+P**) and select an `.fbx` file. The model appears with every texture
  it uses, on the viewer's black background, and you can look around it:
  - drag with the left button to orbit around the model;
  - drag with the right or middle button to pan;
  - roll the wheel, or hold **Alt** and drag with the right button, to zoom;
  - double-click, or press **F** or **Home**, to frame the whole model again.

The installer's **Show .fbx thumbnails and previews in File Explorer** option
sets this up. It is on by default, and uninstalling 3D Review removes it again.

## What a thumbnail can show

Windows hands a thumbnail maker the file's contents but not where the file is,
which keeps it safely apart from Explorer itself. So a thumbnail can only find
textures that are embedded in the FBX file, or written down in it with a full
path that still exists. Textures that sit beside the model show up in the preview
pane and in the viewer, but not in the thumbnail, which shows those materials in
their plain colors.

A file larger than 512 MB keeps its ordinary icon: reading it would take far
longer than a picture a few dozen pixels across is worth.

## When something looks wrong

- **An old thumbnail.** Windows keeps the thumbnails it has made. It makes a new
  one when a file changes; to clear them all, run Disk Cleanup and tick
  **Thumbnails**.
- **Explorer's shortcuts stop working.** After a click in the preview pane, the
  keyboard goes to the preview, so keys like the arrows do not move through the
  file list. Click the file list again to give it back.
- **No picture at all.** The model may have nothing to draw (an animation saved
  apart from its character), or the file may not be one 3D Review can read. The
  reason is written to the viewer's log, in `%LOCALAPPDATA%\3D Review\logs`.

## On macOS

Finder shows `.fbx` files through Quick Look, drawn by the same renderer:

- **Thumbnails.** Icon, list, column and gallery views, and **Get Info**, show a
  picture of each model from the viewer's default angle, on a see-through
  background.
- **Quick Look.** Select an `.fbx` file and press **Space**, or look at the
  preview in column or gallery view. The model appears on the viewer's black
  background, and you can look around it:
  - drag with the left button to orbit around the model;
  - drag with the right or middle button, or Control-click and drag, to pan;
  - scroll, pinch on a trackpad, or hold **Option** and drag with the right
    button, to zoom;
  - double-click, or press **F** or **Home**, to frame the whole model again.

Both come with the app: there is nothing to install apart from moving
3D Review into Applications. Each can be turned off in System Settings, in the
Quick Look list under **General > Login Items & Extensions**.

### What Finder can show

macOS lets Quick Look read only the file you are looking at. Textures embedded in
the FBX file appear; textures that sit beside it, or anywhere else on disk, do
not, and those materials show in their plain colors. The viewer shows them all.

As on Windows, a file larger than 512 MB keeps its ordinary icon.

### When something looks wrong on a Mac

- **No thumbnails right after installing.** Open 3D Review once, then log out
  and back in, so Finder's thumbnail service sees the new extension.
- **No picture at all.** The model may have nothing to draw, or the file may not
  be one 3D Review can read. Quick Look keeps its own log, in
  `~/Library/Containers/com.psmyles.3d-review.preview/Data/Library/Logs/3D Review`
  for the preview and the same path with `thumbnail` in place of `preview` for
  thumbnails.
