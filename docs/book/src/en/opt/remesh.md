# Remeshing a model

**Remesh** replaces a model's topology with a new one: evenly sized triangles,
sized to follow the shape's own curves, the way an artist retopologizing by hand
would lay them.

That makes it the opposite of [Reduce](operations.md), which takes the triangles
you have and removes some of them. Remesh keeps none of the original layout.
What comes back is a surface of its own, with the materials, texture layout and
colors copied across from the old one.

## When to remesh

Reach for it when the *shape* is right but the triangles are not:

- a 3D scan, which is a fine mesh of tiny, badly-shaped triangles with no edge
  loops anywhere;
- a CAD or photogrammetry export, which triangulates however its own tessellator
  felt like;
- a sculpt, which has far more detail than the shape needs;
- anything you want to animate, since a deforming surface needs edges that run
  along the shape rather than across it.

If the triangles are already sensible and you only want fewer of them, Reduce or
Generate LODs is the better tool: they are faster and they keep the texture
layout exactly.

## What it gives you

Triangles. An even spread of them, at a density that follows how tightly the
shape turns, with your borders and creases kept where they were.

Quads are a planned addition rather than a missing feature: the machinery that
would lay them out is the same machinery that would make the triangles line up
into rows, and neither is here yet. What you get today is even in *density* and
unstructured in *layout* - the faces are the right size everywhere, but they do
not form the tidy grid a by-hand retopology has.

## How it works

Worth knowing, because it explains what the settings do and what the operation
will and will not do to a model.

