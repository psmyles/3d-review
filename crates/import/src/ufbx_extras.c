/*
 * The source-property capture (see ufbx_extras.h for the shape of the data).
 *
 * Called by `review_import_load_fbx` once the geometry, skin, morph and
 * animation passes have run, against the same ufbx scene, so the numbering it
 * uses (nodes, materials, corners, logical vertices, faces, morph channels,
 * clips) is exactly the one the model was published with.
 *
 * Unlike the geometry passes this uses growable arrays rather than a
 * count→fill pair: nothing here is sized by an arithmetic budget that a second
 * pass could drift from, every push is bounds-checked by construction, and the
 * only failure is out-of-memory, which is reported rather than overrun. The two
 * arenas (`strings`, `bytes`) hold every string and binary payload as offsets,
 * so the capture owns a fixed set of allocations and `review_import_free_extras`
 * is a plain list of `free`s — on every path, success or failure.
 */
#include "ufbx_extras.h"

#include "ufbx.h"

#include <stdlib.h>
#include <string.h>

/* Declared in ufbx_bridge.c: the geometry output this capture is indexed like,
   and its error type. Only the fields the capture reads are named here. */
typedef struct review_import_error review_import_error;
void review_import_set_error_message(review_import_error *out_error, const char *message);

/* --------------------------------------------------------------------------
 * Growable storage
 * -------------------------------------------------------------------------- */

static int review_extras_grow(void **data, size_t *capacity, size_t needed, size_t elem_size)
{
    size_t new_capacity;
    void *grown;
    if (needed <= *capacity) {
        return 1;
    }
    new_capacity = *capacity ? *capacity : 16;
    while (new_capacity < needed) {
        if (new_capacity > SIZE_MAX / 2) {
            return 0;
        }
        new_capacity *= 2;
    }
    if (new_capacity > SIZE_MAX / elem_size) {
        return 0;
    }
    grown = realloc(*data, new_capacity * elem_size);
    if (!grown) {
        return 0;
    }
    *data = grown;
    *capacity = new_capacity;
    return 1;
}

/* Append `count` elements of `elem_size` bytes; returns the index of the first
   appended element, or SIZE_MAX on failure. */
static size_t review_extras_push(void **data, size_t *count, size_t *capacity, size_t elem_size,
                                 const void *elems, size_t elem_count)
{
    size_t first = *count;
    if (elem_count > SIZE_MAX - first) {
        return SIZE_MAX;
    }
    if (!review_extras_grow(data, capacity, first + elem_count, elem_size)) {
        return SIZE_MAX;
    }
    if (elem_count > 0) {
        if (elems) {
            memcpy((char *)*data + first * elem_size, elems, elem_count * elem_size);
        } else {
            memset((char *)*data + first * elem_size, 0, elem_count * elem_size);
        }
    }
    *count = first + elem_count;
    return first;
}

/* Every list in the capture is kept as (data, count, capacity) triples in this
   builder while filling, then handed to the output struct. */
typedef struct review_extras_builder {
    review_import_extras *out;
    const ufbx_scene *scene;
    int failed;

    size_t prop_capacity;
    size_t node_capacity, light_capacity, camera_capacity, lod_group_capacity, lod_level_capacity;
    size_t material_capacity, material_texture_capacity, texture_capacity, texture_layer_capacity,
        video_capacity;
    size_t mesh_capacity, color_set_capacity, color_value_capacity, edge_capacity, face_capacity,
        vertex_crease_capacity, face_group_capacity, extra_skin_capacity, extra_cluster_capacity,
        extra_skin_offset_capacity, extra_influence_capacity, dq_weight_capacity;
    size_t pose_capacity, pose_entry_capacity;
    size_t display_layer_capacity, layer_node_capacity;
    size_t selection_set_capacity, selection_node_capacity, selection_index_capacity;
    size_t anim_stack_capacity, stack_layer_capacity, anim_layer_capacity, anim_prop_capacity,
        anim_curve_capacity, anim_key_capacity;
} review_extras_builder;

/* Each list gets a tiny typed push helper. They all return the first appended
   index, SIZE_MAX on OOM (and mark the builder failed). */
#define REVIEW_EXTRAS_LIST(name, type, field, count_field, cap_field)                       \
    static size_t review_extras_push_##name(review_extras_builder *b, const type *elem, \
                                            size_t n)                                   \
    {                                                                                   \
        size_t first = review_extras_push((void **)&b->out->field, &b->out->count_field, \
                                          &b->cap_field, sizeof(type), elem, n);        \
        if (first == SIZE_MAX) {                                                        \
            b->failed = 1;                                                              \
        }                                                                               \
        return first;                                                                   \
    }

REVIEW_EXTRAS_LIST(prop, review_import_prop, props, prop_count, prop_capacity)
REVIEW_EXTRAS_LIST(node, review_import_node_extras, nodes, node_count, node_capacity)
REVIEW_EXTRAS_LIST(light, review_import_light_extras, lights, light_count, light_capacity)
REVIEW_EXTRAS_LIST(camera, review_import_camera_extras, cameras, camera_count, camera_capacity)
REVIEW_EXTRAS_LIST(lod_group, review_import_lod_group_extras, lod_groups, lod_group_count, lod_group_capacity)
REVIEW_EXTRAS_LIST(lod_level, review_import_lod_level, lod_levels, lod_level_count, lod_level_capacity)
REVIEW_EXTRAS_LIST(material, review_import_material_extras, materials, material_count, material_capacity)
REVIEW_EXTRAS_LIST(material_texture, review_import_material_texture, material_textures, material_texture_count, material_texture_capacity)
REVIEW_EXTRAS_LIST(texture, review_import_texture_extras, textures, texture_count, texture_capacity)
REVIEW_EXTRAS_LIST(texture_layer, review_import_texture_layer, texture_layers, texture_layer_count, texture_layer_capacity)
REVIEW_EXTRAS_LIST(video, review_import_video_extras, videos, video_count, video_capacity)
REVIEW_EXTRAS_LIST(mesh, review_import_mesh_extras, meshes, mesh_count, mesh_capacity)
REVIEW_EXTRAS_LIST(color_set, review_import_color_set, color_sets, color_set_count, color_set_capacity)
REVIEW_EXTRAS_LIST(color_value, double, color_values, color_value_count, color_value_capacity)
REVIEW_EXTRAS_LIST(edge, uint32_t, edges, edge_count, edge_capacity)
REVIEW_EXTRAS_LIST(vertex_crease, double, vertex_crease, vertex_crease_count, vertex_crease_capacity)
REVIEW_EXTRAS_LIST(face_group, review_import_face_group, face_groups, face_group_count, face_group_capacity)
REVIEW_EXTRAS_LIST(extra_skin, review_import_extra_skin, extra_skins, extra_skin_count, extra_skin_capacity)
REVIEW_EXTRAS_LIST(extra_cluster, review_import_extra_cluster, extra_clusters, extra_cluster_count, extra_cluster_capacity)
REVIEW_EXTRAS_LIST(extra_skin_offset, uint32_t, extra_skin_offsets, extra_skin_offset_count, extra_skin_offset_capacity)
REVIEW_EXTRAS_LIST(extra_influence, review_import_extra_influence, extra_influences, extra_influence_count, extra_influence_capacity)
REVIEW_EXTRAS_LIST(dq_weight, review_import_dq_weight, dq_weights, dq_weight_count, dq_weight_capacity)
REVIEW_EXTRAS_LIST(pose, review_import_pose_extras, poses, pose_count, pose_capacity)
REVIEW_EXTRAS_LIST(pose_entry, review_import_pose_entry, pose_entries, pose_entry_count, pose_entry_capacity)
REVIEW_EXTRAS_LIST(display_layer, review_import_display_layer_extras, display_layers, display_layer_count, display_layer_capacity)
REVIEW_EXTRAS_LIST(layer_node, uint32_t, layer_nodes, layer_node_count, layer_node_capacity)
REVIEW_EXTRAS_LIST(selection_set, review_import_selection_set_extras, selection_sets, selection_set_count, selection_set_capacity)
REVIEW_EXTRAS_LIST(selection_node, review_import_selection_node, selection_nodes, selection_node_count, selection_node_capacity)
REVIEW_EXTRAS_LIST(selection_index, uint32_t, selection_indices, selection_index_count, selection_index_capacity)
REVIEW_EXTRAS_LIST(anim_stack, review_import_anim_stack_extras, anim_stacks, anim_stack_count, anim_stack_capacity)
REVIEW_EXTRAS_LIST(stack_layer, uint32_t, stack_layers, stack_layer_count, stack_layer_capacity)
REVIEW_EXTRAS_LIST(anim_layer, review_import_anim_layer_extras, anim_layers, anim_layer_count, anim_layer_capacity)
REVIEW_EXTRAS_LIST(anim_prop, review_import_anim_prop_extras, anim_props, anim_prop_count, anim_prop_capacity)
REVIEW_EXTRAS_LIST(anim_curve, review_import_anim_curve_extras, anim_curves, anim_curve_count, anim_curve_capacity)
REVIEW_EXTRAS_LIST(anim_key, review_import_anim_key, anim_keys, anim_key_count, anim_key_capacity)

