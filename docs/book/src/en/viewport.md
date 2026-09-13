# The 3D viewport

The viewport is the big area where your model is drawn. You look at it through
a camera that circles around the model, and you can move that camera with the
mouse.

## Moving the camera

- **Left-drag** turns the camera around the model. This is called orbiting.
- **Middle-drag** slides the view sideways or up and down. Shift plus right-drag
  does the same. This is called panning.
- **The mouse wheel** moves closer or further away. Alt plus right-drag does the
  same. This is called zooming.

Camera moves glide over about a third of a second instead of jumping, so it
always stays clear which way the model turned.

**Flying.** Hold the **right mouse button** and the camera looks around instead
of orbiting, just like in Unity or Unreal. While you hold it, `W` `A` `S` `D`
move you forward, left, back and right, `Q` and `E` move you down and up, and
the wheel changes how fast you fly. The speed matches the size of what you are
looking at, so a tiny prop and a huge level both feel the same to fly through.

## Framing

Framing means moving the camera so the thing you care about fills the view.

- `F` frames the whole model. If you have selected a part, it frames that part
  instead; press `F` again to go back to the whole model.
- `R` returns the camera to where it started.

While an animation is selected, `F` frames the whole area the animation moves
through, not just the pose you happen to be on. See
[Skinning and animation](animation.md).

## Display toggles

The group of buttons on the right of the toolbar turns extra helpers on and
off:

- the floor grid with its axis lines,
- the axis gizmo, a small marker whose arms you can click to look straight down
  an axis,
- the pivot marker, which shows the model's own center point,
- the [bounding box](panels/bounding-box.md), a box drawn tightly around the
  model with its size on the edges,
- the [skeleton overlay](panels/skeleton.md), for models that have bones.

## Background

The background behind the model is a button in the status bar. Click it to step
through black, three shades of grey, white, and a soft gradient. See
[Background](panels/background.md).
