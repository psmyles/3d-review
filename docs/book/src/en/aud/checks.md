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

## Transforms

- **Scaled objects**, **Mirrored objects** and **Unfrozen transforms**: a scale,
  a mirror, a rotation or an offset pivot left on an object.
- **Off-origin pivot**: a top-level object away from the world origin.
- **Scene unit** and **Up axis**: the file's, against the engine's.

## UVs

- **Missing UVs**: parts without UVs, or with different UV sets from the
  others.
- **UVs outside 0-1**, **Overlapping UVs** and **Flipped UVs**.
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
  fills the screen. A LOD cannot help; the mesh itself needs reducing.

The triangle figures are an estimate worked out from each part's size,
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
