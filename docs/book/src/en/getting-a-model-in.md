# Getting a model in

3D Review opens FBX files. FBX is the most common way to hand a model from a
modelling program to a game engine, and it keeps everything an artist cares
about: the shape, the texture layout, the materials, the bones and the
animations. Other file types are not supported yet.

There are four ways to open a file:

- Drag the file from a folder and drop it onto the window.
- Press the open shortcut (see [Keyboard and mouse](keyboard.md)), or choose
  **Open File...** from [the menu](menu.md), to get a file dialog. You can
  pick more than one file in it.
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

If you open another file with **Open File...** while one is still loading,
the new one wins, and you will never be shown the wrong model. A file you drop
on the window while another is loading waits for it instead, so animations
dropped while their model is still loading are added to it.

## A model and its animation files

Game characters often come as one file holding the model and its skeleton, and
separate files holding its animations. You can open them together: pick them
all in the file dialog, drop them all on the window at once, or give them all
on the command line. 3D Review works out which file is the model - the one with
a mesh and no animations of its own - and plays the other files' animations on
it (see [Skinning and animation](animation.md)). They are matched to the model
by the names of its bones and blend shapes, so they have to be made for the
same skeleton.

You can also add animation files to a model that is already open:

- **Add Animations...** in [the menu](menu.md) asks for files to add. A file
  that does not match the model is not loaded, and a card says so.
- Dropping a file on the window adds it when it is an animation for the model
  on screen. Anything else, such as a different model, replaces what is open,
  as a drop always has.
- Choosing a file from **Open Recent** works the same way as dropping it.

An animation file can carry a copy of the model's meshes, for instance to hold
facial animation. It still counts as an animation file as long as every mesh in
it is one the model has, and only its animation is used.

Only one model is shown at a time. A second model opened together with the
first is left out, and a card lists it. If you opened an animation file on its
own and then drop its model on the window, the model takes over, and the
animations you had open play on it.

## Starting over

The new-file shortcut, or **Close File** in [the menu](menu.md), puts the
viewer back the way it was when it started: no model, the camera in its home
position, and every panel as it was.

## What is not supported on purpose

Texture files that a game engine has already compressed for itself (KTX2 and
DDS files) are not read. This tool is for looking at the original artwork, the
files an artist actually paints and saves. Engine-compressed files are made by
the engine, not by people, so you will not normally have one to look at.