/* The per-face and per-edge layer arrays grow together with `face_count` /
   `edge_count`, so they are pushed as a set. */
static int review_extras_push_face_layers(review_extras_builder *b, size_t n)
{
    review_import_extras *o = b->out;
    size_t first = o->face_count;
    if (n == 0) {
        return 1;
    }
    if (n > SIZE_MAX - first) {
        b->failed = 1;
        return 0;
    }
    if (!review_extras_grow((void **)&o->face_smoothing, &b->face_capacity, first + n, sizeof(uint8_t))) {
        b->failed = 1;
        return 0;
    }
    /* The three arrays share one capacity counter, so grow the other two to the
       same size whenever the first grew. */
    {
        uint8_t *hole = (uint8_t *)realloc(o->face_hole, b->face_capacity * sizeof(uint8_t));
        uint32_t *group;
        if (!hole) {
            b->failed = 1;
            return 0;
        }
        o->face_hole = hole;
        group = (uint32_t *)realloc(o->face_group, b->face_capacity * sizeof(uint32_t));
        if (!group) {
            b->failed = 1;
            return 0;
        }
        o->face_group = group;
    }
    memset(o->face_smoothing + first, 0, n * sizeof(uint8_t));
    memset(o->face_hole + first, 0, n * sizeof(uint8_t));
    memset(o->face_group + first, 0, n * sizeof(uint32_t));
    o->face_count = first + n;
    return 1;
}

static int review_extras_push_edge_layers(review_extras_builder *b, size_t n)
{
    review_import_extras *o = b->out;
    size_t first = o->edge_count;
    size_t pair_count;
    if (n == 0) {
        return 1;
    }
    if (n > SIZE_MAX / 2 || n > SIZE_MAX - first) {
        b->failed = 1;
        return 0;
    }
    pair_count = (first + n) * 2;
    if (!review_extras_grow((void **)&o->edges, &b->edge_capacity, pair_count, sizeof(uint32_t))) {
        b->failed = 1;
        return 0;
    }
    {
        /* `edge_capacity` counts uint32 pairs' halves; the per-edge arrays need
           half as many entries, but sharing the larger count is harmless. */
        uint8_t *smoothing = (uint8_t *)realloc(o->edge_smoothing, b->edge_capacity * sizeof(uint8_t));
        double *crease;
        uint8_t *visibility;
        if (!smoothing) {
            b->failed = 1;
            return 0;
        }
        o->edge_smoothing = smoothing;
        crease = (double *)realloc(o->edge_crease, b->edge_capacity * sizeof(double));
        if (!crease) {
            b->failed = 1;
            return 0;
        }
        o->edge_crease = crease;
        visibility = (uint8_t *)realloc(o->edge_visibility, b->edge_capacity * sizeof(uint8_t));
        if (!visibility) {
            b->failed = 1;
            return 0;
        }
        o->edge_visibility = visibility;
    }
    memset(o->edges + first * 2, 0, n * 2 * sizeof(uint32_t));
    memset(o->edge_smoothing + first, 0, n * sizeof(uint8_t));
    memset(o->edge_crease + first, 0, n * sizeof(double));
    memset(o->edge_visibility + first, 0, n * sizeof(uint8_t));
    o->edge_count = first + n;
    return 1;
}

static review_import_str review_extras_str(review_extras_builder *b, ufbx_string str)
{
    review_import_str out = { 0, 0 };
    size_t first;
    if (str.length == 0 || !str.data) {
        return out;
    }
    if (str.length > UINT32_MAX) {
        b->failed = 1;
        return out;
    }
    first = review_extras_push((void **)&b->out->strings, &b->out->string_count,
                               &b->out->string_capacity, 1, str.data, str.length);
    if (first == SIZE_MAX || first > UINT32_MAX) {
        b->failed = 1;
        return out;
    }
    out.offset = (uint32_t)first;
    out.length = (uint32_t)str.length;
    return out;
}

static review_import_bytes review_extras_bytes(review_extras_builder *b, ufbx_blob blob)
{
    review_import_bytes out = { 0, 0 };
    size_t first;
    if (blob.size == 0 || !blob.data) {
        return out;
    }
    first = review_extras_push((void **)&b->out->bytes, &b->out->byte_count,
                               &b->out->byte_capacity, 1, blob.data, blob.size);
    if (first == SIZE_MAX) {
        b->failed = 1;
        return out;
    }
    out.offset = first;
    out.length = blob.size;
    return out;
}

static void review_extras_matrix(double out[16], const ufbx_matrix *matrix)
{
    size_t col;
    for (col = 0; col < 4; col++) {
        out[col * 4 + 0] = matrix->cols[col].x;
        out[col * 4 + 1] = matrix->cols[col].y;
        out[col * 4 + 2] = matrix->cols[col].z;
        out[col * 4 + 3] = (col == 3) ? 1.0 : 0.0;
    }
}

/* Capture every explicit (non-synthetic) property of `props`. */
static review_import_prop_range review_extras_props(review_extras_builder *b, const ufbx_props *props)
{
    review_import_prop_range range = { 0, 0 };
    size_t index;
    size_t first = b->out->prop_count;
    if (!props) {
        return range;
    }
    for (index = 0; index < props->props.count; index++) {
        const ufbx_prop *src = &props->props.data[index];
        review_import_prop dst;
        if (src->flags & UFBX_PROP_FLAG_SYNTHETIC) {
            continue;
        }
        memset(&dst, 0, sizeof(dst));
        dst.name = review_extras_str(b, src->name);
        dst.type = (uint32_t)src->type;
        dst.flags = (uint32_t)src->flags;
        dst.value_int = src->value_int;
        dst.value_real[0] = src->value_real_arr[0];
        dst.value_real[1] = src->value_real_arr[1];
        dst.value_real[2] = src->value_real_arr[2];
        dst.value_real[3] = src->value_real_arr[3];
        dst.value_str = review_extras_str(b, src->value_str);
        dst.value_blob = review_extras_bytes(b, src->value_blob);
        review_extras_push_prop(b, &dst, 1);
    }
    if (first > UINT32_MAX || b->out->prop_count - first > UINT32_MAX) {
        b->failed = 1;
        return range;
    }
    range.first = (uint32_t)first;
    range.count = (uint32_t)(b->out->prop_count - first);
    return range;
}

/* --------------------------------------------------------------------------
 * Scene / nodes / attributes
 * -------------------------------------------------------------------------- */

static void review_extras_capture_scene(review_extras_builder *b)
{
    const ufbx_scene *scene = b->scene;
    review_import_scene_extras *dst = &b->out->scene;
    memset(dst, 0, sizeof(*dst));
    dst->creator = review_extras_str(b, scene->metadata.creator);
    dst->filename = review_extras_str(b, scene->metadata.filename);
    dst->original_file_path = review_extras_str(b, scene->metadata.original_file_path);
    dst->version = scene->metadata.version;
    dst->ascii = scene->metadata.ascii ? 1u : 0u;
    dst->original_vendor = review_extras_str(b, scene->metadata.original_application.vendor);
    dst->original_name = review_extras_str(b, scene->metadata.original_application.name);
    dst->original_version = review_extras_str(b, scene->metadata.original_application.version);
    dst->latest_vendor = review_extras_str(b, scene->metadata.latest_application.vendor);
    dst->latest_name = review_extras_str(b, scene->metadata.latest_application.name);
    dst->latest_version = review_extras_str(b, scene->metadata.latest_application.version);
    dst->scene_props = review_extras_props(b, &scene->metadata.scene_props);
    dst->settings_props = review_extras_props(b, &scene->settings.props);
    dst->axis_right = (uint32_t)scene->settings.axes.right;
    dst->axis_up = (uint32_t)scene->settings.axes.up;
    dst->axis_front = (uint32_t)scene->settings.axes.front;
    dst->original_axis_up = (uint32_t)scene->settings.original_axis_up;
    dst->unit_meters = scene->settings.unit_meters;
    dst->original_unit_meters = scene->settings.original_unit_meters;
    dst->frames_per_second = scene->settings.frames_per_second;
    dst->ambient_color[0] = scene->settings.ambient_color.x;
    dst->ambient_color[1] = scene->settings.ambient_color.y;
    dst->ambient_color[2] = scene->settings.ambient_color.z;
    dst->default_camera = review_extras_str(b, scene->settings.default_camera);
    dst->time_mode = (uint32_t)scene->settings.time_mode;
    dst->time_protocol = (uint32_t)scene->settings.time_protocol;
    dst->snap_mode = (uint32_t)scene->settings.snap_mode;
}