1. It measures how tightly the surface turns at every point, and from that works
   out how big a face should be there. That is the [face size
   field](#vary-face-size).
2. It finds the edges that have to survive - open borders, and creases if you
   asked for them - and walks them into chains, then puts points along each
   chain at the spacing the field asked for. **Borders are decided first**,
   before anything else gets a point, which is why an outline comes back as one
   clean loop.
3. It scatters the remaining points over the surface at that same density, then
   shuffles them until each sits in the middle of its own patch.
4. It **merges the original mesh down** so that each patch becomes one point.

The fourth step is the one that matters most. The new surface is the old one
with edges taken out of it - never a fresh surface laid over the top. A merge
can only ever *remove* an edge from a mesh that was already whole, so it cannot
punch a hole, drop a border, or tear a thin part off. A leaf one quad wide comes
back as a leaf.

Where merging a patch all the way down would break the surface, the operation
stops and leaves the extra points in place. The result is very slightly denser
there than you asked for, which is the trade being made on purpose.

## How dense

Two ways to say it:

- **Ratio of current** - a share of each object's *current* triangle count. This
  is per object, so every object keeps its relative level of detail, which is
  what you want when you are rebuilding a whole asset in one go. The share is
  read against triangles, so it matches the Tris figure on the stats card.
- **Face count** - one budget for the whole selection, shared out between the
  objects by surface area. That is the only split that gives the same face size
  everywhere, which is what "3000 faces for this asset" means. An object with its
  own [override](overrides.md) takes exactly the count it asks for and leaves the
  shared pool, so pinning one object does not quietly re-weight the rest.

**The number is the number you get**, within a few percent, and it is worked out
rather than searched for: the face count you ask for is turned into a point count
by a piece of arithmetic that holds for any surface, and that many points are
placed. Nothing is rebuilt twice to creep up on it.

The few percent is the one thing that can move it. Where merging a patch all the
way down would break the surface the operation leaves the extra points in place,
so a model with a lot of awkward geometry comes back slightly denser than asked.
It is reported when it is more than a fraction of the model.

**A rebuild can only ever make a model simpler.** It works by merging the mesh
you gave it, so asking for more faces than the object already has will not add
any - it comes back at about its current density, and says so. If you need more
detail than the source holds, that is a subdivision, which this is not.

Very small objects are left alone - below about sixteen triangles there is no
surface to work from, and you are told which objects were skipped.

## Vary face size

Left at 0, every face comes out the same size, and a broad flat panel gets as
many of them as a tight rounded edge. That is rarely what you want: the flat part
needs four faces and the curve needs forty.

**Vary face size** moves the budget instead of raising it. The surface is
measured for how much it *turns* over the distance one face spans, and faces are
made smaller where it turns and larger where it does not - so the count stays put
and the detail moves to where it does something. On a stone pedestal at the
default setting the flat top goes from an even grid of twelve-millimetre faces to
a coarse one, and the rounded rim gains the faces it gave up, for the same total.

Three things it is careful about, all of which matter on real assets:

- **Bumps smaller than a face are not detail.** A sculpt or a scan is rough
  everywhere at the scale of one source triangle; if that counted, the whole
  object would read as detailed and nothing would be gained. What is measured is
  the turn across a whole face, so roughness finer than that - which no face size
  could have captured anyway - is ignored, while a fillet or a rim is not.
- **A face is never asked to be finer than the triangles underneath it.** The
  rebuild has to follow the surface it is given, and it cannot resolve something
  the original does not.
- **What counts is the tightest direction, not the average one.** A branch or a
  pipe turns right around its girth and not at all along its length, and a face
  has to be small enough for the tight direction; the other one is free. Reading
  the average instead is what used to leave a tapering branch at one face size
  from trunk to tip.

The two ends of the slider are both real rules rather than arbitrary amounts. At
the default of 0.5, face size follows the square root of how tightly the surface
turns, which is the setting that spends the same amount of *error* everywhere - a
face of twice the size bulges away from a curve by four times as much, so the
square root is what balances it. At 1 face size follows the turn directly, so
every face turns through the same angle. That is what a hand retopology looks
like, and it is much stronger: a branch of half the thickness gets faces of half
the size rather than of seven tenths.

How much it varies depends on the shape, not only on the setting. On a stone
pedestal - a broad flat top with a rounded rim - there is a lot for it to find;
on something evenly curved all over, a sphere say, there is nothing to move and
the setting does nothing at all. That is the field working, not failing.

## Sharp edges and open borders

**Keep sharp edges** lays the new edges along the model's own creases instead of
running the pattern straight over them. Turn it on for hard-surface and CAD
parts, where a rebuilt surface that rounds off a 90-degree corner is useless;
leave it off for organic shapes, where it only breaks up the flow. **Sharp
above** is how sharp a fold has to be before it counts.

**Follow open borders** is on by default and holds the new edges against any open
edge, so a border comes back as one clean loop rather than a ragged fringe.
Leave it on unless you have a reason not to: it is what keeps a leaf, a sheet of
cloth or a wall with no thickness looking like itself.

**Smoothing** is how many rounds of evening-out to run over the result. Two is a
good default: a little makes the faces more uniform, a lot rounds off detail. It
never moves a point on a border, so no amount of it will soften an outline.

## How long it takes

Objects are rebuilt **on every core at once** - one object per core, claimed as
each core comes free, so the long ones are not stuck behind each other. Inside
one object the work is spread across cores too, so a single large model is not
left on one.

The rebuild is **the same every time**, at any core count, on any machine. That
is a property of how it is built rather than a setting you can lose: every stage
either has no order to it or breaks ties by a fixed rule, so there is nothing for
the scheduling to change. There used to be a **Reproducible** switch here,
trading speed for repeatability. It is gone because there is no longer anything
to trade.

Nothing is rebuilt twice to land on the face count, either - it is worked out in
advance. A run costs what it costs once.

The notice at the bottom of the window names the step the run is on, and for a
rebuild it names each object as it finishes and how many are done
(`Remesh: leaf_012_mesh (3 of 13)`). If that line is moving, so is the run. To
make it finish sooner, ask for fewer faces or [exclude](overrides.md) the objects
you do not need rebuilt.

**Changing a setting while a rebuild is going stops it**, and the new settings go
in straight away. You do not wait out a result you have already moved past.

## Materials, UVs and colors come back by projection

The rebuilt surface shares no point with the original, so nothing can simply be
carried over. Instead every new face and corner is looked up on the old surface -
"what is directly underneath this?" - and reads its material, texture
coordinates, color sets and shading direction from there.

Two things follow that are worth knowing.

A face takes the material of whatever is under its *middle*, so a face that
straddles a boundary between two materials lands in the one it mostly covers.
Boundaries therefore end up on the new edges nearest to where they were, and they
stay crisp.

The same is true of texture seams and hard edges: a corner reads from its own
side of a seam, never across it, so an island boundary lands on a new edge rather
than smearing across a face. That is why a rebuilt model can carry a few more
points than its face count implies - the extra ones are the seam corners, exactly
as they were in the original.

## What is lost

Everything tied to the old points, because the old points are gone:

- **bones and shape keys**. An object that bends is skipped entirely and left as
  it is, with a note saying so. Remesh is for still objects.
- **per-point creases** used by subdivision.
- **the original polygon edges**, along with any per-edge data the file carried.

Tangents are rebuilt from the new surface, as they are after any operation that
changes the shape.

## Where it goes in the list

Above the reorder operations, and **below** any simplifier whose result you want
to rebuild. A Reduce or a Generate LODs running after a Remesh takes the even
surface it just made and thins it out again, which is rarely what you meant.
Remesh *after* a simplify is the usual order, and a Weld Vertices or an Optimize
Vertex Fetch below it changes nothing about the layout.

A [Shrinkwrap](shrinkwrap.md) above a Remesh is the pairing both operations exist
for: the wrap fuses a pile of overlapping parts into one skin, and what it
produces is dense and irregular, so the Remesh is what makes it even.

Baking ambient occlusion works in either order, though below the Remesh is
usually what you mean, so the bake describes the surface you are keeping.
