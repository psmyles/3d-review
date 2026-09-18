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

The two settings below Only quads belong to it alone. **Vary face size** lets the
faces get smaller where the shape is detailed and larger where it is flat,
instead of holding one size everywhere. **Thorough layout** is the slower solver
mentioned above.

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

Either way the number is a target, not a promise: the rebuild works from a face
*size* rather than a count, and it typically lands a fifth or so above what you
asked for. Very small objects are left alone - below about sixteen triangles
there is no surface to work from, and you are told which objects were skipped.

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

**Reproducible** is on by default and takes the slower path through the stages
whose result would otherwise depend on how the work happened to be shared out
between processor cores. Leave it on: a preview that shifts under you while you
drag a slider is worse than one that takes a moment longer.

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
