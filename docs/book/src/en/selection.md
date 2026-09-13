# Selecting in the viewport

Clicking a part of the model in the viewport is the quickest way to find out
what it is. On a file with a thousand parts it is much quicker than hunting for
the right row in the [Outliner](outliner-inspector.md).

Clicking is not always what you want, though. Most of the time you are turning
the camera, and a click that selected something every time you let go of a drag
would be a nuisance. So selecting is a **mode** you switch on.

## The two modes

The mouse button on the right of the toolbar switches between them, and so does
the `Q` key.

**View** is the normal mode, and how the viewer has always behaved. The left
mouse button turns the camera and nothing else.

**Select** adds clicking. Left-drag still turns the camera exactly as before -
only a click that does not move the mouse counts as a click. So you can orbit,
let go, and nothing is selected; or click without moving, and you select what is
under the pointer.

When you turn Select on, whatever is under the pointer lights up as you move
over it. That highlight is how you know what a click would pick before you
commit to it.

`Q` also moves the camera downward while you hold the right mouse button, as
part of flying. That still works: while the right button is down, `Q` flies, and
the rest of the time it switches modes.

## Selecting several parts

- **Click** selects one part, replacing whatever was selected.
- **Ctrl-click** adds the part, or removes it if it was already selected.
- **Shift-click** adds the part and leaves everything else selected.
- **Clicking empty space** clears the selection. Holding Ctrl or Shift while you
  click empty space does nothing, so a missed click in the middle of building up
  a selection cannot wipe it out.
- **`Esc`** clears the selection from anywhere.

On Mac, use `Cmd` wherever this says `Ctrl`.

The Outliner takes the same two modifiers, so you can start a selection in one
and finish it in the other. There is one difference, because a list has an order
that a 3D view does not: in the Outliner, Shift-click takes every row **between**
the last one you clicked and this one, which is how you grab a run of parts in
one go.

Everything you select is shown tinted in the viewport for as long as it stays
selected, and every row of it is lit in the Outliner. With more than one part
selected the Inspector shows a summary - how many parts, how many triangles, and
their names - instead of the details of any one of them.

Selecting is undoable. `Ctrl+Z` steps back through the selections you made, and
`Ctrl+Y` forward again.

## Bones

When the [skeleton overlay](panels/skeleton.md) is on, Select picks **bones**
instead of parts of the mesh. This is deliberate: the reason to have the
skeleton drawn is that you are looking at the rig, and the mesh is in the way of
it.

Bones are picked by their drawn shape, with a little room around thin ones so a
finger bone is still easy to hit. A bone selected this way is the same selection
the Outliner makes, so it drives the
[Skin Weights](panels/material-mode.md) heat map in the same way.

Turn the skeleton overlay off and Select goes back to picking the mesh.

## Animated models

While an animation is selected, clicking picks the model **where it is drawn**,
not where its bind pose sits. A raised arm is selected by clicking the raised
arm.

The highlight that follows the pointer is switched off while an animation is
actually playing, since the shape under the pointer changes faster than it could
usefully be shown.

## Hidden parts, isolate, and Opt

A part hidden with the eye icon in the Outliner cannot be clicked - it is not
there to click. While a selection is isolated, only the isolated parts can be
picked.

Selecting works in the [Opt workspace](opt/index.md) too, on either side of the
split view, and picks the part in the view you clicked in. That is how you reach
[per-object settings](opt/overrides.md) without going through the Outliner. Opt
always draws the bind pose, so a pick there ignores the animation.

Selecting does not work in the UV or Texture workspaces, which have nothing to
pick.
