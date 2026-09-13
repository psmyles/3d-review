# Getting a model in

3D Review opens FBX files. FBX is the most common way to hand a model from a
modelling program to a game engine, and it keeps everything an artist cares
about: the shape, the texture layout, the materials, the bones and the
animations. Other file types are not supported yet.

There are four ways to open a file:

- Drag the file from a folder and drop it onto the window.
- Press the open shortcut (see [Keyboard and mouse](keyboard.md)) to get a
  file dialog.
- Double-click the empty view.
- Open the file from your desktop. On Windows the installer teaches the system
  that `.fbx` files belong to 3D Review; on macOS the app offers itself in
  Finder's Open With menu. You can also give the file path on the command line.

## What happens while a file loads

Loading happens in the background, and a small progress card appears at the
bottom of the view. The model shows up on screen as soon as it can be drawn.
A few measurements that nothing on screen needs right away keep arriving after
that, such as how far each animation moves and the `GPU Verts` number on the
stats card. You do not have to wait for them; they simply fill in when ready.

If you open a second file while the first is still loading, the second one
wins. You will never be shown the wrong model.

## Starting over

The new-file shortcut puts the viewer back the way it was when it started:
no model, the camera in its home position, and every panel as it was.

## What is not supported on purpose

Texture files that a game engine has already compressed for itself (KTX2 and
DDS files) are not read. This tool is for looking at the original artwork, the
files an artist actually paints and saves. Engine-compressed files are made by
the engine, not by people, so you will not normally have one to look at.
