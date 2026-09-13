# Keyboard and mouse

File commands use the key your computer normally uses for them: `Ctrl` on
Windows and `Cmd` on macOS. We call that the primary modifier, and the labels
on screen show whichever one applies to you.

## Mouse

**3D viewport** - left-drag turns the camera around the model. Right-drag
looks around, and while you hold it you can fly with `W` `A` `S` `D` and `Q`
`E`; the wheel changes the flying speed. Middle-drag slides the view, and so
does Shift with right-drag. The wheel zooms, and so does Alt with right-drag.
Double-clicking the empty view opens a file.

**UV and Texture viewports** - left-drag slides, right-drag zooms, and the
wheel zooms.

Pinching on a trackpad zooms in all three.

Right-click any tool button in the toolbar or status bar to open its options
panel.

## Keys

| Key | What it does |
| --- | --- |
| `` ` `` | Show or hide the wireframe on top of the model |
| `1` / `2` / `3` | Wireframe only / Unlit / Shaded |
| `I` | Show or hide the stats card |
| `G` | Show or hide the grid |
| `F` | Frame the model, or the selected part |
| `R` | Put the camera back where it started (in UV and Tex, refit the image) |
| `W` `A` `S` `D` `Q` `E` | Fly the camera (while the right mouse button is held) |
| `Space` | Play or pause the selected animation |
| `,` / `.` | Step one frame back or forward |
| `X` | In the Opt overlay, swap which model is solid |
| `F1` | Open this manual, at the page for whatever is under the pointer |
| `Esc` | Clear the selection |
| Primary + `N` | Start over with an empty viewer |
| Primary + `O` | Open a model |
| Primary + `Z` | Undo |
| Primary + `Y`, or Primary + Shift + `Z` | Redo |

## What undo covers

Undo covers the things you *edit*: what is selected in the Outliner, which
parts are hidden, material settings, which texture is in which slot, the
texture pool, and the list of operations in Opt. Dragging a slider or a color
picker counts as one step, however long you drag.

Undo does not touch how you are *looking* at the model: the camera, the grid,
the shading mode, anti-aliasing, ambient occlusion, tone mapping, the
environment, the UV and Tex views, and animation playback.
