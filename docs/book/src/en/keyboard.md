# Keyboard and mouse

File commands use the platform's own modifier: `Ctrl` on Windows, `Cmd` on
macOS. The on-screen labels follow the platform.

## Mouse

**3D viewport** - left-drag orbits, right-drag looks around (and flies with
`WASD` and `QE`; the wheel sets the fly speed while it is held), middle-drag
pans, as does Shift with right-drag. Alt with right-drag zooms, and so does the
wheel. Double-clicking the empty viewport opens a file.

**UV and Texture viewports** - left-drag pans, right-drag zooms, wheel zooms.

Trackpad pinch zooms in all three viewports.

Right-click any toolbar or status-bar tool button to open its options panel.

## Keys

| Key | Action |
| --- | --- |
| `` ` `` | Toggle the wireframe overlay |
| `1` / `2` / `3` | Wireframe only / Unlit / Shaded |
| `I` | Toggle the stats display |
| `G` | Toggle the grid |
| `F` | Frame the model, or the selection |
| `R` | Reset the camera (refits the image in UV and Tex) |
| `W` `A` `S` `D` `Q` `E` | Fly the camera (while the right button is held) |
| `Space` | Play / pause the selected clip |
| `,` / `.` | Previous / next frame of the selected clip |
| `X` | Swap source and processed in the Opt overlay |
| `F1` | Open this manual, at the panel under the pointer |
| `Esc` | Clear the selection |
| Primary + `N` | Reset to the start state |
| Primary + `O` | Open a model |
| Primary + `Z` | Undo |
| Primary + `Y`, or Primary + Shift + `Z` | Redo |

## What undo covers

Undo covers **document edits**: Outliner selection, mesh visibility, material
parameters, texture slot bindings, the texture pool, and the Opt operation stack.
A continuous slider or color-picker drag collapses into a single step.

**View** state is deliberately left out: the camera, grid, shading mode,
antialiasing, ambient occlusion, tone mapping, environment, the UV and Tex
viewports, and animation playback.
