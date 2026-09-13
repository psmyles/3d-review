# Vertex Colors

Shows the mesh's vertex-color attribute instead of its material.

## Settings

**Mode**:

- *RGB* - the color channels as color.
- *Alpha* - the alpha channel as greyscale.
- *RGB with alpha* - the colors, with alpha driving opacity.

**Color set** - which set to show, on a mesh carrying more than one.

## What it is for

Vertex colors carry very different payloads depending on the pipeline: a tint, a
blend mask between materials, a wind or foliage weight, or baked ambient
occlusion. Viewing the alpha channel separately is usually what tells you which,
because a mask is almost always in a single channel.

The [Opt workspace](../opt/ao.md) can bake ambient occlusion into a vertex color
set, and switching that operation on selects the matching mode here
automatically.
