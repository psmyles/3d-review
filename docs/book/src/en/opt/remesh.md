# Remeshing a model

**Remesh** throws away the model's topology and lays down a new surface in its
place: evenly sized faces that follow the shape's own curves, the way an artist
retopologizing by hand would lay them.

That makes it the opposite of [Reduce](operations.md), which takes the triangles
you have and removes some of them. Remesh keeps nothing: not one point, not one
edge. What comes back is a new surface sitting on top of the old one, with the
materials, texture layout and colors copied across.

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

## Triangles or quads

**Mostly quads** is the default and is what you almost always want. Four-sided
faces are what modelling packages and subdivision surfaces are built around, and
they are what makes edge loops readable. The pattern cannot be all quads
everywhere - wherever the flow has to split or merge, the rebuilt surface leaves
a triangle. On a typical game model four faces in five come back as quads; on a
model with a lot of creases and open borders it is nearer three in four.

**Only quads** is a different tool: instead of reading a pattern off a field it
*solves* for one, as an integer layout over the whole surface, which is what lets
it promise there is not a single triangle in the result. The price is that it
needs one closed shell to work on - see below.

**Triangles** gives an even triangle mesh instead, with no quads at all. Use it
when whatever consumes the model triangulates anyway and you would rather see
the real triangle count.

The quads are real. The viewport's wireframe draws four edges around each one,
the stats card counts them under Polys, and the export writes them to the file
as quads.

## Why Only quads sometimes falls back

When Only quads cannot run, it quietly becomes Mostly quads and tells you why.
There are three reasons, and each has a different answer.

**The object is not one closed shell.** The solver walks the surface as a set of
faces that each border exactly two others; a kitbash of interpenetrating parts,
or a surface that pinches to a point, is not that. Nothing can be done to the
settings to fix it - the fix is geometric, and it is what
[Shrinkwrap](shrinkwrap.md) is for: put one above the Remesh and it fuses the
object into a single closed shell first, after which Only quads runs.

**The layout could not be solved.** Some closed shells still defeat it: the
system it builds comes out singular and there is no layout to return. The
message names the stage that gave up. Try **Thorough layout**, which solves it a
slower and more careful way, or a different face count - a target the surface
divides into more evenly often goes through.

**This build has no solver.** The quad solver is an optional vendored library.
Only quads stays in the list either way, so a preset that asks for it still means
what it says, but a build without it always falls back.

**Thorough layout**, below Only quads, is that slower solver, and it belongs to
Only quads alone.

## How dense

Two ways to say it:

- **Ratio of current** - a share of each object's *current* triangle count. This
  is per object, so every object keeps its relative level of detail, which is
  what you want when you are rebuilding a whole asset in one go. The share is
  read against triangles, so it matches the Tris figure on the stats card; one
  quad counts as two.
- **Face count** - one budget for the whole selection, shared out between the
  objects by surface area. That is the only split that gives the same face size
  everywhere, which is what "3000 faces for this asset" means. An object with its
  own [override](overrides.md) takes exactly the count it asks for and leaves the
  shared pool, so pinning one object does not quietly re-weight the rest.

The number is what you get, within a few percent. It is not free, though: the
rebuild works from a face *size* rather than a count, so how many faces that
turns into is measured and the size corrected, which means an object that lands
wide of the mark is rebuilt more than once. The one case that can still land
further out is Only quads with a strong [face-size variation](#vary-face-size):
its layout answers in steps rather than smoothly, and a step can be large enough
that no face size lands on the number - expect up to a sixth out there. Very
small objects are left alone - below about sixteen triangles there is no surface
to work from, and you are told which objects were skipped.

## Vary face size

Left at 0, every face comes out the same size, and a broad flat panel gets as
many of them as a tight rounded edge. That is rarely what you want: the flat part
needs four faces and the curve needs forty.

**Vary face size** moves the budget instead of raising it. The surface is
measured for how much it *turns* over the distance one face spans, and faces are
made smaller where it turns and larger where it does not - so the count stays put
and the detail moves to where it does something. On a stone pedestal at the
default setting the flat top goes from an even grid of twelve-millimetre quads to
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

Strength is not free above the default. The quad pattern has to resolve a change
in face size somewhere, and it resolves a steep one with singularities rather
than with a gradient - so a stronger setting comes back with more triangles mixed
into Mostly quads (measured on a sculpted head at the same count: 73% quads at 0,
64% at the default, 56% at 1), and Only quads can miss its face count by more
than it usually does. Both are worth paying on a silhouette-critical object and
neither is worth paying by default.

It applies to every topology, and which one varies most depends on the shape. On
a stone pedestal - a broad flat top with a rounded rim - Only quads varies the
most, spreading face sizes seven-fold where Mostly quads manages a bit over two.
On a driftwood branch, where the change is a gradual thinning rather than an
edge, it is the other way round and Only quads varies the least.

## Sharp edges and open borders

**Keep sharp edges** lays the new edges along the model's own creases instead of
running the pattern straight over them. Turn it on for hard-surface and CAD
parts, where a rebuilt surface that rounds off a 90-degree corner is useless;
leave it off for organic shapes, where it only breaks up the flow. **Sharp
above** is how sharp a fold has to be before it counts.

**Follow open borders** is on by default and holds the new edges against any open
edge, so a border comes back as one clean loop rather than a ragged fringe.

**Smoothing** is how many rounds of evening-out to run over the result. Two is a
good default: a little makes the faces more uniform, a lot rounds off detail.

**Reproducible** is on by default and runs the rebuild on one core, because
spreading it across several changes the answer: the same model rebuilt at one
thread, at two and at eight gives three different meshes, and none of them is
wrong - the solver simply settles differently depending on the order the work
lands in. Leave it on. A preview that shifts under you while you drag a slider is
worse than one that takes a moment longer, and "a moment" is what it costs: about
twice as long on a large object, and no difference at all on a scene of small
ones. Turn it off only for a one-off rebuild of something big where you do not
care about matching it again.

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
than smearing across a face. That is why a rebuilt model has slightly more points
than it has quads - the extra ones are the seam corners, exactly as they were in
the original.

## What is lost

Everything tied to the old points, because the old points are gone:

- **bones and shape keys**. An object that bends is skipped entirely and left as
  it is, with a note saying so. Remesh is for still objects.
- **per-point creases** used by subdivision.
- **the original polygon edges**, along with any per-edge data the file carried.

Tangents are rebuilt from the new surface, as they are after any operation that
changes the shape.

## Where it goes in the list

Above the reorder operations, and **below** any simplifier you want to keep. A
Reduce or a Generate LODs running after a Remesh rebuilds its faces as triangles
and undoes the quads; the run tells you when the list is in that order. Remesh
*after* a simplify is fine, and so is a Weld Vertices or an Optimize Vertex Fetch
below it - those keep the quads.

Baking ambient occlusion works in either order, though below the Remesh is
usually what you mean, so the bake describes the surface you are keeping.
