/*
 * The ufbx_write side of the export bridge. See export_bridge.h for the shape of
 * the data and why the whole write lives in one C function.
 *
 * Discipline, mirroring `ufbx_bridge.c` (invariant 7): every allocation is freed
 * on every path (through the single `cleanup:` label), every input is
 * bounds-checked before use, and every failure reports a message rather than a
 * bare code.
 */
#include "export_bridge.h"

#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#endif

#include "ufbx_write.h"

/* FBX version to write: 7.7 (FBX 2020), the newest revision, as every tool in
 * the pipeline exports. The vendored writer keys its binary layout on
 * `>= 7500`, so 7.7 shares the modern layout every current importer reads.
 * `src/ufbxw_probe.c` writes the same number and the tests check it. */
#define RVO_FBX_VERSION 7700

/* The source reader's property type codes (`review_model::extras::PropType`). */
enum {
    RVO_PROP_UNKNOWN = 0,
    RVO_PROP_BOOLEAN = 1,
    RVO_PROP_INTEGER = 2,
    RVO_PROP_NUMBER = 3,
    RVO_PROP_VECTOR = 4,
    RVO_PROP_COLOR = 5,
    RVO_PROP_COLOR_WITH_ALPHA = 6,
    RVO_PROP_STRING = 7,
    RVO_PROP_DATE_TIME = 8,
    RVO_PROP_TRANSLATION = 9,
    RVO_PROP_ROTATION = 10,
    RVO_PROP_SCALING = 11,
    RVO_PROP_DISTANCE = 12,
    RVO_PROP_COMPOUND = 13,
    RVO_PROP_BLOB = 14,
    RVO_PROP_REFERENCE = 15
};

/* The reader's flag bits this bridge reads (`review_model::extras::PropFlags`). */
enum {
    RVO_FLAG_ANIMATABLE = 0x1,
    RVO_FLAG_USER_DEFINED = 0x2,
    RVO_FLAG_HIDDEN = 0x4,
    RVO_FLAG_NO_VALUE = 0x10000,
    RVO_FLAG_VALUE_REAL = 0x100000,
    RVO_FLAG_VALUE_VEC2 = 0x200000,
    RVO_FLAG_VALUE_VEC3 = 0x400000,
    RVO_FLAG_VALUE_VEC4 = 0x800000,
    RVO_FLAG_VALUE_INT = 0x1000000,
    RVO_FLAG_VALUE_STR = 0x2000000,
    RVO_FLAG_VALUE_BLOB = 0x4000000
};

/* `a * b` with explicit overflow rejection, mirroring `ufbx_bridge.c`'s helper of
   the same shape: a plain multiply before an allocation defeats the allocator's
   own overflow check. Returns 1 and leaves `*out` untouched on overflow. */
static int rvo_mul_overflows(size_t a, size_t b, size_t *out)
{
    if (a != 0 && b > SIZE_MAX / a) {
        return 1;
    }
    *out = a * b;
    return 0;
}

