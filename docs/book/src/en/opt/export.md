# Exporting

Nothing is saved until you ask. Exporting writes the processed model, and its
chain of levels if there is one, to a new FBX or glb file.

## Settings

**File type** - FBX, or glb (binary glTF, the format Godot, Bevy and the web
read directly).

**Packaging** - one file holding every level as parts named `MeshName_LOD0`
through `MeshName_LODn` (the naming most game engines recognise on their
own), or a separate file for each level.

**Hierarchy** - rebuild the original tree of parts, so the exported file
behaves just like the original, or write every part flat at the top level,
placed where it sits in the world.

**Format** - for FBX, binary or text (ASCII).

**Tangents** - for glb: the source's own, MikkTSpace tangents worked out from the
written mesh, or none (see [Normals and tangents](normals.md#tangents-in-a-glb)).

**Keep custom attributes** - for glb, whether to write the model's custom vertex
attributes: the extra data a glTF file can carry per vertex under a name
starting with an underscore, such as a CAD face id. Turn it off for the files a
game loads.

FBX files are written as FBX version 7.7, in the same unit the original file
used. glb files are in metres with +Y up, as glTF always is; a model from FBX is
turned into those axes.

## What the export keeps

For an FBX, everything the operations did not change:

- **Shape** - four-sided and many-sided faces survive wherever no operation
  rebuilt the surface. A simplified level is written as triangles.
- **Materials** - just as they were, with their texture paths, embedded images
  and layered textures.
- **Structure** - pivots, rotation orders, custom properties, lights, cameras,
  empties and LOD groups.
- **Rigging** - skeletons, bind poses, blend shapes and the original animation
  curves.
- **Extra data** - color sets, smoothing, creases, holes, polygon groups,
  display layers, selection sets, and the scene's own settings.

For a glb, from a glTF model:

- **Structure** - every node with its transform and its extras (the custom
  properties a glTF file carries), each mesh's name and extras.
- **Materials** - exactly as the file had them, with their textures.
- **Vertex data** - every UV set, the vertex colour in the precision it came in,
  tangents where the file had them, and the custom attributes in their own types.
  A vertex the operations left alone is written exactly as the file stored it,
  so a model put through a stack that changes nothing comes out the same.

A glb does not carry skins, blend shapes or animation yet.

The export report lists only things that were genuinely lost.

## Presets

A list of operations, with its per-object overrides and export settings, can
be saved as a small preset file and loaded again later. That is what lets a
team share one setup and reuse it across many models of the same kind.
