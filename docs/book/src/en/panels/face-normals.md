# Face Normals

Draws one line per face, from the face centre along its normal.

## Settings

**Length** - how far each line extends, in meters. Updates live.

**Color** - the line color. Also live.

## What it is for

Face normals are the fastest way to find flipped faces: a face whose normal
points into the model instead of out of it shows its line going the wrong way,
and its surface goes dark or invisible with backface culling on (the default).

For a per-vertex read, which is what smoothing actually uses, see
[Vertex Normals](vertex-normals.md).

## Notes

The line buffer is built when the overlay turns on and freed when it turns off,
so the plain shaded view carries none of it. Changing the length or color rebuilds
it live.

On a skinned model the lines deform with the mesh, so they stay attached through
an animation rather than sitting on the bind pose.