static void review_extras_capture_light(review_extras_builder *b, const ufbx_light *light,
                                        review_import_node_extras *dst)
{
    review_import_light_extras out;
    size_t index;
    memset(&out, 0, sizeof(out));
    out.color[0] = light->color.x;
    out.color[1] = light->color.y;
    out.color[2] = light->color.z;
    out.intensity = light->intensity;
    out.local_direction[0] = light->local_direction.x;
    out.local_direction[1] = light->local_direction.y;
    out.local_direction[2] = light->local_direction.z;
    out.type = (uint32_t)light->type;
    out.decay = (uint32_t)light->decay;
    out.area_shape = (uint32_t)light->area_shape;
    out.inner_angle = light->inner_angle;
    out.outer_angle = light->outer_angle;
    out.cast_light = light->cast_light ? 1u : 0u;
    out.cast_shadows = light->cast_shadows ? 1u : 0u;
    index = review_extras_push_light(b, &out, 1);
    dst->attribute_kind = REVIEW_IMPORT_ATTRIB_LIGHT;
    dst->attribute_index = index == SIZE_MAX || index > INT32_MAX ? -1 : (int32_t)index;
}

static void review_extras_capture_camera(review_extras_builder *b, const ufbx_camera *camera,
                                         review_import_node_extras *dst)
{
    review_import_camera_extras out;
    size_t index;
    memset(&out, 0, sizeof(out));
    out.projection_mode = (uint32_t)camera->projection_mode;
    out.resolution_is_pixels = camera->resolution_is_pixels ? 1u : 0u;
    out.resolution[0] = camera->resolution.x;
    out.resolution[1] = camera->resolution.y;
    out.field_of_view_deg[0] = camera->field_of_view_deg.x;
    out.field_of_view_deg[1] = camera->field_of_view_deg.y;
    out.orthographic_extent = camera->orthographic_extent;
    out.aspect_ratio = camera->aspect_ratio;
    out.near_plane = camera->near_plane;
    out.far_plane = camera->far_plane;
    out.aspect_mode = (uint32_t)camera->aspect_mode;
    out.aperture_mode = (uint32_t)camera->aperture_mode;
    out.gate_fit = (uint32_t)camera->gate_fit;
    out.aperture_format = (uint32_t)camera->aperture_format;
    out.focal_length_mm = camera->focal_length_mm;
    out.film_size_inch[0] = camera->film_size_inch.x;
    out.film_size_inch[1] = camera->film_size_inch.y;
    out.aperture_size_inch[0] = camera->aperture_size_inch.x;
    out.aperture_size_inch[1] = camera->aperture_size_inch.y;
    out.squeeze_ratio = camera->squeeze_ratio;
    index = review_extras_push_camera(b, &out, 1);
    dst->attribute_kind = REVIEW_IMPORT_ATTRIB_CAMERA;
    dst->attribute_index = index == SIZE_MAX || index > INT32_MAX ? -1 : (int32_t)index;
}

static void review_extras_capture_lod_group(review_extras_builder *b, const ufbx_lod_group *group,
                                            review_import_node_extras *dst)
{
    review_import_lod_group_extras out;
    size_t index;
    size_t level;
    size_t first = b->out->lod_level_count;
    memset(&out, 0, sizeof(out));
    out.relative_distances = group->relative_distances ? 1u : 0u;
    out.ignore_parent_transform = group->ignore_parent_transform ? 1u : 0u;
    out.use_distance_limit = group->use_distance_limit ? 1u : 0u;
    out.distance_limit_min = group->distance_limit_min;
    out.distance_limit_max = group->distance_limit_max;
    for (level = 0; level < group->lod_levels.count; level++) {
        review_import_lod_level entry;
        entry.distance = group->lod_levels.data[level].distance;
        entry.display = (uint32_t)group->lod_levels.data[level].display;
        review_extras_push_lod_level(b, &entry, 1);
    }
    if (first > UINT32_MAX || group->lod_levels.count > UINT32_MAX) {
        b->failed = 1;
        return;
    }
    out.level_first = (uint32_t)first;
    out.level_count = (uint32_t)group->lod_levels.count;
    index = review_extras_push_lod_group(b, &out, 1);
    dst->attribute_kind = REVIEW_IMPORT_ATTRIB_LOD_GROUP;
    dst->attribute_index = index == SIZE_MAX || index > INT32_MAX ? -1 : (int32_t)index;
}

static void review_extras_capture_nodes(review_extras_builder *b)
{
    const ufbx_scene *scene = b->scene;
    size_t node_index;
    for (node_index = 0; node_index < scene->nodes.count; node_index++) {
        const ufbx_node *node = scene->nodes.data[node_index];
        review_import_node_extras dst;
        memset(&dst, 0, sizeof(dst));
        dst.attribute_index = -1;
        if (!node) {
            /* Mirrors the geometry pass's tolerance of a null entry: an inert
               row, identity geometry transform. */
            dst.geometry_to_node[0] = dst.geometry_to_node[5] = dst.geometry_to_node[10] =
                dst.geometry_to_node[15] = 1.0;
            dst.visible = 1;
            review_extras_push_node(b, &dst, 1);
            continue;
        }
        dst.props = review_extras_props(b, &node->props);
        dst.rotation_order = (uint32_t)node->rotation_order;
        dst.inherit_mode = (uint32_t)node->inherit_mode;
        dst.original_inherit_mode = (uint32_t)node->original_inherit_mode;
        review_extras_matrix(dst.geometry_to_node, &node->geometry_to_node);
        if (node->is_root) {
            dst.synthetic = REVIEW_IMPORT_SYNTHETIC_ROOT;
        } else if (node->is_scale_helper) {
            dst.synthetic = REVIEW_IMPORT_SYNTHETIC_SCALE_HELPER;
        } else if (node->is_geometry_transform_helper) {
            dst.synthetic = REVIEW_IMPORT_SYNTHETIC_GEOMETRY_TRANSFORM_HELPER;
        }
        dst.visible = node->visible ? 1u : 0u;
        if (node->attrib) {
            dst.attribute_name = review_extras_str(b, node->attrib->name);
            dst.attribute_props = review_extras_props(b, &node->attrib->props);
            switch (node->attrib_type) {
            case UFBX_ELEMENT_MESH:
                dst.attribute_kind = REVIEW_IMPORT_ATTRIB_MESH;
                break;
            case UFBX_ELEMENT_BONE:
                dst.attribute_kind = REVIEW_IMPORT_ATTRIB_BONE;
                break;
            case UFBX_ELEMENT_LIGHT:
                review_extras_capture_light(b, (const ufbx_light *)node->attrib, &dst);
                break;
            case UFBX_ELEMENT_CAMERA:
                review_extras_capture_camera(b, (const ufbx_camera *)node->attrib, &dst);
                break;
            case UFBX_ELEMENT_EMPTY:
                dst.attribute_kind = REVIEW_IMPORT_ATTRIB_EMPTY;
                break;
            case UFBX_ELEMENT_LOD_GROUP:
                review_extras_capture_lod_group(b, (const ufbx_lod_group *)node->attrib, &dst);
                break;
            default:
                dst.attribute_kind = REVIEW_IMPORT_ATTRIB_OTHER;
                break;
            }
        }
        review_extras_push_node(b, &dst, 1);
    }
}

/* --------------------------------------------------------------------------
 * Materials, textures, videos
 * -------------------------------------------------------------------------- */

