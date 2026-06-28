#include "ufbx_bridge.h"

#include "ufbx.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>

static void review_import_set_error(review_import_error *out_error, const char *message)
{
    if (!out_error) {
        return;
    }

    out_error->message[0] = '\0';
    if (!message) {
        return;
    }

    strncpy(out_error->message, message, sizeof(out_error->message) - 1);
    out_error->message[sizeof(out_error->message) - 1] = '\0';
}

static char *review_import_dup_string_len(const char *data, size_t length)
{
    char *result = (char*)malloc(length + 1);
    if (!result) {
        return NULL;
    }

    if (length > 0) {
        memcpy(result, data, length);
    }
    result[length] = '\0';
    return result;
}

static char *review_import_dup_ufbx_string(ufbx_string str)
{
    if (!str.data || str.length == 0) {
        return review_import_dup_string_len("", 0);
    }
    return review_import_dup_string_len(str.data, str.length);
}

static void review_import_free_materials(review_import_material *materials, size_t material_count)
{
    size_t index;
    for (index = 0; index < material_count; index++) {
        free(materials[index].name);
    }
    free(materials);
}

static void review_import_free_warnings(review_import_warning *warnings, size_t warning_count)
{
    size_t index;
    for (index = 0; index < warning_count; index++) {
        free(warnings[index].message);
    }
    free(warnings);
}

static void review_import_free_uv_set_names(char **names, size_t name_count)
{
    size_t index;
    if (!names) {
        return;
    }
    for (index = 0; index < name_count; index++) {
        free(names[index]);
    }
    free(names);
}

/* Replace the scene's UV-set name table with the names of `mesh`'s UV sets, in
   source order. Called for the mesh that defines a new `uv_set_count` so the
   stored names always match the channel count. Returns 1 on success, 0 on OOM
   (leaving any previous table freed). */
static int review_import_capture_uv_set_names(review_import_scene *scene, const ufbx_mesh *mesh)
{
    size_t count = mesh->uv_sets.count;
    char **names;
    size_t index;

    review_import_free_uv_set_names(scene->uv_set_names, scene->uv_set_name_count);
    scene->uv_set_names = NULL;
    scene->uv_set_name_count = 0;

    if (count == 0) {
        return 1;
    }

    names = (char**)calloc(count, sizeof(char*));
    if (!names) {
        return 0;
    }

    for (index = 0; index < count; index++) {
        names[index] = review_import_dup_ufbx_string(mesh->uv_sets.data[index].name);
        if (!names[index]) {
            review_import_free_uv_set_names(names, index);
            return 0;
        }
    }

    scene->uv_set_names = names;
    scene->uv_set_name_count = count;
    return 1;
}

static void review_import_free_nodes(review_import_node *nodes, size_t node_count)
{
    size_t index;
    if (!nodes) {
        return;
    }
    for (index = 0; index < node_count; index++) {
        free(nodes[index].name);
    }
    free(nodes);
}

void review_import_free_scene(review_import_scene *scene)
{
    if (!scene) {
        return;
    }

    free(scene->name);
    free(scene->vertices);
    free(scene->indices);
    free(scene->faces);
    free(scene->tri_to_face);
    free(scene->uvs);
    review_import_free_materials(scene->materials, scene->material_count);
    review_import_free_warnings(scene->warnings, scene->warning_count);
    review_import_free_uv_set_names(scene->uv_set_names, scene->uv_set_name_count);
    review_import_free_nodes(scene->nodes, scene->node_count);
    free(scene->tri_material);
    free(scene->tri_node);
    memset(scene, 0, sizeof(*scene));
}

static float review_import_length3(const float value[3])
{
    return (float)sqrt((double)value[0] * value[0] + (double)value[1] * value[1] + (double)value[2] * value[2]);
}

static void review_import_normalize3(float value[3], const float fallback[3])
{
    float length = review_import_length3(value);
    if (length <= 1e-8f) {
        value[0] = fallback[0];
        value[1] = fallback[1];
        value[2] = fallback[2];
        return;
    }

    value[0] /= length;
    value[1] /= length;
    value[2] /= length;
}