static void rvo_set_error(char *error, size_t error_length, const char *message)
{
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

/* Copy ufbx_write's own error description out, falling back to `fallback` when
 * it left none. */
static void rvo_set_ufbxw_error(char *error, size_t error_length, const ufbxw_error *source,
                                const char *fallback)
{
    if (source && source->description_length > 0) {
        rvo_set_error(error, error_length, source->description);
    } else {
        rvo_set_error(error, error_length, fallback);
    }
}

static ufbxw_vec3 rvo_vec3(const double *v)
{
    ufbxw_vec3 out;
    out.x = v[0];
    out.y = v[1];
    out.z = v[2];
    return out;
}

static const char *rvo_str_or(const char *text, const char *fallback)
{
    return text ? text : fallback;
}

/* --------------------------------------------------------------------------
 * Authored properties
 * -------------------------------------------------------------------------- */

/* The ufbx_write type an *added* property gets, from what the source declared.
 * Standard names are on the element's template and never reach this: it is
 * for user properties and the exotic extras a DCC writes, whose exact original
 * type string the reader does not keep. */
static ufbxw_prop_type rvo_add_type(const rvo_export_prop *prop)
{
    int user = (prop->flags & RVO_FLAG_USER_DEFINED) != 0;
    switch (prop->type) {
    case RVO_PROP_BOOLEAN:
        return user ? UFBXW_PROP_TYPE_USER_BOOL : UFBXW_PROP_TYPE_BOOL;
    case RVO_PROP_INTEGER:
        return UFBXW_PROP_TYPE_INT;
    case RVO_PROP_NUMBER:
        return UFBXW_PROP_TYPE_DOUBLE;
    case RVO_PROP_VECTOR:
        return user ? UFBXW_PROP_TYPE_USER_VECTOR : UFBXW_PROP_TYPE_VECTOR3D;
    case RVO_PROP_COLOR:
        return UFBXW_PROP_TYPE_COLOR_RGB;
    case RVO_PROP_COLOR_WITH_ALPHA:
        return UFBXW_PROP_TYPE_COLOR_RGBA;
    case RVO_PROP_STRING:
        return user ? UFBXW_PROP_TYPE_USER_STRING : UFBXW_PROP_TYPE_STRING;
    case RVO_PROP_DATE_TIME:
        return UFBXW_PROP_TYPE_DATE_TIME;
    case RVO_PROP_TRANSLATION:
        return UFBXW_PROP_TYPE_LCL_TRANSLATION;
    case RVO_PROP_ROTATION:
        return UFBXW_PROP_TYPE_LCL_ROTATION;
    case RVO_PROP_SCALING:
        return UFBXW_PROP_TYPE_LCL_SCALING;
    case RVO_PROP_DISTANCE:
        return UFBXW_PROP_TYPE_DISTANCE;
    case RVO_PROP_COMPOUND:
        return UFBXW_PROP_TYPE_COMPOUND;
    case RVO_PROP_BLOB:
        return UFBXW_PROP_TYPE_BLOB;
    default:
        break;
    }
    /* Unknown / reference: go by what value the file carried. */
    if (prop->flags & RVO_FLAG_VALUE_VEC4) {
        return UFBXW_PROP_TYPE_COLOR_RGBA;
    }
    if (prop->flags & RVO_FLAG_VALUE_VEC3) {
        return UFBXW_PROP_TYPE_VECTOR3D;
    }
    if (prop->flags & RVO_FLAG_VALUE_VEC2) {
        return UFBXW_PROP_TYPE_VECTOR2D;
    }
    if (prop->flags & RVO_FLAG_VALUE_REAL) {
        return UFBXW_PROP_TYPE_DOUBLE;
    }
    if (prop->flags & RVO_FLAG_VALUE_INT) {
        return UFBXW_PROP_TYPE_INT;
    }
    if (prop->flags & RVO_FLAG_VALUE_BLOB) {
        return UFBXW_PROP_TYPE_BLOB;
    }
    return UFBXW_PROP_TYPE_STRING;
}

/* Apply one authored property to `id`: set it when the element's template
 * declares it (its declared data type decides which value field is read), add
 * it with a derived type otherwise, then carry the authored flags. A value the
 * file left empty is skipped rather than written as zero. */
static void rvo_apply_prop(ufbxw_scene *out, ufbxw_id id, const rvo_export_prop *prop)
{
    const char *name = prop->name;
    ufbxw_prop_data_type existing = ufbxw_get_prop_data_type(out, id, name);
    ufbxw_vec2 v2;
    ufbxw_vec3 v3;
    ufbxw_vec4 v4;
    /* The source wrote this property explicitly, so the export does too — even
     * where the value equals the template default the writer would otherwise
     * elide it for. */
    uint32_t flags = (prop->flags & (RVO_FLAG_ANIMATABLE | RVO_FLAG_USER_DEFINED | RVO_FLAG_HIDDEN)) |
                     UFBXW_PROP_FLAG_EXPLICIT;

    if (prop->flags & RVO_FLAG_NO_VALUE) {
        return;
    }
    v2.x = prop->value_real[0];
    v2.y = prop->value_real[1];
    v3 = rvo_vec3(prop->value_real);
    v4.x = prop->value_real[0];
    v4.y = prop->value_real[1];
    v4.z = prop->value_real[2];
    v4.w = prop->value_real[3];

    if (existing != UFBXW_PROP_DATA_NONE) {
        switch (existing) {
        case UFBXW_PROP_DATA_BOOL:
            ufbxw_set_bool(out, id, name, prop->value_int != 0);
            break;
        case UFBXW_PROP_DATA_INT32:
            ufbxw_set_int(out, id, name, (int32_t)prop->value_int);
            break;
        case UFBXW_PROP_DATA_INT64:
            ufbxw_set_int64(out, id, name, prop->value_int);
            break;
        case UFBXW_PROP_DATA_REAL:
            ufbxw_set_real(out, id, name, prop->value_real[0]);
            break;
        case UFBXW_PROP_DATA_VEC2:
            ufbxw_set_vec2(out, id, name, v2);
            break;
        case UFBXW_PROP_DATA_VEC3:
            ufbxw_set_vec3(out, id, name, v3);
            break;
        case UFBXW_PROP_DATA_VEC4:
            ufbxw_set_vec4(out, id, name, v4);
            break;
        case UFBXW_PROP_DATA_STRING:
            ufbxw_set_string(out, id, name, prop->value_str);
            break;
        default:
            /* Compound headers, ids, and the user-typed structs: the template
             * already holds what the writer needs. */
            return;
        }
    } else {
        ufbxw_prop_type type = rvo_add_type(prop);
        switch (type) {
        case UFBXW_PROP_TYPE_BOOL:
        case UFBXW_PROP_TYPE_USER_BOOL:
            ufbxw_add_bool(out, id, name, type, prop->value_int != 0);
            break;
        case UFBXW_PROP_TYPE_INT:
            ufbxw_add_int(out, id, name, type, (int32_t)prop->value_int);
            break;
        case UFBXW_PROP_TYPE_DOUBLE:
            ufbxw_add_real(out, id, name, type, prop->value_real[0]);
            break;
        case UFBXW_PROP_TYPE_VECTOR2D:
            ufbxw_add_vec2(out, id, name, type, v2);
            break;
        case UFBXW_PROP_TYPE_VECTOR3D:
        case UFBXW_PROP_TYPE_USER_VECTOR:
        case UFBXW_PROP_TYPE_COLOR_RGB:
        case UFBXW_PROP_TYPE_LCL_TRANSLATION:
        case UFBXW_PROP_TYPE_LCL_ROTATION:
        case UFBXW_PROP_TYPE_LCL_SCALING:
            ufbxw_add_vec3(out, id, name, type, v3);
            break;
        case UFBXW_PROP_TYPE_COLOR_RGBA:
            ufbxw_add_vec4(out, id, name, type, v4);
            break;
        case UFBXW_PROP_TYPE_STRING:
        case UFBXW_PROP_TYPE_USER_STRING:
        case UFBXW_PROP_TYPE_DATE_TIME:
            ufbxw_add_string(out, id, name, type, prop->value_str);
            break;
        case UFBXW_PROP_TYPE_DISTANCE:
            ufbxw_add_real_string(out, id, name, type, prop->value_real[0], prop->value_str);
            break;
        case UFBXW_PROP_TYPE_BLOB:
            ufbxw_add_blob(out, id, name, type, prop->blob, prop->blob_length);
            break;
        case UFBXW_PROP_TYPE_COMPOUND:
            /* A compound is only a header for its `Parent|Child` members, which
             * arrive as their own properties. */
            return;
        default:
            return;
        }
    }

    ufbxw_set_prop_flags(out, id, name, flags);
}

static void rvo_apply_props(ufbxw_scene *out, ufbxw_id id, const rvo_export_scene *scene,
                            rvo_export_prop_range range)
{
    for (uint32_t i = 0; i < range.count; i++) {
        rvo_apply_prop(out, id, &scene->props[(size_t)range.first + i]);
    }
}

/* --------------------------------------------------------------------------
 * Validation
 * -------------------------------------------------------------------------- */

static int rvo_validate_range(const rvo_export_scene *scene, rvo_export_prop_range range)
{
    return (size_t)range.first <= scene->prop_count &&
           range.count <= scene->prop_count - (size_t)range.first;
}

/* Validate the whole payload before touching ufbx_write, so a malformed scene
 * fails with a description rather than part-way through a write. */
static int rvo_validate(const rvo_export_scene *scene, char *error, size_t error_length)
{
    if (!scene) {
        rvo_set_error(error, error_length, "no scene supplied");
        return -1;
    }
    if (scene->node_count == 0 || !scene->nodes) {
        rvo_set_error(error, error_length, "scene has no nodes");
        return -1;
    }
    if (scene->mesh_count == 0 || !scene->meshes) {
        rvo_set_error(error, error_length, "scene has no meshes");
        return -1;
    }
    if (scene->material_count > 0 && !scene->materials) {
        rvo_set_error(error, error_length, "material count is non-zero but the array is null");
        return -1;
    }
    if (scene->prop_count > 0 && !scene->props) {
        rvo_set_error(error, error_length, "property count is non-zero but the array is null");
        return -1;
    }
    if (scene->texture_count > 0 && !scene->textures) {
        rvo_set_error(error, error_length, "texture count is non-zero but the array is null");
        return -1;
    }
    if (scene->video_count > 0 && !scene->videos) {
        rvo_set_error(error, error_length, "video count is non-zero but the array is null");
        return -1;
    }
    if (!rvo_validate_range(scene, scene->settings_props) ||
        !rvo_validate_range(scene, scene->scene_info_props)) {
        rvo_set_error(error, error_length, "the scene's property ranges exceed the table");
        return -1;
    }

    for (size_t i = 0; i < scene->prop_count; i++) {
        const rvo_export_prop *prop = &scene->props[i];
        if (!prop->name || !prop->value_str) {
            rvo_set_error(error, error_length, "a property has a null name or value");
            return -1;
        }
        if (prop->blob_length > 0 && !prop->blob) {
            rvo_set_error(error, error_length, "a property declares blob bytes but supplies none");
            return -1;
        }
    }

    for (size_t i = 0; i < scene->node_count; i++) {
        const rvo_export_node *node = &scene->nodes[i];
        if (!node->name) {
            rvo_set_error(error, error_length, "a node has a null name");
            return -1;
        }
        /* A parent must already exist when this node is created, and a node can
         * never be its own ancestor — both are guaranteed by requiring the
         * parent to come earlier in the array. */
        if (node->parent != RVO_NO_PARENT &&
            (node->parent < 0 || (size_t)node->parent >= i)) {
            rvo_set_error(error, error_length, "a node's parent is not an earlier node");
            return -1;
        }
        if (!rvo_validate_range(scene, node->props) ||
            !rvo_validate_range(scene, node->attribute_props)) {
            rvo_set_error(error, error_length, "a node's property range exceeds the table");
            return -1;
        }
        if (node->attribute_kind > RVO_ATTRIB_LOD_GROUP) {
            rvo_set_error(error, error_length, "a node has an unknown attribute kind");
            return -1;
        }
    }

    for (size_t i = 0; i < scene->material_count; i++) {
        const rvo_export_material *material = &scene->materials[i];
        if (!rvo_validate_range(scene, material->props)) {
            rvo_set_error(error, error_length, "a material's property range exceeds the table");
            return -1;
        }
        if (material->shader > RVO_SHADER_CUSTOM) {
            rvo_set_error(error, error_length, "a material has an unknown shader kind");
            return -1;
        }
        if (material->texture_count > 0 && !material->textures) {
            rvo_set_error(error, error_length, "a material declares textures but supplies none");
            return -1;
        }
        for (size_t t = 0; t < material->texture_count; t++) {
            const rvo_export_material_texture *texture = &material->textures[t];
            if (!texture->prop || texture->texture < 0 ||
                (size_t)texture->texture >= scene->texture_count) {
                rvo_set_error(error, error_length, "a material references an unknown texture");
                return -1;
            }
        }
    }

    for (size_t i = 0; i < scene->texture_count; i++) {
        const rvo_export_texture *texture = &scene->textures[i];
        if (!rvo_validate_range(scene, texture->props)) {
            rvo_set_error(error, error_length, "a texture's property range exceeds the table");
            return -1;
        }
        if (texture->content_length > 0 && !texture->content) {
            rvo_set_error(error, error_length, "a texture declares content but supplies none");
            return -1;
        }
        if (texture->video >= 0 && (size_t)texture->video >= scene->video_count) {
            rvo_set_error(error, error_length, "a texture references an unknown video");
            return -1;
        }
        if (texture->layer_count > 0 && !texture->layers) {
            rvo_set_error(error, error_length, "a texture declares layers but supplies none");
            return -1;
        }
        for (size_t l = 0; l < texture->layer_count; l++) {
            int32_t layer = texture->layers[l].texture;
            /* A layer must be an *earlier* texture, so every layered texture's
             * members exist before it is assembled and no texture layers itself. */
            if (layer < 0 || (size_t)layer >= i) {
                rvo_set_error(error, error_length, "a layered texture references a later texture");
                return -1;
            }
        }
    }

    for (size_t i = 0; i < scene->video_count; i++) {
        const rvo_export_video *video = &scene->videos[i];
        if (!rvo_validate_range(scene, video->props)) {
            rvo_set_error(error, error_length, "a video's property range exceeds the table");
            return -1;
        }
        if (video->content_length > 0 && !video->content) {
            rvo_set_error(error, error_length, "a video declares content but supplies none");
            return -1;
        }
    }

    if (scene->pose_count > 0 && !scene->poses) {
        rvo_set_error(error, error_length, "pose count is non-zero but the array is null");
        return -1;
    }
    for (size_t i = 0; i < scene->pose_count; i++) {
        const rvo_export_pose *pose = &scene->poses[i];
        if (pose->node_count > 0 && !pose->nodes) {
            rvo_set_error(error, error_length, "a pose declares nodes but supplies none");
            return -1;
        }
        for (size_t n = 0; n < pose->node_count; n++) {
            if (pose->nodes[n].node < 0 || (size_t)pose->nodes[n].node >= scene->node_count) {
                rvo_set_error(error, error_length, "a pose references a node outside the scene");
                return -1;
            }
        }
    }

    if ((scene->anim_stack_count > 0 && !scene->anim_stacks) ||
        (scene->anim_layer_count > 0 && !scene->anim_layers) ||
        (scene->display_layer_count > 0 && !scene->display_layers) ||
        (scene->selection_set_count > 0 && !scene->selection_sets)) {
        rvo_set_error(error, error_length, "an element count is non-zero but its array is null");
        return -1;
    }
    if (scene->active_stack >= 0 && (size_t)scene->active_stack >= scene->anim_stack_count) {
        rvo_set_error(error, error_length, "the active animation stack does not exist");
        return -1;
    }
    for (size_t i = 0; i < scene->anim_stack_count; i++) {
        if (!rvo_validate_range(scene, scene->anim_stacks[i].props)) {
            rvo_set_error(error, error_length, "an animation stack's property range exceeds the table");
            return -1;
        }
    }
    for (size_t i = 0; i < scene->anim_layer_count; i++) {
        const rvo_export_anim_layer *layer = &scene->anim_layers[i];
        if (layer->stack < 0 || (size_t)layer->stack >= scene->anim_stack_count ||
            !rvo_validate_range(scene, layer->props)) {
            rvo_set_error(error, error_length, "an animation layer names a stack the scene lacks");
            return -1;
        }
        if (layer->anim_prop_count > 0 && !layer->anim_props) {
            rvo_set_error(error, error_length, "an animation layer declares properties but supplies none");
            return -1;
        }
        for (size_t p = 0; p < layer->anim_prop_count; p++) {
            const rvo_export_anim_prop *prop = &layer->anim_props[p];
            size_t limit = 0;
            if (!prop->prop_name) {
                rvo_set_error(error, error_length, "an animated property has no name");
                return -1;
            }
            switch (prop->target_kind) {
            case RVO_TARGET_NODE:
            case RVO_TARGET_NODE_ATTRIBUTE: limit = scene->node_count; break;
            case RVO_TARGET_MATERIAL: limit = scene->material_count; break;
            case RVO_TARGET_TEXTURE: limit = scene->texture_count; break;
            case RVO_TARGET_VIDEO: limit = scene->video_count; break;
            case RVO_TARGET_BLEND_CHANNEL: limit = scene->mesh_count; break;
            case RVO_TARGET_DISPLAY_LAYER: limit = scene->display_layer_count; break;
            case RVO_TARGET_ANIM_LAYER: limit = scene->anim_layer_count; break;
            default:
                rvo_set_error(error, error_length, "an animated property has an unknown target kind");
                return -1;
            }
            if (prop->target < 0 || (size_t)prop->target >= limit) {
                rvo_set_error(error, error_length, "an animated property targets an element the scene lacks");
                return -1;
            }
            if (prop->target_kind == RVO_TARGET_BLEND_CHANNEL &&
                (prop->target2 < 0 || (size_t)prop->target2 >= scene->meshes[prop->target].blend_channel_count)) {
                rvo_set_error(error, error_length, "an animated property targets a blend channel the mesh lacks");
                return -1;
            }
            for (size_t c = 0; c < 3; c++) {
                const rvo_export_curve *curve = prop->curves[c];
                if (curve && curve->key_count > 0 && !curve->keys) {
                    rvo_set_error(error, error_length, "a curve declares keys but supplies none");
                    return -1;
                }
                if (curve && (curve->pre_mode > UFBXW_EXTRAPOLATION_REPEAT_RELATIVE ||
                              curve->post_mode > UFBXW_EXTRAPOLATION_REPEAT_RELATIVE)) {
                    rvo_set_error(error, error_length, "a curve has an unknown extrapolation");
                    return -1;
                }
            }
        }
    }
    for (size_t i = 0; i < scene->display_layer_count; i++) {
        const rvo_export_display_layer *layer = &scene->display_layers[i];
        if (!rvo_validate_range(scene, layer->props) || (layer->node_count > 0 && !layer->nodes)) {
            rvo_set_error(error, error_length, "a display layer is malformed");
            return -1;
        }
        for (size_t n = 0; n < layer->node_count; n++) {
            if (layer->nodes[n] < 0 || (size_t)layer->nodes[n] >= scene->node_count) {
                rvo_set_error(error, error_length, "a display layer references a node outside the scene");
                return -1;
            }
        }
    }
    for (size_t i = 0; i < scene->selection_set_count; i++) {
        const rvo_export_selection_set *set = &scene->selection_sets[i];
        if (!rvo_validate_range(scene, set->props) || (set->node_count > 0 && !set->nodes)) {
            rvo_set_error(error, error_length, "a selection set is malformed");
            return -1;
        }
        for (size_t n = 0; n < set->node_count; n++) {
            const rvo_export_selection_node *entry = &set->nodes[n];
            if (entry->node < 0 || (size_t)entry->node >= scene->node_count) {
                rvo_set_error(error, error_length, "a selection set references a node outside the scene");
                return -1;
            }
            if ((entry->vertex_count > 0 && !entry->vertices) || (entry->edge_count > 0 && !entry->edges) ||
                (entry->face_count > 0 && !entry->faces)) {
                rvo_set_error(error, error_length, "a selection node declares members but supplies none");
                return -1;
            }
        }
    }

    for (size_t i = 0; i < scene->mesh_count; i++) {
        const rvo_export_mesh *mesh = &scene->meshes[i];
        if (!mesh->name || !mesh->positions || !mesh->indices) {
            rvo_set_error(error, error_length, "a mesh is missing its name, positions or indices");
            return -1;
        }
        if (mesh->node < 0 || (size_t)mesh->node >= scene->node_count) {
            rvo_set_error(error, error_length, "a mesh references a node outside the scene");
            return -1;
        }
        if (mesh->vertex_count == 0 || mesh->face_count == 0 || mesh->index_count == 0 ||
            !mesh->face_offsets) {
            rvo_set_error(error, error_length, "a mesh has no geometry");
            return -1;
        }
        /* FBX indices are 32-bit signed, so a stream that overflows them cannot
         * be written at all — reject it here rather than truncate the offsets
         * into a scrambled mesh. */
        if (mesh->index_count > (size_t)INT32_MAX || mesh->face_count > (size_t)INT32_MAX) {
            rvo_set_error(error, error_length, "a mesh has more corners than FBX can index");
            return -1;
        }
        if (mesh->face_offsets[0] != 0 || (size_t)mesh->face_offsets[mesh->face_count] != mesh->index_count) {
            rvo_set_error(error, error_length, "a mesh's face offsets do not span its corners");
            return -1;
        }
        for (size_t f = 0; f < mesh->face_count; f++) {
            int32_t first = mesh->face_offsets[f];
            int32_t end = mesh->face_offsets[f + 1];
            if (first < 0 || end < first + 3) {
                rvo_set_error(error, error_length, "a mesh has a face with fewer than three corners");
                return -1;
            }
        }
        for (size_t e = 0; e < mesh->edge_count; e++) {
            if (!mesh->edges || mesh->edges[e] < 0 || (size_t)mesh->edges[e] >= mesh->index_count) {
                rvo_set_error(error, error_length, "a mesh edge addresses no corner");
                return -1;
            }
        }
        if (mesh->color_set_count > 0 && !mesh->color_sets) {
            rvo_set_error(error, error_length, "a mesh declares color sets but supplies none");
            return -1;
        }
        for (size_t c = 0; c < mesh->color_set_count; c++) {
            if (!mesh->color_sets[c].values) {
                rvo_set_error(error, error_length, "a mesh has a color set without values");
                return -1;
            }
        }
        if (!rvo_validate_range(scene, mesh->props)) {
            rvo_set_error(error, error_length, "a mesh's property range exceeds the table");
            return -1;
        }
        if (mesh->skin_count > 0 && !mesh->skins) {
            rvo_set_error(error, error_length, "a mesh declares skins but supplies none");
            return -1;
        }
        for (size_t k = 0; k < mesh->skin_count; k++) {
            const rvo_export_skin *skin = &mesh->skins[k];
            if (skin->skinning_type > 3) {
                rvo_set_error(error, error_length, "a skin has an unknown skinning type");
                return -1;
            }
            if (skin->cluster_count > 0 && !skin->clusters) {
                rvo_set_error(error, error_length, "a skin declares clusters but supplies none");
                return -1;
            }
            if (skin->bind_pose >= 0 && (size_t)skin->bind_pose >= scene->pose_count) {
                rvo_set_error(error, error_length, "a skin references a pose the scene lacks");
                return -1;
            }
            for (size_t c = 0; c < skin->cluster_count; c++) {
                const rvo_export_cluster *cluster = &skin->clusters[c];
                if (cluster->bone < 0 || (size_t)cluster->bone >= scene->node_count) {
                    rvo_set_error(error, error_length, "a skin cluster references a node outside the scene");
                    return -1;
                }
                if (cluster->weight_count > 0 && (!cluster->vertices || !cluster->weights)) {
                    rvo_set_error(error, error_length, "a skin cluster declares weights but supplies none");
                    return -1;
                }
                for (size_t w = 0; w < cluster->weight_count; w++) {
                    if (cluster->vertices[w] < 0 || (size_t)cluster->vertices[w] >= mesh->vertex_count) {
                        rvo_set_error(error, error_length, "a skin weight addresses no vertex");
                        return -1;
                    }
                }
            }
            if (skin->dq_count > 0 && (!skin->dq_vertices || !skin->dq_weights)) {
                rvo_set_error(error, error_length, "a skin declares blend weights but supplies none");
                return -1;
            }
            for (size_t w = 0; w < skin->dq_count; w++) {
                if (skin->dq_vertices[w] < 0 || (size_t)skin->dq_vertices[w] >= mesh->vertex_count) {
                    rvo_set_error(error, error_length, "a blend weight addresses no vertex");
                    return -1;
                }
            }
        }
        if (mesh->blend_channel_count > 0 && !mesh->blend_channels) {
            rvo_set_error(error, error_length, "a mesh declares blend channels but supplies none");
            return -1;
        }
        for (size_t b = 0; b < mesh->blend_channel_count; b++) {
            const rvo_export_blend_channel *channel = &mesh->blend_channels[b];
            if (channel->shape_count > 0 && !channel->shapes) {
                rvo_set_error(error, error_length, "a blend channel declares shapes but supplies none");
                return -1;
            }
            for (size_t k = 0; k < channel->shape_count; k++) {
                const rvo_export_blend_shape *shape = &channel->shapes[k];
                if (shape->offset_count > 0 && (!shape->vertices || !shape->offsets)) {
                    rvo_set_error(error, error_length, "a blend shape declares offsets but supplies none");
                    return -1;
                }
                for (size_t o = 0; o < shape->offset_count; o++) {
                    if (shape->vertices[o] < 0 || (size_t)shape->vertices[o] >= mesh->vertex_count) {
                        rvo_set_error(error, error_length, "a blend-shape offset addresses no vertex");
                        return -1;
                    }
                }
            }
        }
        /* Attribute streams are read past the count handed over: `3 *
         * vertex_count` doubles for positions and normals, `4 *` for colors and
         * tangents, `2 *` for each UV set. The widest of those is what has to fit. */
        if (mesh->vertex_count > SIZE_MAX / 4) {
            rvo_set_error(error, error_length, "a mesh has an implausible vertex count");
            return -1;
        }
        for (size_t c = 0; c < mesh->index_count; c++) {
            int32_t index = mesh->indices[c];
            if (index < 0 || (size_t)index >= mesh->vertex_count) {
                rvo_set_error(error, error_length, "a mesh index addresses no vertex");
                return -1;
            }
        }
        if (mesh->uv_set_count > 0 && (!mesh->uv_sets || !mesh->uv_set_names)) {
            rvo_set_error(error, error_length, "a mesh declares UV sets but supplies none");
            return -1;
        }
        for (size_t s = 0; s < mesh->uv_set_count; s++) {
            if (!mesh->uv_sets[s] || !mesh->uv_set_names[s]) {
                rvo_set_error(error, error_length, "a mesh has a null UV set");
                return -1;
            }
        }
        if (mesh->material_slot_count > 0) {
            if (!mesh->material_slots) {
                rvo_set_error(error, error_length, "a mesh declares materials but supplies none");
                return -1;
            }
            for (size_t s = 0; s < mesh->material_slot_count; s++) {
                int32_t slot = mesh->material_slots[s];
                if (slot < 0 || (size_t)slot >= scene->material_count) {
                    rvo_set_error(error, error_length, "a mesh references an unknown material");
                    return -1;
                }
            }
            if (mesh->material_slot_count > 1) {
                if (!mesh->face_materials) {
                    rvo_set_error(error, error_length,
                                  "a multi-material mesh has no per-face assignment");
                    return -1;
                }
                for (size_t f = 0; f < mesh->face_count; f++) {
                    int32_t slot = mesh->face_materials[f];
                    if (slot < 0 || (size_t)slot >= mesh->material_slot_count) {
                        rvo_set_error(error, error_length,
                                      "a face references a material slot the mesh does not list");
                        return -1;
                    }
                }
            }
        }
    }

    return 0;
}

int review_export_available(void)
{
    return 1;
}

/* --------------------------------------------------------------------------
 * Element creation
 * -------------------------------------------------------------------------- */

static void rvo_write_scene_settings(ufbxw_scene *out, const rvo_export_scene *scene)
{
    ufbxw_id settings = ufbxw_get_global_settings_id(out);

    /* Declare the file's unit before anything else, so every coordinate written
     * below is read back at the size it was authored. */
    if (scene->unit_scale_cm > 0.0) {
        ufbxw_scene_set_unit_scale_factor(out, scene->unit_scale_cm);
    }
    if (scene->axis_right >= 0 && scene->axis_right <= UFBXW_COORDINATE_AXIS_NEGATIVE_Z &&
        scene->axis_up >= 0 && scene->axis_up <= UFBXW_COORDINATE_AXIS_NEGATIVE_Z &&
        scene->axis_front >= 0 && scene->axis_front <= UFBXW_COORDINATE_AXIS_NEGATIVE_Z) {
        ufbxw_coordinate_axes axes;
        axes.right = (ufbxw_coordinate_axis)scene->axis_right;
        axes.up = (ufbxw_coordinate_axis)scene->axis_up;
        axes.front = (ufbxw_coordinate_axis)scene->axis_front;
        ufbxw_scene_set_coordinate_axes(out, axes);
    }
    if (scene->time_mode >= 0 && scene->time_mode <= UFBXW_TIME_MODE_59_94_FPS) {
        ufbxw_scene_set_time_mode(out, (ufbxw_time_mode)scene->time_mode);
        if (scene->time_mode == UFBXW_TIME_MODE_CUSTOM && scene->frame_rate > 0.0) {
            ufbxw_scene_set_custom_frame_rate(out, scene->frame_rate);
        }
    } else if (scene->frame_rate > 0.0) {
        ufbxw_scene_set_time_mode(out, UFBXW_TIME_MODE_CUSTOM);
        ufbxw_scene_set_custom_frame_rate(out, scene->frame_rate);
    }

    /* The rest of the authored settings by name — with the unit kept as
     * declared above: the source figure reaches us through a float and the
     * snapped one is what the coordinates were scaled by. */
    for (uint32_t i = 0; i < scene->settings_props.count; i++) {
        const rvo_export_prop *prop = &scene->props[(size_t)scene->settings_props.first + i];
        if (strcmp(prop->name, "UnitScaleFactor") == 0) {
            continue;
        }
        rvo_apply_prop(out, settings, prop);
    }

    {
        ufbxw_save_info info;
        memset(&info, 0, sizeof(info));
        info.application_vendor = ufbxw_str("3D Review");
        info.application_name = ufbxw_str(rvo_str_or(scene->application_name, "3D Review"));
        info.application_version = ufbxw_str(rvo_str_or(scene->application_version, ""));
        info.original_application_vendor = ufbxw_str(rvo_str_or(scene->original_application_vendor, ""));
        info.original_application_name = ufbxw_str(rvo_str_or(scene->original_application_name, ""));
        info.original_application_version = ufbxw_str(rvo_str_or(scene->original_application_version, ""));
        info.original_filename = ufbxw_str(rvo_str_or(scene->original_filename, ""));
        ufbxw_set_save_info(out, &info);
    }
    {
        ufbxw_id scene_info = ufbxw_get_scene_info_id(out);
        for (uint32_t i = 0; i < scene->scene_info_props.count; i++) {
            const rvo_export_prop *prop = &scene->props[(size_t)scene->scene_info_props.first + i];
            /* The save-info block above owns these. */
            if (strncmp(prop->name, "Original", 8) == 0 || strncmp(prop->name, "LastSaved", 9) == 0 ||
                strcmp(prop->name, "DocumentUrl") == 0 || strcmp(prop->name, "SrcDocumentUrl") == 0) {
                continue;
            }
            rvo_apply_prop(out, scene_info, prop);
        }
    }
}

static ufbxw_node rvo_write_node(ufbxw_scene *out, const rvo_export_scene *scene,
                                 const rvo_export_node *source, const ufbxw_node *nodes,
                                 ufbxw_id *out_attribute)
{
    ufbxw_node node = ufbxw_create_node(out);
    ufbxw_set_name(out, node.id, source->name);
    if (source->parent != RVO_NO_PARENT) {
        ufbxw_node_set_parent(out, node, nodes[source->parent]);
    }

    if (source->authored_transform) {
        /* The authored properties define the transform: `RotationOrder` first
         * so the Euler triplet written after it is interpreted as the file did. */
        for (uint32_t i = 0; i < source->props.count; i++) {
            const rvo_export_prop *prop = &scene->props[(size_t)source->props.first + i];
            if (strcmp(prop->name, "RotationOrder") == 0) {
                rvo_apply_prop(out, node.id, prop);
            }
        }
        for (uint32_t i = 0; i < source->props.count; i++) {
            const rvo_export_prop *prop = &scene->props[(size_t)source->props.first + i];
            if (strcmp(prop->name, "RotationOrder") != 0) {
                rvo_apply_prop(out, node.id, prop);
            }
        }
    } else {
        ufbxw_quat rotation;
        ufbxw_node_set_translation(out, node, rvo_vec3(source->translation));
        rotation.x = source->rotation[0];
        rotation.y = source->rotation[1];
        rotation.z = source->rotation[2];
        rotation.w = source->rotation[3];
        ufbxw_node_set_rotation_quat(out, node, rotation, UFBXW_ROTATION_ORDER_XYZ);
        ufbxw_node_set_scaling(out, node, rvo_vec3(source->scaling));
        rvo_apply_props(out, node.id, scene, source->props);
    }

    {
        ufbxw_id attribute = 0;
        switch (source->attribute_kind) {
        case RVO_ATTRIB_BONE:
            attribute = ufbxw_create_bone(out, UFBXW_BONE_LIMB_NODE, node).id;
            break;
        case RVO_ATTRIB_LIGHT:
            attribute = ufbxw_create_light(out, node).id;
            break;
        case RVO_ATTRIB_CAMERA:
            attribute = ufbxw_create_camera(out, node).id;
            break;
        case RVO_ATTRIB_NULL:
            attribute = ufbxw_create_null(out, node).id;
            break;
        case RVO_ATTRIB_LOD_GROUP:
            attribute = ufbxw_create_lod_group(out, node).id;
            break;
        default:
            break;
        }
        if (attribute != 0) {
            ufbxw_set_name(out, attribute, rvo_str_or(source->attribute_name, ""));
            rvo_apply_props(out, attribute, scene, source->attribute_props);
        }
        *out_attribute = attribute;
    }
    return node;
}

static ufbxw_material rvo_write_material(ufbxw_scene *out, const rvo_export_scene *scene,
                                         const rvo_export_material *source)
{
    ufbxw_material_type type = UFBXW_MATERIAL_FBX_PHONG;
    ufbxw_material material;
    switch (source->shader) {
    case RVO_SHADER_LAMBERT:
        type = UFBXW_MATERIAL_FBX_LAMBERT;
        break;
    case RVO_SHADER_CUSTOM:
        type = UFBXW_MATERIAL_CUSTOM;
        break;
    default:
        break;
    }
    material = ufbxw_create_material(out, type);
    ufbxw_set_name(out, material.id, rvo_str_or(source->name, ""));
    if (type == UFBXW_MATERIAL_CUSTOM && source->shading_model && source->shading_model[0]) {
        ufbxw_set_string(out, material.id, "ShadingModel", source->shading_model);
    }
    if (source->props.count > 0) {
        rvo_apply_props(out, material.id, scene, source->props);
    } else {
        /* No authored properties: the viewer's PBR figures onto the classic
         * slots a Phong has for them. */
        ufbxw_set_vec3(out, material.id, "DiffuseColor", rvo_vec3(source->base_color));
        ufbxw_set_vec3(out, material.id, "EmissiveColor", rvo_vec3(source->emissive));
        if (type == UFBXW_MATERIAL_FBX_PHONG) {
            ufbxw_set_real(out, material.id, "ShininessExponent", source->shininess_exponent);
            ufbxw_set_real(out, material.id, "ReflectionFactor", source->reflection_factor);
        }
    }
    return material;
}

static ufbxw_video rvo_write_video(ufbxw_scene *out, const rvo_export_scene *scene,
                                   const rvo_export_video *source)
{
    ufbxw_video video = ufbxw_create_video(out);
    ufbxw_set_name(out, video.id, rvo_str_or(source->name, ""));
    ufbxw_video_set_filename(out, video, rvo_str_or(source->filename, ""));
    ufbxw_video_set_relative_filename(out, video, rvo_str_or(source->relative_filename, ""));
    if (source->content_length > 0) {
        ufbxw_video_set_content(out, video, ufbxw_copy_byte_array(out, source->content, source->content_length));
    }
    rvo_apply_props(out, video.id, scene, source->props);
    return video;
}

static ufbxw_texture rvo_write_texture(ufbxw_scene *out, const rvo_export_scene *scene,
                                       const rvo_export_texture *source, const ufbxw_texture *textures,
                                       const ufbxw_video *videos)
{
    ufbxw_texture texture = ufbxw_create_texture(out, source->layered ? UFBXW_TEXTURE_LAYERED : UFBXW_TEXTURE_FILE);
    ufbxw_set_name(out, texture.id, rvo_str_or(source->name, ""));
    if (!source->layered) {
        ufbxw_texture_set_filename(out, texture, rvo_str_or(source->filename, ""));
        ufbxw_texture_set_relative_filename(out, texture, rvo_str_or(source->relative_filename, ""));
        if (source->video >= 0) {
            ufbxw_texture_set_video(out, texture, videos[source->video]);
        } else if (source->content_length > 0) {
            ufbxw_texture_set_content(out, texture,
                                      ufbxw_copy_byte_array(out, source->content, source->content_length));
        }
    }
    rvo_apply_props(out, texture.id, scene, source->props);
    for (size_t l = 0; l < source->layer_count; l++) {
        const rvo_export_texture_layer *layer = &source->layers[l];
        ufbxw_texture_add_layer(out, texture, textures[layer->texture], layer->blend_mode, layer->alpha);
    }
    return texture;
}

static ufbxw_matrix rvo_matrix(const double *m)
{
    ufbxw_matrix out;
    memcpy(out.m, m, sizeof(out.m));
    return out;
}

/* Skin deformers and blend shapes of one mesh. `poses` are the bind poses
 * already written, indexed as the payload's. */
static void rvo_write_deform(ufbxw_scene *out, const rvo_export_mesh *source, ufbxw_mesh mesh,
                             const ufbxw_node *nodes, const ufbxw_bind_pose *poses,
                             ufbxw_blend_channel *out_channels)
{
    for (size_t k = 0; k < source->skin_count; k++) {
        const rvo_export_skin *skin = &source->skins[k];
        ufbxw_skin_deformer deformer = ufbxw_create_skin_deformer(out, mesh);
        ufbxw_skin_deformer_set_skinning_type(out, deformer, (ufbxw_skinning_type)skin->skinning_type);
        for (size_t c = 0; c < skin->cluster_count; c++) {
            const rvo_export_cluster *source_cluster = &skin->clusters[c];
            ufbxw_skin_cluster cluster = ufbxw_create_skin_cluster(out, deformer, nodes[source_cluster->bone]);
            ufbxw_set_name(out, cluster.id, rvo_str_or(source_cluster->name, ""));
            if (source_cluster->weight_count > 0) {
                ufbxw_skin_cluster_set_weights(
                    out, cluster, ufbxw_copy_int_array(out, source_cluster->vertices, source_cluster->weight_count),
                    ufbxw_copy_real_array(out, source_cluster->weights, source_cluster->weight_count));
            }
            ufbxw_skin_cluster_set_transform(out, cluster, rvo_matrix(source_cluster->transform));
            ufbxw_skin_cluster_set_link_transform(out, cluster, rvo_matrix(source_cluster->transform_link));
        }
        if (skin->dq_count > 0) {
            ufbxw_skin_deformer_set_dual_quaternion_weights(
                out, deformer, ufbxw_copy_int_array(out, skin->dq_vertices, skin->dq_count),
                ufbxw_copy_real_array(out, skin->dq_weights, skin->dq_count));
        }
        if (skin->bind_pose >= 0 && poses) {
            ufbxw_skin_deformer_set_bind_pose(out, deformer, poses[skin->bind_pose]);
        }
    }

    if (source->blend_channel_count > 0) {
        ufbxw_blend_deformer deformer = ufbxw_create_blend_deformer(out, mesh);
        for (size_t b = 0; b < source->blend_channel_count; b++) {
            const rvo_export_blend_channel *source_channel = &source->blend_channels[b];
            ufbxw_blend_channel channel = ufbxw_create_blend_channel(out, deformer);
            out_channels[b] = channel;
            ufbxw_set_name(out, channel.id, rvo_str_or(source_channel->name, ""));
            ufbxw_blend_channel_set_weight(out, channel, source_channel->weight);
            for (size_t k = 0; k < source_channel->shape_count; k++) {
                const rvo_export_blend_shape *source_shape = &source_channel->shapes[k];
                ufbxw_blend_shape shape = ufbxw_create_blend_shape(out);
                ufbxw_set_name(out, shape.id, rvo_str_or(source_shape->name, ""));
                ufbxw_blend_shape_set_offsets(
                    out, shape, ufbxw_copy_int_array(out, source_shape->vertices, source_shape->offset_count),
                    ufbxw_copy_vec3_array(out, (const ufbxw_vec3 *)source_shape->offsets, source_shape->offset_count));
                if (source_shape->normals) {
                    ufbxw_blend_shape_set_normals(
                        out, shape,
                        ufbxw_copy_vec3_array(out, (const ufbxw_vec3 *)source_shape->normals, source_shape->offset_count));
                }
                ufbxw_blend_channel_add_shape(out, channel, shape, source_shape->target_weight);
            }
        }
    }
}

/* The element an animated property targets, resolved against what was written. */
static ufbxw_id rvo_anim_target(const rvo_export_scene *scene, const rvo_export_anim_prop *prop,
                                const ufbxw_node *nodes, const ufbxw_id *attributes,
                                const ufbxw_material *materials, const ufbxw_texture *textures,
                                const ufbxw_video *videos, ufbxw_blend_channel *const *mesh_channels,
                                const ufbxw_display_layer *display_layers, const ufbxw_anim_layer *layers)
{
    (void)scene;
    switch (prop->target_kind) {
    case RVO_TARGET_NODE: return nodes[prop->target].id;
    case RVO_TARGET_NODE_ATTRIBUTE: return attributes[prop->target];
    case RVO_TARGET_MATERIAL: return materials ? materials[prop->target].id : 0;
    case RVO_TARGET_TEXTURE: return textures ? textures[prop->target].id : 0;
    case RVO_TARGET_VIDEO: return videos ? videos[prop->target].id : 0;
    case RVO_TARGET_BLEND_CHANNEL:
        return mesh_channels && mesh_channels[prop->target] ? mesh_channels[prop->target][prop->target2].id : 0;
    case RVO_TARGET_DISPLAY_LAYER: return display_layers ? display_layers[prop->target].id : 0;
    case RVO_TARGET_ANIM_LAYER: return layers ? layers[prop->target].id : 0;
    default: return 0;
    }
}

/* Write one authored curve onto a component curve of an animated property. */
static void rvo_write_curve(ufbxw_scene *out, ufbxw_anim_curve curve, const rvo_export_curve *source)
{
    int64_t previous = INT64_MIN;
    for (size_t k = 0; k < source->key_count; k++) {
        const rvo_export_key *key = &source->keys[k];
        ufbxw_keyframe_real out_key;
        /* Two keys on one ktime cannot be told apart by a reader; the first wins. */
        if (key->time == previous) {
            continue;
        }
        previous = key->time;
        memset(&out_key, 0, sizeof(out_key));
        out_key.time = key->time;
        out_key.value = key->value;
        out_key.flags = key->flags;
        out_key.weight_left = key->weight_left;
        out_key.weight_right = key->weight_right;
        out_key.slope_left = key->slope_left;
        out_key.slope_right = key->slope_right;
        ufbxw_anim_curve_add_keyframe_key(out, curve, out_key);
    }
    ufbxw_anim_curve_set_pre_extrapolation(out, curve, (ufbxw_extrapolation_type)source->pre_mode);
    ufbxw_anim_curve_set_pre_extrapolation_repeat_count(out, curve, source->pre_repeat);
    ufbxw_anim_curve_set_post_extrapolation(out, curve, (ufbxw_extrapolation_type)source->post_mode);
    ufbxw_anim_curve_set_post_extrapolation_repeat_count(out, curve, source->post_repeat);
}

/* A 0/1 int buffer from a byte flag array — the form the boolean layers are
 * written in. Copied into the scene, so the scratch can go. */
/* The flags widened into a buffer ufbx_write owns, filled in place. There is no
   scratch allocation of our own to fail: if ufbx_write cannot allocate it
   records the failure on the scene, and the `ufbxw_get_error` check before the
   save turns that into the export's error rather than an empty layer. */
static ufbxw_int_buffer rvo_bool_ints(ufbxw_scene *out, const uint8_t *flags, size_t count)
{
    ufbxw_int_buffer buffer = ufbxw_create_int_buffer(out, count);
    ufbxw_int_list ints = ufbxw_edit_int_buffer(out, buffer);
    if (!ints.data || ints.count != count) {
        return buffer;
    }
    for (size_t i = 0; i < count; i++) {
        ints.data[i] = flags[i] ? 1 : 0;
    }
    return buffer;
}

/* --------------------------------------------------------------------------
 * Output file
 *
 * ufbx_write's own `ufbxw_save_file` opens with the narrow `fopen`, which on
 * Windows reads the path in the ANSI code page - so a UTF-8 path with any
 * non-ASCII character (a user folder named `Zoe` with a diaeresis) fails to open
 * or lands on a mangled name - caps the path at 1023 bytes, and seeks with an
 * `int` offset, which corrupts any file past 2 GB. The bridge therefore opens
 * the file itself and hands ufbx_write a stream: UTF-8 widened to UTF-16 for
 * `_wfopen` on Windows, 64-bit seeks, and a close whose failure (the final flush
 * hitting a full disk) is reported rather than dropped.
 * -------------------------------------------------------------------------- */

typedef struct rvo_file_stream {
    FILE *file;
    int failed;
} rvo_file_stream;

static FILE *rvo_open_for_write(const char *path)
{
#if defined(_WIN32)
    int wide_length = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, path, -1, NULL, 0);
    wchar_t *wide = NULL;
    FILE *file = NULL;
    if (wide_length <= 0) {
        return NULL;
    }
    wide = (wchar_t *)malloc((size_t)wide_length * sizeof(wchar_t));
    if (!wide) {
        return NULL;
    }
    if (MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, path, -1, wide, wide_length) == wide_length) {
        file = _wfopen(wide, L"wb");
    }
    free(wide);
    return file;
