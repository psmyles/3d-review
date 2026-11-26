# Convex hulls for collision

**Convex Hull** replaces an object with its convex hull: the smallest shape
with no dents that holds all of it, like a sheet of plastic shrunk around it.
A physics engine wants exactly that for a collision mesh, because a convex
shape is the cheapest kind to test against, and most engines limit how many
vertices one may have.

## Which objects

Only the objects whose names match **Object names** become hulls; the rest
pass through untouched, so the operation can sit in any stack. To start with
the names are `*_hull` and Unreal's `UCX_*`, the usual names for a convex
collision mesh: make one in the modelling program as a copy of the part (or a
rough shape around it), name it so, and the operation does the rest. `*` makes
a hull of every object. `*` matches any run of characters, `?` any one, and
`[...]` any of the listed ones; separate patterns with commas.

A [per-object override](overrides.md) can give one object its own settings,
or leave it out.

## How many vertices

**Most vertices** is the limit: 32 to start with, the usual budget for a game's
collision hull; PhysX and Unreal take at most 255. When the full hull has more,
the operation builds it from the points that stick out furthest, one at a
time, until it has this many. Every point it leaves out is within a small
distance inside the hull, so the hull is a little smaller than the object
rather than bigger. The audit's [Collider vertex limit](../aud/checks.md#geometry)
check counts the same vertices.

## What it keeps

Materials, the texture layout and colors are copied onto the hull from the
nearest point of the object, as [Shrinkwrap](shrinkwrap.md) copies them. Its
faces are flat. A skinned object, or one with blend shapes, is left as it is:
no skin weight authored for the old surface fits the new one.

Put the operation above **Generate LODs** and every level shares one hull, or
leave the colliders out of the levels with an override; below it, each level's
hull is made from that level's own simplified object.
