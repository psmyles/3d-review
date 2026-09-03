#include "ufbx_bridge.h"

#include "ufbx.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>

static void review_import_set_error(review_import_error *out_error, const char *message)
{
    size_t length;
    if (!out_error) {
        return;
    }

    out_error->message[0] = '\0';
    if (!message) {
        return;
    }

    /* Truncating memcpy rather than strncpy: the same bounded copy without the
       deprecated call, and the same shape `export_bridge.c` uses. */
    length = strlen(message);
    if (length >= sizeof(out_error->message)) {
        length = sizeof(out_error->message) - 1;
    }
    memcpy(out_error->message, message, length);
    out_error->message[length] = '\0';
}

/* `a * b` with explicit overflow rejection, for the allocation-size and buffer
   arithmetic below (a plain multiply before malloc defeats calloc's own
   overflow check). Returns 1 and leaves `*out` untouched on overflow. */
static int review_import_mul_overflows(size_t a, size_t b, size_t *out)
{
    if (a != 0 && b > SIZE_MAX / a) {
        return 1;
    }
    *out = a * b;
    return 0;
}

static char *review_import_dup_string_len(const char *data, size_t length)
{
    char *result;
    if (length == SIZE_MAX) {
        return NULL;
    }
    result = (char*)malloc(length + 1);
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
    if (!materials) {
        return;
    }
    for (index = 0; index < material_count; index++) {
        free(materials[index].name);
    }
    free(materials);
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

    free(scene->vertices);
    free(scene->indices);
    free(scene->faces);
    free(scene->tri_to_face);
    free(scene->uvs);
    review_import_free_materials(scene->materials, scene->material_count);
    review_import_free_uv_set_names(scene->uv_set_names, scene->uv_set_name_count);
    review_import_free_nodes(scene->nodes, scene->node_count);
    free(scene->tri_material);
    free(scene->tri_node);
    free(scene->corner_source_vertex);
    free(scene->skin_offsets);
    free(scene->skin_bones);
    free(scene->skin_weights);
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

/* Bounds-checked ufbx vertex-attribute reads. The plain `ufbx_get_vertex_*`
   inline accessors guard with `ufbx_assert`, which compiles out under NDEBUG
   (i.e. in every release build) — a malformed face indexing past an attribute
   array would then read out of bounds. The `ufbx_catch_*` variants report the
   violation instead; a tripped read clears `*ok` and returns zero. */
static ufbx_vec2 review_import_get_vec2(const ufbx_vertex_vec2 *attr, size_t index, int *ok)
{
    ufbx_panic panic;
    panic.did_panic = false;
    {
        ufbx_vec2 value = ufbx_catch_get_vertex_vec2(&panic, attr, index);
        if (panic.did_panic) {
            *ok = 0;
            memset(&value, 0, sizeof(value));
        }
        return value;
    }
}

static ufbx_vec3 review_import_get_vec3(const ufbx_vertex_vec3 *attr, size_t index, int *ok)
{
    ufbx_panic panic;
    panic.did_panic = false;
    {
        ufbx_vec3 value = ufbx_catch_get_vertex_vec3(&panic, attr, index);
        if (panic.did_panic) {
            *ok = 0;
            memset(&value, 0, sizeof(value));
        }
        return value;
    }
}

static ufbx_vec4 review_import_get_vec4(const ufbx_vertex_vec4 *attr, size_t index, int *ok)
{
    ufbx_panic panic;
    panic.did_panic = false;
    {
        ufbx_vec4 value = ufbx_catch_get_vertex_vec4(&panic, attr, index);
        if (panic.did_panic) {
            *ok = 0;
            memset(&value, 0, sizeof(value));
        }
        return value;
    }
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

    /* New slots must stay addressable by a uint32_t tri_material entry below the
       UINT32_MAX no-material sentinel. */
    if (scene->material_count >= UINT32_MAX - 1) {
        return UINT32_MAX;
    }

    owned_name = review_import_dup_string_len(name_data, name_length);
    if (!owned_name) {
        return UINT32_MAX;
    }

    {
        size_t new_bytes;
        review_import_material *new_materials;
        review_import_material *slot;
        if (review_import_mul_overflows(scene->material_count + 1, sizeof(review_import_material), &new_bytes)) {
            free(owned_name);
            return UINT32_MAX;
        }
        new_materials = (review_import_material*)realloc(scene->materials, new_bytes);
        if (!new_materials) {
            free(owned_name);
            return UINT32_MAX;
        }

        scene->materials = new_materials;
        slot = &scene->materials[scene->material_count];
        slot->name = owned_name;
        /* Seed the editable material table's import defaults (Phase 1). */
        review_import_material_base_color_linear(material, slot->base_color);
        slot->smoothness = review_import_material_smoothness(material);
        slot->metallic = review_import_material_metallic(material);
        review_import_material_emissive(material, slot->emissive);
        scene->material_count += 1;
    }

    return (uint32_t)(scene->material_count - 1);
}

/* The skin deformer this bridge reads for `mesh`, or NULL when the mesh isn't
   skinned. A mesh may carry several deformers (layered skins); we read the
   first, which is what every DCC writes for a normal single-skin character and
   what the viewer's bind-pose display needs. */
static const ufbx_skin_deformer *review_import_mesh_skin(const ufbx_mesh *mesh)
{
    if (!mesh || mesh->skin_deformers.count == 0) {
        return NULL;
    }
    return mesh->skin_deformers.data[0];
}

/* Count — and, when `out_bones` is non-NULL, emit — the valid skin influences of
   logical vertex `vertex`.

   This is the *single* definition of "valid influence" that the count and fill
   passes share (invariant 7's two-pass discipline): if they could disagree, the
   fill would either overrun its budget or leave calloc-zeroed phantom influences
   at the tail. An influence counts when its weight row is in range, its cluster
   index resolves, the cluster names a real bone node, and the weight itself is
   positive and finite. Anything else is skipped rather than failing the import —
   a single broken cluster shouldn't cost the artist the whole mesh.

   In emit mode `capacity` caps the writes, so even if the two passes somehow
   diverged the result is a reconcile error, never a heap overrun. Returns the
   number of influences counted (or emitted). */
static size_t review_import_skin_influences(
    const ufbx_skin_deformer *deformer,
    size_t vertex,
    uint32_t *out_bones,
    float *out_weights,
    size_t capacity
)
{
    ufbx_skin_vertex skin_vertex;
    size_t emitted = 0;
    size_t weight_index;

    if (!deformer || vertex >= deformer->vertices.count) {
        return 0;
    }

    skin_vertex = deformer->vertices.data[vertex];
    for (weight_index = 0; weight_index < skin_vertex.num_weights; weight_index++) {
        size_t global = (size_t)skin_vertex.weight_begin + weight_index;
        ufbx_skin_weight weight;
        const ufbx_skin_cluster *cluster;

        /* `ufbx_skin_deformer.vertices` is always in bounds, but the weight range
           it points at is only as trustworthy as the file. */
        if (global >= deformer->weights.count) {
            break;
        }
        weight = deformer->weights.data[global];
        if (weight.cluster_index >= deformer->clusters.count) {
            continue;
        }
        cluster = deformer->clusters.data[weight.cluster_index];
        if (!cluster || !cluster->bone_node) {
            continue;
        }
        /* `clean_skin_weights` drops negative / zero / NaN weights, but not an
           infinity. `> 0.0` is false for NaN, and the upper test rejects +inf,
           so what survives is always a finite positive weight — which is what
           `review_model::SkinData::validate` demands. */
        if (!(weight.weight > 0.0) || !(weight.weight < 1e30)) {
            continue;
        }
        if (out_bones) {
            if (emitted >= capacity) {
                break;
            }
            /* ufbx guarantees a node's typed_id is its index in `scene->nodes`,
               which is also its index in our own node table — so a skin influence
               and an Outliner row name the same bone with the same number. */
            out_bones[emitted] = cluster->bone_node->typed_id;
            out_weights[emitted] = (float)weight.weight;
        }
        emitted++;
    }

    return emitted;
}

/* The count pass's totals, reconciled against the fill pass's actual offsets
   before the counts are published (a drifted fill would otherwise leave
   calloc-zeroed phantom triangles at the tail of every per-triangle array). */
typedef struct review_import_totals {
    /* Per-corner expanded vertex count (one entry per face corner). */
    size_t corners;
    size_t faces;
    size_t triangles;
    /* Source DCC logical vertex count (sum of each mesh's num_vertices). */
    size_t source_vertices;
    /* Total valid (bone, weight) skin influences across every logical vertex. */
    size_t skin_influences;
} review_import_totals;

/* Count pass: walk every mesh-bearing node, total the corners / faces /
   triangles the fill pass will produce, resolve the UV-set count (+ names), and
   record the source-DCC vertex stat. Returns 1 on success, 0 with `out_error`
   set. */
static int review_import_count_pass(
    const ufbx_scene *scene,
    review_import_scene *out_scene,
    review_import_totals *totals,
    review_import_error *out_error
)
{
    size_t node_index;

    for (node_index = 0; node_index < scene->nodes.count; node_index++) {
        ufbx_node *node = scene->nodes.data[node_index];
        ufbx_mesh *mesh = node ? node->mesh : NULL;
        const ufbx_skin_deformer *deformer;
        size_t face_index;
        size_t vertex_index;

        if (!mesh || !mesh->vertex_position.exists) {
            continue;
        }

        if (mesh->uv_sets.count > out_scene->uv_set_count) {
            out_scene->uv_set_count = (uint32_t)mesh->uv_sets.count;
            /* Capture the names from the mesh that defines the channel count, so
               the dropdown lists every set in source-file order. */
            if (!review_import_capture_uv_set_names(out_scene, mesh)) {
                review_import_set_error(out_error, "out of memory while recording UV set names");
                return 0;
            }
        } else if (mesh->vertex_uv.exists && out_scene->uv_set_count == 0) {
            out_scene->uv_set_count = 1;
        }

        /* The faithful Verts stat (invariant 5): the mesh's logical vertex
           count as authored, not the per-corner expansion below. An instanced
           mesh counts once per node, matching the geometry fill. */
        totals->source_vertices += mesh->num_vertices;
        /* `corner_source_vertex` entries are uint32_t and index this numbering,
           so bound it the same way the corner/face counts are bounded below. */
        if (totals->source_vertices > UINT32_MAX) {
            review_import_set_error(out_error, "FBX mesh exceeds the 32-bit vertex index limit");
            return 0;
        }

        /* Skin influences, counted through the same predicate the fill pass
           emits with, so the two can never disagree. */
        deformer = review_import_mesh_skin(mesh);
        if (deformer) {
            for (vertex_index = 0; vertex_index < mesh->num_vertices; vertex_index++) {
                totals->skin_influences +=
                    review_import_skin_influences(deformer, vertex_index, NULL, NULL, 0);
            }
            /* CSR row starts are uint32_t offsets into this array. */
            if (totals->skin_influences > UINT32_MAX) {
                review_import_set_error(out_error, "FBX skin exceeds the 32-bit influence limit");
                return 0;
            }
        }

        for (face_index = 0; face_index < mesh->faces.count; face_index++) {
            ufbx_face face = mesh->faces.data[face_index];
            totals->corners += face.num_indices;
            totals->faces += 1;
            if (face.num_indices >= 3) {
                totals->triangles += face.num_indices - 2;
            }
            /* Corner indices are stored as uint32_t (`indices`,
               `faces[].first_index`), and face indices as uint32_t
               (`tri_to_face`) — reject a scene that exceeds them rather than
               silently truncating. Checked inside the loop so the running sums
               can never wrap. */
            if (totals->corners > UINT32_MAX || totals->faces > UINT32_MAX) {
                review_import_set_error(out_error, "FBX mesh exceeds the 32-bit vertex index limit");
                return 0;
            }
        }
    }

    return 1;
}

/* Allocate every output array from the count-pass totals, with explicit
   overflow-checked size arithmetic, and publish the counts. Returns 1 on
   success, 0 with `out_error` set (partial allocations are freed by the
   caller's `review_import_free_scene`). */
static int review_import_alloc_geometry(
    review_import_scene *out_scene,
    const review_import_totals *totals,
    review_import_error *out_error
)
{
    size_t index_count;

    if (review_import_mul_overflows(totals->triangles, 3, &index_count)) {
        review_import_set_error(out_error, "FBX mesh is too large to import");
        return 0;
    }

    out_scene->vertices = (review_import_vertex*)calloc(totals->corners, sizeof(review_import_vertex));
    out_scene->indices = (uint32_t*)calloc(index_count, sizeof(uint32_t));
    out_scene->faces = (review_import_face*)calloc(totals->faces, sizeof(review_import_face));
    out_scene->tri_to_face = (uint32_t*)calloc(totals->triangles, sizeof(uint32_t));
    out_scene->tri_material = (uint32_t*)calloc(totals->triangles, sizeof(uint32_t));
    out_scene->tri_node = (uint32_t*)calloc(totals->triangles, sizeof(uint32_t));
    if (!out_scene->vertices || !out_scene->indices || !out_scene->faces ||
        !out_scene->tri_to_face || !out_scene->tri_material || !out_scene->tri_node) {
        review_import_set_error(out_error, "out of memory while allocating imported mesh");
        return 0;
    }

    out_scene->vertex_count = totals->corners;
    out_scene->index_count = index_count;
    out_scene->face_count = totals->faces;
    out_scene->tri_to_face_count = totals->triangles;
    out_scene->tri_material_count = totals->triangles;
    out_scene->tri_node_count = totals->triangles;
    out_scene->source_vertex_count = totals->source_vertices;

    /* The corner -> logical-vertex map is allocated for every model, skinned or
       not: it is 4 bytes per render vertex, and having it unconditionally keeps
       the fill pass free of a "is this scene skinned" branch. */
    out_scene->corner_source_vertex = (uint32_t*)calloc(totals->corners, sizeof(uint32_t));
    if (!out_scene->corner_source_vertex) {
        review_import_set_error(out_error, "out of memory while allocating imported mesh");
        return 0;
    }
    out_scene->corner_source_vertex_count = totals->corners;

    /* Skin weights, in CSR form over the logical vertices. Unskinned scenes
       allocate nothing and marshal to `skin: None`. */
    if (totals->skin_influences > 0) {
        out_scene->skin_offsets = (uint32_t*)calloc(totals->source_vertices + 1, sizeof(uint32_t));
        out_scene->skin_bones = (uint32_t*)calloc(totals->skin_influences, sizeof(uint32_t));
        out_scene->skin_weights = (float*)calloc(totals->skin_influences, sizeof(float));
        if (!out_scene->skin_offsets || !out_scene->skin_bones || !out_scene->skin_weights) {
            review_import_set_error(out_error, "out of memory while allocating skin weights");
            return 0;
        }
        out_scene->skin_offset_count = totals->source_vertices + 1;
        out_scene->skin_influence_count = totals->skin_influences;
    }

    /* Only multi-set models need separate per-channel UV storage; single-set
       models keep using review_import_vertex::uv (channel 0). */
    if (out_scene->uv_set_count > 1) {
        size_t uv_value_count;
        if (review_import_mul_overflows(totals->corners, (size_t)out_scene->uv_set_count, &uv_value_count) ||
            review_import_mul_overflows(uv_value_count, 2, &uv_value_count)) {
            review_import_set_error(out_error, "FBX UV channels are too large to import");
            return 0;
        }
        out_scene->uvs = (float*)calloc(uv_value_count, sizeof(float));
        if (!out_scene->uvs) {
            review_import_set_error(out_error, "out of memory while allocating UV channels");
            return 0;
        }
        out_scene->uv_value_count = uv_value_count;
    }

    return 1;
}

/* Fill one expanded corner vertex (position / normal / uv / tangent /
   vertex-color, plus the multi-set UV channels) from mesh attribute index
   `mesh_index`. Returns 1 on success, 0 when the face references out-of-range
   attribute data (a malformed file). */
static int review_import_fill_vertex(
    review_import_scene *out_scene,
    const ufbx_mesh *mesh,
    const ufbx_matrix *geometry_to_world,
    const ufbx_matrix *normal_matrix,
    size_t mesh_index,
    size_t vertex_offset
)
{
    int ok = 1;
    review_import_vertex *dst = &out_scene->vertices[vertex_offset];
    ufbx_vec3 position = ufbx_transform_position(
        geometry_to_world,
        review_import_get_vec3(&mesh->vertex_position, mesh_index, &ok)
    );
    ufbx_vec3 normal = mesh->vertex_normal.exists
        ? ufbx_transform_direction(normal_matrix, review_import_get_vec3(&mesh->vertex_normal, mesh_index, &ok))
        : ufbx_zero_vec3;
    ufbx_vec2 uv = mesh->vertex_uv.exists
        ? review_import_get_vec2(&mesh->vertex_uv, mesh_index, &ok)
        : ufbx_zero_vec2;
    ufbx_vec3 tangent = mesh->vertex_tangent.exists
        ? ufbx_transform_direction(normal_matrix, review_import_get_vec3(&mesh->vertex_tangent, mesh_index, &ok))
        : ufbx_zero_vec3;
    ufbx_vec4 vertex_color;
    const float fallback_normal[3] = { 0.0f, 1.0f, 0.0f };
    const float fallback_tangent[3] = { 1.0f, 0.0f, 0.0f };

    /* Mesh vertex-color attribute (the DCC color set), kept separate
       from the baked material base color above. Stored as authored
       (no sRGB re-encode): the renderer treats RGB as gamma-space and
       round-trips it, so the displayed color matches the file. */
    if (mesh->vertex_color.exists) {
        vertex_color = review_import_get_vec4(&mesh->vertex_color, mesh_index, &ok);
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
            size_t base = ((size_t)channel * out_scene->vertex_count + vertex_offset) * 2;

            if (channel < mesh->uv_sets.count) {
                ufbx_uv_set uv_set = mesh->uv_sets.data[channel];
                if (uv_set.vertex_uv.exists) {
                    ufbx_vec2 set_uv = review_import_get_vec2(&uv_set.vertex_uv, mesh_index, &ok);
                    channel_u = (float)set_uv.x;
                    channel_v = (float)set_uv.y;
                }
            }

            out_scene->uvs[base + 0] = channel_u;
            out_scene->uvs[base + 1] = channel_v;
        }
    }

    return ok;
}

