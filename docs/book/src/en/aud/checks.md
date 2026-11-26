# The checks

Every check below can be switched on or off, and given its own severity and
limits, in the [profile](profiles.md). The starting values depend on the
profile's engine.

## Geometry

- **Degenerate triangles**: triangles with no area. They draw nothing and can
  break normals and baking.
- **Non-manifold edges**: edges shared by more than two faces.
- **Loose points**: points no face uses.
- **Unwelded points**: separate points of one part at the same place.
- **N-gons**: faces with more than four corners.
- **Missing normals** and **Missing tangents**: the file did not carry them.
- **Inside-out faces**: faces whose normals point against their winding. A
  part mirrored on purpose is not reported here.
- **Mostly hard edges**: parts where most edges are hard.
- **Triangle budget** and **Draw calls**: more than the profile allows.
- **Indexed vertex color**: for a game that reads a vertex color channel as an
  index into its palette. Each face's channel must hold one whole index (a
  value of n/255), below the palette's size, the same at every corner. Pick
  the channel and the palette size in the profile. A file without vertex
  colors reads as white, index 255. An index painted as an sRGB color and
  stored linear - what Blender's byte color attributes export - needs
  **Painted in sRGB** on, so it is read back through the sRGB curve: index 1
  arrives as about 0.0003, not 1/255. Export colors as 16-bit or float; 8 bits
  keep a linear index exact but not an sRGB one.
- **Collider vertex limit**: collision meshes, picked by name (`*_hull` and
  `UCX_*` to start with), with more points than a physics engine takes for a
  hull. 32 to start with.

## Transforms

- **Scaled objects**, **Mirrored objects** and **Unfrozen transforms**: a scale,
  a mirror, a rotation or an offset pivot left on an object.
- **Off-origin pivot**: a top-level object away from the world origin.
- **Scene unit** and **Up axis**: the file's, against the engine's.

## UVs

- **Missing UVs**: parts without UVs, parts with no UVs in a set the other
  parts fill (sets are matched by their place in the list, not by name, as
  engines read them), and parts with their sets in a different order.
- **UVs outside 0-1**, **Overlapping UVs** and **Flipped UVs**.
- **UV stretch**: faces whose UVs stretch them one way more than the other,
  past the limit (2 to start with: one direction twice the other). It is the
  same measure the [UV stretch view](views.md) paints and the
  [Unwrap UVs](../opt/unwrap.md) operation's island stretch limit uses.
- **Missing lightmap UVs**, **Overlapping lightmap UVs**, **Lightmap
  padding** and **Lightmap UVs outside 0-1**: the lightmap UV set is checked at
  the lightmap's own resolution, because that is what decides whether light
  leaks. All four read the UV set the profile names - the second one by
  default, where engines expect the lightmap.

## Skin

- **Too many bone influences** and **Unnormalized weights**, per point.
- **Bones per mesh**, **Unused bones** and **Bind pose differs from the rest
  pose**.

## Density

- **Texel density**: texture pixels per meter, for the texture size the profile
  names, against its target.
- **Needs a LOD**: the screen size below which a part's triangles get too small,
  so it should switch to a LOD there.
- **Too dense for its size**: a part whose triangles are too small even when it
  fills the screen. A LOD cannot help; the mesh itself needs reducing. While
  this check is off, Needs a LOD lists these parts instead, at a screen size
  above 1, so turning it off never hides them.

- **LOD triangle budget**: a part with more triangles than its level allows.
  The level comes from a `_LOD<n>` name on the part or a group above it; a part
  with none is level 0. Set a budget for levels 0 to 3; level 3's holds for
  every level after it, and 0 means no budget.

The Needs a LOD and Too dense figures are an estimate worked out from each part's size,
surface and triangle count. They assume the part is roughly rounded and that
about half its triangles face the camera.

## Naming

- **Unsafe characters in names**, **Naming convention**, **LOD names**,
  **Orphaned collision** and **Asset prefix**. Name rules are written as
  patterns: `*` matches any run of characters, `?` any one, and `[abc]` any of
  the listed ones, so `SM_*` means "starts with SM_".

## Hierarchy

- **Empty groups**, **Duplicate names**, **Several top-level objects**,
  **Lights and cameras** and **Materials per mesh**.
- **Required extras**: objects, picked by name, that must carry a property the
  game reads - a glTF node's `extras`, or an FBX object's custom property.
  List each as `key`, or `key:type` to check its type too: `string`, `number`,
  `bool`, `array`, `object` or `null`. An FBX vector or color counts as an
  array. A type the check does not know stops the check, as a bad name pattern
  does.

The checks added for a game pipeline's profile (UV stretch, Indexed vertex
color, Collider vertex limit, LOD triangle budget, Required extras) start off
in every built-in profile: switch them on in a profile of your own.
