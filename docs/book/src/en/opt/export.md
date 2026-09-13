# Exporting

Nothing is saved until you ask. Exporting writes the processed model, and its
chain of levels if there is one, to a new FBX file.

## Settings

**Packaging** - one file holding every level as parts named `MeshName_LOD0`
through `MeshName_LODn` (the naming most game engines recognise on their
own), or a separate file for each level.

**Hierarchy** - rebuild the original tree of parts, so the exported file
behaves just like the original, or write every part flat at the top level,
placed where it sits in the world.

**Format** - binary or text (ASCII) FBX.

Files are written as FBX version 7.7, in the same unit the original file used.

## What the export keeps

Everything the operations did not change:

- **Shape** - four-sided and many-sided faces survive wherever no operation
  rebuilt the surface. A simplified level is written as triangles, and the
  report tells you which operation did it.
- **Materials** - just as they were, with their texture paths, embedded images
  and layered textures.
- **Structure** - pivots, rotation orders, custom properties, lights, cameras,
  empties and LOD groups.
- **Rigging** - skeletons, bind poses, blend shapes and the original animation
  curves.
- **Extra data** - color sets, smoothing, creases, holes, polygon groups,
  display layers, selection sets, and the scene's own settings.

The export report lists only things that were genuinely lost.

## Presets

A list of operations, with its per-object overrides and export settings, can
be saved as a small preset file and loaded again later. That is what lets a
team share one setup and reuse it across many models of the same kind.