/* Fill pass: expand every mesh-bearing node's faces into the flat output
   arrays sized by the count pass, then reconcile the actual offsets against
   the counted totals so a drifted triangulation can never publish phantom
   zeroed triangles. Returns 1 on success, 0 with `out_error` set. */
static int review_import_fill_pass(
    const ufbx_scene *scene,
    review_import_scene *out_scene,
    const review_import_totals *totals,
    review_import_error *out_error
)
{
    size_t node_index;
    size_t vertex_offset = 0;
    size_t face_offset = 0;
    size_t index_offset = 0;
    size_t tri_offset = 0;
    /* Start of this node's block in the global logical-vertex numbering, grown
       by `mesh->num_vertices` per mesh-bearing node in exactly the order the
       count pass accumulated `source_vertices`. An instanced mesh therefore gets
       one distinct block per instancing node, matching the geometry. */
    size_t logical_base = 0;

    for (node_index = 0; node_index < scene->nodes.count; node_index++) {
        ufbx_node *node = scene->nodes.data[node_index];
        ufbx_mesh *mesh = node ? node->mesh : NULL;
        ufbx_matrix normal_matrix;
        uint32_t *triangle_buffer = NULL;
        size_t triangle_buffer_size = 0;
        size_t triangle_buffer_bytes = 0;
        size_t face_index;

        if (!mesh || !mesh->vertex_position.exists) {
            continue;
        }

        normal_matrix = ufbx_matrix_for_normals(&node->geometry_to_world);
        if (review_import_mul_overflows(mesh->max_face_triangles > 0 ? mesh->max_face_triangles : 1, 3, &triangle_buffer_size) ||
            review_import_mul_overflows(triangle_buffer_size, sizeof(uint32_t), &triangle_buffer_bytes)) {
            review_import_set_error(out_error, "FBX face is too large to triangulate");
            return 0;
        }
        triangle_buffer = (uint32_t*)malloc(triangle_buffer_bytes);
        if (!triangle_buffer) {
            review_import_set_error(out_error, "out of memory while triangulating imported mesh");
            return 0;
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
                /* A slot matching neither list stays NULL and imports as the
                   UINT32_MAX no-material sentinel. */
            }

            out_scene->faces[face_offset].first_index = (uint32_t)local_face_first_vertex;
            out_scene->faces[face_offset].index_count = face.num_indices;

            for (corner_index = 0; corner_index < face.num_indices; corner_index++) {
                size_t mesh_index = (size_t)face.index_begin + corner_index;
                uint32_t logical_vertex;
                if (!review_import_fill_vertex(
                        out_scene, mesh, &node->geometry_to_world, &normal_matrix,
                        mesh_index, vertex_offset)) {
                    review_import_set_error(out_error, "malformed FBX: face references out-of-range vertex data");
                    free(triangle_buffer);
                    return 0;
                }
                /* Project this expanded corner back onto the logical vertex it
                   came from — the index skin weights are stored against. */
                if (mesh_index >= mesh->vertex_indices.count) {
                    review_import_set_error(out_error, "malformed FBX: face references out-of-range vertex data");
                    free(triangle_buffer);
                    return 0;
                }
                logical_vertex = mesh->vertex_indices.data[mesh_index];
                if (logical_vertex >= mesh->num_vertices) {
                    review_import_set_error(out_error, "malformed FBX: face references out-of-range vertex data");
                    free(triangle_buffer);
                    return 0;
                }
                out_scene->corner_source_vertex[vertex_offset] =
                    (uint32_t)(logical_base + logical_vertex);
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

                /* The count pass budgeted exactly `num_indices - 2` triangles
                   for this face; a short return (ufbx refuses a face whose
                   index range is out of the mesh's bounds) must fail the import
                   here, not leave zeroed phantom triangles in the tail. */
                if (triangle_count != face.num_indices - 2) {
                    review_import_set_error(out_error, "malformed FBX: face triangulation drifted from the counted total");
                    free(triangle_buffer);
                    return 0;
                }

                /* Resolve the material slot first so each triangle can record it
                   into the parallel `tri_material` array below. */
                if (face_material_ptr) {
                    material_slot = review_import_add_material(out_scene, face_material_ptr);
                    if (material_slot == UINT32_MAX) {
                        review_import_set_error(out_error, "out of memory while recording FBX materials");
                        free(triangle_buffer);
                        return 0;
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
            }

            face_offset += 1;
        }

        free(triangle_buffer);
        logical_base += mesh->num_vertices;
    }

    /* Reconcile the fill against the count pass (belt-and-braces over the
       per-face check above): the published counts are the counted totals, so
       any drift here would mean silently wrong geometry. */
    if (vertex_offset != totals->corners || face_offset != totals->faces ||
        tri_offset != totals->triangles || index_offset != totals->triangles * 3 ||
        logical_base != totals->source_vertices) {
        review_import_set_error(out_error, "malformed FBX: mesh data drifted between the count and fill passes");
        return 0;
    }

    return 1;
}

/* Skin fill: walk the mesh-bearing nodes in the same order as the geometry fill
   and lay every logical vertex's influences down as one CSR row. Unskinned
   meshes (and vertices with no valid influence) still get a row — an empty one —
   so `skin_offsets` stays monotonic and indexable for every logical vertex in the
   scene, not just the skinned ones. Returns 1 on success, 0 with `out_error`
   set. */
static int review_import_fill_skin(
    const ufbx_scene *scene,
    review_import_scene *out_scene,
    const review_import_totals *totals,
    review_import_error *out_error
)
{
    size_t node_index;
    size_t logical_base = 0;
    size_t emitted = 0;

    /* Unskinned scene: `review_import_alloc_geometry` allocated nothing, and the
       Rust side marshals this to `skin: None`. */
    if (totals->skin_influences == 0) {
        return 1;
    }

    for (node_index = 0; node_index < scene->nodes.count; node_index++) {
        ufbx_node *node = scene->nodes.data[node_index];
        ufbx_mesh *mesh = node ? node->mesh : NULL;
        const ufbx_skin_deformer *deformer;
        size_t vertex_index;

        if (!mesh || !mesh->vertex_position.exists) {
            continue;
        }

        deformer = review_import_mesh_skin(mesh);
        for (vertex_index = 0; vertex_index < mesh->num_vertices; vertex_index++) {
            out_scene->skin_offsets[logical_base + vertex_index] = (uint32_t)emitted;
            emitted += review_import_skin_influences(
                deformer,
                vertex_index,
                out_scene->skin_bones + emitted,
                out_scene->skin_weights + emitted,
                totals->skin_influences - emitted
            );
        }
        logical_base += mesh->num_vertices;
    }

    /* The CSR terminator: row `source_vertices` is one past the last vertex. */
    out_scene->skin_offsets[totals->source_vertices] = (uint32_t)emitted;

    if (logical_base != totals->source_vertices || emitted != totals->skin_influences) {
        review_import_set_error(out_error, "malformed FBX: skin weights drifted between the count and fill passes");
        return 0;
    }

    return 1;
}

/* Capture the full scene-graph hierarchy (every node, mesh-bearing or not) for
   the Outliner. Walks `scene->nodes` in the same order as the geometry fill so
   `mesh_part_index` lines up with that traversal. Display metadata only —
   geometry is already world-baked by the fill pass. Returns 1 on success, 0
   with `out_error` set. */
static int review_import_capture_nodes(
    const ufbx_scene *scene,
    review_import_scene *out_scene,
    review_import_error *out_error
)
{
    size_t mesh_part_counter = 0;
    size_t node_index;

    if (scene->nodes.count == 0) {
        return 1;
    }

    /* `parent` / `mesh_part_index` are int32_t and `tri_node` entries uint32_t;
       reject a node table that can't be indexed by them. */
    if (scene->nodes.count > INT32_MAX) {
        review_import_set_error(out_error, "FBX scene graph exceeds the 32-bit node limit");
        return 0;
    }

    out_scene->nodes = (review_import_node*)calloc(scene->nodes.count, sizeof(review_import_node));
    if (!out_scene->nodes) {
        review_import_set_error(out_error, "out of memory while recording scene nodes");
        return 0;
    }
    out_scene->node_count = scene->nodes.count;

    for (node_index = 0; node_index < scene->nodes.count; node_index++) {
        ufbx_node *node = scene->nodes.data[node_index];
        review_import_node *dst = &out_scene->nodes[node_index];
        ufbx_matrix transform;
        size_t col;

        /* Both geometry passes tolerate a null entry in `scene->nodes`; this
           walk must agree with them, or a scene they load leaves the Outliner
           dereferencing nothing. The row stays as calloc left it — no name, no
           parent, no mesh part, kind `OTHER` — apart from an identity transform,
           since an all-zero matrix is not one anything can be placed by. */
        if (!node) {
            dst->parent = -1;
            dst->mesh_part_index = -1;
            for (col = 0; col < 4; col++) {
                dst->transform[col * 4 + col] = 1.0f;
            }
            continue;
        }

        transform = node->node_to_world;
        dst->name = review_import_dup_ufbx_string(node->name);
        if (!dst->name) {
            review_import_set_error(out_error, "out of memory while recording scene node names");
            return 0;
        }

        dst->parent = node->parent ? (int32_t)node->parent->typed_id : -1;

        if (node->mesh && node->mesh->vertex_position.exists) {
            dst->mesh_part_index = (int32_t)mesh_part_counter++;
            /* The same `num_vertices` the count pass summed into
               `source_vertex_count`, kept per node so a scoped Verts stat is a
               sum of measured values rather than an apportioning. The count
               pass already rejected a scene whose running total exceeds
               UINT32_MAX, so no single mesh can overflow this. */
            dst->source_vertex_count = (uint32_t)node->mesh->num_vertices;
        } else {
            dst->mesh_part_index = -1;
        }

        /* Classify from the node's attribute. A node with no attribute at all is
           a transform-only null, which every DCC calls an empty / group. */
        if (!node->attrib) {
            dst->kind = REVIEW_IMPORT_NODE_EMPTY;
        } else {
            switch (node->attrib_type) {
            case UFBX_ELEMENT_MESH:   dst->kind = REVIEW_IMPORT_NODE_MESH; break;
            case UFBX_ELEMENT_BONE:   dst->kind = REVIEW_IMPORT_NODE_BONE; break;
            case UFBX_ELEMENT_LIGHT:  dst->kind = REVIEW_IMPORT_NODE_LIGHT; break;
            case UFBX_ELEMENT_CAMERA: dst->kind = REVIEW_IMPORT_NODE_CAMERA; break;
            case UFBX_ELEMENT_EMPTY:  dst->kind = REVIEW_IMPORT_NODE_EMPTY; break;
            default:                  dst->kind = REVIEW_IMPORT_NODE_OTHER; break;
            }
        }
        /* `node->bone` is set whenever the node carries a Skeleton attribute,
           including the exotic multi-attribute case `attrib_type` would miss. */
        if (node->bone) {
            dst->kind = REVIEW_IMPORT_NODE_BONE;
            dst->bone_radius = (float)node->bone->radius;
            dst->bone_relative_length = (float)node->bone->relative_length;
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

    return 1;
}

int review_import_load_fbx(
    const char *path,
    review_import_scene *out_scene,
    review_import_error *out_error
)
{
    ufbx_load_opts load_opts = { 0 };
    ufbx_error error;
    ufbx_scene *scene = NULL;
    review_import_totals totals = { 0 };
    int success = 0;

    memset(&error, 0, sizeof(error));
    if (out_scene) {
        memset(out_scene, 0, sizeof(*out_scene));
    }
    review_import_set_error(out_error, NULL);

    if (!path || !out_scene) {
        review_import_set_error(out_error, "invalid import arguments");
        return 0;
    }

    load_opts.generate_missing_normals = true;
    /* Drop negative / zero / NaN skin weights at parse time so the bridge never
       has to publish an influence the model layer's validate would reject. */
    load_opts.clean_skin_weights = true;
    /* Normalize every file to meters so 1 world unit == 1 m regardless of the
       DCC's authoring units (Maya exports centimeters, so a 1 m cube is 100
       units otherwise). With the default space conversion (TRANSFORM_ROOT) the
       unit scale folds into the root transform and so into `geometry_to_world`,
       which we already apply to every vertex in the fill pass. */
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

    if (!review_import_count_pass(scene, out_scene, &totals, out_error)) {
        goto cleanup;
    }

    if (totals.corners == 0 || totals.triangles == 0) {
        review_import_set_error(out_error, "no triangulatable mesh data found in FBX");
        goto cleanup;
    }

    if (!review_import_alloc_geometry(out_scene, &totals, out_error) ||
        !review_import_fill_pass(scene, out_scene, &totals, out_error) ||
        !review_import_fill_skin(scene, out_scene, &totals, out_error) ||
        !review_import_capture_nodes(scene, out_scene, out_error)) {
        goto cleanup;
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
