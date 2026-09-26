# Simplification settings

Simplifying means removing triangles while keeping the shape as close to the
original as possible. **Reduce** and **Generate LODs** both do this, and they
share the same settings.

## Algorithm

- **Standard** - keeps the surface connected the way it was, and only watches
  how far the surface moves.
- **Preserve Attributes** - also tries to keep the shading directions, the
  texture layout and the colors from drifting. You can say how much each one
  matters.
- **Sloppy** - does not bother keeping the surface connected. It is much
  faster and nearly always reaches the target, but it can close up holes and
  merge nearby pieces, so it suits the smallest, most distant levels.

## Target

You give a share of triangles to keep, and a limit on how far the surface may
move. The simplifier stops early rather than go past that limit. So if a model
cannot be reduced that far without visible damage, you get back a model with
more triangles than you asked for, rather than a damaged one.

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
  shape itself. Without it, a small area where those change quickly (a busy
  texture seam, say) can hold the whole model back and push the error figure far
  above how far the surface really moved. With it, the simplifier spends its
  effort more evenly and the error figure reads closer to a real distance.

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
