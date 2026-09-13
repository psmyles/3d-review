# The 3D viewport

An orbit camera with framing, panning and zooming, in perspective or orthographic
projection.

## Moving the camera

Left-drag orbits. Middle-drag pans, as does Shift with the right button. The
wheel zooms, and so does Alt with the right button. Camera moves ease over a
third of a second rather than snapping, so it stays clear which way the model
turned.

Holding the right mouse button looks around instead of orbiting, and - as in
Unity and Unreal - arms a flycam: `W` `A` `S` `D` move along the view, `Q` and `E`
drop and lift, and the wheel sets how fast. The speed scales with the size of what
is framed, so a 2 cm prop and a 200 m level both handle the same.

## Framing

`F` frames the model, or the selection when one is active; pressing it again goes
back to the whole model. `R` returns to the home framing. While an animation clip
is selected, `F` frames that clip's whole range of motion rather than the frame
you happen to be on - see [Skinning and animation](animation.md).

## Display toggles

The toolbar's right-hand group holds the display toggles:

- the floor grid with axes,
- the axis gizmo, whose axes can be clicked to snap the camera to them,
- the object pivot marker,
- the [bounding box](panels/bounding-box.md),
- the [skeleton overlay](panels/skeleton.md) for rigged models.

## Background

The viewport background is a status-bar button: black, 25/50/75% grey, white, or
a gradient. See [Background](panels/background.md).