#else
    return fopen(path, "wb");
#endif
}

static bool rvo_file_write(void *user, uint64_t offset, const void *data, size_t size)
{
    rvo_file_stream *stream = (rvo_file_stream *)user;
#if defined(_WIN32)
    int seek = offset > (uint64_t)INT64_MAX ? -1 : _fseeki64(stream->file, (__int64)offset, SEEK_SET);
#else
    int seek = offset > (uint64_t)INT64_MAX ? -1 : fseeko(stream->file, (off_t)offset, SEEK_SET);
#endif
    if (seek != 0 || fwrite(data, 1, size, stream->file) != size) {
        stream->failed = 1;
        return false;
    }
    return true;
}

/* Save `out` to `path`, returning 1 on success. On failure `error` holds why. */
static int rvo_save_to_path(ufbxw_scene *out, const char *path, const ufbxw_save_opts *opts,
                            char *error, size_t error_length)
{
    rvo_file_stream file_stream;
    ufbxw_write_stream stream;
    ufbxw_error save_error;
    int saved;

    file_stream.file = rvo_open_for_write(path);
    file_stream.failed = 0;
    if (!file_stream.file) {
        rvo_set_error(error, error_length, "failed to open the output file for writing");
        return 0;
    }

    memset(&stream, 0, sizeof(stream));
    stream.write_fn = rvo_file_write;
    /* No close_fn: the file is closed below, where a failed close can be seen. */
    stream.user = &file_stream;

    memset(&save_error, 0, sizeof(save_error));
    saved = ufbxw_save_stream(out, &stream, opts, &save_error) ? 1 : 0;
    if (fclose(file_stream.file) != 0) {
        file_stream.failed = 1;
    }
    if (!saved) {
        rvo_set_ufbxw_error(error, error_length, &save_error, "failed to write the FBX file");
        return 0;
    }
    if (file_stream.failed) {
        rvo_set_error(error, error_length, "failed to write the FBX file (the disk may be full)");
        return 0;
    }
    return 1;
}

