# Wrapping a model in one skin

**Shrinkwrap** replaces each object with a single closed skin that hugs it. The
shape is measured onto a three-dimensional grid - for every point, how far it is
from the surface and whether it is inside or out - and a new surface is pulled
back out of the result.

What comes out is one watertight piece, whatever went in.

There are two ways of making that skin, chosen with **Method** - see
the next section. Everything else on this page applies to both unless it says
otherwise.

## When to wrap

- **A kitbash.** Most game props are a pile of overlapping parts: a crate is six
  planks, four corner brackets and eight nails, each a closed box of its own,
  all pushed into each other. A wrap fuses them into one solid.
- **Before a Remesh.** What a wrap produces is one skin, but a dense and
  irregular one - it is a grid pulled out of a measurement, not a layout. A
  [Remesh](remesh.md) below it turns that into evenly sized triangles at the
  density you asked for.
- **A collision shape.** A wrap with a negative offset sits just inside the
  original, which is what a physics shape wants.
- **A distant LOD.** At a low detail setting the wrap keeps the silhouette and
  throws away everything you could not see from far away anyway.
- **A model with holes, or with parts turned inside out.** Neither bothers it -
  see below.

## Method

- **Distance field** (the default) measures, for every point of the grid, how
  far it is from the surface and which side of it it is on, then pulls the skin
  out where that crosses zero. It is the method that does not mind a model with
  holes or parts turned inside out (see further down for why),
  and the only one with an **Offset**. Anything thinner than one grid step - a
  leaf card, a flag, a sheet of paper - is lost.
- **Voxel** comes from meshoptimizer. It marks which grid cells the surface
  passes through and builds the skin around them. Thin sheets **survive at any
  detail**: a leaf comes back as a very thin, two-sided surface rather than
  disappearing. **Fit to surface** (on by default) moves the new points onto
  the original so edges and corners stay crisp. It is also several times faster
  and makes far fewer triangles for the same detail - on a real prop, about a
  quarter. What it cannot do is look past holes: it fills an object solid by
  flooding in from outside, so a gap wider than a grid step lets the outside in
  and the inside surfaces are kept.

Pick Voxel for foliage, cloth, capes and anything else built from thin sheets,
or when speed matters. Pick Distance field for a model with holes or inside-out
parts, or when you need an Offset.

A voxel skin's thin parts are two surfaces lying back to back. They draw
correctly as long as back faces are hidden, which is the viewer's default; with
**Backface Rendering** turned on they flicker.

### Voxel settings

- **Detail** - grid steps across the longest side, from 4 to 256.
- **Two-sided skin** - wraps every surface in its own thin skin instead of
  filling the object solid, which keeps inside surfaces. It doubles the
  triangles and the skin can pass through itself, so it is off by default.
- **Triangles** - the voxel method simplifies its own skin before the materials,
  texture layout and colors are copied back. That order matters: once the
  texture seams are copied onto the skin they hold a simplifier back, which is
  why a Reduce placed below a wrap does worse. By default it keeps as many
  triangles as the object had (**Share** 1.0); **Count** asks for a number per
  object, and **Keep all** skips it. Simplifying keeps the folded rims of thin
  sheets (see [Preserve folds](simplify.md)). **Even triangles** trades a little
  accuracy for better-shaped triangles.

Voxel starts with its normals **worked out fresh** rather than read off the
original: where a thin sheet's two sides lie a hair apart, reading the original
would give both sides the same shading. You can switch it back.

## Detail

**Detail** is how many grid steps fit across the object's longest side, and it is
the only real knob. Everything follows from it: how much of the shape survives,
how wide a gap gets bridged, and how long the wrap takes.

- Low (32-48) rounds the object off and closes almost any gap. Good for a
  collision shape or a distant level of detail.
- The default (64) is the useful setting for a prop: the silhouette and the big
  shapes survive, the fiddly bits do not.
- High (128+) keeps fine detail, and costs more than it looks like it should.

Cost is the thing to watch. The number of grid steps cubes, and the new surface
comes out at roughly six triangles per step it crosses - so a real prop wrapped
at 48 is about 170,000 triangles, at 64 about 300,000, and at 128 about 1.3
**million**. Past that you are paying for detail the Remesh below is about to
throw away again, which is the pairing failing rather than improving.

Wrapping is not the finished result, which is why the count matters less than it
sounds: the step below it replaces every triangle the wrap emits. If a wrap needs
more memory than a run is worth, it quietly drops to a coarser setting and tells
you.

## Offset

**Offset** pushes the skin away from the object, in meters. Zero sits on the
surface.

A small positive offset is the usual fix for a wrap that came out with holes in
it: bridging gaps is exactly what pushing the surface outward does. A negative
offset pulls the skin inside the original, which is what a collision shape wants
- it should never poke through what the player sees.

**Keep largest piece only** is on by default and throws away every closed piece
but the biggest. Wrapping a messy object routinely leaves small blobs around
stray specks of geometry, and none of them is wanted.

## Why holes and inside-out parts do not matter

The hard question a wrap has to answer is not "how far is the surface" but
"which side of it am I on", and the obvious ways of answering it need a model
that is already tidy. Counting how many times a ray out of the point crosses the
surface needs a closed one - the exact thing that is missing. Reading the nearest
triangle's facing puts the boundary wherever the nearest scrap of geometry
happens to point, so one inverted part turns a whole region inside out.

This uses the **winding number** instead: how much of the surface, in total,
wraps around the point. It is 1 inside a closed shape and 0 outside, 2 where two
parts overlap - which still reads as solid, which is the whole trick - and it
sags gently rather than flipping where the model has a hole. A model with a
missing face, a part turned inside out or fifty overlapping boxes all wrap
correctly, without being repaired first.

## What comes out

Triangles - an even, gridded field of them, many more than the original had. The
wrap is not a tidy-up on its own; it is the step that makes a tidy-up possible.
In practice it goes **above** a [Remesh](remesh.md), which replaces its triangles
with an even quad layout, or above a [Reduce](simplify.md) if triangles are what
you want.

Materials, texture layout and colors are read back off the original and copied
across, the same way [a remesh does it](remesh.md). So is the shading, unless you
set **Normals** to **Worked out fresh**, which builds it from the skin itself -
often the better choice for a wrap, since where overlapping parts were fused the
original has no sensible shading to read ([more](normals.md)). What cannot come with it is
anything tied to the old points: bones, shape keys and per-point creases. An
object that bends is skipped and left as it is, with a note saying so.

## Where it goes in the list

First, or near it. Everything else in the list works better on a closed object
than on a kitbash, and the wrap throws away the fine detail that an operation
running before it would have spent its time on.

The one order worth spelling out is **Shrinkwrap, then Remesh**. That pair is the
reason both operations exist in the same tool: the wrap turns a pile of
overlapping parts into a single skin, and the remesh turns that skin's dense,
irregular grid into the even surface you actually wanted.
