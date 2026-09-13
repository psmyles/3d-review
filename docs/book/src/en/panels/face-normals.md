# Face Normals

Every face of a model has a front. The face normal is an imaginary arrow
pointing straight out of that front. This overlay draws one short line from
the middle of each face along its normal, so you can see which way every face
is pointing.

## Settings

**Length** - how long each line is, in meters. The change shows right away.

**Color** - the color of the lines. This changes right away too.

## What it is for

Face normals are the fastest way to find a face that is flipped inside out.
Such a face has its line pointing into the model instead of out of it, and the
face itself looks dark or vanishes entirely when backface rendering is off
(which it is to begin with).

To see the directions used for smooth shading, which are stored on the points
rather than the faces, see [Vertex Normals](vertex-normals.md).

## Notes

The lines are built when you switch the overlay on and thrown away when you
switch it off, so the plain view stays light. Changing the length or color
rebuilds them on the spot.

On a model with bones, the lines move with the model, so they stay attached
during an animation instead of being left behind in the resting pose.
