# Outliner and Inspector

These are the two side panels. The Outliner, on the left, is a list of
everything in the file. The Inspector, on the right, shows details about
whatever you have selected.

## The Outliner

The Outliner is a panel on the left that you can drag wider or narrower. It has
two or three tabs.

**Scene** lists every part of the model. A model is usually built as a tree,
where parts are attached to other parts (a hand is attached to an arm, which is
attached to a body). You can see it as that tree, with lines showing what is
attached to what, or as a plain list. A search box filters the list as you
type, and matches are shown flat so one is never hidden inside a folded branch.
Small filter buttons let you show only certain kinds of parts, such as just the
bones.

Clicking a row selects that part. The part lights up in the view, the
Inspector shows its details, and the bounding box and `F` framing follow it.
The arrow keys move up and down the rows.

Parts that have a shape carry an eye icon that shows or hides them. Hold the
primary modifier (`Ctrl` on Windows, `Cmd` on macOS) while clicking the eye to
show only that one part and hide all the others. On bone rows, that modifier
adds the bone to your selection and Shift selects a whole run of bones, which
is how you feed the skin-weight heat map.

**Materials** lists the materials, with duplicates merged into one entry.
Selecting one highlights every face that uses it and opens it in the Inspector.

**Animations** lists the animations in the file. This tab only appears when
the file has animations, and never in the Opt workspace. See
[Skinning and animation](animation.md).

## The Inspector

The Inspector is the panel on the right. For a selected part it shows facts
about it that you can read but not change. For several selected bones, it shows
a summary. For a material it shows the three editable sections described in
[Materials and textures](materials.md).

In the Opt workspace the Inspector changes job: it shows the settings of the
operation you have selected, the export settings, or the per-object settings of
the part you have selected.
