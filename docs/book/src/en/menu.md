# The menu

The button at the far left of the toolbar opens the menu. It is there in every
workspace, and has four parts.

## File

- **Open File...** asks for a model to open, the same as the open shortcut
  (see [Keyboard and mouse](keyboard.md)).
- **Open Recent** lists the last ten models you opened, the newest at the
  top. Point at one to see where it is saved, and click it to open it again.
  **Clear Recent Files** at the bottom empties the list. A file that has been
  moved or deleted is taken off the list the next time you try to open it.
  The list is kept between sessions whether or not **Remember Settings** is
  on, in the same settings file (see [below](#remember-settings)), and it is
  greyed out until you have opened something.
- **Close File** puts the viewer back the way it was when it started: no
  model, and the camera in its home position. It is the same as the new-file
  shortcut, and is greyed out while no model is open.
- **Exit** closes the viewer.

## Preferences

- **Remember Settings** is a switch. A tick next to it means it is on. See
  [below](#remember-settings) for what it keeps.
- **User Name...** opens a box with the name your
  [review comments](comments.md) and replies are signed with. Type the new name
  and press **Save** (or Enter); **Cancel**, `Esc` or a click outside the box
  keeps the old one. A name is needed, so **Save** waits until there is one.
  The name is kept between sessions whether or not **Remember Settings** is on,
  in the same settings file. Comments already written keep the name they were
  signed with.

## Debug

- **View Log** opens the log window, which lists what the viewer has
  reported since it started. See [below](#the-log).
- **Tracy Profiler** is for developers measuring the viewer's performance.
  While it is ticked, the viewer starts with the
  [Tracy profiler](https://github.com/wolfpld/tracy) enabled, the same as
  starting it with `--tracy`. The change takes effect the next time you start
  the viewer, not straight away.

## Help

- **About** shows which version of the viewer this is, which graphics system
  it is drawing with, and a link to the project's page on GitHub.
- **Documentation** opens this manual.
- **Check for Updates** asks GitHub whether a newer version has been released.
  If there is one, the releases page opens in your web browser so you can
  download it; if not, a note says you already have the latest version. It
  needs an internet connection, and it is the only time the viewer goes online.
  It never checks by itself.
- **Report an Issue** opens a new issue on the project's GitHub page in your
  web browser, where you can describe a problem or ask for something.
- **Credits** opens the list of the people and projects the viewer is built
  on, in your web browser.

## The log

The viewer keeps a log of what it does: which version it is and what it is
drawing with, each file you open with how long it took and what is in it,
each optimization run and export, every message it has shown you, and any
warning or error along the way.

**View Log** shows this session's part of it in a window, newest at the
bottom, updating as new lines arrive. Each line has the time it happened, how
serious it is, which part of the viewer said it (in square brackets), and
the message itself. The switches along the top choose which kinds of line
are shown:

- **Debug** is extra detail for developers, such as how long each stage of
  a file load took. It is hidden until you tick it.
- **Info** is things happening as they should.
- **Warning** is something that went through, but not entirely as asked.
- **Error** is something that did not happen.

**Clear** empties the window. It does not touch the log file, so nothing a
bug report would need is lost. You can select the text in the window and
copy it.

The same lines, all four kinds, are saved to a file for the rest of the
day. Every time you start the viewer that day adds to the same file, and the
first start on a later day deletes it, so no log is kept for more than a
day. The file's location is shown at the bottom of the window:

- Windows: `%LOCALAPPDATA%\3D Review\logs\3d-review-<date>.log`
- macOS: `~/Library/Logs/3D Review/3d-review-<date>.log`

The log file is a good thing to attach when you
[report an issue](#help).

## Remember settings

While **Remember Settings** is on, the choices you make in the tools' options
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

Turning **Remember Settings** off does not change anything on screen. It only
means the next start begins from the usual settings again.

The settings are saved in a small text file in your user settings folder,
next to the file that remembers where the window was:

- Windows: `%APPDATA%\3D Review\settings.cfg`
- macOS: `~/Library/Application Support/3D Review/settings.cfg`
