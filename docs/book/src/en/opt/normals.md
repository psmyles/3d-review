# Normals and tangents

A model's **normals** are the directions its surface faces at each point. They
decide how light falls across it: whether an edge reads as a sharp corner or
as a smooth curve. **Tangents** ride along with them and tell a normal map
which way is "across" the texture.

Neither is part of the shape. Both are worked out from it, which is why an
operation that changes the shape has to work them out again.

## Recalculate Normals

**Recalculate Normals** throws the model's normals away and works out new ones
from the shape itself.

- **Crease angle** - where two faces meet at a sharper angle than this, the
  edge stays hard; gentler edges are smoothed over. The default, 60 degrees,
  keeps a box's corners crisp and a cylinder's sides round. 180 smooths
  everything; a small angle makes nearly every edge hard, for a faceted look.
- **Smoothing** - relaxes the result over neighbouring faces. Zero, the
  default, leaves it exactly as worked out. A little is useful on a surface
  built from a grid, such as a [Shrinkwrap](shrinkwrap.md), where plain
  averaging still shows the small steps.

It works on each object as a whole, so the border between two materials on one
object is not made into a hard edge.

The hard and soft edge markings saved with the model are rewritten to match,
and any smoothing groups are dropped. Without that, another tool opening the
export could rebuild the old shading from the old markings and undo the
operation.

### When to use it

- **After a Weld that ignores normals.** Joining the hard edges of a flat-shaded
  model is what lets a [simplify](simplify.md) work across them, but it leaves
  the shading smooth everywhere. Recalculating afterwards puts back the edges
  that should be hard. Put it **below** the Reduce, so the simplifier is not held
  back by the edges it would put back.
- **A model with broken shading.** A scan exported with a separate normal per
  face, or a model whose normals were never set up, comes out cleanly shaded.

### What it leaves alone

- Objects that bend with **bones** are recalculated too. Their skin weights are
  copied across exactly wherever a point has to be split in two.
- Objects with **blend shapes** are skipped, with a note saying so. Their shapes
  store shading changes relative to the old normals, and new normals would
  contradict them.
- Objects excluded from the operation, as with every operation.

## Rebuilt surfaces

[Remesh](remesh.md) and [Shrinkwrap](shrinkwrap.md) both lay down a new surface,
and both offer a **Normals** choice for it:

- **From the original** - the default for both. Each new point reads the
  shading of the original surface underneath it, which keeps the artist's
  smooth and hard edges.
- **Worked out fresh** - the new surface's normals are built from its own shape,
  exactly as Recalculate Normals would, with the same crease angle and smoothing
  settings. Choose this where the original has no sensible shading to give: where
  a wrap fused overlapping parts, or where a thin sheet sits so close to another
  that a point can read the wrong side of it.

Working them out fresh looks at the whole object at once, so the border between
two materials is not made into a hard edge.

## Tangents

Whenever an operation changes the shape or the normals, the tangents are worked
out again when the run finishes. Nothing needs to be added to the list for that.

One thing is worth knowing. Where a model's texture layout is **mirrored** - the
left half of a face painted with the right half's texture flipped, which is how
most symmetric game models save texture space - a point on the mirror line needs
two tangents, one for each side. It is split into two points there, exactly as
an engine would split it. So a stack that changes the shape can show a few more
points on the stats card than the same stack did before, and they are real: the
export writes them, and a game would draw them.