/* Game-asset DCC color comes from the material's base/diffuse color, not a
   vertex-color layer. Prefer the PBR base color, fall back to the legacy FBX
   diffuse color, then to white. RGB only — the viewer renders meshes opaque.
   Returns the resolved color in *linear* space (the material table seeds a PBR
   uniform from it directly). */
static void review_import_material_base_color_linear(const ufbx_material *material, float out_color[3])
{
    out_color[0] = 1.0f;
    out_color[1] = 1.0f;
    out_color[2] = 1.0f;

    if (!material) {
        return;
    }

    if (material->pbr.base_color.has_value) {
        out_color[0] = (float)material->pbr.base_color.value_vec4.x;
        out_color[1] = (float)material->pbr.base_color.value_vec4.y;
        out_color[2] = (float)material->pbr.base_color.value_vec4.z;
    } else if (material->fbx.diffuse_color.has_value) {
        out_color[0] = (float)material->fbx.diffuse_color.value_vec4.x;
        out_color[1] = (float)material->fbx.diffuse_color.value_vec4.y;
        out_color[2] = (float)material->fbx.diffuse_color.value_vec4.z;
    }
}

/* Resolve the material's metalness in [0,1], defaulting to 0 (dielectric) when
   the material declares none. */
static float review_import_material_metallic(const ufbx_material *material)
{
    float metallic = 0.0f;

    if (material && material->pbr.metalness.has_value) {
        metallic = (float)material->pbr.metalness.value_real;
    }

    if (metallic < 0.0f) {
        metallic = 0.0f;
    } else if (metallic > 1.0f) {
        metallic = 1.0f;
    }
    return metallic;
}

/* Resolve the material's emissive color (linear RGB) = emission_color scaled by
   emission_factor, defaulting to black when the material declares none. */
static void review_import_material_emissive(const ufbx_material *material, float out_color[3])
{
    float factor = 1.0f;

    out_color[0] = 0.0f;
    out_color[1] = 0.0f;
    out_color[2] = 0.0f;

    if (!material) {
        return;
    }

    if (material->pbr.emission_factor.has_value) {
        factor = (float)material->pbr.emission_factor.value_real;
    }

    if (material->pbr.emission_color.has_value) {
        out_color[0] = (float)material->pbr.emission_color.value_vec4.x * factor;
        out_color[1] = (float)material->pbr.emission_color.value_vec4.y * factor;
        out_color[2] = (float)material->pbr.emission_color.value_vec4.z * factor;
    }
}

/* Resolve the material's smoothness (Unity-style glossiness) in [0,1]. ufbx
   normalizes shininess/glossiness models into `pbr.roughness`, mapping a
   glossiness authoring into `pbr.glossiness` as well, so prefer the explicit
   glossiness, fall back to `1 - roughness`, then to a neutral 0.5 when the
   material declares neither. */
static float review_import_material_smoothness(const ufbx_material *material)
{
    float smoothness = 0.5f;

    if (!material) {
        return smoothness;
    }

    if (material->pbr.glossiness.has_value) {
        smoothness = (float)material->pbr.glossiness.value_real;
    } else if (material->pbr.roughness.has_value) {
        smoothness = 1.0f - (float)material->pbr.roughness.value_real;
    }

    if (smoothness < 0.0f) {
        smoothness = 0.0f;
    } else if (smoothness > 1.0f) {
        smoothness = 1.0f;
    }
    return smoothness;
}

static uint32_t review_import_add_material(review_import_scene *scene, const ufbx_material *material)
{
    size_t index;
    const char *name_data = material ? material->name.data : NULL;
    size_t name_length = material ? material->name.length : 0;
    char *owned_name = NULL;

    if (!name_data || name_length == 0) {
        name_data = "Default";
        name_length = 7;
    }

    for (index = 0; index < scene->material_count; index++) {
        const char *existing_name = scene->materials[index].name;
        if (!existing_name) {
            continue;
        }

        if (strlen(existing_name) == name_length && memcmp(existing_name, name_data, name_length) == 0) {
            return (uint32_t)index;
        }
    }

    owned_name = review_import_dup_string_len(name_data, name_length);
    if (!owned_name) {
        return UINT32_MAX;
    }

    {
        review_import_material *new_materials = (review_import_material*)realloc(
            scene->materials,
            (scene->material_count + 1) * sizeof(review_import_material)
        );
        review_import_material *slot;
        if (!new_materials) {
            free(owned_name);
            return UINT32_MAX;
        }

        scene->materials = new_materials;
        slot = &scene->materials[scene->material_count];
        slot->name = owned_name;
        slot->draw_count = 0;
        /* Seed the editable material table's import defaults (Phase 1). */
        review_import_material_base_color_linear(material, slot->base_color);
        slot->smoothness = review_import_material_smoothness(material);
        slot->metallic = review_import_material_metallic(material);
        review_import_material_emissive(material, slot->emissive);
        scene->material_count += 1;
    }

    return (uint32_t)(scene->material_count - 1);
}

