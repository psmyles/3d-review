/*
 * A test-only probe of the review patches carried on the vendored ufbx_write
 * (`third_party/ufbx-write/review.patch`): it writes one small scene that uses
 * every patched entry point, so `tests/ufbxw_patches.rs` can read the file back
 * through the vendored ufbx reader and inspect its ASCII form.
 *
 * Compiled by `build.rs` only for non-release profiles (`cfg(has_ufbxw_probe)`),
 * so the shipped binary carries none of it. It deliberately does not touch the
 * production bridge in `export_bridge.c`: what is under test here is the
 * *writer*, not the exporter's use of it.
 *
 * Same discipline as the bridge: one scene, freed on every path.
 */
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include "ufbx_write.h"

/* Must agree with `RVO_FBX_VERSION` in export_bridge.c: the probe checks the
 * version the pipeline writes. */
#define RVO_PROBE_FBX_VERSION 7700

static void rvo_probe_error(char *error, size_t error_length, const ufbxw_error *source,
                            const char *fallback)
{
    const char *message = fallback;
    if (source && source->description_length > 0) {
        message = source->description;
    }
    if (!error || error_length == 0) {
        return;
    }
    size_t length = strlen(message);
    if (length >= error_length) {
        length = error_length - 1;
    }
    memcpy(error, message, length);
    error[length] = '\0';
}

static ufbxw_vec3 rvo_probe_vec3(double x, double y, double z)
{
    ufbxw_vec3 v;
    v.x = x;
    v.y = y;
    v.z = z;
    return v;
}

static ufbxw_node rvo_probe_node(ufbxw_scene *scene, const char *name, ufbxw_node parent)
{
    ufbxw_node node = ufbxw_create_node(scene);
    ufbxw_set_name(scene, node.id, name);
    if (parent.id) {
        ufbxw_node_set_parent(scene, node, parent);
    }
    return node;
}

