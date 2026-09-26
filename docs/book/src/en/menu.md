# The menu

The button at the far left of the toolbar opens the menu. It is there in every
workspace.

- **Open file...** asks for a model to open, the same as the open shortcut
  (see [Keyboard and mouse](keyboard.md)).
- **Close file** puts the viewer back the way it was when it started: no
  model, and the camera in its home position. It is the same as the new-file
  shortcut, and is greyed out while no model is open.
- **Remember settings** is a switch. A tick next to it means it is on.
- **Exit** closes the viewer.

## Remember settings

While **Remember settings** is on, the choices you make in the tools' options
windows are kept when you close the viewer, and are back the next time you
start it. That covers everything those windows set:

- the colors, lengths and sizes of the wireframe, bounding box, normals,
  UV seams and skeleton, and which meshes the bounding box wraps;
- the UV checker's pattern and how many times it repeats;
- which channels vertex colors show, the material mode and the buffer shown;
- the background, the environment and its strength and turn, and whether the
  environment is drawn behind the model;
- the ambient occlusion, tone mapping and edge smoothing settings, including
  whether each of them is on.

What is *not* kept is which tools are switched on in the toolbar, such as
whether the wireframe or the normals are showing. Those are about the model in
front of you, so every session starts with them at their usual settings.

Turning **Remember settings** off does not change anything on screen. It only
means the next start begins from the usual settings again.

The settings are saved in a small text file in your user settings folder,
next to the file that remembers where the window was:

- Windows: `%APPDATA%\3D Review\settings.cfg`
- macOS: `~/Library/Application Support/3D Review/settings.cfg`
