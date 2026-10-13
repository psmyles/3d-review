# Simplification settings

Simplifying means removing triangles while keeping the shape as close to the
original as possible. **Reduce** and **Generate LODs** both do this, and they
share the same settings.

## Algorithm

- **Preserve Attributes** (the default) - keeps the surface connected the way
  it was, and weighs the shading directions, the texture layout and the colors
  as well as the shape. It also moves the points it keeps to where they best
  fit the original surface, and works out their shading direction, texture
  position and color again there. You can say how much each kind of data
  matters.
- **Standard** - keeps the surface connected, only watches how far the surface
  moves, and leaves every point it keeps exactly as it was in the original.
- **Sloppy** - does not bother keeping the surface connected. It is much
  faster and nearly always reaches the target, but it can close up holes and
  merge nearby pieces, so it suits the smallest, most distant levels.

### Why the shading stays clean

A point's shading direction describes the few small faces around it. Most
models have softened edges: the points along a bevel lean into it, so the
corner catches the light. When a simplifier removes the points in the middle of
a flat face, the faces that are left stretch from bevel to bevel, and if the
points keep their old directions, that lean spreads across the whole face as
dark and light patches. This is what **Standard** does, and why it is no longer
the default. **Preserve Attributes** works out each kept point's direction for
the larger faces it now belongs to, so a flat face stays flat. It also keeps
the extra points that hold a soft edge in place.

Wherever the surface is split - an open edge, a texture seam, a hard edge, a
border between two materials - a point either stays exactly where it is or
moves together with every copy of itself, so none of those can open up.

## Target

You give a share of triangles to keep, and a limit on how far the surface may
move. The share is counted per object: all of an object's materials are
simplified together, and the triangles go wherever the shape needs them most.
A thin, finely detailed trim and a broad panel that share one object are not
each cut to the same share. The panel, which loses nothing by giving up
triangles, pays for most of it.

The simplifier stops early rather than go past the limit. So if a model cannot
be reduced that far without visible damage, you get back a model with more
triangles than you asked for, rather than a damaged one. The limit is on the
shape only: how much the shading or texture layout changes does not count
against it, so **Preserve Attributes** reaches the same triangle count as
**Standard** would. The error each level shows on the stats card is measured
the same way.

## Flags

Lock the border (never move points on an open edge), measure the error as a
real distance instead of a fraction, remove loose pieces as it goes,
regularize (keep triangles evenly shaped, with a lighter version), and allow
collapses across seams.

Two more are newer and marked experimental by the library they come from, so
they are off unless you tick them:

- **Preserve folds** - keeps the rim of a double-sided surface from wearing
  away. A leaf card or a cloth panel modelled as two faces back to back has a
  sharp fold along its edge, and a plain simplify tends to eat into it. This
  keeps it, at a small cost in speed. It matters most below a
  [Shrinkwrap](shrinkwrap.md) using the Voxel method, which makes thin parts
  exactly that way.
- **Clamp attribute error** - only offered with **Preserve Attributes**. It
  stops the shading, texture layout and colors from counting for more than the
  shape itself when the simplifier chooses what to remove. Without it, a small
  area where those change quickly (a busy texture seam, say) can keep more than
  its share of the triangles. With it, the simplifier spends them more evenly.

After a **Reduce**, the stats card shows the error it reached on the base model
too, not only on generated levels, so you can compare settings directly.

## When a simplify barely removes anything

This is usually seams, not a bug.

Remember that every face corner arrives as its own point. On a model whose
shading directions or texture layout differ at every corner, such as a 3D scan
that was saved with a separate normal per face, *every* edge looks like a
seam, and a simplifier that keeps the surface connected is not allowed to fold
across one.

To give you an idea: a real game model joins up from 369,000 points to 108,000
and hits its targets with the normal settings, while such a scan goes from
249,882 triangles to 249,880 until either a Weld Vertices operation that
ignores normals, or the "collapse across seams" flag, frees it.

The run notices when this happens and tells you. The normal settings are right
for ordinary game models, which is what this tool is for, so the fix is to add
a [Weld Vertices](operations.md) operation or tick the flag, not to loosen the
defaults.