int review_ufbxw_patch_probe(const char *path, int ascii, char *error, size_t error_length)
{
    int status = 1;
    ufbxw_scene *scene = NULL;

    ufbxw_scene_opts scene_opts;
    memset(&scene_opts, 0, sizeof(scene_opts));
    scene_opts.no_default_anim_stack = true;
    scene_opts.no_default_anim_layer = true;
    scene = ufbxw_create_scene(&scene_opts);
    if (!scene) {
        rvo_probe_error(error, error_length, NULL, "could not create the ufbx_write scene");
        return status;
    }

    ufbxw_node no_parent = { 0 };
    /* The elements the animation, display layer and selection set below refer to. */
    ufbxw_node locator_node = { 0 };
    ufbxw_node props_node = { 0 };
    ufbxw_node quad_node = { 0 };
    ufbxw_material quad_material = { 0 };

    /* P2: a Null attribute with a non-default Size. */
    {
        ufbxw_node node = rvo_probe_node(scene, "Locator", no_parent);
        ufbxw_node_set_translation(scene, node, rvo_probe_vec3(1.0, 2.0, 3.0));
        ufbxw_null null_attr = ufbxw_create_null(scene, node);
        ufbxw_set_real(scene, null_attr.id, "Size", 42.5);
        locator_node = node;
    }

    /* P1: user-defined properties of every value kind the exporter emits, with
     * explicit flags. */
    {
        ufbxw_node node = rvo_probe_node(scene, "Props", no_parent);
        props_node = node;
        ufbxw_add_int(scene, node.id, "MyInt", UFBXW_PROP_TYPE_INT, 7);
        ufbxw_set_prop_flags(scene, node.id, "MyInt", UFBXW_PROP_FLAG_USER);
        ufbxw_add_real(scene, node.id, "MyNumber", UFBXW_PROP_TYPE_DOUBLE, 2.5);
        ufbxw_set_prop_flags(scene, node.id, "MyNumber",
                             UFBXW_PROP_FLAG_USER | UFBXW_PROP_FLAG_ANIMATABLE);
        ufbxw_add_string(scene, node.id, "MyNote", UFBXW_PROP_TYPE_STRING, "hello");
        ufbxw_set_prop_flags(scene, node.id, "MyNote", UFBXW_PROP_FLAG_USER);
        ufbxw_add_real(scene, node.id, "MyHidden", UFBXW_PROP_TYPE_DOUBLE, 9.0);
        ufbxw_set_prop_flags(scene, node.id, "MyHidden", UFBXW_PROP_FLAG_HIDDEN);
        ufbxw_add_vec3(scene, node.id, "MyVector", UFBXW_PROP_TYPE_VECTOR, rvo_probe_vec3(1, 2, 3));
        ufbxw_set_prop_flags(scene, node.id, "MyVector", UFBXW_PROP_FLAG_USER);
        static const unsigned char blob[5] = { 'b', 'l', 'o', 'b', '!' };
        ufbxw_add_blob(scene, node.id, "MyBlob", UFBXW_PROP_TYPE_BLOB, blob, sizeof(blob));
        ufbxw_set_prop_flags(scene, node.id, "MyBlob", UFBXW_PROP_FLAG_USER);
        /* A template property re-flagged: still written with its value. */
        ufbxw_node_set_translation(scene, node, rvo_probe_vec3(4.0, 5.0, 6.0));
        ufbxw_set_prop_flags(scene, node.id, "Lcl Translation",
                             UFBXW_PROP_FLAG_ANIMATABLE | UFBXW_PROP_FLAG_USER);
    }

    /* P4: a LOD group with three levels over three children. */
    {
        ufbxw_node root = rvo_probe_node(scene, "LodRoot", no_parent);
        ufbxw_lod_group group = ufbxw_create_lod_group(scene, root);
        ufbxw_lod_group_add_level(scene, group, 0.0, 0);
        ufbxw_lod_group_add_level(scene, group, 120.0, 1);
        ufbxw_lod_group_add_level(scene, group, 480.0, 2);
        ufbxw_set_bool(scene, group.id, "MinMaxDistance", true);
        ufbxw_set_real(scene, group.id, "MinDistance", 5.0);
        ufbxw_set_real(scene, group.id, "MaxDistance", 600.0);
        ufbxw_set_bool(scene, group.id, "WorldSpace", true);
        rvo_probe_node(scene, "LodRoot_LOD0", root);
        rvo_probe_node(scene, "LodRoot_LOD1", root);
        rvo_probe_node(scene, "LodRoot_LOD2", root);
    }

    /* P2 (camera TypeFlags). */
    {
        ufbxw_node node = rvo_probe_node(scene, "Cam", no_parent);
        ufbxw_camera camera = ufbxw_create_camera(scene, node);
        ufbxw_set_real(scene, camera.id, "FocalLength", 35.0);
    }

    /* P3 + P4: two quads sharing an edge, every topology layer, and a layered
     * texture on the material. */
    {
        ufbxw_node node = rvo_probe_node(scene, "Quad", no_parent);
        ufbxw_mesh mesh = ufbxw_create_mesh(scene);
        ufbxw_set_name(scene, mesh.id, "Quad");
        quad_node = node;

        static const ufbxw_vec3 positions[6] = {
            { 0, 0, 0 }, { 1, 0, 0 }, { 1, 1, 0 }, { 0, 1, 0 }, { 2, 0, 0 }, { 2, 1, 0 },
        };
        static const int32_t indices[8] = { 0, 1, 2, 3, 1, 4, 5, 2 };
        static const int32_t face_offsets[3] = { 0, 4, 8 };
        /* Unique edges as corner indices: the four of face 0, then the three of
         * face 1 that are not the shared (1, 2) edge. */
        static const int32_t edges[7] = { 0, 1, 2, 3, 4, 5, 6 };
        static const double edge_crease[7] = { 0.25, 0.5, 0.75, 1.0, 0.0, 0.125, 0.375 };
        static const int32_t edge_smoothing[7] = { 1, 0, 1, 0, 1, 1, 0 };
        static const int32_t edge_visibility[7] = { 1, 1, 0, 1, 1, 0, 1 };
        static const double vertex_crease[6] = { 0.1, 0.2, 0.3, 0.4, 0.5, 0.6 };
        static const int32_t holes[2] = { 0, 1 };
        static const int32_t groups[2] = { 3, 7 };

        ufbxw_mesh_set_vertices(scene, mesh, ufbxw_copy_vec3_array(scene, positions, 6));
        ufbxw_mesh_set_polygons(scene, mesh, ufbxw_copy_int_array(scene, indices, 8),
                                ufbxw_copy_int_array(scene, face_offsets, 3));
        ufbxw_mesh_set_fbx_edges(scene, mesh, ufbxw_copy_int_array(scene, edges, 7));

        ufbxw_mesh_attribute_desc desc;

        memset(&desc, 0, sizeof(desc));
        desc.mapping = UFBXW_ATTRIBUTE_MAPPING_EDGE;
        desc.values = ufbxw_copy_int_array(scene, edge_smoothing, 7).id;
        ufbxw_mesh_set_attribute(scene, mesh, UFBXW_MESH_ATTRIBUTE_SMOOTHING, 0, &desc);

        memset(&desc, 0, sizeof(desc));
        desc.mapping = UFBXW_ATTRIBUTE_MAPPING_EDGE;
        desc.values = ufbxw_copy_real_array(scene, edge_crease, 7).id;
        ufbxw_mesh_set_attribute(scene, mesh, UFBXW_MESH_ATTRIBUTE_EDGE_CREASE, 0, &desc);

        memset(&desc, 0, sizeof(desc));
        desc.mapping = UFBXW_ATTRIBUTE_MAPPING_VERTEX;
        desc.values = ufbxw_copy_real_array(scene, vertex_crease, 6).id;
        ufbxw_mesh_set_attribute(scene, mesh, UFBXW_MESH_ATTRIBUTE_VERTEX_CREASE, 0, &desc);

        memset(&desc, 0, sizeof(desc));
        desc.mapping = UFBXW_ATTRIBUTE_MAPPING_POLYGON;
        desc.values = ufbxw_copy_int_array(scene, holes, 2).id;
        ufbxw_mesh_set_attribute(scene, mesh, UFBXW_MESH_ATTRIBUTE_HOLE, 0, &desc);

        memset(&desc, 0, sizeof(desc));
        desc.mapping = UFBXW_ATTRIBUTE_MAPPING_EDGE;
        desc.values = ufbxw_copy_int_array(scene, edge_visibility, 7).id;
        ufbxw_mesh_set_attribute(scene, mesh, UFBXW_MESH_ATTRIBUTE_VISIBILITY, 0, &desc);

        memset(&desc, 0, sizeof(desc));
        desc.mapping = UFBXW_ATTRIBUTE_MAPPING_POLYGON;
        desc.values = ufbxw_copy_int_array(scene, groups, 2).id;
        ufbxw_mesh_set_attribute(scene, mesh, UFBXW_MESH_ATTRIBUTE_POLYGON_GROUP, 0, &desc);

        /* Two named vertex-color sets, per corner (P3's neighbour: the set
         * beyond the first is what the extras carry). */
        {
            static const ufbxw_vec4 base_colors[8] = {
                { 1, 0, 0, 1 }, { 0, 1, 0, 1 }, { 0, 0, 1, 1 }, { 1, 1, 0, 1 },
                { 0, 1, 1, 1 }, { 1, 0, 1, 1 }, { 0.5, 0.5, 0.5, 1 }, { 0.25, 0.75, 0.125, 1 },
            };
            static const ufbxw_vec4 mask_colors[8] = {
                { 0.1, 0.1, 0.1, 1 }, { 0.2, 0.2, 0.2, 1 }, { 0.3, 0.3, 0.3, 1 }, { 0.4, 0.4, 0.4, 1 },
                { 0.5, 0.5, 0.5, 1 }, { 0.6, 0.6, 0.6, 1 }, { 0.7, 0.7, 0.7, 1 }, { 0.8, 0.8, 0.8, 1 },
            };
            static const int32_t corner_ids[8] = { 0, 1, 2, 3, 4, 5, 6, 7 };
            ufbxw_mesh_set_colors_indexed(scene, mesh, 0, ufbxw_copy_vec4_array(scene, base_colors, 8),
                                          ufbxw_copy_int_array(scene, corner_ids, 8),
                                          UFBXW_ATTRIBUTE_MAPPING_POLYGON_VERTEX);
            ufbxw_mesh_set_attribute_name(scene, mesh, UFBXW_MESH_ATTRIBUTE_COLOR, 0, "Base");
            ufbxw_mesh_set_colors_indexed(scene, mesh, 1, ufbxw_copy_vec4_array(scene, mask_colors, 8),
                                          ufbxw_copy_int_array(scene, corner_ids, 8),
                                          UFBXW_ATTRIBUTE_MAPPING_POLYGON_VERTEX);
            ufbxw_mesh_set_attribute_name(scene, mesh, UFBXW_MESH_ATTRIBUTE_COLOR, 1, "Mask");
        }

        ufbxw_mesh_add_instance(scene, mesh, node);

        /* A bone skinning the quads: a blend-type skin with dual-quaternion
         * weights on two vertices, a second skin layer, and a blend shape —
         * the deform data the exporter has to write back. */
        {
            ufbxw_node bone_node = rvo_probe_node(scene, "Bone", no_parent);
            ufbxw_node_set_translation(scene, bone_node, rvo_probe_vec3(0.0, 0.0, 1.0));
            ufbxw_create_bone(scene, UFBXW_BONE_LIMB_NODE, bone_node);

            static const int32_t all_vertices[6] = { 0, 1, 2, 3, 4, 5 };
            static const double full_weights[6] = { 1, 1, 1, 1, 1, 1 };
            static const int32_t dq_vertices[2] = { 0, 3 };
            static const double dq_weights[2] = { 0.5, 0.75 };
            ufbxw_matrix identity;
            ufbxw_matrix link;
            memset(&identity, 0, sizeof(identity));
            identity.m00 = identity.m11 = identity.m22 = identity.m33 = 1.0;
            link = identity;
            link.m23 = 1.0;

            ufbxw_skin_deformer skin = ufbxw_create_skin_deformer(scene, mesh);
            ufbxw_skin_deformer_set_skinning_type(scene, skin, UFBXW_SKINNING_TYPE_BLEND);
            ufbxw_skin_cluster cluster = ufbxw_create_skin_cluster(scene, skin, bone_node);
            ufbxw_set_name(scene, cluster.id, "Bone");
            ufbxw_skin_cluster_set_weights(scene, cluster, ufbxw_copy_int_array(scene, all_vertices, 6),
                                           ufbxw_copy_real_array(scene, full_weights, 6));
            ufbxw_skin_cluster_set_transform(scene, cluster, identity);
            ufbxw_skin_cluster_set_link_transform(scene, cluster, link);
            ufbxw_skin_deformer_set_dual_quaternion_weights(scene, skin,
                                                            ufbxw_copy_int_array(scene, dq_vertices, 2),
                                                            ufbxw_copy_real_array(scene, dq_weights, 2));

            static const int32_t layer_vertices[1] = { 1 };
            static const double layer_weights[1] = { 0.25 };
            ufbxw_skin_deformer layer = ufbxw_create_skin_deformer(scene, mesh);
            ufbxw_skin_deformer_set_skinning_type(scene, layer, UFBXW_SKINNING_TYPE_LINEAR);
            ufbxw_skin_cluster layer_cluster = ufbxw_create_skin_cluster(scene, layer, bone_node);
            ufbxw_set_name(scene, layer_cluster.id, "BoneLayer");
            ufbxw_skin_cluster_set_weights(scene, layer_cluster, ufbxw_copy_int_array(scene, layer_vertices, 1),
                                           ufbxw_copy_real_array(scene, layer_weights, 1));
            ufbxw_skin_cluster_set_transform(scene, layer_cluster, identity);
            ufbxw_skin_cluster_set_link_transform(scene, layer_cluster, link);

            static const int32_t shape_vertices[2] = { 0, 2 };
            static const ufbxw_vec3 shape_offsets[2] = { { 0, 0, 0.5 }, { 0, 0, 1.0 } };
            static const ufbxw_vec3 shape_normals[2] = { { 0, 0, 1 }, { 0, 0, 1 } };
            ufbxw_blend_deformer blend = ufbxw_create_blend_deformer(scene, mesh);
            ufbxw_blend_channel channel = ufbxw_create_blend_channel(scene, blend);
            ufbxw_set_name(scene, channel.id, "Smile");
            ufbxw_blend_channel_set_weight(scene, channel, 25.0);
            ufbxw_blend_shape shape = ufbxw_create_blend_shape(scene);
            ufbxw_set_name(scene, shape.id, "Smile");
            ufbxw_blend_shape_set_offsets(scene, shape, ufbxw_copy_int_array(scene, shape_vertices, 2),
                                          ufbxw_copy_vec3_array(scene, shape_offsets, 2));
            ufbxw_blend_shape_set_normals(scene, shape, ufbxw_copy_vec3_array(scene, shape_normals, 2));
            ufbxw_blend_channel_add_shape(scene, channel, shape, 100.0);
        }

        ufbxw_material material = ufbxw_create_material(scene, UFBXW_MATERIAL_FBX_PHONG);
        ufbxw_set_name(scene, material.id, "Mat");
        quad_material = material;
        ufbxw_node_set_material(scene, node, 0, material);
        ufbxw_mesh_set_single_material(scene, mesh, 0);

        ufbxw_texture base = ufbxw_create_texture(scene, UFBXW_TEXTURE_FILE);
        ufbxw_set_name(scene, base.id, "Base");
        ufbxw_texture_set_filename(scene, base, "C:/tex/base.png");
        ufbxw_texture_set_relative_filename(scene, base, "tex/base.png");

        ufbxw_texture detail = ufbxw_create_texture(scene, UFBXW_TEXTURE_FILE);
        ufbxw_set_name(scene, detail.id, "Detail");
        ufbxw_texture_set_filename(scene, detail, "C:/tex/detail.png");
        ufbxw_texture_set_relative_filename(scene, detail, "tex/detail.png");

        ufbxw_texture layered = ufbxw_create_texture(scene, UFBXW_TEXTURE_LAYERED);
        ufbxw_set_name(scene, layered.id, "Layered");
        ufbxw_texture_add_layer(scene, layered, base, 5, 1.0);
        ufbxw_texture_add_layer(scene, layered, detail, 1, 0.5);
        ufbxw_material_set_texture(scene, material, "DiffuseColor", layered);
    }

    /* A display layer and a selection set over the nodes above. */
    {
        ufbxw_display_layer layer = ufbxw_create_display_layer(scene);
        ufbxw_set_name(scene, layer.id, "Layer1");
        ufbxw_set_vec3(scene, layer.id, "Color", rvo_probe_vec3(0.25, 0.5, 0.75));
        ufbxw_display_layer_add_node(scene, layer, quad_node);
        ufbxw_display_layer_add_node(scene, layer, locator_node);

        static const int32_t selected_vertices[2] = { 0, 2 };
        static const int32_t selected_faces[1] = { 1 };
        ufbxw_selection_set set = ufbxw_create_selection_set(scene);
        ufbxw_set_name(scene, set.id, "Sel");
        ufbxw_selection_node selection = ufbxw_create_selection_node(scene, set);
        ufbxw_selection_node_set_node(scene, selection, quad_node);
        ufbxw_selection_node_set_include_node(scene, selection, true);
        ufbxw_selection_node_set_vertices(scene, selection, ufbxw_copy_int_array(scene, selected_vertices, 2));
        ufbxw_selection_node_set_polygons(scene, selection, ufbxw_copy_int_array(scene, selected_faces, 1));
    }

    /* Authored animation: a stack with one layer driving a translation with
     * cubic user tangents, a user property linearly, a material color and a
     * visibility with constant steps. */
    {
        ufbxw_anim_stack stack = ufbxw_create_anim_stack(scene);
        ufbxw_set_name(scene, stack.id, "Take");
        ufbxw_anim_stack_set_time_range(scene, stack, 0, 2 * UFBXW_KTIME_SECOND);
        ufbxw_anim_layer layer = ufbxw_create_anim_layer(scene, stack);
        ufbxw_set_name(scene, layer.id, "BaseLayer");

        ufbxw_anim_prop translation = ufbxw_node_animate_translation(scene, locator_node, layer);
        for (size_t component = 0; component < 3; component++) {
            ufbxw_anim_curve curve = ufbxw_anim_get_curve(scene, translation, component);
            ufbxw_keyframe_real key;
            memset(&key, 0, sizeof(key));
            key.flags = UFBXW_KEYFRAME_INTERPOLATION_CUBIC | UFBXW_KEYFRAME_TANGENT_USER |
                        UFBXW_KEYFRAME_TANGENT_BROKEN | UFBXW_KEYFRAME_WEIGHTED_LEFT |
                        UFBXW_KEYFRAME_WEIGHTED_RIGHT;
            key.time = 0;
            key.value = 1.0 + (double)component;
            key.weight_left = 0.4;
            key.weight_right = 0.25;
            key.slope_left = 0.5;
            key.slope_right = 2.0;
            ufbxw_anim_curve_add_keyframe_key(scene, curve, key);
            key.time = UFBXW_KTIME_SECOND;
            key.value = 5.0 + (double)component;
            key.weight_left = 0.5;
            key.weight_right = 0.3;
            key.slope_left = -1.0;
            key.slope_right = 0.75;
            ufbxw_anim_curve_add_keyframe_key(scene, curve, key);
            key.time = 2 * UFBXW_KTIME_SECOND;
            key.value = 3.0 + (double)component;
            key.weight_left = 0.2;
            key.weight_right = 0.333333;
            key.slope_left = 0.25;
            key.slope_right = 0.0;
            ufbxw_anim_curve_add_keyframe_key(scene, curve, key);
            ufbxw_anim_curve_set_post_extrapolation(scene, curve, UFBXW_EXTRAPOLATION_REPEAT);
            ufbxw_anim_curve_set_post_extrapolation_repeat_count(scene, curve, 3);
        }

        ufbxw_anim_prop number = ufbxw_animate_prop(scene, props_node.id, "MyNumber", layer);
        ufbxw_anim_add_keyframe_real(scene, number, 0, 2.5, UFBXW_KEYFRAME_INTERPOLATION_LINEAR);
        ufbxw_anim_add_keyframe_real(scene, number, UFBXW_KTIME_SECOND, 7.5, UFBXW_KEYFRAME_INTERPOLATION_LINEAR);

        ufbxw_anim_prop color = ufbxw_animate_prop(scene, quad_material.id, "DiffuseColor", layer);
        ufbxw_anim_add_keyframe_vec3(scene, color, 0, rvo_probe_vec3(1.0, 0.0, 0.0), UFBXW_KEYFRAME_INTERPOLATION_LINEAR);
        ufbxw_anim_add_keyframe_vec3(scene, color, 2 * UFBXW_KTIME_SECOND, rvo_probe_vec3(0.0, 0.0, 1.0), UFBXW_KEYFRAME_INTERPOLATION_LINEAR);

        /* P5: a curve node that carries only its defaults — a curve on Y
         * alone, none on X and Z. */
        ufbxw_anim_prop scaling = ufbxw_animate_prop_masked(scene, props_node.id, "Lcl Scaling", layer, 1u << 1);
        ufbxw_anim_set_default_value(scene, scaling, 0, 1.0);
        ufbxw_anim_set_default_value(scene, scaling, 1, 1.0);
        ufbxw_anim_set_default_value(scene, scaling, 2, 1.0);
        {
            ufbxw_anim_curve curve = ufbxw_anim_get_curve(scene, scaling, 0);
            ufbxw_keyframe_real key;
            memset(&key, 0, sizeof(key));
            key.flags = UFBXW_KEYFRAME_INTERPOLATION_LINEAR;
            key.time = 0;
            key.value = 1.0;
            ufbxw_anim_curve_add_keyframe_key(scene, curve, key);
            key.time = UFBXW_KTIME_SECOND;
            key.value = 2.0;
            ufbxw_anim_curve_add_keyframe_key(scene, curve, key);
        }
        ufbxw_anim_prop rotation = ufbxw_animate_prop_masked(scene, props_node.id, "Lcl Rotation", layer, 0);
        ufbxw_anim_set_default_value(scene, rotation, 0, 0.0);
        ufbxw_anim_set_default_value(scene, rotation, 1, 90.0);
        ufbxw_anim_set_default_value(scene, rotation, 2, 0.0);

        ufbxw_anim_prop visibility = ufbxw_animate_prop(scene, quad_node.id, "Visibility", layer);
        ufbxw_anim_add_keyframe_real(scene, visibility, 0, 1.0, UFBXW_KEYFRAME_INTERPOLATION_CONSTANT);
        ufbxw_anim_add_keyframe_real(scene, visibility, UFBXW_KTIME_SECOND, 0.0, UFBXW_KEYFRAME_INTERPOLATION_CONSTANT_NEXT);
        ufbxw_anim_add_keyframe_real(scene, visibility, 2 * UFBXW_KTIME_SECOND, 1.0, UFBXW_KEYFRAME_INTERPOLATION_CONSTANT);
    }

    ufbxw_error build_error;
    memset(&build_error, 0, sizeof(build_error));
    if (ufbxw_get_error(scene, &build_error)) {
        rvo_probe_error(error, error_length, &build_error, "failed to build the probe scene");
        goto cleanup;
    }

    ufbxw_prepare_scene(scene, &ufbxw_default_prepare_opts);

    ufbxw_save_opts save_opts;
    memset(&save_opts, 0, sizeof(save_opts));
    save_opts.format = ascii ? UFBXW_SAVE_FORMAT_ASCII : UFBXW_SAVE_FORMAT_BINARY;
    save_opts.version = RVO_PROBE_FBX_VERSION;

    ufbxw_error save_error;
    memset(&save_error, 0, sizeof(save_error));
    if (!ufbxw_save_file(scene, path, &save_opts, &save_error)) {
        rvo_probe_error(error, error_length, &save_error, "failed to write the probe file");
        goto cleanup;
    }

    status = 0;

cleanup:
    ufbxw_free_scene(scene);
    return status;
}
