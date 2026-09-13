# Vertex Normals

Draws one line per vertex, along that vertex's shading normal.

## Settings

**Length** - how far each line extends, in meters. Updates live.

**Color** - the line color. Also live.

## What it is for

Vertex normals are what smoothing actually uses, so this is the view that
explains a shading artifact the geometry does not account for. A hard edge shows
as two lines diverging at one corner; a smoothed edge shows one averaged line.

Where a model was exported with per-face normals, every corner carries its own,
and the [stats panel](../stats.md)'s `Vtx Splits` row will read several hundred
percent.

## Notes

Like the other line overlays, the buffer exists only while the overlay is on, and
it deforms with a skinned mesh.
