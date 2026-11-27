# Unwrapping UVs

**Unwrap UVs** makes a fresh set of texture coordinates (UVs). It works the way
an unwrap in Blender or Maya does, in three steps, and its settings come in the
same order:

1. **Seams** - where the surface is cut into islands.
2. **Unfold** - how each island is flattened.
3. **Layout** - where the islands go: packed into the 0-1 square, or tiled at
   one texel density.

You don't have to mark any seams: wherever an island won't lie flat without
stretching, the unwrap cuts it further on its own. Islands never overlap
themselves or fold over, and they all wind the same way, so normal maps and
painted text come out the right way round.

Adding the operation switches the comparison to the
[UV view](comparison.md#layouts): the result under the checker beside its
layout, in the UV map the operation writes. While it runs, each object appears
with its new UVs as soon as it is done.

Put it after **Generate LODs** in the list and it runs on every level, so each
level gets UVs made for its own triangles rather than ones stretched by the
simplifier. It never changes the shape.

## UV map and existing UVs

**UV map** - which UV map to write: one of the model's own, listed by number
and name and replaced, or **new set**, added after its last. In a glTF file,
map 0 is `TEXCOORD_0`.

**Existing UVs** - the model's own UVs, if they are worth learning from. Their
seams can be kept (**Keep existing seams**), and tiled islands can be turned to
lie the way they did. Choose **None** for a model whose UVs are missing or no
use.

## Seams

An island never crosses a change of material, an open edge, or an edge shared
by more than two faces. On top of that:

**Keep existing seams** - cut wherever the existing UVs are cut, so their
islands stay apart. This also gives a full cylinder the seam that opens it.

**Cut sharp edges** and **Sharp angle** - cut along every edge sharper than the
angle, as a hard-surface model wants.

**Cut along attribute** - cut wherever a value the model carries per vertex
changes: a CAD face id (`_CAD_FACE` in a glTF file), or the U of a UV map when
a pipeline carries the id that way in an FBX file.

**Max stretch** - how far an island may stretch the texture before the unwrap
cuts it further. Each region the seams above bound is first flattened whole. If
that region can't lie flat (a closed shape, a band all the way round), or would
stretch more than this, it is cut into the fewest pieces that do. The value is an
average over the island: 1.10 means a texel is stretched about 10% at most on
average, and any single triangle at most 2:1. Lower it for less stretch and more
seams; raise it for fewer, larger islands.

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
as the existing UVs did, centred where they were. Off, the islands are packed
apart at the density.

**Keep existing where better** (Tile) - per island, keep the existing UVs
instead, scaled to the density, wherever they stretch less than the fresh
unwrap. A long thin strip often unwraps better from its original parameters.

## Recipes

**An organic model with no usable UVs** (a sculpt, a scan): Existing UVs None,
Max stretch 1.10, Minimum stretch, Pack into 0-1. Raise Max stretch towards
1.2 for fewer islands.

**A hard-surface model**: add **Cut sharp edges** at about 60 degrees, so each
panel is its own island.

**Lightmaps or a bake**: Pack into 0-1, with **Texture size** set to the bake's
resolution and **Padding** at 4 texels or more.

**CAD parts for a tiling texture**: Layout Tile at texel density, Cut along
attribute `_CAD_FACE`, Existing UVs 0 with Keep existing seams, Match existing
orientation and Keep existing where better on, Texel density 1.0. Each CAD face
becomes an island, cut again only where it won't lie flat, laid out at one texel
density and turned as the CAD UVs ran.
