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

In [Select mode](selection.md) a left **click** - a press and release that does
not move - picks whatever is under the pointer, while left-**drag** still turns
the camera. Ctrl-click adds or removes a part, and Shift-click adds one.

**UV and Texture viewports** - left-drag slides, right-drag zooms, and the
wheel zooms.

Pinching on a trackpad zooms in all three.

Right-click any tool button in the toolbar or status bar to open its options
panel.

## Keys

### The view

- `` ` `` - show or hide the wireframe on top of the model.
- `1` / `2` / `3` - wireframe only, unlit, or shaded.
- `I` - show or hide the stats card.
- `G` - show or hide the grid.
- `F` - frame the model, or the selected part. In Aud, with a finding picked,
  frame the parts it marks.
- `Q` - switch between turning the camera and
  [selecting by clicking](selection.md). While the right mouse button is held it
  keeps its flying meaning instead, and moves you down.
- `R` - put the camera back where it started. In UV and Tex, refit the image.
- `W` `A` `S` `D` `Q` `E` - fly the camera, while the right mouse button is
  held.

### Animation

- `Space` - play or pause the selected animation.
- `,` / `.` - step one frame back or forward.

### Everything else

- `X` - in the Opt overlay, swap which model is solid.
- `F1` - open this manual, at the page for whatever is under the pointer.
- `Esc` - clear the selection, in the viewport or the Outliner. In UV it also
  takes away the texture behind the layout. In Aud it lets go of the picked
  finding.
- Primary + `N` - start over with an empty viewer.
- Primary + `O` - open a model.
- Primary + `Z` - undo.
- Primary + `Y`, or Primary + Shift + `Z` - redo.

## What undo covers

Undo covers the things you *edit*: what is selected - in the Outliner or by
clicking in the viewport, including selections of several parts at once - which
parts are hidden, material settings, which texture is in which slot, the texture
pool, the list of operations in Opt, and the audit profile in Aud. Dragging a
slider or a color picker
counts as one step, however long you drag.

Undo does not touch how you are *looking* at the model: the camera, the grid,
the shading mode, anti-aliasing, ambient occlusion, tone mapping, the
environment, the UV and Tex views, animation playback, whether clicking
selects, and which finding is picked in Aud.
