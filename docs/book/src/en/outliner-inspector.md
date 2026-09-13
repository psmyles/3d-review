# Outliner and Inspector

## The Outliner

A dockable, resizable left panel with two or three tabs.

**Scene** lists the node hierarchy, indented and collapsible with parent guide
lines, or as a flat list. A search box filters both tabs and lists matches flat,
so a hit is never buried in a folded branch. Node-kind filter glyphs narrow what
is listed.

Clicking a row selects it, which drives the viewport highlight (with a selection
flash), the Inspector, and the selection-scoped bounding box and framing. Arrow
keys walk the rows.

Mesh rows carry a visibility eye. Holding the platform's primary modifier while
clicking the eye isolates that mesh instead of toggling it. On bone rows, that
modifier adds to the selection and Shift takes a range, which is what feeds the
skin weight heat map.

**Materials** lists the materials with duplicates merged. Selecting one
highlights its faces in the viewport and opens it in the Inspector.

**Animations** lists the clips. It is present only for a file that carries
animation, and never in the Opt workspace. See
[Skinning and animation](animation.md).

## The Inspector

The right panel. For a selected node it shows read-only stats; for a multi-bone
selection, a summary. For a material it shows the three editable sections
described in [Materials and textures](materials.md).

In the Opt workspace the Inspector retargets at the selected operation's
parameters, the export settings, or the selected object's overrides.