static void review_extras_capture_textures(review_extras_builder *b)
{
    const ufbx_scene *scene = b->scene;
    size_t index;
    for (index = 0; index < scene->textures.count; index++) {
        const ufbx_texture *texture = scene->textures.data[index];
        review_import_texture_extras dst;
        size_t layer;
        size_t layer_first = b->out->texture_layer_count;
        memset(&dst, 0, sizeof(dst));
        dst.video = -1;
        if (!texture) {
            review_extras_push_texture(b, &dst, 1);
            continue;
        }
        dst.name = review_extras_str(b, texture->name);
        dst.type = (uint32_t)texture->type;
        dst.filename = review_extras_str(b, texture->filename);
        dst.absolute_filename = review_extras_str(b, texture->absolute_filename);
        dst.relative_filename = review_extras_str(b, texture->relative_filename);
        dst.uv_set = review_extras_str(b, texture->uv_set);
        dst.wrap_u = (uint32_t)texture->wrap_u;
        dst.wrap_v = (uint32_t)texture->wrap_v;
        dst.has_uv_transform = texture->has_uv_transform ? 1u : 0u;
        dst.uv_translation[0] = texture->uv_transform.translation.x;
        dst.uv_translation[1] = texture->uv_transform.translation.y;
        dst.uv_translation[2] = texture->uv_transform.translation.z;
        dst.uv_rotation[0] = texture->uv_transform.rotation.x;
        dst.uv_rotation[1] = texture->uv_transform.rotation.y;
        dst.uv_rotation[2] = texture->uv_transform.rotation.z;
        dst.uv_rotation[3] = texture->uv_transform.rotation.w;
        dst.uv_scale[0] = texture->uv_transform.scale.x;
        dst.uv_scale[1] = texture->uv_transform.scale.y;
        dst.uv_scale[2] = texture->uv_transform.scale.z;
        dst.content = review_extras_bytes(b, texture->content);
        if (texture->video && texture->video->typed_id < scene->videos.count &&
            texture->video->typed_id <= INT32_MAX) {
            dst.video = (int32_t)texture->video->typed_id;
        }
        for (layer = 0; layer < texture->layers.count; layer++) {
            const ufbx_texture_layer *src = &texture->layers.data[layer];
            review_import_texture_layer entry;
            if (!src->texture || src->texture->typed_id >= scene->textures.count) {
                continue;
            }
            entry.texture = src->texture->typed_id;
            entry.blend_mode = (uint32_t)src->blend_mode;
            entry.alpha = src->alpha;
            review_extras_push_texture_layer(b, &entry, 1);
        }
        if (layer_first > UINT32_MAX || b->out->texture_layer_count - layer_first > UINT32_MAX) {
            b->failed = 1;
            return;
        }
        dst.layer_first = (uint32_t)layer_first;
        dst.layer_count = (uint32_t)(b->out->texture_layer_count - layer_first);
        dst.props = review_extras_props(b, &texture->props);
        review_extras_push_texture(b, &dst, 1);
    }
}

static void review_extras_capture_videos(review_extras_builder *b)
{
    const ufbx_scene *scene = b->scene;
    size_t index;
    for (index = 0; index < scene->videos.count; index++) {
        const ufbx_video *video = scene->videos.data[index];
        review_import_video_extras dst;
        memset(&dst, 0, sizeof(dst));
        if (!video) {
            review_extras_push_video(b, &dst, 1);
            continue;
        }
        dst.name = review_extras_str(b, video->name);
        dst.filename = review_extras_str(b, video->filename);
        dst.absolute_filename = review_extras_str(b, video->absolute_filename);
        dst.relative_filename = review_extras_str(b, video->relative_filename);
        dst.content = review_extras_bytes(b, video->content);
        dst.props = review_extras_props(b, &video->props);
        review_extras_push_video(b, &dst, 1);
    }
}

/* `material_sources[i]` is the ufbx material the bridge's deduplicated slot `i`
   came from (NULL for a slot with no source). */
static void review_extras_capture_materials(review_extras_builder *b,
                                            const ufbx_material *const *material_sources,
                                            size_t material_count)
{
    const ufbx_scene *scene = b->scene;
    size_t index;
    for (index = 0; index < material_count; index++) {
        const ufbx_material *material = material_sources ? material_sources[index] : NULL;
        review_import_material_extras dst;
        size_t texture;
        size_t texture_first = b->out->material_texture_count;
        memset(&dst, 0, sizeof(dst));
        if (!material) {
            review_extras_push_material(b, &dst, 1);
            continue;
        }
        dst.props = review_extras_props(b, &material->props);
        dst.shader_type = (uint32_t)material->shader_type;
        dst.shading_model = review_extras_str(b, material->shading_model_name);
        for (texture = 0; texture < material->textures.count; texture++) {
            const ufbx_material_texture *src = &material->textures.data[texture];
            review_import_material_texture entry;
            if (!src->texture || src->texture->typed_id >= scene->textures.count) {
                continue;
            }
            entry.material_prop = review_extras_str(b, src->material_prop);
            entry.shader_prop = review_extras_str(b, src->shader_prop);
            entry.texture = src->texture->typed_id;
            review_extras_push_material_texture(b, &entry, 1);
        }
        if (texture_first > UINT32_MAX || b->out->material_texture_count - texture_first > UINT32_MAX) {
            b->failed = 1;
            return;
        }
        dst.texture_first = (uint32_t)texture_first;
        dst.texture_count = (uint32_t)(b->out->material_texture_count - texture_first);
        review_extras_push_material(b, &dst, 1);
    }
}

/* --------------------------------------------------------------------------
 * Meshes
 * -------------------------------------------------------------------------- */

static ufbx_vec4 review_extras_get_vec4(const ufbx_vertex_vec4 *attr, size_t index, int *ok)
{
    ufbx_panic panic;
    ufbx_vec4 value;
    panic.did_panic = false;
    value = ufbx_catch_get_vertex_vec4(&panic, attr, index);
    if (panic.did_panic) {
        *ok = 0;
        memset(&value, 0, sizeof(value));
    }
    return value;
}

static ufbx_real review_extras_get_real(const ufbx_vertex_real *attr, size_t index, int *ok)
{
    ufbx_panic panic;
    ufbx_real value;
    panic.did_panic = false;
    value = ufbx_catch_get_vertex_real(&panic, attr, index);
    if (panic.did_panic) {
        *ok = 0;
        value = 0.0;
    }
    return value;
}

/* The bases the geometry fill gave each mesh part, recomputed here with the
   same traversal and skip rule. */
typedef struct review_extras_part_bases {
    uint32_t node;
    size_t corner_first;
    size_t corner_count;
    size_t logical_first;
    size_t logical_count;
    size_t face_first;
    size_t face_count;
} review_extras_part_bases;

