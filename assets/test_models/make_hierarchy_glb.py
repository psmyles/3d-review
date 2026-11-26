# Blender: assets/test_models/hierarchy.glb, the glTF fixture with something of everything the
# glb path keeps. An empty `rig` (custom properties -> node extras) at (1, 2, 3) carries
# `panel`, a two-material grid with a palette colour, an integer `_CAD_FACE` per material and
# mesh custom properties (-> mesh extras), and `mirror`, a copy of it scaled -1 in X; the
# materials carry custom properties too.
# blender -b --factory-startup --python make_hierarchy_glb.py -- <out.glb>
import sys

import bpy

out = sys.argv[sys.argv.index("--") + 1]
bpy.ops.wm.read_factory_settings(use_empty=True)

rig = bpy.data.objects.new("rig", None)
bpy.context.scene.collection.objects.link(rig)
rig.location = (1.0, 2.0, 3.0)
rig["part"] = "weapon"
rig["hp"] = 40.0
rig["limits"] = [-45.0, 45.0]

bpy.ops.mesh.primitive_grid_add(x_subdivisions=8, y_subdivisions=8, size=1.0)
panel = bpy.context.active_object
panel.name = "panel"
panel.data.name = "panel_mesh"
panel.data["lod"] = "fine"
panel.parent = rig
panel.rotation_euler = (0.3, 0.0, 0.5)

steel = bpy.data.materials.new("steel")
steel["palette"] = 3
paint = bpy.data.materials.new("paint")
paint.diffuse_color = (0.8, 0.2, 0.1, 1.0)
panel.data.materials.append(steel)
panel.data.materials.append(paint)
me = panel.data
for polygon in me.polygons:
    polygon.material_index = 0 if polygon.center.x < 0.0 else 1
color = me.color_attributes.new("Color", "FLOAT_COLOR", "CORNER")
cad = me.attributes.new("_CAD_FACE", "INT", "CORNER")
for polygon in me.polygons:
    index = polygon.material_index
    for loop in polygon.loop_indices:
        color.data[loop].color = ((3 + 6 * index) / 255.0, 0.0, 0.0, 1.0)
        cad.data[loop].value = 7 + index
me.color_attributes.active_color = color

mirror = panel.copy()
mirror.data = panel.data.copy()
mirror.name = "mirror"
mirror.data.name = "mirror_mesh"
mirror.scale = (-1.0, 1.0, 1.0)
mirror.location = (0.0, 0.0, -2.0)
bpy.context.scene.collection.objects.link(mirror)
mirror.parent = rig

bpy.ops.export_scene.gltf(
    filepath=out,
    export_format="GLB",
    export_attributes=True,
    export_vertex_color="ACTIVE",
    export_extras=True,
    export_yup=True,
)
