### The Inspector side panel: node stats, the material editor, and the texture
### pool.

ui-inspector-empty = Select a node or material in the Outliner.
ui-inspector-node-gone = Node no longer exists.
ui-inspector-material-gone = Material no longer exists.

## Node rows

ui-inspector-node-heading = Node - { $name }
ui-inspector-type = Type
ui-inspector-position = Position
ui-inspector-children = Children
ui-inspector-triangles = Triangles
ui-inspector-mesh-part = Mesh part
ui-inspector-bones = Bones
ui-inspector-blend-shapes = Blend shapes
ui-inspector-radius = Radius
ui-inspector-relative-length = Relative length

## Skinning

ui-inspector-skinning = Skinning
    .description = The deformer method the file declared. Dual-quaternion skins are
        evaluated as linear blends here, which is why they are named as such.
ui-inspector-max-influences = Max influences
    .description = The most bones any one vertex is weighted to. Engines cap this - four
        is the common limit, eight the generous one - and a mesh over the cap has its
        smallest weights dropped on import.
ui-inspector-influenced-verts = Influenced verts
    .description = How many vertices this bone moves at all.
ui-inspector-share-of-mesh = Share of mesh
    .description = What fraction of the mesh's vertices this bone influences.

## Material

ui-inspector-material = Material
ui-inspector-workflow = Workflow
    .description = Native metallic-roughness, or Unity-style smoothness, where the slider
        and any bound map read as smoothness and the shader flips them.
ui-inspector-transparency = Transparency
    .description = Opaque, alpha-blended, or alpha-clipped at a cutoff.
ui-inspector-cutoff = Cutoff
    .description = The alpha below which a clipped material discards the pixel entirely.
ui-inspector-base-color = Base color
ui-inspector-roughness = Roughness
ui-inspector-smoothness = Smoothness
ui-inspector-metallic = Metallic
    .description = Read from the file's own metalness alone. A classic Lambert or Phong
        material declares none, and its reflectivity slot is deliberately not read as
        metalness - doing so made most real game assets arrive as mirrors.
ui-inspector-emissive = Emissive

## Textures

ui-inspector-texture-mapping = Texture mapping
    .description = Bind a pooled image and a channel to each PBR slot. Single-value slots
        pick one channel, so an ORM-packed map can feed three slots from one file.
ui-inspector-texture-files = Texture files
    .description = The scene-wide pool. Each image is decoded once and shared by every
        material that references it, and watched on disk so a save in your paint package
        shows up here.
ui-inspector-add-textures = Add textures...
ui-inspector-remove-texture = Remove texture
ui-inspector-no-textures = No textures imported.
ui-inspector-drop-textures = Drop texture files here to add
# Placeholder in a slot's dropdown when no image is bound.
ui-inspector-no-texture = <texture>

# The empty entry at the top of a texture slot's dropdown: picking it unbinds
# whatever is in the slot.
ui-inspector-select-texture = select texture
ui-inspector-bones-selected = { $count ->
        [one] One bone selected
       *[other] { $count } bones selected
    }
