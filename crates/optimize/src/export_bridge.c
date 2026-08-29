/*
 * The ufbx_write side of the export bridge. See export_bridge.h for the shape of
 * the data and why the whole write lives in one C function.
 *
 * Discipline, mirroring `ufbx_bridge.c` (invariant 7): every allocation is freed
 * on every path (there is exactly one — the ufbxw scene — released through the
 * single `cleanup:` label), every input is bounds-checked before use, and every
 * failure reports a message rather than a bare code.
 */
#include "export_bridge.h"

#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "ufbx_write.h"

/* FBX version to write. 7500 is the modern binary revision every current
 * importer reads, and the version ufbx_write's own example uses. */
#define RVO_FBX_VERSION 7500

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
        if (mesh->vertex_count == 0 || mesh->triangle_count == 0) {
            rvo_set_error(error, error_length, "a mesh has no geometry");
            return -1;
        }
        /* The index count is `3 * triangle_count`; guard the multiply so a
         * hostile count can't wrap into a small allocation. */
        if (mesh->triangle_count > SIZE_MAX / 3) {
            rvo_set_error(error, error_length, "a mesh has an implausible triangle count");
            return -1;
        }
        for (size_t c = 0; c < mesh->triangle_count * 3; c++) {
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
                for (size_t f = 0; f < mesh->triangle_count; f++) {
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

int review_export_fbx(const rvo_export_scene *scene, const char *path, int ascii, char *error,
                      size_t error_length)
{
    int status = -1;
    ufbxw_scene *out = NULL;
    ufbxw_node *nodes = NULL;
    ufbxw_material *materials = NULL;
    /* Per-face offsets for `set_polygons`; rebuilt per mesh, freed at cleanup. */
    int32_t *face_offsets = NULL;
    size_t face_offsets_capacity = 0;

    if (!path) {
        rvo_set_error(error, error_length, "no output path supplied");
        return -1;
    }
    if (rvo_validate(scene, error, error_length) != 0) {
        return -1;
    }

    ufbxw_scene_opts scene_opts;
    memset(&scene_opts, 0, sizeof(scene_opts));
    out = ufbxw_create_scene(&scene_opts);
    if (!out) {
        rvo_set_error(error, error_length, "could not create the FBX scene");
        return -1;
    }

    /* Declare the file's unit before anything else, so every coordinate written
     * below is read back at the size it was authored. */
    if (scene->unit_scale_cm > 0.0) {
        ufbxw_scene_set_unit_scale_factor(out, scene->unit_scale_cm);
    }

    nodes = (ufbxw_node *)calloc(scene->node_count, sizeof(ufbxw_node));
    if (!nodes) {
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

    /* Nodes. Parents are guaranteed to come first (checked above), so one
     * forward pass can wire the hierarchy as it goes. */
    for (size_t i = 0; i < scene->node_count; i++) {
        const rvo_export_node *source = &scene->nodes[i];
        ufbxw_node node = ufbxw_create_node(out);
        ufbxw_set_name(out, node.id, source->name);
        ufbxw_node_set_translation(out, node, rvo_vec3(source->translation));
        ufbxw_quat rotation;
        rotation.x = source->rotation[0];
        rotation.y = source->rotation[1];
        rotation.z = source->rotation[2];
        rotation.w = source->rotation[3];
        ufbxw_node_set_rotation_quat(out, node, rotation, UFBXW_ROTATION_ORDER_XYZ);
        ufbxw_node_set_scaling(out, node, rvo_vec3(source->scaling));
        if (source->parent != RVO_NO_PARENT) {
            ufbxw_node_set_parent(out, node, nodes[source->parent]);
        }
        nodes[i] = node;
    }

    /* Materials. FBX Lambert is the lowest common denominator every importer
     * reads; the source's base and emissive colors map onto its own slots. */
    for (size_t i = 0; i < scene->material_count; i++) {
        const rvo_export_material *source = &scene->materials[i];
        ufbxw_material material = ufbxw_create_material(out, UFBXW_MATERIAL_FBX_LAMBERT);
        ufbxw_set_name(out, material.id, source->name ? source->name : "");
        ufbxw_set_vec3(out, material.id, "DiffuseColor", rvo_vec3(source->base_color));
        ufbxw_set_vec3(out, material.id, "EmissiveColor", rvo_vec3(source->emissive));
        materials[i] = material;
    }

    /* Meshes. */
    for (size_t i = 0; i < scene->mesh_count; i++) {
        const rvo_export_mesh *source = &scene->meshes[i];
        size_t index_count = source->triangle_count * 3;

        ufbxw_mesh mesh = ufbxw_create_mesh(out);
        ufbxw_set_name(out, mesh.id, source->name);
        ufbxw_mesh_add_instance(out, mesh, nodes[source->node]);

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

        /* Face offsets: one per face plus a terminator. Grown as needed and
         * reused across meshes, which is only safe because it is copied in. */
        size_t offsets_needed = source->triangle_count + 1;
        if (offsets_needed > face_offsets_capacity) {
            int32_t *grown = (int32_t *)realloc(face_offsets, offsets_needed * sizeof(int32_t));
            if (!grown) {
                rvo_set_error(error, error_length, "out of memory allocating face offsets");
                goto cleanup;
            }
            face_offsets = grown;
            face_offsets_capacity = offsets_needed;
        }
        for (size_t f = 0; f < offsets_needed; f++) {
            face_offsets[f] = (int32_t)(f * 3);
        }
        ufbxw_mesh_set_polygons(out, mesh, ufbxw_copy_int_array(out, source->indices, index_count),
                                ufbxw_copy_int_array(out, face_offsets, offsets_needed));

        /* Attributes are vertex-mapped: the optimizer already splits a vertex
         * wherever any attribute differs. */
        if (source->normals) {
            ufbxw_mesh_set_normals(out, mesh,
                                   ufbxw_copy_vec3_array(out, (const ufbxw_vec3 *)source->normals,
                                                         source->vertex_count),
                                   UFBXW_ATTRIBUTE_MAPPING_VERTEX);
        }
        for (size_t s = 0; s < source->uv_set_count; s++) {
            ufbxw_mesh_set_uvs(out, mesh, (int32_t)s,
                               ufbxw_copy_vec2_array(out, (const ufbxw_vec2 *)source->uv_sets[s],
                                                     source->vertex_count),
                               UFBXW_ATTRIBUTE_MAPPING_VERTEX);
            ufbxw_mesh_set_attribute_name(out, mesh, UFBXW_MESH_ATTRIBUTE_UV, (int32_t)s,
                                          source->uv_set_names[s]);
        }
        if (source->colors) {
            ufbxw_mesh_set_colors(out, mesh, 0,
                                  ufbxw_copy_vec4_array(out, (const ufbxw_vec4 *)source->colors,
                                                        source->vertex_count),
                                  UFBXW_ATTRIBUTE_MAPPING_VERTEX);
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
                out, mesh, ufbxw_copy_int_array(out, source->face_materials, source->triangle_count));
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

    ufbxw_error save_error;
    memset(&save_error, 0, sizeof(save_error));
    if (!ufbxw_save_file(out, path, &save_opts, &save_error)) {
        rvo_set_ufbxw_error(error, error_length, &save_error, "failed to write the FBX file");
        goto cleanup;
    }

    status = 0;

cleanup:
    free(face_offsets);
    free(materials);
    free(nodes);
    ufbxw_free_scene(out);
    return status;
}