static void review_extras_capture_extra_skins(review_extras_builder *b, const ufbx_mesh *mesh,
                                              const review_extras_part_bases *bases,
                                              review_import_mesh_extras *dst)
{
    const ufbx_scene *scene = b->scene;
    size_t deformer_index;
    size_t skin_first = b->out->extra_skin_count;
    for (deformer_index = 1; deformer_index < mesh->skin_deformers.count; deformer_index++) {
        const ufbx_skin_deformer *deformer = mesh->skin_deformers.data[deformer_index];
        review_import_extra_skin skin;
        size_t cluster_index;
        size_t vertex;
        size_t cluster_first = b->out->extra_cluster_count;
        size_t offset_first = b->out->extra_skin_offset_count;
        size_t influence_first = b->out->extra_influence_count;
        uint32_t *cluster_map;
        if (!deformer) {
            continue;
        }
        cluster_map = (uint32_t *)malloc((deformer->clusters.count ? deformer->clusters.count : 1) * sizeof(uint32_t));
        if (!cluster_map) {
            b->failed = 1;
            return;
        }
        for (cluster_index = 0; cluster_index < deformer->clusters.count; cluster_index++) {
            const ufbx_skin_cluster *cluster = deformer->clusters.data[cluster_index];
            review_import_extra_cluster entry;
            size_t local;
            cluster_map[cluster_index] = UINT32_MAX;
            if (!cluster || !cluster->bone_node || cluster->bone_node->typed_id >= scene->nodes.count) {
                continue;
            }
            memset(&entry, 0, sizeof(entry));
            entry.bone = cluster->bone_node->typed_id;
            entry.name = review_extras_str(b, cluster->name);
            review_extras_matrix(entry.mesh_node_to_bone, &cluster->mesh_node_to_bone);
            review_extras_matrix(entry.bind_to_world, &cluster->bind_to_world);
            local = b->out->extra_cluster_count - cluster_first;
            if (local > UINT32_MAX) {
                b->failed = 1;
                free(cluster_map);
                return;
            }
            cluster_map[cluster_index] = (uint32_t)local;
            review_extras_push_extra_cluster(b, &entry, 1);
        }
        for (vertex = 0; vertex < mesh->num_vertices; vertex++) {
            uint32_t row_start = (uint32_t)(b->out->extra_influence_count - influence_first);
            size_t weight_index;
            review_extras_push_extra_skin_offset(b, &row_start, 1);
            if (vertex >= deformer->vertices.count) {
                continue;
            }
            {
                ufbx_skin_vertex skin_vertex = deformer->vertices.data[vertex];
                for (weight_index = 0; weight_index < skin_vertex.num_weights; weight_index++) {
                    size_t global = (size_t)skin_vertex.weight_begin + weight_index;
                    ufbx_skin_weight weight;
                    review_import_extra_influence influence;
                    if (global >= deformer->weights.count) {
                        break;
                    }
                    weight = deformer->weights.data[global];
                    if (weight.cluster_index >= deformer->clusters.count ||
                        cluster_map[weight.cluster_index] == UINT32_MAX) {
                        continue;
                    }
                    if (!(weight.weight > 0.0) || !(weight.weight < 1e30)) {
                        continue;
                    }
                    influence.cluster = cluster_map[weight.cluster_index];
                    influence.weight = (float)weight.weight;
                    review_extras_push_extra_influence(b, &influence, 1);
                }
            }
        }
        {
            uint32_t row_end = (uint32_t)(b->out->extra_influence_count - influence_first);
            review_extras_push_extra_skin_offset(b, &row_end, 1);
        }
        free(cluster_map);
        if (cluster_first > UINT32_MAX || offset_first > UINT32_MAX || influence_first > UINT32_MAX ||
            b->out->extra_influence_count - influence_first > UINT32_MAX) {
            b->failed = 1;
            return;
        }
        memset(&skin, 0, sizeof(skin));
        skin.method = (uint32_t)deformer->skinning_method;
        skin.max_weights_per_vertex = deformer->max_weights_per_vertex > UINT32_MAX
            ? UINT32_MAX
            : (uint32_t)deformer->max_weights_per_vertex;
        skin.cluster_first = (uint32_t)cluster_first;
        skin.cluster_count = (uint32_t)(b->out->extra_cluster_count - cluster_first);
        skin.offset_first = (uint32_t)offset_first;
        skin.influence_first = (uint32_t)influence_first;
        skin.influence_count = (uint32_t)(b->out->extra_influence_count - influence_first);
        review_extras_push_extra_skin(b, &skin, 1);
    }
    if (skin_first > UINT32_MAX) {
        b->failed = 1;
        return;
    }
    dst->extra_skin_first = (uint32_t)skin_first;
    dst->extra_skin_count = (uint32_t)(b->out->extra_skin_count - skin_first);

    /* Dual-quaternion blend weights of the primary deformer. */
    {
        size_t dq_first = b->out->dq_weight_count;
        const ufbx_skin_deformer *primary = mesh->skin_deformers.count > 0 ? mesh->skin_deformers.data[0] : NULL;
        if (primary) {
            size_t index;
            size_t count = primary->dq_vertices.count < primary->dq_weights.count
                ? primary->dq_vertices.count
                : primary->dq_weights.count;
            for (index = 0; index < count; index++) {
                review_import_dq_weight entry;
                uint32_t vertex = primary->dq_vertices.data[index];
                if (vertex >= mesh->num_vertices) {
                    continue;
                }
                entry.logical_vertex = (uint32_t)(bases->logical_first + vertex);
                entry.weight = primary->dq_weights.data[index];
                review_extras_push_dq_weight(b, &entry, 1);
            }
        }
        if (dq_first > UINT32_MAX) {
            b->failed = 1;
            return;
        }
        dst->dq_first = (uint32_t)dq_first;
        dst->dq_count = (uint32_t)(b->out->dq_weight_count - dq_first);
    }
}