/* --------------------------------------------------------------------------
 * Entry point
 * -------------------------------------------------------------------------- */

int review_export_fbx(const rvo_export_scene *scene, const char *path, int ascii, char *error,
                      size_t error_length)
{
    int status = -1;
    ufbxw_scene *out = NULL;
    ufbxw_node *nodes = NULL;
    ufbxw_material *materials = NULL;
    ufbxw_texture *textures = NULL;
    ufbxw_video *videos = NULL;
    ufbxw_bind_pose *poses = NULL;
    ufbxw_id *attributes = NULL;
    ufbxw_blend_channel **mesh_channels = NULL;
    ufbxw_anim_stack *stacks = NULL;
    ufbxw_anim_layer *layers = NULL;
    ufbxw_display_layer *display_layers = NULL;
    /* Tangent xyz / w split for the writer's separate `values_w` stream. */
    double *tangent_xyz = NULL;
    double *tangent_w = NULL;
    double *binormals = NULL;
    size_t tangent_capacity = 0;

    if (!path) {
        rvo_set_error(error, error_length, "no output path supplied");
        return -1;
    }
    if (rvo_validate(scene, error, error_length) != 0) {
        return -1;
    }

    ufbxw_scene_opts scene_opts;
    memset(&scene_opts, 0, sizeof(scene_opts));
    /* ufbx_write otherwise seeds every scene with a "Take 001" stack and its
     * BaseLayer, so a static mesh comes back out of the tool carrying an empty
     * animation — which the viewer duly lists in the Outliner's Animations tab
     * and every DCC shows as a take. Animation is written explicitly, from the
     * source's own stacks, so there is nothing for a default one to hold. */
    scene_opts.no_default_anim_stack = true;
    scene_opts.no_default_anim_layer = true;
    out = ufbxw_create_scene(&scene_opts);
    if (!out) {
        rvo_set_error(error, error_length, "could not create the FBX scene");
        return -1;
    }

    rvo_write_scene_settings(out, scene);

    nodes = (ufbxw_node *)calloc(scene->node_count, sizeof(ufbxw_node));
    attributes = (ufbxw_id *)calloc(scene->node_count, sizeof(ufbxw_id));
    mesh_channels = (ufbxw_blend_channel **)calloc(scene->mesh_count, sizeof(ufbxw_blend_channel *));
    if (!nodes || !attributes || !mesh_channels) {
        rvo_set_error(error, error_length, "out of memory allocating nodes");
        goto cleanup;
    }
    if (scene->material_count > 0) {
        materials = (ufbxw_material *)calloc(scene->material_count, sizeof(ufbxw_material));
        if (!materials) {
            rvo_set_error(error, error_length, "out of memory allocating materials");
            goto cleanup;
        }
    }
    if (scene->texture_count > 0) {
        textures = (ufbxw_texture *)calloc(scene->texture_count, sizeof(ufbxw_texture));
        if (!textures) {
            rvo_set_error(error, error_length, "out of memory allocating textures");
            goto cleanup;
        }
    }
    if (scene->video_count > 0) {
        videos = (ufbxw_video *)calloc(scene->video_count, sizeof(ufbxw_video));
        if (!videos) {
            rvo_set_error(error, error_length, "out of memory allocating videos");
            goto cleanup;
        }
    }

    /* Nodes. Parents are guaranteed to come first (checked above), so one
     * forward pass can wire the hierarchy as it goes. */
    for (size_t i = 0; i < scene->node_count; i++) {
        nodes[i] = rvo_write_node(out, scene, &scene->nodes[i], nodes, &attributes[i]);
    }

    /* Authored bind poses, before the skins that bind against them. */
    if (scene->pose_count > 0) {
        poses = (ufbxw_bind_pose *)calloc(scene->pose_count, sizeof(ufbxw_bind_pose));
        if (!poses) {
            rvo_set_error(error, error_length, "out of memory allocating poses");
            goto cleanup;
        }
        for (size_t i = 0; i < scene->pose_count; i++) {
            const rvo_export_pose *source = &scene->poses[i];
            poses[i] = ufbxw_create_bind_pose(out);
            ufbxw_set_name(out, poses[i].id, rvo_str_or(source->name, ""));
            for (size_t n = 0; n < source->node_count; n++) {
                ufbxw_bind_pose_add_node(out, poses[i], nodes[source->nodes[n].node],
                                         rvo_matrix(source->nodes[n].matrix));
            }
        }
    }

    /* Videos before the textures that reference them, textures before the
     * layered textures and materials that reference them (validation ordered
     * the layers). */
    for (size_t i = 0; i < scene->video_count; i++) {
        videos[i] = rvo_write_video(out, scene, &scene->videos[i]);
    }
    for (size_t i = 0; i < scene->texture_count; i++) {
        textures[i] = rvo_write_texture(out, scene, &scene->textures[i], textures, videos);
    }
    for (size_t i = 0; i < scene->material_count; i++) {
        const rvo_export_material *source = &scene->materials[i];
        materials[i] = rvo_write_material(out, scene, source);
        for (size_t t = 0; t < source->texture_count; t++) {
            ufbxw_material_set_texture(out, materials[i], source->textures[t].prop,
                                       textures[source->textures[t].texture]);
        }
    }

    /* Meshes. */
    for (size_t i = 0; i < scene->mesh_count; i++) {
        const rvo_export_mesh *source = &scene->meshes[i];
        size_t index_count = source->index_count;

        ufbxw_mesh mesh = ufbxw_create_mesh(out);
        ufbxw_set_name(out, mesh.id, source->name);
        ufbxw_mesh_add_instance(out, mesh, nodes[source->node]);
        rvo_apply_props(out, mesh.id, scene, source->props);

        /* Every buffer is handed over with `ufbxw_copy_*` rather than
         * `ufbxw_view_*`. The view variants keep a *borrowed* pointer that must
         * stay valid until the save completes, which makes the bridge's
         * correctness depend on the lifetime of memory owned two languages away —
         * and made the scratch `face_offsets` below dangle the moment it was
         * reused for a second mesh. Copying costs one transient duplicate of the
         * mesh during a user-initiated export and buys a bridge whose contract is
         * simply "nothing is borrowed past this call". */
        ufbxw_mesh_set_vertices(
            out, mesh, ufbxw_copy_vec3_array(out, (const ufbxw_vec3 *)source->positions,
                                             source->vertex_count));

        /* The polygon-vertex stream and its face offsets, as the caller cut
         * them (validated above). */
        ufbxw_mesh_set_polygons(out, mesh, ufbxw_copy_int_array(out, source->indices, index_count),
                                ufbxw_copy_int_array(out, source->face_offsets, source->face_count + 1));
        if (source->edge_count > 0) {
            ufbxw_mesh_set_fbx_edges(out, mesh, ufbxw_copy_int_array(out, source->edges, source->edge_count));
        }

        /* Attribute values are one per vertex: the optimizer already splits a
         * vertex wherever any attribute differs.
         *
         * Normals are written vertex-mapped and Direct, which is what ufbx_write
         * does for them (the normal layer forbids an index array).
         *
         * UV and color layers are handed over POLYGON_VERTEX-mapped and indexed
         * by the mesh's own index buffer instead — the same values, reached
         * through the PolygonVertexIndex stream. Their non-indexed setters ask
         * ufbx_write to `generate_indices`, which dedups the values and emits an
         * index array *as long as the value array*; paired with VERTEX mapping
         * that produces `ByVertice` + `IndexToDirect` with a per-control-point
         * index array. FBX allows that shape, but readers that assume an indexed
         * UV/color layer is per polygon vertex — Unity among them, which rejects
         * the mesh with "has invalid UV coordinates" / "invalid vertex Colors"
         * and blames the exporting tool — cannot read it. `ByPolygonVertex` +
         * `IndexToDirect` is what every DCC writes, and its index array length
         * matches PolygonVertexIndex, so there is nothing left to guess. */
        if (source->normals) {
            ufbxw_mesh_set_normals(out, mesh,
                                   ufbxw_copy_vec3_array(out, (const ufbxw_vec3 *)source->normals,
                                                         source->vertex_count),
                                   UFBXW_ATTRIBUTE_MAPPING_VERTEX);
        }
        if (source->tangents && source->normals) {
            /* Tangents and binormals are vertex-mapped like normals. The
             * binormal is derived from the authored handedness: `w · (n × t)`. */
            size_t needed = source->vertex_count;
            if (needed > tangent_capacity) {
                size_t bytes;
                double *grown;
                if (rvo_mul_overflows(needed, 3 * sizeof(double), &bytes)) {
                    rvo_set_error(error, error_length, "a mesh has an implausible vertex count");
                    goto cleanup;
                }
                grown = (double *)realloc(tangent_xyz, bytes);
                if (!grown) {
                    rvo_set_error(error, error_length, "out of memory allocating tangents");
                    goto cleanup;
                }
                tangent_xyz = grown;
                grown = (double *)realloc(binormals, bytes);
                if (!grown) {
                    rvo_set_error(error, error_length, "out of memory allocating tangents");
                    goto cleanup;
                }
                binormals = grown;
                grown = (double *)realloc(tangent_w, needed * sizeof(double));
                if (!grown) {
                    rvo_set_error(error, error_length, "out of memory allocating tangents");
                    goto cleanup;
                }
                tangent_w = grown;
                tangent_capacity = needed;
            }
            for (size_t v = 0; v < needed; v++) {
                const double *t = &source->tangents[v * 4];
                const double *n = &source->normals[v * 3];
                double w = t[3] < 0.0 ? -1.0 : 1.0;
                tangent_xyz[v * 3 + 0] = t[0];
                tangent_xyz[v * 3 + 1] = t[1];
                tangent_xyz[v * 3 + 2] = t[2];
                tangent_w[v] = w;
                binormals[v * 3 + 0] = w * (n[1] * t[2] - n[2] * t[1]);
                binormals[v * 3 + 1] = w * (n[2] * t[0] - n[0] * t[2]);
                binormals[v * 3 + 2] = w * (n[0] * t[1] - n[1] * t[0]);
            }
            {
                ufbxw_mesh_attribute_desc desc;
                memset(&desc, 0, sizeof(desc));
                desc.mapping = UFBXW_ATTRIBUTE_MAPPING_VERTEX;
                desc.values = ufbxw_copy_vec3_array(out, (const ufbxw_vec3 *)tangent_xyz, needed).id;
                desc.values_w = ufbxw_copy_real_array(out, tangent_w, needed).id;
                ufbxw_mesh_set_attribute(out, mesh, UFBXW_MESH_ATTRIBUTE_TANGENT, 0, &desc);
                memset(&desc, 0, sizeof(desc));
                desc.mapping = UFBXW_ATTRIBUTE_MAPPING_VERTEX;
                desc.values = ufbxw_copy_vec3_array(out, (const ufbxw_vec3 *)binormals, needed).id;
                ufbxw_mesh_set_attribute(out, mesh, UFBXW_MESH_ATTRIBUTE_BINORMAL, 0, &desc);
            }
        }
        for (size_t s = 0; s < source->uv_set_count; s++) {
            ufbxw_mesh_set_uvs_indexed(
                out, mesh, (int32_t)s,
                ufbxw_copy_vec2_array(out, (const ufbxw_vec2 *)source->uv_sets[s],
                                      source->vertex_count),
                ufbxw_copy_int_array(out, source->indices, index_count),
                UFBXW_ATTRIBUTE_MAPPING_POLYGON_VERTEX);
            ufbxw_mesh_set_attribute_name(out, mesh, UFBXW_MESH_ATTRIBUTE_UV, (int32_t)s,
                                          source->uv_set_names[s]);
        }
        if (source->colors) {
            ufbxw_mesh_set_colors_indexed(
                out, mesh, 0,
                ufbxw_copy_vec4_array(out, (const ufbxw_vec4 *)source->colors,
                                      source->vertex_count),
                ufbxw_copy_int_array(out, source->indices, index_count),
                UFBXW_ATTRIBUTE_MAPPING_POLYGON_VERTEX);
            if (source->color_set_name && source->color_set_name[0]) {
                ufbxw_mesh_set_attribute_name(out, mesh, UFBXW_MESH_ATTRIBUTE_COLOR, 0,
                                              source->color_set_name);
            }
        }
        for (size_t c = 0; c < source->color_set_count; c++) {
            const rvo_export_color_set *set = &source->color_sets[c];
            int32_t index = (int32_t)c + 1;
            ufbxw_mesh_set_colors_indexed(
                out, mesh, index,
                ufbxw_copy_vec4_array(out, (const ufbxw_vec4 *)set->values, source->vertex_count),
                ufbxw_copy_int_array(out, source->indices, index_count),
                UFBXW_ATTRIBUTE_MAPPING_POLYGON_VERTEX);
            if (set->name && set->name[0]) {
                ufbxw_mesh_set_attribute_name(out, mesh, UFBXW_MESH_ATTRIBUTE_COLOR, index, set->name);
            }
        }

        /* The topology layers the source authored (review patch P3 for the
         * writer's side of them). Smoothing prefers the per-edge form when the
         * file had it, as ufbx reads either. Boolean layers go out as 0/1 ints,
         * which is what every reader converts from. */
        {
            ufbxw_mesh_attribute_desc desc;
            if (source->edge_smoothing && source->edge_count > 0) {
                memset(&desc, 0, sizeof(desc));
                desc.mapping = UFBXW_ATTRIBUTE_MAPPING_EDGE;
                desc.values = rvo_bool_ints(out, source->edge_smoothing, source->edge_count).id;
                ufbxw_mesh_set_attribute(out, mesh, UFBXW_MESH_ATTRIBUTE_SMOOTHING, 0, &desc);
            } else if (source->face_smoothing) {
                memset(&desc, 0, sizeof(desc));
                desc.mapping = UFBXW_ATTRIBUTE_MAPPING_POLYGON;
                desc.values = rvo_bool_ints(out, source->face_smoothing, source->face_count).id;
                ufbxw_mesh_set_attribute(out, mesh, UFBXW_MESH_ATTRIBUTE_SMOOTHING, 0, &desc);
            }
            if (source->edge_crease && source->edge_count > 0) {
                memset(&desc, 0, sizeof(desc));
                desc.mapping = UFBXW_ATTRIBUTE_MAPPING_EDGE;
                desc.values = ufbxw_copy_real_array(out, source->edge_crease, source->edge_count).id;
                ufbxw_mesh_set_attribute(out, mesh, UFBXW_MESH_ATTRIBUTE_EDGE_CREASE, 0, &desc);
            }
            if (source->edge_visibility && source->edge_count > 0) {
                memset(&desc, 0, sizeof(desc));
                desc.mapping = UFBXW_ATTRIBUTE_MAPPING_EDGE;
                desc.values = rvo_bool_ints(out, source->edge_visibility, source->edge_count).id;
                ufbxw_mesh_set_attribute(out, mesh, UFBXW_MESH_ATTRIBUTE_VISIBILITY, 0, &desc);
            }
            if (source->face_hole) {
                memset(&desc, 0, sizeof(desc));
                desc.mapping = UFBXW_ATTRIBUTE_MAPPING_POLYGON;
                desc.values = rvo_bool_ints(out, source->face_hole, source->face_count).id;
                ufbxw_mesh_set_attribute(out, mesh, UFBXW_MESH_ATTRIBUTE_HOLE, 0, &desc);
            }
            if (source->face_group) {
                memset(&desc, 0, sizeof(desc));
                desc.mapping = UFBXW_ATTRIBUTE_MAPPING_POLYGON;
                desc.values = ufbxw_copy_int_array(out, source->face_group, source->face_count).id;
                ufbxw_mesh_set_attribute(out, mesh, UFBXW_MESH_ATTRIBUTE_POLYGON_GROUP, 0, &desc);
            }
            if (source->vertex_crease) {
                memset(&desc, 0, sizeof(desc));
                desc.mapping = UFBXW_ATTRIBUTE_MAPPING_VERTEX;
                desc.values = ufbxw_copy_real_array(out, source->vertex_crease, source->vertex_count).id;
                ufbxw_mesh_set_attribute(out, mesh, UFBXW_MESH_ATTRIBUTE_VERTEX_CREASE, 0, &desc);
            }
        }

        /* Materials connect to the *node*, and per-face assignment indexes that
         * node's connection order — which is why the caller passes local slot
         * indices rather than global ones. */
        for (size_t s = 0; s < source->material_slot_count; s++) {
            ufbxw_connect(out, UFBXW_CONNECTION_MATERIAL, materials[source->material_slots[s]].id,
                          nodes[source->node].id);
        }
        if (source->material_slot_count == 1) {
            ufbxw_mesh_set_single_material(out, mesh, 0);
        } else if (source->material_slot_count > 1) {
            ufbxw_mesh_set_face_material(
                out, mesh, ufbxw_copy_int_array(out, source->face_materials, source->face_count));
        }

        if (source->blend_channel_count > 0) {
            mesh_channels[i] = (ufbxw_blend_channel *)calloc(source->blend_channel_count, sizeof(ufbxw_blend_channel));
            if (!mesh_channels[i]) {
                rvo_set_error(error, error_length, "out of memory allocating blend channels");
                goto cleanup;
            }
        }
        rvo_write_deform(out, source, mesh, nodes, poses, mesh_channels[i]);
    }

    /* Display layers and selection sets, over the nodes and meshes above. */
    if (scene->display_layer_count > 0) {
        display_layers = (ufbxw_display_layer *)calloc(scene->display_layer_count, sizeof(ufbxw_display_layer));
        if (!display_layers) {
            rvo_set_error(error, error_length, "out of memory allocating display layers");
            goto cleanup;
        }
        for (size_t i = 0; i < scene->display_layer_count; i++) {
            const rvo_export_display_layer *source = &scene->display_layers[i];
            display_layers[i] = ufbxw_create_display_layer(out);
            ufbxw_set_name(out, display_layers[i].id, rvo_str_or(source->name, ""));
            rvo_apply_props(out, display_layers[i].id, scene, source->props);
            for (size_t n = 0; n < source->node_count; n++) {
                ufbxw_display_layer_add_node(out, display_layers[i], nodes[source->nodes[n]]);
            }
        }
    }
    for (size_t i = 0; i < scene->selection_set_count; i++) {
        const rvo_export_selection_set *source = &scene->selection_sets[i];
        ufbxw_selection_set set = ufbxw_create_selection_set(out);
        ufbxw_set_name(out, set.id, rvo_str_or(source->name, ""));
        rvo_apply_props(out, set.id, scene, source->props);
        for (size_t n = 0; n < source->node_count; n++) {
            const rvo_export_selection_node *entry = &source->nodes[n];
            ufbxw_selection_node selection = ufbxw_create_selection_node(out, set);
            ufbxw_selection_node_set_node(out, selection, nodes[entry->node]);
            ufbxw_selection_node_set_include_node(out, selection, entry->include_node != 0);
            if (entry->vertex_count > 0) {
                ufbxw_selection_node_set_vertices(out, selection, ufbxw_copy_int_array(out, entry->vertices, entry->vertex_count));
            }
            if (entry->edge_count > 0) {
                ufbxw_selection_node_set_edges(out, selection, ufbxw_copy_int_array(out, entry->edges, entry->edge_count));
            }
            if (entry->face_count > 0) {
                ufbxw_selection_node_set_polygons(out, selection, ufbxw_copy_int_array(out, entry->faces, entry->face_count));
            }
        }
    }

    /* Animation last: every property a curve targets exists by now. */
    if (scene->anim_stack_count > 0) {
        stacks = (ufbxw_anim_stack *)calloc(scene->anim_stack_count, sizeof(ufbxw_anim_stack));
        layers = (ufbxw_anim_layer *)calloc(scene->anim_layer_count ? scene->anim_layer_count : 1, sizeof(ufbxw_anim_layer));
        if (!stacks || !layers) {
            rvo_set_error(error, error_length, "out of memory allocating animation");
            goto cleanup;
        }
        for (size_t i = 0; i < scene->anim_stack_count; i++) {
            const rvo_export_anim_stack *source = &scene->anim_stacks[i];
            stacks[i] = ufbxw_create_anim_stack(out);
            ufbxw_set_name(out, stacks[i].id, rvo_str_or(source->name, ""));
            ufbxw_anim_stack_set_time_range(out, stacks[i], source->time_begin, source->time_end);
            rvo_apply_props(out, stacks[i].id, scene, source->props);
        }
        for (size_t i = 0; i < scene->anim_layer_count; i++) {
            const rvo_export_anim_layer *source = &scene->anim_layers[i];
            layers[i] = ufbxw_create_anim_layer(out, stacks[source->stack]);
            ufbxw_set_name(out, layers[i].id, rvo_str_or(source->name, ""));
            ufbxw_anim_layer_set_weight(out, layers[i], source->weight);
            rvo_apply_props(out, layers[i].id, scene, source->props);
        }
        for (size_t i = 0; i < scene->anim_layer_count; i++) {
            const rvo_export_anim_layer *source = &scene->anim_layers[i];
            for (size_t p = 0; p < source->anim_prop_count; p++) {
                const rvo_export_anim_prop *prop = &source->anim_props[p];
                ufbxw_id target = rvo_anim_target(scene, prop, nodes, attributes, materials, textures, videos,
                                                  mesh_channels, display_layers, layers);
                ufbxw_anim_prop anim;
                if (target == 0) {
                    continue;
                }
                /* Only the components the source keyed get a curve (patch
                 * P5): a curve node carrying just its defaults must come
                 * back that way, since a reader treats any curve as a
                 * non-constant value (ufbx then adds scale helpers). */
                uint32_t curve_mask = 0;
                size_t curve_slot = 0;
                for (size_t c = 0; c < 3; c++) {
                    if (prop->curves[c]) {
                        curve_mask |= 1u << c;
                    }
                }
                anim = ufbxw_animate_prop_masked(out, target, prop->prop_name, layers[i], curve_mask);
                if (anim.id == 0) {
                    /* The property does not exist on the element (the source
                     * animated something it never declared); nothing to bind. */
                    continue;
                }
                for (size_t c = 0; c < 3; c++) {
                    ufbxw_anim_curve curve;
                    ufbxw_anim_set_default_value(out, anim, c, prop->default_value[c]);
                    if (!prop->curves[c]) {
                        continue;
                    }
                    curve = ufbxw_anim_get_curve(out, anim, curve_slot++);
                    if (curve.id == 0) {
                        continue;
                    }
                    rvo_write_curve(out, curve, prop->curves[c]);
                }
            }
        }
        if (scene->active_stack >= 0) {
            ufbxw_set_active_anim_stack(out, stacks[scene->active_stack]);
        }
    }

    ufbxw_error build_error;
    memset(&build_error, 0, sizeof(build_error));
    if (ufbxw_get_error(out, &build_error)) {
        rvo_set_ufbxw_error(error, error_length, &build_error, "failed to build the FBX scene");
        goto cleanup;
    }

    ufbxw_prepare_scene(out, &ufbxw_default_prepare_opts);

    ufbxw_save_opts save_opts;
    memset(&save_opts, 0, sizeof(save_opts));
    save_opts.format = ascii ? UFBXW_SAVE_FORMAT_ASCII : UFBXW_SAVE_FORMAT_BINARY;
    save_opts.version = RVO_FBX_VERSION;

    if (!rvo_save_to_path(out, path, &save_opts, error, error_length)) {
        goto cleanup;
    }

    status = 0;

cleanup:
    free(tangent_xyz);
    free(tangent_w);
    free(binormals);
    if (mesh_channels) {
        for (size_t i = 0; i < scene->mesh_count; i++) {
            free(mesh_channels[i]);
        }
    }
    free(mesh_channels);
    free(attributes);
    free(display_layers);
    free(layers);
    free(stacks);
    free(poses);
    free(videos);
    free(textures);
    free(materials);
    free(nodes);
    ufbxw_free_scene(out);
    return status;
}
