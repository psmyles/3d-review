# Camera

The Camera options set how fast the camera moves when you turn, zoom or fly,
and how wide the perspective camera sees. Right-click the perspective /
orthographic button on the right of the toolbar to open them. A left-click on
that button still switches between the two cameras.

## Settings

**Orbit Speed** - how far the camera turns as you drag: orbiting with the left
button, looking around with the right button, and dragging the axis gizmo. 1 is
the normal speed; 2 turns twice as far for the same drag, 0.5 half as far.

**Zoom Speed** - how far one step of the mouse wheel moves the camera closer or
further away. Alt plus right-drag and a trackpad pinch follow it too. It only
changes the 3D views; the [UV workspace](../uv.md) keeps its own zoom.

**Fly Speed** - how fast `W` `A` `S` `D`, `Q` and `E` move the camera while you
hold the right mouse button. Turning the wheel while you fly changes this same
setting, so the slider always shows the speed you are flying at. At 1 the
speed already matches the size of the model, so a tiny prop and a huge level
feel the same to fly through; this scales that.

### Perspective only

The window says which settings only apply to the perspective camera. Today
that is one:

**Field of View** - how wide the perspective camera sees, in degrees from the
top of the view to the bottom, from 10 to 120. It starts at 50. A small angle
flattens the model, like a long camera lens; a large one exaggerates depth,
like a wide one.

The model keeps its size on screen as you change it. The camera moves closer
for a wider angle and further away for a narrower one, so only the amount of
perspective changes. That is also why it does nothing to the orthographic
camera, which has no perspective to change, and the slider is greyed out
while that camera is in use.

## Notes

The settings apply to every 3D view, including both halves of the
[Opt comparison](../opt/comparison.md) when its cameras are not linked.

They are view settings, so undo does not step through them. With
[Remember Settings](../menu.md#remember-settings) on, they are kept for the
next time you start the viewer.

**Reset all** puts every setting back to where it started.
