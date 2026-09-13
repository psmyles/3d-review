# Baking ambient occlusion

Repeatable CPU raycasts over a hemisphere at each vertex, against a triangle
tree, spread across threads. Adding the operation switches the viewport to the
matching [vertex-color mode](../panels/vertex-colors.md), and the exporter
already writes vertex colors, so nothing else is needed.

Unlike the [screen-space effect](../panels/ambient-occlusion.md), this is baked
into the mesh and travels with it.

## Settings

**Quality** - 32, 64, 128 or 512 rays per vertex.

**Max distance** - how far a surface can be and still block light, in world
meters. Zero means the whole scene.

**Intensity** - a power curve on visibility, matching the viewport AO panel.

**Target** - alpha, RGB greyscale, multiplied into RGB, or a single R, G or B
channel. Optional sRGB encoding for the RGB targets; alpha is always linear.

## Two rules that make it usable on real game FBXs

**Excluded objects still block light.** An object [excluded](overrides.md) from
the stack is not written to, but it still occludes. That is what you want for a
hero prop that should keep its own bake.

**Hidden nodes do neither.** A node hidden in the Outliner neither blocks light
nor gets baked, so hide the collision shell first. Toggling an eye reruns the
stack while a bake is enabled, for exactly this reason.

## Blockers are grouped by LOD suffix

Game assets carry their whole LOD chain as co-located siblings in one file, and
casting rays from one LOD onto another's near-coincident surfaces shreds the
result.

Occluders are therefore partitioned by the `_LOD<n>` name suffix, taken from the
node itself or its nearest named ancestor:

- each LOD bakes only against its own group plus every node with no suffix,
- suffix-less nodes bake against the lowest LOD present.

So an artist can bake an entire visible chain correctly in one run.
