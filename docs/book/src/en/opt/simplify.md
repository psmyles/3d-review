# Simplification settings

**Reduce** and **Generate LODs** share one simplifier setup.

## Algorithm

- **Standard** - keeps topology, measuring position error only.
- **Preserve Attributes** - also penalizes normal, UV and color drift, with
  per-attribute weights.
- **Sloppy** - ignores topology. Much faster and hits the target far more
  reliably, but can close holes and merge nearby shells, so it suits the smallest
  levels.

## Target

A triangle ratio plus an error limit the simplifier may not exceed. It stops
short of the ratio rather than going past the error, so a mesh that cannot be
reduced that far without visible damage comes back larger than you asked for
rather than wrong.

## Flags

Lock the border, absolute error units, prune disconnected parts, regularize (and
a lighter variant), and permissive collapses across attribute seams.

## When a simplify barely removes anything

This is usually attribute seams, not a bug.

Import splits every face corner into its own vertex, so a mesh whose normals or
UVs differ at every corner - a scan with generated per-face normals, for
instance - presents *every* edge as a discontinuity, and a topology-preserving
collapse cannot cross one.

Measured: a real game asset welds 369k vertices down to 108k and hits its LOD
targets with the default settings, while such a scan stalls at 249,882 triangles
going to 249,880 until either a position-only weld (normals excluded) or the
permissive flag unblocks it.

The run detects the stall and says so. The default weld comparing normals is
*correct* for static game meshes, which is what this tool targets, so the fix is
to add a [Weld Vertices](operations.md) operation or set the permissive flag -
not to loosen the defaults.
