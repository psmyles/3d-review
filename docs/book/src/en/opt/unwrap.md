# Unwrapping UVs

**Unwrap UVs** makes a fresh set of texture coordinates (UVs). It works the way
an unwrap in Blender or Maya does, in three steps, and its settings come in the
same order:

1. **Seams** - where the surface is cut into islands.
2. **Unfold** - how each island is flattened.
3. **Layout** - where the islands go: packed into the 0-1 square, or tiled at
   one texel density.

You don't have to mark any seams: the unwrap places them itself, the way an
artist would - as few as each part needs, where they show least. Islands never
overlap themselves or fold over, and they all wind the same way, so normal maps
and painted text come out the right way round. No seam ever runs through the
middle of a quad or other polygon: a polygon's corners always share one UV each,
so an exported quad keeps its shape in the texture.

Adding the operation switches the comparison to the
[UV view](comparison.md#layouts): the result under the checker beside its
layout, in the UV map the operation writes. While it runs, each object appears
with its new UVs as soon as it is done.

Put it after **Generate LODs** in the list and it runs on every level, so each
level gets UVs made for its own triangles rather than ones stretched by the
simplifier. It never changes the shape.

## Which UVs to write and learn from

**UV index to write** - which UV map the unwrap writes: one of the model's own,
listed by number and name and replaced, or **new set**, added after its last.
In a glTF file, map 0 is `TEXCOORD_0`.

**Use existing UVs from** - which of the model's own UV maps to learn from, if
they are worth learning from. Their seams can guide the new ones or cut outright
(**Existing seams**), and tiled islands can be turned to lie the way they did.
Choose **None** for a model whose UVs are missing or no use.

## Seams

An island never crosses a change of material, an open edge, or an edge shared
by more than two faces. The settings below add seams of their own; then the
unwrap places the rest.

**Existing seams** - what the existing UVs' seams do:

- **Prefer** - the seams the unwrap needs go along them where they can, and
  nowhere else is cut for them. Good for a model whose UVs are roughly right
  but cut into too many pieces.
- **Cut** - cut wherever the existing UVs are cut, so their islands stay apart.
- **Ignore** - place seams as if the model had no UVs.

**Cut sharp edges** and **Sharp angle** - cut along every edge sharper than the
angle, as a hard-surface model wants.

**Cut along attribute** - cut wherever a value the model carries per vertex
changes: a CAD face id (`_CAD_FACE` in a glTF file), a CAD region id
(`_CAD_REGION`: faces joined across smooth, straight edges, such as a fillet
and the faces either side of it), or the U of a UV map when a pipeline carries
the id that way in an FBX file.

**Max stretch** - how far an island may stretch the texture before the unwrap
adds seams to it. The value is an average over the island: 1.20 means a texel
is stretched about 20% at most on average, and any single triangle at most 2:1
(five times as far past 1). Lower it for less stretch and more seams; raise it
for fewer, larger islands.

### Where the unwrap places seams

Each region the seams above bound is first flattened whole. A region with holes
in it - an eye socket, say - flattens with the holes left as holes. When a region
can't lie flat or stretches more than **Max stretch**, the unwrap cuts it the way
an artist would:

1. A closed shape is opened along one seam between its two far ends. A shape
   with a handle (a mug's) is cut through the handle.
2. While it still stretches too much, a seam is run from where it stretches most
   - the tip of an ear, the top of a head - to the nearest seam or edge. It
   splits the surface open there without making a new island. The open end of a
   tube (a sleeve, a trunk) is first tried cut straight along its length, which
   unrolls it into a strip.
3. Only when seams can't bring it within the limit, or a part laid flat would
   fold over the rest (an ear over the head), is the region split in two, along
   the cheapest line between the parts.

A seam costs least where it shows least: along a sharp crease, in a hollow (where
an ear meets the head), in a crevice the light barely reaches, and along the
existing UVs' seams. It costs most across a smooth, open, rounded surface, which
is where it would be seen. Seams also prefer to run straight along the model's
edge loops rather than zig-zag across them.

## Unfold

**Method** - as in Blender:

- **Minimum stretch** keeps the texel density even across curved islands,
  giving up a little angle where the surface curves two ways at once.
- **Conformal** keeps angles exactly and lets the density drift: a curved island
  comes out larger at its middle than its rim.

## Layout

**Layout**:

- **Pack into 0-1** places every island in the square without overlapping,
  for anything painted or baked onto the model pixel by pixel: lightmaps,
  ambient occlusion, a hand-painted texture. Each object gets its own square.
- **Tile at texel density** keeps every island at one density, for textures that
  repeat across the surface, such as metal, paint or wear. Turned to match the
  existing UVs, islands sit where those were and may overlap, as a tiling
  texture allows.

**Texture size** and **Padding** (Pack into 0-1) - the texture the UVs are made
for, and the gap left between islands in its texels, so filtering never reads
one island's colour into another.

**Rotate islands** - let the packing turn islands, a quarter turn at a time and
each along its longest side, to fit them tighter.

**Texel density** (Tile) - UV units per metre: at 1.0, a texture tiles once per
metre of surface.

**Match existing orientation** (Tile) - turn each island, or mirror it, to lie
as the existing UVs did, centred where they were. An island the existing UVs
cover in several pieces (CAD faces joined into one region, each with UVs of its
own) is turned to lie as its largest piece did. Off, the islands are packed
apart at the density.

**Keep existing where better** (Tile) - per island, keep the existing UVs
instead, scaled to the density, wherever they stretch less than the fresh
unwrap. A long thin strip often unwraps better from its original parameters.
Only an island the existing UVs cover in one piece is kept, so keeping them
never adds a seam.

## Recipes

**An organic model with no usable UVs** (a sculpt, a scan): Use existing UVs from None,
Max stretch 1.20, Minimum stretch, Pack into 0-1. Raise Max stretch towards
1.3 for fewer islands, lower it towards 1.10 for less stretch.

**A hard-surface model**: the seams already prefer sharp edges, so a box comes
out cut along its edges. Add **Cut sharp edges** at about 60 degrees to make
each panel its own island.

**Lightmaps or a bake**: Pack into 0-1, with **Texture size** set to the bake's
resolution and **Padding** at 4 texels or more.

**CAD parts for a tiling texture**: Layout Tile at texel density, Cut along
attribute `_CAD_REGION`, Use existing UVs from 0 with Existing seams Prefer,
Match existing orientation and Keep existing where better on, Texel density
1.0. Each CAD region becomes an island, cut again only where it won't lie flat,
laid out at one texel density and turned as the CAD UVs ran. A fillet along a
straight edge is in one region with the faces either side of it, so the
texture runs over it without a seam; on a filleted part this halves the seams
across smooth surface, with no more stretch. Fillets round a curved edge stay
islands of their own, since no flat layout keeps a texture's direction through
them. Existing seams is Prefer, not Cut, because each face of a region has UVs
of its own, and cutting at them would cut the region apart again.

For a file that carries only `_CAD_FACE`, cut along that with Existing seams
Cut: every CAD face becomes its own island, fillets included.
