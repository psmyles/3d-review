# Getting a model in

FBX is the only import format for now. It is read by vendored `ufbx` through a
single C bridge, so quad topology and the file's own vertex counts survive intact
instead of being flattened by a glTF round-trip.

There are four ways to open a file:

- Drag and drop it onto the window.
- Press the open shortcut for a file dialog.
- Double-click the empty viewport.
- Pass a path on the command line, or open an `.fbx` from the desktop. The
  Windows installer registers the file type, and on macOS the app bundle offers
  itself for `.fbx` in Finder's Open With.

## What happens during a load

Import runs on a background worker with a progress card at the bottom of the
viewport. The model appears as soon as it is drawable, and the measurements that
nothing on screen needs yet keep arriving after it: each animation clip's range
of motion, and the per-draw-group vertex figures the stats card reports as
`GPU Verts`.

A newer request replaces one still in progress, so opening two files in a row
never shows the wrong result.

## Starting over

The new-file shortcut returns the viewer to its launch state: no model, the home
camera, and every panel as it was.

## What is deliberately not supported

Engine-cooked compressed texture formats (KTX2, DDS) are not read. This tool is
for reviewing *source* art, and those formats are produced inside an engine's
content pipeline rather than hand-authored.