static int review_extras_capture_mesh(review_extras_builder *b, const ufbx_node *node,
                                      const ufbx_mesh *mesh, const review_extras_part_bases *bases)
{
    review_import_mesh_extras dst;
    uint32_t *corner_of_index = NULL;
    size_t face_index;
    size_t running = 0;
    size_t index;
    int ok = 1;

    memset(&dst, 0, sizeof(dst));
    dst.node = bases->node;
    dst.name = review_extras_str(b, mesh->name);
    dst.props = review_extras_props(b, &mesh->props);
    if (bases->corner_first > UINT32_MAX || bases->corner_count > UINT32_MAX ||
        bases->logical_first > UINT32_MAX || bases->logical_count > UINT32_MAX ||
        bases->face_first > UINT32_MAX || bases->face_count > UINT32_MAX) {
        return 0;
    }
    dst.corner_first = (uint32_t)bases->corner_first;
    dst.corner_count = (uint32_t)bases->corner_count;
    dst.logical_first = (uint32_t)bases->logical_first;
    dst.logical_count = (uint32_t)bases->logical_count;
    dst.face_first = (uint32_t)bases->face_first;
    dst.face_count = (uint32_t)bases->face_count;
    dst.tangents_authored = mesh->vertex_tangent.exists ? 1u : 0u;
    dst.reversed_winding = mesh->reversed_winding ? 1u : 0u;
    dst.subdivision_preview_levels = mesh->subdivision_preview_levels;
    dst.subdivision_render_levels = mesh->subdivision_render_levels;
    dst.subdivision_display_mode = (uint32_t)mesh->subdivision_display_mode;
    dst.subdivision_boundary = (uint32_t)mesh->subdivision_boundary;
    dst.subdivision_uv_boundary = (uint32_t)mesh->subdivision_uv_boundary;

    /* mesh index (a position in `vertex_indices`) -> global corner, walking the
       faces exactly as the geometry fill did. */
    if (mesh->num_indices > 0) {
        corner_of_index = (uint32_t *)malloc(mesh->num_indices * sizeof(uint32_t));
        if (!corner_of_index) {
            b->failed = 1;
            return 0;
        }
        memset(corner_of_index, 0xFF, mesh->num_indices * sizeof(uint32_t));
    }
    for (face_index = 0; face_index < mesh->faces.count; face_index++) {
        ufbx_face face = mesh->faces.data[face_index];
        size_t corner;
        for (corner = 0; corner < face.num_indices; corner++) {
            size_t mesh_index = (size_t)face.index_begin + corner;
            if (mesh_index < mesh->num_indices && running < bases->corner_count) {
                corner_of_index[mesh_index] = (uint32_t)(bases->corner_first + running);
            }
            running++;
        }
    }
    if (running != bases->corner_count) {
        free(corner_of_index);
        return 0;
    }

    /* Color sets: set 0's values are the geometry's vertex colors already; the
       rest are captured per corner in the same corner order. */
    {
        size_t set_first = b->out->color_set_count;
        size_t set_index;
        for (set_index = 0; set_index < mesh->color_sets.count; set_index++) {
            const ufbx_color_set *set = &mesh->color_sets.data[set_index];
            review_import_color_set entry;
            memset(&entry, 0, sizeof(entry));
            entry.name = review_extras_str(b, set->name);
            entry.index = set->index;
            entry.value_first = UINT32_MAX;
            if (set_index > 0 && set->vertex_color.exists) {
                size_t value_first = b->out->color_value_count;
                if (value_first > UINT32_MAX) {
                    b->failed = 1;
                    break;
                }
                for (face_index = 0; face_index < mesh->faces.count; face_index++) {
                    ufbx_face face = mesh->faces.data[face_index];
                    size_t corner;
                    for (corner = 0; corner < face.num_indices; corner++) {
                        ufbx_vec4 color = review_extras_get_vec4(&set->vertex_color, (size_t)face.index_begin + corner, &ok);
                        double values[4];
                        values[0] = color.x;
                        values[1] = color.y;
                        values[2] = color.z;
                        values[3] = color.w;
                        review_extras_push_color_value(b, values, 4);
                    }
                }
                entry.value_first = (uint32_t)value_first;
            }
            review_extras_push_color_set(b, &entry, 1);
        }
        if (set_first > UINT32_MAX) {
            b->failed = 1;
        } else {
            dst.color_set_first = (uint32_t)set_first;
            dst.color_set_count = (uint32_t)(b->out->color_set_count - set_first);
        }
    }

    /* Edges and their layers. */
    {
        size_t edge_first = b->out->edge_count;
        size_t edge_count = mesh->edges.count;
        if (edge_first > UINT32_MAX || edge_count > UINT32_MAX) {
            b->failed = 1;
        } else if (review_extras_push_edge_layers(b, edge_count)) {
            review_import_extras *o = b->out;
            dst.has_edge_smoothing = mesh->edge_smoothing.count >= edge_count && edge_count > 0 ? 1u : 0u;
            dst.has_edge_crease = mesh->edge_crease.count >= edge_count && edge_count > 0 ? 1u : 0u;
            dst.has_edge_visibility = mesh->edge_visibility.count >= edge_count && edge_count > 0 ? 1u : 0u;
            for (index = 0; index < edge_count; index++) {
                ufbx_edge edge = mesh->edges.data[index];
                uint32_t a = edge.a < mesh->num_indices ? corner_of_index[edge.a] : UINT32_MAX;
                uint32_t c = edge.b < mesh->num_indices ? corner_of_index[edge.b] : UINT32_MAX;
                o->edges[(edge_first + index) * 2 + 0] = a;
                o->edges[(edge_first + index) * 2 + 1] = c;
                if (dst.has_edge_smoothing) {
                    o->edge_smoothing[edge_first + index] = mesh->edge_smoothing.data[index] ? 1u : 0u;
                }
                if (dst.has_edge_crease) {
                    o->edge_crease[edge_first + index] = mesh->edge_crease.data[index];
                }
                if (dst.has_edge_visibility) {
                    o->edge_visibility[edge_first + index] = mesh->edge_visibility.data[index] ? 1u : 0u;
                }
            }
            dst.edge_first = (uint32_t)edge_first;
            dst.edge_count = (uint32_t)edge_count;
        }
    }

    /* Per-face layers, indexed by global face. */
    if (review_extras_push_face_layers(b, bases->face_count)) {
        review_import_extras *o = b->out;
        size_t count = bases->face_count;
        dst.has_face_smoothing = mesh->face_smoothing.count >= count && count > 0 ? 1u : 0u;
        dst.has_face_hole = mesh->face_hole.count >= count && count > 0 ? 1u : 0u;
        dst.has_face_group = mesh->face_group.count >= count && count > 0 ? 1u : 0u;
        for (index = 0; index < count; index++) {
            if (dst.has_face_smoothing) {
                o->face_smoothing[bases->face_first + index] = mesh->face_smoothing.data[index] ? 1u : 0u;
            }
            if (dst.has_face_hole) {
                o->face_hole[bases->face_first + index] = mesh->face_hole.data[index] ? 1u : 0u;
            }
            if (dst.has_face_group) {
                o->face_group[bases->face_first + index] = mesh->face_group.data[index];
            }
        }
    }

    /* Face groups (polygon group names). */
    {
        size_t group_first = b->out->face_group_count;
        for (index = 0; index < mesh->face_groups.count; index++) {
            review_import_face_group entry;
            entry.id = mesh->face_groups.data[index].id;
            entry.name = review_extras_str(b, mesh->face_groups.data[index].name);
            review_extras_push_face_group(b, &entry, 1);
        }
        if (group_first > UINT32_MAX) {
            b->failed = 1;
        } else {
            dst.face_group_first = (uint32_t)group_first;
            dst.face_group_count = (uint32_t)(b->out->face_group_count - group_first);
        }
    }

    /* Vertex creases, per logical vertex, indexed globally. The flat array must
       cover every logical vertex up to this part's end, so parts without the
       layer still push zeros. */
    {
        size_t needed = bases->logical_first + bases->logical_count;
        while (b->out->vertex_crease_count < needed && !b->failed) {
            double zero = 0.0;
            review_extras_push_vertex_crease(b, &zero, 1);
        }
        if (mesh->vertex_crease.exists && bases->logical_count > 0) {
            dst.has_vertex_crease = 1;
            for (index = 0; index < bases->logical_count && index < mesh->vertex_first_index.count; index++) {
                uint32_t first_corner = mesh->vertex_first_index.data[index];
                if (first_corner == UFBX_NO_INDEX || first_corner >= mesh->num_indices) {
                    continue;
                }
                b->out->vertex_crease[bases->logical_first + index] =
                    review_extras_get_real(&mesh->vertex_crease, first_corner, &ok);
            }
        }
    }

    review_extras_capture_extra_skins(b, mesh, bases, &dst);

    free(corner_of_index);
    (void)node;
    if (!ok) {
        return 0;
    }
    review_extras_push_mesh(b, &dst, 1);
    return 1;
}

static int review_extras_capture_meshes(review_extras_builder *b)
{
    const ufbx_scene *scene = b->scene;
    size_t node_index;
    review_extras_part_bases bases;
    memset(&bases, 0, sizeof(bases));
    for (node_index = 0; node_index < scene->nodes.count; node_index++) {
        const ufbx_node *node = scene->nodes.data[node_index];
        const ufbx_mesh *mesh = node ? node->mesh : NULL;
        size_t corners = 0;
        size_t face_index;
        if (!mesh || !mesh->vertex_position.exists) {
            continue;
        }
        for (face_index = 0; face_index < mesh->faces.count; face_index++) {
            corners += mesh->faces.data[face_index].num_indices;
        }
        bases.node = (uint32_t)node_index;
        bases.corner_count = corners;
        bases.logical_count = mesh->num_vertices;
        bases.face_count = mesh->faces.count;
        if (!review_extras_capture_mesh(b, node, mesh, &bases)) {
            return 0;
        }
        bases.corner_first += corners;
        bases.logical_first += mesh->num_vertices;
        bases.face_first += mesh->faces.count;
    }
    return 1;
}

/* --------------------------------------------------------------------------
 * Poses, display layers, selection sets
 * -------------------------------------------------------------------------- */

static void review_extras_capture_poses(review_extras_builder *b)
{
    const ufbx_scene *scene = b->scene;
    size_t index;
    for (index = 0; index < scene->poses.count; index++) {
        const ufbx_pose *pose = scene->poses.data[index];
        review_import_pose_extras dst;
        size_t entry_first = b->out->pose_entry_count;
        size_t bone;
        memset(&dst, 0, sizeof(dst));
        if (!pose) {
            review_extras_push_pose(b, &dst, 1);
            continue;
        }
        dst.name = review_extras_str(b, pose->name);
        dst.is_bind_pose = pose->is_bind_pose ? 1u : 0u;
        dst.props = review_extras_props(b, &pose->props);
        for (bone = 0; bone < pose->bone_poses.count; bone++) {
            const ufbx_bone_pose *src = &pose->bone_poses.data[bone];
            review_import_pose_entry entry;
            if (!src->bone_node || src->bone_node->typed_id >= scene->nodes.count) {
                continue;
            }
            entry.node = src->bone_node->typed_id;
            review_extras_matrix(entry.bone_to_world, &src->bone_to_world);
            review_extras_push_pose_entry(b, &entry, 1);
        }
        if (entry_first > UINT32_MAX) {
            b->failed = 1;
            return;
        }
        dst.entry_first = (uint32_t)entry_first;
        dst.entry_count = (uint32_t)(b->out->pose_entry_count - entry_first);
        review_extras_push_pose(b, &dst, 1);
    }
}

static void review_extras_capture_display_layers(review_extras_builder *b)
{
    const ufbx_scene *scene = b->scene;
    size_t index;
    for (index = 0; index < scene->display_layers.count; index++) {
        const ufbx_display_layer *layer = scene->display_layers.data[index];
        review_import_display_layer_extras dst;
        size_t node_first = b->out->layer_node_count;
        size_t node;
        memset(&dst, 0, sizeof(dst));
        if (!layer) {
            review_extras_push_display_layer(b, &dst, 1);
            continue;
        }
        dst.name = review_extras_str(b, layer->name);
        dst.visible = layer->visible ? 1u : 0u;
        dst.frozen = layer->frozen ? 1u : 0u;
        dst.ui_color[0] = layer->ui_color.x;
        dst.ui_color[1] = layer->ui_color.y;
        dst.ui_color[2] = layer->ui_color.z;
        for (node = 0; node < layer->nodes.count; node++) {
            const ufbx_node *member = layer->nodes.data[node];
            uint32_t id;
            if (!member || member->typed_id >= scene->nodes.count) {
                continue;
            }
            id = member->typed_id;
            review_extras_push_layer_node(b, &id, 1);
        }
        if (node_first > UINT32_MAX) {
            b->failed = 1;
            return;
        }
        dst.node_first = (uint32_t)node_first;
        dst.node_count = (uint32_t)(b->out->layer_node_count - node_first);
        dst.props = review_extras_props(b, &layer->props);
        review_extras_push_display_layer(b, &dst, 1);
    }
}