static int review_import_material_used(uint32_t *used_slots, size_t used_count, uint32_t slot)
{
    size_t index;
    for (index = 0; index < used_count; index++) {
        if (used_slots[index] == slot) {
            return 1;
        }
    }
    return 0;
}

int review_import_load_fbx(
    const char *path,
    const review_import_options *options,
    review_import_scene *out_scene,
    review_import_error *out_error
)
{
    ufbx_load_opts load_opts = { 0 };
    ufbx_error error;
    ufbx_scene *scene = NULL;
    size_t total_vertices = 0;
    size_t total_faces = 0;
    size_t total_triangles = 0;
    size_t node_index;
    size_t vertex_offset = 0;
    size_t face_offset = 0;
    size_t index_offset = 0;
    size_t tri_offset = 0;
    int success = 0;

    memset(&error, 0, sizeof(error));
    if (out_scene) {
        memset(out_scene, 0, sizeof(*out_scene));
    }
    review_import_set_error(out_error, NULL);
    (void)options;

    if (!path || !out_scene) {
        review_import_set_error(out_error, "invalid import arguments");
        return 0;
    }

    load_opts.generate_missing_normals = true;
    /* Normalize every file to meters so 1 world unit == 1 m regardless of the
       DCC's authoring units (Maya exports centimeters, so a 1 m cube is 100
       units otherwise). With the default space conversion (TRANSFORM_ROOT) the
       unit scale folds into the root transform and so into `geometry_to_world`,
       which we already apply to every vertex below. */
    load_opts.target_unit_meters = 1.0;
    scene = ufbx_load_file(path, &load_opts, &error);
    if (!scene) {
        char buffer[256];
        ufbx_format_error(buffer, sizeof(buffer), &error);
        review_import_set_error(out_error, buffer);
        return 0;
    }

    /* Record what the file claimed its unit was, before our target_unit_meters
       normalization rescaled everything to meters. Surfaced in the stats panel
       so a mis-authored export is visible rather than silently trusted. */
    out_scene->source_unit_meters = (float)scene->settings.original_unit_meters;

    for (node_index = 0; node_index < scene->nodes.count; node_index++) {
        ufbx_node *node = scene->nodes.data[node_index];
        ufbx_mesh *mesh = node ? node->mesh : NULL;
        size_t face_index;

        if (!mesh || !mesh->vertex_position.exists) {
            continue;
        }

        if (mesh->uv_sets.count > out_scene->uv_set_count) {
            out_scene->uv_set_count = (uint32_t)mesh->uv_sets.count;
            /* Capture the names from the mesh that defines the channel count, so
               the dropdown lists every set in source-file order. */
            if (!review_import_capture_uv_set_names(out_scene, mesh)) {
                review_import_set_error(out_error, "out of memory while recording UV set names");
                goto cleanup;
            }
        } else if (mesh->vertex_uv.exists && out_scene->uv_set_count == 0) {
            out_scene->uv_set_count = 1;
        }

        for (face_index = 0; face_index < mesh->faces.count; face_index++) {
            ufbx_face face = mesh->faces.data[face_index];
            total_vertices += face.num_indices;
            total_faces += 1;
            if (face.num_indices >= 3) {
                total_triangles += face.num_indices - 2;
            }
        }
    }

    if (total_vertices == 0 || total_triangles == 0) {
        review_import_set_error(out_error, "no triangulatable mesh data found in FBX");
        goto cleanup;
    }

    out_scene->vertices = (review_import_vertex*)calloc(total_vertices, sizeof(review_import_vertex));
    out_scene->indices = (uint32_t*)calloc(total_triangles * 3, sizeof(uint32_t));
    out_scene->faces = (review_import_face*)calloc(total_faces, sizeof(review_import_face));
    out_scene->tri_to_face = (uint32_t*)calloc(total_triangles, sizeof(uint32_t));
    out_scene->tri_material = (uint32_t*)calloc(total_triangles, sizeof(uint32_t));
    out_scene->tri_node = (uint32_t*)calloc(total_triangles, sizeof(uint32_t));
    if (!out_scene->vertices || !out_scene->indices || !out_scene->faces ||
        !out_scene->tri_to_face || !out_scene->tri_material || !out_scene->tri_node) {
        review_import_set_error(out_error, "out of memory while allocating imported mesh");
        goto cleanup;
    }

    out_scene->vertex_count = total_vertices;
    out_scene->index_count = total_triangles * 3;
    out_scene->face_count = total_faces;
    out_scene->tri_to_face_count = total_triangles;
    out_scene->tri_material_count = total_triangles;
    out_scene->tri_node_count = total_triangles;

    /* Only multi-set models need separate per-channel UV storage; single-set
       models keep using review_import_vertex::uv (channel 0). */
    if (out_scene->uv_set_count > 1) {
        size_t uv_value_count = total_vertices * (size_t)out_scene->uv_set_count * 2;
        out_scene->uvs = (float*)calloc(uv_value_count, sizeof(float));
        if (!out_scene->uvs) {
            review_import_set_error(out_error, "out of memory while allocating UV channels");
            goto cleanup;
        }
        out_scene->uv_value_count = uv_value_count;
    }

    for (node_index = 0; node_index < scene->nodes.count; node_index++) {
        ufbx_node *node = scene->nodes.data[node_index];
        ufbx_mesh *mesh = node ? node->mesh : NULL;
        ufbx_matrix normal_matrix;
        uint32_t *triangle_buffer = NULL;
        size_t triangle_buffer_size = 0;
        uint32_t *used_material_slots = NULL;
        size_t used_material_count = 0;
        size_t used_material_capacity = 0;
        int node_contributed_draw = 0;
        int node_has_triangles = 0;
        size_t face_index;

        if (!mesh || !mesh->vertex_position.exists) {
            continue;
        }

        normal_matrix = ufbx_matrix_for_normals(&node->geometry_to_world);
        triangle_buffer_size = mesh->max_face_triangles > 0 ? mesh->max_face_triangles * 3 : 3;
        triangle_buffer = (uint32_t*)malloc(triangle_buffer_size * sizeof(uint32_t));
        if (!triangle_buffer) {
            review_import_set_error(out_error, "out of memory while triangulating imported mesh");
            free(used_material_slots);
            goto cleanup;
        }

        for (face_index = 0; face_index < mesh->faces.count; face_index++) {
            ufbx_face face = mesh->faces.data[face_index];
            size_t local_face_first_vertex = vertex_offset;
            size_t corner_index;
            ufbx_material *face_material_ptr = NULL;

            if (mesh->face_material.count > face_index) {
                uint32_t face_material = mesh->face_material.data[face_index];
                if (face_material < node->materials.count) {
                    face_material_ptr = node->materials.data[face_material];
                } else if (face_material < mesh->materials.count) {
                    face_material_ptr = mesh->materials.data[face_material];
                }
            }

            out_scene->faces[face_offset].first_index = (uint32_t)local_face_first_vertex;
            out_scene->faces[face_offset].index_count = face.num_indices;

            for (corner_index = 0; corner_index < face.num_indices; corner_index++) {
                size_t mesh_index = (size_t)face.index_begin + corner_index;
                review_import_vertex *dst = &out_scene->vertices[vertex_offset];
                ufbx_vec3 position = ufbx_transform_position(
                    &node->geometry_to_world,
                    ufbx_get_vertex_vec3(&mesh->vertex_position, mesh_index)
                );
                ufbx_vec3 normal = mesh->vertex_normal.exists
                    ? ufbx_transform_direction(&normal_matrix, ufbx_get_vertex_vec3(&mesh->vertex_normal, mesh_index))
                    : ufbx_zero_vec3;
                ufbx_vec2 uv = mesh->vertex_uv.exists
                    ? ufbx_get_vertex_vec2(&mesh->vertex_uv, mesh_index)
                    : ufbx_zero_vec2;
                ufbx_vec3 tangent = mesh->vertex_tangent.exists
                    ? ufbx_transform_direction(&normal_matrix, ufbx_get_vertex_vec3(&mesh->vertex_tangent, mesh_index))
                    : ufbx_zero_vec3;
                ufbx_vec4 vertex_color;
                const float fallback_normal[3] = { 0.0f, 1.0f, 0.0f };
                const float fallback_tangent[3] = { 1.0f, 0.0f, 0.0f };

                /* Mesh vertex-color attribute (the DCC color set), kept separate
                   from the baked material base color above. Stored as authored
                   (no sRGB re-encode): the renderer treats RGB as gamma-space and
                   round-trips it, so the displayed color matches the file. */
                if (mesh->vertex_color.exists) {
                    vertex_color = ufbx_get_vertex_vec4(&mesh->vertex_color, mesh_index);
                } else {
                    vertex_color.x = 1.0;
                    vertex_color.y = 1.0;
                    vertex_color.z = 1.0;
                    vertex_color.w = 1.0;
                }

                dst->position[0] = (float)position.x;
                dst->position[1] = (float)position.y;
                dst->position[2] = (float)position.z;

                dst->normal[0] = (float)normal.x;
                dst->normal[1] = (float)normal.y;
                dst->normal[2] = (float)normal.z;
                review_import_normalize3(dst->normal, fallback_normal);

                dst->uv[0] = (float)uv.x;
                dst->uv[1] = (float)uv.y;

                dst->tangent[0] = (float)tangent.x;
                dst->tangent[1] = (float)tangent.y;
                dst->tangent[2] = (float)tangent.z;
                if (mesh->vertex_tangent.exists) {
                    review_import_normalize3(dst->tangent, fallback_tangent);
                    dst->tangent[3] = 1.0f;
                } else {
                    /* No tangent layer in the file: leave a zero tangent (skip the
                       constant fallback) so the Rust importer detects it and
                       synthesizes a real per-vertex tangent basis from the UVs +
                       normals. A constant placeholder tangent yields a garbage TBN
                       and smeared normal-mapped shading. */
                    dst->tangent[0] = 0.0f;
                    dst->tangent[1] = 0.0f;
                    dst->tangent[2] = 0.0f;
                    dst->tangent[3] = 0.0f;
                }

                dst->vertex_color[0] = (float)vertex_color.x;
                dst->vertex_color[1] = (float)vertex_color.y;
                dst->vertex_color[2] = (float)vertex_color.z;
                dst->vertex_color[3] = (float)vertex_color.w;

                if (out_scene->uvs) {
                    uint32_t channel;
                    for (channel = 0; channel < out_scene->uv_set_count; channel++) {
                        float channel_u = 0.0f;
                        float channel_v = 0.0f;
                        size_t base = ((size_t)channel * total_vertices + vertex_offset) * 2;

                        if (channel < mesh->uv_sets.count) {
                            ufbx_uv_set uv_set = mesh->uv_sets.data[channel];
                            if (uv_set.vertex_uv.exists) {
                                ufbx_vec2 set_uv = ufbx_get_vertex_vec2(&uv_set.vertex_uv, mesh_index);
                                channel_u = (float)set_uv.x;
                                channel_v = (float)set_uv.y;
                            }
                        }

                        out_scene->uvs[base + 0] = channel_u;
                        out_scene->uvs[base + 1] = channel_v;
                    }
                }

                vertex_offset += 1;
            }

            if (face.num_indices >= 3) {
                uint32_t triangle_count = ufbx_triangulate_face(
                    triangle_buffer,
                    triangle_buffer_size,
                    mesh,
                    face
                );
                uint32_t triangle_index;
                uint32_t material_slot = UINT32_MAX;

                /* Resolve the material slot first so each triangle can record it
                   into the parallel `tri_material` array below. */
                if (face_material_ptr) {
                    material_slot = review_import_add_material(out_scene, face_material_ptr);
                    if (material_slot == UINT32_MAX) {
                        review_import_set_error(out_error, "out of memory while recording FBX materials");
                        free(triangle_buffer);
                        free(used_material_slots);
                        goto cleanup;
                    }
                }

                for (triangle_index = 0; triangle_index < triangle_count * 3; triangle_index++) {
                    out_scene->indices[index_offset++] = (uint32_t)local_face_first_vertex +
                        (triangle_buffer[triangle_index] - face.index_begin);
                }

                for (triangle_index = 0; triangle_index < triangle_count; triangle_index++) {
                    out_scene->tri_to_face[tri_offset] = (uint32_t)face_offset;
                    out_scene->tri_material[tri_offset] = material_slot;
                    /* `node_index` indexes `scene->nodes`, and ufbx guarantees
                       node->typed_id == that index, so it doubles as the
                       review_import_node index recorded for the Outliner. */
                    out_scene->tri_node[tri_offset] = (uint32_t)node_index;
                    tri_offset++;
                }

                node_has_triangles = 1;

                if (material_slot != UINT32_MAX) {
                    if (!review_import_material_used(used_material_slots, used_material_count, material_slot)) {
                        if (used_material_count == used_material_capacity) {
                            size_t new_capacity = used_material_capacity > 0 ? used_material_capacity * 2 : 4;
                            uint32_t *new_used_slots = (uint32_t*)realloc(
                                used_material_slots,
                                new_capacity * sizeof(uint32_t)
                            );
                            if (!new_used_slots) {
                                review_import_set_error(out_error, "out of memory while recording FBX draw calls");
                                free(triangle_buffer);
                                free(used_material_slots);
                                goto cleanup;
                            }

                            used_material_slots = new_used_slots;
                            used_material_capacity = new_capacity;
                        }

                        used_material_slots[used_material_count++] = material_slot;
                        out_scene->materials[material_slot].draw_count += 1;
                        out_scene->draw_count += 1;
                    }
                    node_contributed_draw = 1;
                }
            }

            face_offset += 1;
        }

        if (!node_contributed_draw && node_has_triangles) {
            out_scene->draw_count += 1;
        }

        free(triangle_buffer);
        free(used_material_slots);
    }

    /* Capture the full scene-graph hierarchy (every node, mesh-bearing or not)
       for the Outliner. Walks `scene->nodes` in the same order as the geometry
       fill so `mesh_part_index` lines up with that traversal. Display metadata
       only — geometry is already world-baked above. */
    if (scene->nodes.count > 0) {
        size_t mesh_part_counter = 0;

        out_scene->nodes = (review_import_node*)calloc(scene->nodes.count, sizeof(review_import_node));
        if (!out_scene->nodes) {
            review_import_set_error(out_error, "out of memory while recording scene nodes");
            goto cleanup;
        }
        out_scene->node_count = scene->nodes.count;

        for (node_index = 0; node_index < scene->nodes.count; node_index++) {
            ufbx_node *node = scene->nodes.data[node_index];
            review_import_node *dst = &out_scene->nodes[node_index];
            ufbx_matrix transform = node->node_to_world;
            size_t col;

            dst->name = review_import_dup_ufbx_string(node->name);
            if (!dst->name) {
                review_import_set_error(out_error, "out of memory while recording scene node names");
                goto cleanup;
            }

            dst->parent = node->parent ? (int32_t)node->parent->typed_id : -1;

            if (node->mesh && node->mesh->vertex_position.exists) {
                dst->mesh_part_index = (int32_t)mesh_part_counter++;
            } else {
                dst->mesh_part_index = -1;
            }

            /* node_to_world is a column-major affine (4 columns of 3); expand to a
               full column-major 4x4 with the implicit [0,0,0,1] bottom row. */
            for (col = 0; col < 4; col++) {
                dst->transform[col * 4 + 0] = (float)transform.cols[col].x;
                dst->transform[col * 4 + 1] = (float)transform.cols[col].y;
                dst->transform[col * 4 + 2] = (float)transform.cols[col].z;
                dst->transform[col * 4 + 3] = (col == 3) ? 1.0f : 0.0f;
            }
        }
    }

    success = 1;

cleanup:
    if (!success) {
        review_import_free_scene(out_scene);
    }
    if (scene) {
        ufbx_free_scene(scene);
    }
    return success;
}
