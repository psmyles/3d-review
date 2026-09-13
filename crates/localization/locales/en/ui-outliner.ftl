### The Outliner side panel: its tabs, the scene tree, the materials list and the
### animation clips.

## Tabs

ui-outliner-tab-scene = Scene
ui-outliner-tab-materials = Materials
ui-outliner-tab-animations = Animations

## Scene tab chrome

ui-outliner-view-tree = Showing the scene hierarchy
    .description = Click to switch to a plain list. The plain list is quicker when you
        know the name of a part but not where it sits in the tree.
ui-outliner-view-flat = Showing a flat node list
    .description = Click to switch to the tree view. The tree shows which parts are
        attached to which, and where a part gets its position from.

ui-outliner-search-hint = Search
    .description = Filters both tabs as you type. Matches are shown in a plain list, so
        one is never hidden inside a folded branch.

# The node-kind filter glyphs; { $kind } is the kind's own name.
ui-outliner-filter-kind = { $kind } nodes
    .description = Show only this kind of part. You can turn several kinds on at once.

## Row actions

ui-outliner-show-mesh = Show mesh
    .description = { $modifier }+click: show only this part, and hide all the others.
ui-outliner-hide-mesh = Hide mesh
    .description = { $modifier }+click: show only this part, and hide all the others.
        A hidden part is left out of the "visible" column of the stats, and is skipped
        completely by an ambient-occlusion bake.

## Empty and fallback states

ui-outliner-no-nodes = No scene nodes.
ui-outliner-no-matches = No matches.
ui-outliner-all-filtered = Every node type is filtered out.
ui-outliner-no-clips = No animation clips.

# A node the source file left unnamed.
ui-outliner-unnamed-node = Node { $index }
# A clip the source file left unnamed.
ui-outliner-unnamed-clip = Clip { $index }

## Animation clips

# The trailing readout on a clip row: frame count and duration.
ui-outliner-clip-summary = { $frames } f - { $duration } s
ui-outliner-clip-tooltip = { $frames } frames at { $fps } fps - { $duration } s
    .description = Click to pick this animation and show the playback controls. Click it
        again to put the model back in its resting pose.

ui-outliner-no-materials = No materials.