/* The mesh part of `node`, for mapping a selection node's mesh-local indices
   into the global spaces; NULL when the node carries no captured mesh. */
static const review_import_mesh_extras *review_extras_part_of_node(const review_extras_builder *b, uint32_t node)
{
    size_t index;
    for (index = 0; index < b->out->mesh_count; index++) {
        if (b->out->meshes[index].node == node) {
            return &b->out->meshes[index];
        }
    }
    return NULL;
}

static void review_extras_capture_selection_sets(review_extras_builder *b)
{
    const ufbx_scene *scene = b->scene;
    size_t index;
    for (index = 0; index < scene->selection_sets.count; index++) {
        const ufbx_selection_set *set = scene->selection_sets.data[index];
        review_import_selection_set_extras dst;
        size_t node_first = b->out->selection_node_count;
        size_t node_index;
        memset(&dst, 0, sizeof(dst));
        if (!set) {
            review_extras_push_selection_set(b, &dst, 1);
            continue;
        }
        dst.name = review_extras_str(b, set->name);
        dst.props = review_extras_props(b, &set->props);
        for (node_index = 0; node_index < set->nodes.count; node_index++) {
            const ufbx_selection_node *src = set->nodes.data[node_index];
            review_import_selection_node entry;
            const review_import_mesh_extras *part = NULL;
            size_t i;
            if (!src) {
                continue;
            }
            memset(&entry, 0, sizeof(entry));
            entry.node = -1;
            if (src->target_node && src->target_node->typed_id < scene->nodes.count &&
                src->target_node->typed_id <= INT32_MAX) {
                entry.node = (int32_t)src->target_node->typed_id;
                part = review_extras_part_of_node(b, src->target_node->typed_id);
            }
            entry.include_node = src->include_node ? 1u : 0u;
            entry.vertex_first = (uint32_t)b->out->selection_index_count;
            if (part) {
                for (i = 0; i < src->vertices.count; i++) {
                    uint32_t local = src->vertices.data[i];
                    uint32_t global;
                    if (local >= part->logical_count) {
                        continue;
                    }
                    global = part->logical_first + local;
                    review_extras_push_selection_index(b, &global, 1);
                }
            }
            entry.vertex_count = (uint32_t)b->out->selection_index_count - entry.vertex_first;
            entry.edge_first = (uint32_t)b->out->selection_index_count;
            if (part) {
                for (i = 0; i < src->edges.count; i++) {
                    uint32_t local = src->edges.data[i];
                    if (local >= part->edge_count) {
                        continue;
                    }
                    review_extras_push_selection_index(b, &local, 1);
                }
            }
            entry.edge_count = (uint32_t)b->out->selection_index_count - entry.edge_first;
            entry.face_first = (uint32_t)b->out->selection_index_count;
            if (part) {
                for (i = 0; i < src->faces.count; i++) {
                    uint32_t local = src->faces.data[i];
                    uint32_t global;
                    if (local >= part->face_count) {
                        continue;
                    }
                    global = part->face_first + local;
                    review_extras_push_selection_index(b, &global, 1);
                }
            }
            entry.face_count = (uint32_t)b->out->selection_index_count - entry.face_first;
            review_extras_push_selection_node(b, &entry, 1);
        }
        if (node_first > UINT32_MAX || b->out->selection_index_count > UINT32_MAX) {
            b->failed = 1;
            return;
        }
        dst.node_first = (uint32_t)node_first;
        dst.node_count = (uint32_t)(b->out->selection_node_count - node_first);
        review_extras_push_selection_set(b, &dst, 1);
    }
}

/* --------------------------------------------------------------------------
 * Animation: the authored curves
 * -------------------------------------------------------------------------- */

/* The bridge's deduplicated material slot of `material`, or UINT32_MAX. */
static uint32_t review_extras_material_slot(const ufbx_material *const *material_sources,
                                            size_t material_count, const ufbx_material *material)
{
    size_t index;
    if (!material_sources || !material) {
        return UINT32_MAX;
    }
    for (index = 0; index < material_count; index++) {
        if (material_sources[index] == material) {
            return (uint32_t)index;
        }
    }
    return UINT32_MAX;
}

static void review_extras_resolve_target(const review_extras_builder *b, const ufbx_element *element,
                                         const ufbx_material *const *material_sources,
                                         size_t material_count, const uint32_t *channel_of_element,
                                         review_import_anim_prop_extras *dst)
{
    const ufbx_scene *scene = b->scene;
    dst->target_kind = REVIEW_IMPORT_TARGET_UNMAPPED;
    dst->target = 0;
    if (!element) {
        return;
    }
    dst->element_type = (uint32_t)element->type;
    switch (element->type) {
    case UFBX_ELEMENT_NODE:
        if (element->typed_id < scene->nodes.count) {
            dst->target_kind = REVIEW_IMPORT_TARGET_NODE;
            dst->target = element->typed_id;
        }
        break;
    case UFBX_ELEMENT_MATERIAL: {
        uint32_t slot = review_extras_material_slot(material_sources, material_count, (const ufbx_material *)element);
        if (slot != UINT32_MAX) {
            dst->target_kind = REVIEW_IMPORT_TARGET_MATERIAL;
            dst->target = slot;
        }
    } break;
    case UFBX_ELEMENT_TEXTURE:
        if (element->typed_id < scene->textures.count) {
            dst->target_kind = REVIEW_IMPORT_TARGET_TEXTURE;
            dst->target = element->typed_id;
        }
        break;
    case UFBX_ELEMENT_VIDEO:
        if (element->typed_id < scene->videos.count) {
            dst->target_kind = REVIEW_IMPORT_TARGET_VIDEO;
            dst->target = element->typed_id;
        }
        break;
    case UFBX_ELEMENT_BLEND_CHANNEL:
        if (channel_of_element && element->element_id < scene->elements.count &&
            channel_of_element[element->element_id] != UINT32_MAX) {
            dst->target_kind = REVIEW_IMPORT_TARGET_BLEND_CHANNEL;
            dst->target = channel_of_element[element->element_id];
        }
        break;
    case UFBX_ELEMENT_DISPLAY_LAYER:
        if (element->typed_id < scene->display_layers.count) {
            dst->target_kind = REVIEW_IMPORT_TARGET_DISPLAY_LAYER;
            dst->target = element->typed_id;
        }
        break;
    case UFBX_ELEMENT_ANIM_LAYER:
        if (element->typed_id < scene->anim_layers.count) {
            dst->target_kind = REVIEW_IMPORT_TARGET_ANIM_LAYER;
            dst->target = element->typed_id;
        }
        break;
    default:
        if (element->type >= UFBX_ELEMENT_TYPE_FIRST_ATTRIB && element->type <= UFBX_ELEMENT_TYPE_LAST_ATTRIB) {
            /* A node attribute: addressed through the first node that carries it. */
            if (element->instances.count > 0 && element->instances.data[0] &&
                element->instances.data[0]->typed_id < scene->nodes.count) {
                dst->target_kind = REVIEW_IMPORT_TARGET_NODE_ATTRIBUTE;
                dst->target = element->instances.data[0]->typed_id;
            }
        }
        break;
    }
}

