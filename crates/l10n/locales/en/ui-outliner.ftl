### The Outliner side panel: its tabs, the scene tree, the materials list and the
### animation clips.

## Tabs

ui-outliner-tab-scene = Scene
ui-outliner-tab-materials = Materials
ui-outliner-tab-animations = Animations

## Scene tab chrome

ui-outliner-view-tree = Showing the scene hierarchy
    .description = Click for a flat list, which is the faster read when you know a
        node's name but not where it sits.
ui-outliner-view-flat = Showing a flat node list
    .description = Click for the scene hierarchy, which is what shows parenting and
        what a transform is inherited from.

ui-outliner-search-hint = Search
    .description = Filters both tabs. Matches are listed flat, so a hit is never buried
        in a folded branch.

# The node-kind filter glyphs; { $kind } is the kind's own name.
ui-outliner-filter-kind = { $kind } nodes
    .description = Narrow the list to one kind of node. Several can be on at once.

## Row actions

ui-outliner-show-mesh = Show mesh
    .description = { $modifier }+click: show only this mesh, and hide every other.
ui-outliner-hide-mesh = Hide mesh
    .description = { $modifier }+click: show only this mesh, and hide every other.
        A hidden mesh is left out of the visible-scope counts, and out of an
        ambient-occlusion bake entirely.

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
    .description = Click to select this clip and open the playback transport. Click it
        again to return to the rest pose.

ui-outliner-no-materials = No materials.
