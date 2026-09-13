# Vertex Normals

Each point of a model stores a direction, called its normal, that tells the
lighting which way the surface faces at that spot. This overlay draws one
short line at every point along that direction.

## Settings

**Length** - how long each line is, in meters. The change shows right away.

**Color** - the color of the lines. This changes right away too.

## What it is for

Smooth shading is built from these directions, so this is the overlay that
explains a shading oddity the shape itself does not account for. At a sharp
edge you will see two lines fanning apart from the same corner. At a smoothed
edge you will see a single, averaged line.

If a model was saved with a separate normal for every face, every corner
carries its own line, and the `Vtx Splits` row on the
[stats card](../stats.md) will read several hundred percent.

## Notes

Like the other line overlays, the lines only exist while the overlay is on,
and they move with a model that has bones.
