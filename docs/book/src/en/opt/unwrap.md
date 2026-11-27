# Unwrapping UVs

**Unwrap UVs** makes a fresh set of texture coordinates (UVs). It cuts the
surface into islands, flattens each island, and lays them out in one of two
ways:

- **Tiled** - every island at the same texel density, turned to lie the way
  the old UVs did. Islands may overlap: this is for textures that repeat across
  the surface, such as metal, paint or wear.
- **Atlas** - every island packed into the 0-1 square without overlapping. This
  is for anything painted or baked onto the model pixel by pixel: lightmaps,
  ambient occlusion, a hand-painted texture. Each object gets its own square.

Put it after **Generate LODs** in the list and it runs on every level, so each
level gets UVs made for its own triangles rather than ones stretched by the
simplifier. It never changes the shape.

## Where the surface is cut

An island never crosses:

- a change of **material**;
- an open edge, or an edge shared by more than two faces;
- a **seam attribute**, when one is set: an attribute the model carries per
  vertex, such as a CAD face id (`_CAD_FACE` in a glTF file), or the U of a UV
  set when a pipeline carries the id that way in an FBX file;
- a seam of the **reference UVs**, when **Split at reference seams** is on.
  This catches the cut a full cylinder needs inside one CAD face;
- an edge sharper than the **crease angle**, when **Split at creases** is on.

**Charts** decides what happens inside those lines. **Islands** keeps each
region whole wherever it can be flattened in one piece, cutting it further only
where it closes on itself (a band all the way round a cylinder, say).
**Automatic** lets the unwrapper choose its own pieces as well, cutting where the
surface curves: the right choice for a model with no seams worth keeping.

## Settings

**Layout** - Tiled or Atlas, as above.

Adding the operation switches the comparison to the
[UV view](comparison.md#layouts): the result under the checker beside its
layout, in the set the operation writes.

**UV set** - which set to write: one of the model's own, listed by number and
name and replaced, or **new set**, added after its last. The first set to
start with; in a glTF file, set 0 is `TEXCOORD_0`.

**Reference UV set** - the set the new islands are turned to match, compared
with, and cut along. Turn it off for a model whose UVs mean nothing.

**Flattening** (Tiled) - **Minimum stretch** keeps every island's texel density
even, at the cost of a little angle on surfaces curved two ways at once, as a
minimum-stretch unwrap in a modelling program does. **Conformal** keeps angles
exactly and lets the density drift across a curved island.

**Island stretch limit** (Tiled, with Islands) - an island that stretches more
than this on average when flattened in one piece is cut further, where that
helps. A band all the way round a cylinder, with no seam inside its face, needs
this: whole, it flattens only as a ring. At 1, any stretch is cut away; higher
keeps more islands whole, and the texture runs on across them.

**Density** (Tiled) - UV units per metre: at 1.0, a texture tiles once per metre
of surface.

**Align to reference** (Tiled) - turn each island (or mirror it, when that fits
better) to lie as its reference UVs did, centred where they were. Without it, an
island's longest side runs along U.

**Keep the better UVs** (Tiled) - per island, keep the reference UVs instead,
scaled to the density, wherever they stretch less than the fresh unwrap. A long
thin strip often unwraps better from its original parameters.

**Resolution** and **Padding** (Atlas) - the texture size the atlas is meant for,
and the gap left between islands in its texels, so bilinear filtering never
reads one island's colour into another.

## For CAD parts

A part exported from CAD with its face ids as a custom attribute unwraps well
with: Layout Tiled, Seam attribute `_CAD_FACE`, Charts Islands, Flattening
Minimum stretch, Density 1.0, Reference UV set 0 with Align and Keep the better
UVs on. Each CAD face becomes an island, cut again only where its own surface
closes on itself, laid out at one texel density and turned as the CAD UVs ran.