static int32_t review_extras_capture_curve(review_extras_builder *b, const ufbx_anim_curve *curve)
{
    review_import_anim_curve_extras dst;
    size_t key_first = b->out->anim_key_count;
    size_t index;
    size_t curve_index;
    if (!curve) {
        return -1;
    }
    for (index = 0; index < curve->keyframes.count; index++) {
        const ufbx_keyframe *src = &curve->keyframes.data[index];
        review_import_anim_key key;
        key.time = src->time;
        key.value = src->value;
        key.interpolation = (uint32_t)src->interpolation;
        key.left_dx = src->left.dx;
        key.left_dy = src->left.dy;
        key.right_dx = src->right.dx;
        key.right_dy = src->right.dy;
        review_extras_push_anim_key(b, &key, 1);
    }
    if (key_first > UINT32_MAX || curve->keyframes.count > UINT32_MAX) {
        b->failed = 1;
        return -1;
    }
    memset(&dst, 0, sizeof(dst));
    dst.key_first = (uint32_t)key_first;
    dst.key_count = (uint32_t)curve->keyframes.count;
    dst.pre_mode = (uint32_t)curve->pre_extrapolation.mode;
    dst.pre_repeat = curve->pre_extrapolation.repeat_count;
    dst.post_mode = (uint32_t)curve->post_extrapolation.mode;
    dst.post_repeat = curve->post_extrapolation.repeat_count;
    curve_index = review_extras_push_anim_curve(b, &dst, 1);
    if (curve_index == SIZE_MAX || curve_index > INT32_MAX) {
        b->failed = 1;
        return -1;
    }
    return (int32_t)curve_index;
}

static void review_extras_capture_animation(review_extras_builder *b,
                                            const ufbx_material *const *material_sources,
                                            size_t material_count,
                                            const uint32_t *channel_of_element,
                                            const int32_t *clip_of_stack)
{
    const ufbx_scene *scene = b->scene;
    size_t index;

    /* Layers first, in typed order, so a stack's layer list and an anim prop's
       layer target are both plain indices into this table. */
    for (index = 0; index < scene->anim_layers.count; index++) {
        const ufbx_anim_layer *layer = scene->anim_layers.data[index];
        review_import_anim_layer_extras dst;
        size_t prop_first = b->out->anim_prop_count;
        size_t prop_index;
        memset(&dst, 0, sizeof(dst));
        if (!layer) {
            review_extras_push_anim_layer(b, &dst, 1);
            continue;
        }
        dst.name = review_extras_str(b, layer->name);
        dst.weight = layer->weight;
        dst.weight_is_animated = layer->weight_is_animated ? 1u : 0u;
        dst.blended = layer->blended ? 1u : 0u;
        dst.additive = layer->additive ? 1u : 0u;
        dst.compose_rotation = layer->compose_rotation ? 1u : 0u;
        dst.compose_scale = layer->compose_scale ? 1u : 0u;
        dst.props = review_extras_props(b, &layer->props);
        for (prop_index = 0; prop_index < layer->anim_props.count; prop_index++) {
            const ufbx_anim_prop *src = &layer->anim_props.data[prop_index];
            review_import_anim_prop_extras entry;
            size_t component;
            if (!src->anim_value) {
                continue;
            }
            memset(&entry, 0, sizeof(entry));
            review_extras_resolve_target(b, src->element, material_sources, material_count,
                                         channel_of_element, &entry);
            entry.element_name = review_extras_str(b, src->element ? src->element->name : ufbx_empty_string);
            entry.prop_name = review_extras_str(b, src->prop_name);
            entry.default_value[0] = src->anim_value->default_value.x;
            entry.default_value[1] = src->anim_value->default_value.y;
            entry.default_value[2] = src->anim_value->default_value.z;
            for (component = 0; component < 3; component++) {
                entry.curves[component] = review_extras_capture_curve(b, src->anim_value->curves[component]);
            }
            review_extras_push_anim_prop(b, &entry, 1);
        }
        if (prop_first > UINT32_MAX) {
            b->failed = 1;
            return;
        }
        dst.anim_prop_first = (uint32_t)prop_first;
        dst.anim_prop_count = (uint32_t)(b->out->anim_prop_count - prop_first);
        review_extras_push_anim_layer(b, &dst, 1);
    }

    for (index = 0; index < scene->anim_stacks.count; index++) {
        const ufbx_anim_stack *stack = scene->anim_stacks.data[index];
        review_import_anim_stack_extras dst;
        size_t layer_first = b->out->stack_layer_count;
        size_t layer_index;
        memset(&dst, 0, sizeof(dst));
        dst.clip = -1;
        if (!stack) {
            review_extras_push_anim_stack(b, &dst, 1);
            continue;
        }
        dst.name = review_extras_str(b, stack->name);
        dst.props = review_extras_props(b, &stack->props);
        dst.clip = clip_of_stack ? clip_of_stack[index] : -1;
        dst.time_begin = stack->time_begin;
        dst.time_end = stack->time_end;
        for (layer_index = 0; layer_index < stack->layers.count; layer_index++) {
            const ufbx_anim_layer *layer = stack->layers.data[layer_index];
            uint32_t id;
            if (!layer || layer->typed_id >= scene->anim_layers.count) {
                continue;
            }
            id = layer->typed_id;
            review_extras_push_stack_layer(b, &id, 1);
        }
        if (layer_first > UINT32_MAX) {
            b->failed = 1;
            return;
        }
        dst.layer_first = (uint32_t)layer_first;
        dst.layer_count = (uint32_t)(b->out->stack_layer_count - layer_first);
        review_extras_push_anim_stack(b, &dst, 1);
    }
}

/* --------------------------------------------------------------------------
 * Entry points
 * -------------------------------------------------------------------------- */

void review_import_free_extras(review_import_extras *extras)
{
    if (!extras) {
        return;
    }
    free(extras->strings);
    free(extras->bytes);
    free(extras->props);
    free(extras->nodes);
    free(extras->lights);
    free(extras->cameras);
    free(extras->lod_groups);
    free(extras->lod_levels);
    free(extras->materials);
    free(extras->material_textures);
    free(extras->textures);
    free(extras->texture_layers);
    free(extras->videos);
    free(extras->meshes);
    free(extras->color_sets);
    free(extras->color_values);
    free(extras->edges);
    free(extras->edge_smoothing);
    free(extras->edge_crease);
    free(extras->edge_visibility);
    free(extras->face_smoothing);
    free(extras->face_hole);
    free(extras->face_group);
    free(extras->vertex_crease);
    free(extras->face_groups);
    free(extras->extra_skins);
    free(extras->extra_clusters);
    free(extras->extra_skin_offsets);
    free(extras->extra_influences);
    free(extras->dq_weights);
    free(extras->poses);
    free(extras->pose_entries);
    free(extras->display_layers);
    free(extras->layer_nodes);
    free(extras->selection_sets);
    free(extras->selection_nodes);
    free(extras->selection_indices);
    free(extras->anim_stacks);
    free(extras->stack_layers);
    free(extras->anim_layers);
    free(extras->anim_props);
    free(extras->anim_curves);
    free(extras->anim_keys);
    memset(extras, 0, sizeof(*extras));
}

/* Capture everything into `out` (which must be zeroed). `material_sources`
   maps each of the bridge's deduplicated material slots back to its ufbx
   material, `channel_of_element` (sized `scene->elements.count`) each blend
   channel element to the morph channel it became, `clip_of_stack` (sized
   `scene->anim_stacks.count`) each stack to its clip. Returns 1 on success, 0
   with `out` freed and `out_error` set. */
int review_import_capture_extras(const ufbx_scene *scene,
                                 const ufbx_material *const *material_sources,
                                 size_t material_count,
                                 const uint32_t *channel_of_element,
                                 const int32_t *clip_of_stack,
                                 review_import_extras *out,
                                 review_import_error *out_error)
{
    review_extras_builder builder;
    memset(&builder, 0, sizeof(builder));
    builder.out = out;
    builder.scene = scene;

    review_extras_capture_scene(&builder);
    review_extras_capture_nodes(&builder);
    review_extras_capture_textures(&builder);
    review_extras_capture_videos(&builder);
    review_extras_capture_materials(&builder, material_sources, material_count);
    if (!builder.failed && !review_extras_capture_meshes(&builder)) {
        if (!builder.failed) {
            review_import_set_error_message(out_error, "malformed FBX: mesh extras drifted from the geometry pass");
            review_import_free_extras(out);
            return 0;
        }
    }
    review_extras_capture_poses(&builder);
    review_extras_capture_display_layers(&builder);
    review_extras_capture_selection_sets(&builder);
    review_extras_capture_animation(&builder, material_sources, material_count, channel_of_element, clip_of_stack);

    if (builder.failed) {
        review_import_set_error_message(out_error, "out of memory while recording source properties");
        review_import_free_extras(out);
        return 0;
    }
    return 1;
}
