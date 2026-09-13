# Exporting

An explicit export writes the chain to FBX through vendored `ufbx_write`.

## Settings

**Packaging** - one file holding `MeshName_LOD0` through `MeshName_LODn` as
sibling nodes (the naming most engines detect automatically), or one file per
level.

**Hierarchy** - rebuild the original node hierarchy so the export round-trips
like the source asset, or write flat meshes with identity transforms.

**Format** - binary or ASCII FBX.

Files are written as FBX 7.7, in the source file's own unit.

## What the export keeps

Everything the stack did not change:

- **Geometry** - quads and n-gons survive wherever no operation rebuilt the
  buffer. A simplified level is written as triangles, and the report says which
  operation did it.
- **Materials** - as authored, with their texture paths, embedded images and
  layered textures.
- **Hierarchy** - pivots, rotation orders, user properties, lights, cameras,
  nulls and LOD groups.
- **Deformation** - skins, bind poses, blend shapes and the original animation
  curves.
- **Layers and metadata** - color sets, smoothing, creases, holes, polygon
  groups, display layers, selection sets, and the scene's own settings.

The export report lists only genuine losses.

## Presets

An Opt stack saves and loads as a versioned JSON preset, carrying its operations,
per-object overrides and export settings. That is what makes a stack shareable
across a team and reusable across an asset class.
