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
    .description = The method the file asks for when bending the model around its bones.
        The viewer shows every method as the simple linear kind, which is why some are
        labelled that way.
ui-inspector-max-influences = Max influences
    .description = The largest number of bones that pull on any single point. Game engines
        set a limit on this - four is common, eight is generous. If a model goes over
        the limit, the weakest bones are dropped when it is loaded.
ui-inspector-influenced-verts = Influenced verts
    .description = How many points this bone moves at all.
ui-inspector-share-of-mesh = Share of mesh
    .description = What portion of the whole model this bone has some pull on.

## Material

ui-inspector-material = Material
ui-inspector-workflow = Workflow
    .description = Whether this material thinks in terms of roughness or smoothness. They
        are opposites of each other. Some engines, like Unity, use smoothness, so you can
        match that here and the viewer flips the values for you.
ui-inspector-transparency = Transparency
    .description = Solid, softly see-through, or cut out sharply below a certain alpha
        value.
ui-inspector-cutoff = Cutoff
    .description = For the cut-out kind of transparency: any pixel whose alpha is below
        this value is not drawn at all.
ui-inspector-base-color = Base color
ui-inspector-roughness = Roughness
ui-inspector-smoothness = Smoothness
ui-inspector-metallic = Metallic
    .description = Read only from the file's own metalness value. Older material types
        do not have one, and the viewer deliberately does not guess it from their
        reflection setting - doing that made most real game models show up looking
        like mirrors.
ui-inspector-emissive = Emissive

## Textures

ui-inspector-texture-mapping = Texture mapping
    .description = Choose which image, and which color channel of it, feeds each part of
        the material. Parts that need a single value take one channel, so one packed
        image can feed three parts at once.
ui-inspector-texture-files = Texture files
    .description = All the images loaded for this scene. Each one is read once and shared
        by every material that uses it. The viewer also watches them on disk, so when you
        save a change in your paint program it shows up here.
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
