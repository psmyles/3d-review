# Wrapping a model in one skin

**Shrinkwrap** replaces each object with a single closed skin that hugs it. The
shape is measured onto a three-dimensional grid - for every point, how far it is
from the surface and whether it is inside or out - and a new surface is pulled
back out of the result.

What comes out is one watertight piece, whatever went in.

## When to wrap

- **A kitbash.** Most game props are a pile of overlapping parts: a crate is six
  planks, four corner brackets and eight nails, each a closed box of its own,
  all pushed into each other. A wrap fuses them into one solid.
- **Before Only quads.** [Remesh](remesh.md)'s Only quads needs one closed skin
  and refuses a kitbash. A Shrinkwrap above the Remesh is the fix: wrap first,
  then rebuild the wrapped skin out of quads.
- **A collision shape.** A wrap with a negative offset sits just inside the
  original, which is what a physics shape wants.
- **A distant LOD.** At a low detail setting the wrap keeps the silhouette and
  throws away everything you could not see from far away anyway.
- **A model with holes, or with parts turned inside out.** Neither bothers it -
  see below.

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
**million**. Past that the shell is dense and irregular enough that Only quads
gives up on it, which is the pairing failing rather than improving.

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
across, the same way [a remesh does it](remesh.md). What cannot come with it is
anything tied to the old points: bones, shape keys and per-point creases. An
object that bends is skipped and left as it is, with a note saying so.

## Where it goes in the list

First, or near it. Everything else in the list works better on a closed object
than on a kitbash, and the wrap throws away the fine detail that an operation
running before it would have spent its time on.

The one order worth spelling out is **Shrinkwrap, then Remesh with Only quads**.
That pair is the reason both operations exist in the same tool: the wrap turns an
object no quad solver will touch into one it will, and the remesh turns the
wrap's grid of triangles into the quad layout you actually wanted.
