#ifndef REVIEW_IMPORT_UFBX_BRIDGE_H
#define REVIEW_IMPORT_UFBX_BRIDGE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef struct review_import_vertex {
    float position[3];
    float normal[3];
    float uv[2];
    float color[4];
    float tangent[4];
    /* Per-vertex RGBA color from the mesh's vertex-color attribute (the DCC
       color set), distinct from `color` which carries the resolved material
       base color. White (1,1,1,1) when the mesh has no vertex-color layer. */
    float vertex_color[4];
    /* Resolved material smoothness in [0,1] (glossiness == 1 - roughness),
       baked per-vertex like `color` so the shaded view can drive a specular
       highlight without a per-material draw. Defaults to 0.5 when the source
       material declares neither glossiness nor roughness. */
    float smoothness;
} review_import_vertex;

typedef struct review_import_face {
    uint32_t first_index;
    uint32_t index_count;
} review_import_face;

typedef struct review_import_material {
    char *name;
    uint32_t draw_count;
    /* Import defaults seeding the editable material table (Phase 1). Base color
       and emissive are stored in *linear* space (unlike review_import_vertex::color,
       which is sRGB-encoded for the vertex-color shader path). Smoothness is
       glossiness (1 - roughness) in [0,1]; metallic in [0,1]. */
    float base_color[3];
    float smoothness;
    float metallic;
    float emissive[3];
} review_import_material;

/* One scene-graph node. Carries the full hierarchy (every ufbx node, mesh-bearing
   or not) for the Outliner. `transform` is the node_to_world matrix as a
   column-major 4x4 (display metadata only; geometry stays world-baked). */
typedef struct review_import_node {
    char *name;
    /* Index into the scene's `nodes` array of this node's parent, or -1 for the
       root (and any node ufbx left parentless). */
    int32_t parent;
    /* Running index among mesh-bearing nodes, in the same order the geometry fill
       walks `scene->nodes`, or -1 when this node carries no renderable mesh. */
    int32_t mesh_part_index;
    /* node_to_world as a column-major 4x4 (16 floats), last row implicitly
       [0,0,0,1]. */
    float transform[16];
} review_import_node;

typedef struct review_import_warning {
    char *message;
} review_import_warning;

typedef struct review_import_scene {
    char *name;
    review_import_vertex *vertices;
    size_t vertex_count;
    uint32_t *indices;
    size_t index_count;
    review_import_face *faces;
    size_t face_count;
    uint32_t *tri_to_face;
    size_t tri_to_face_count;
    review_import_material *materials;
    size_t material_count;
    review_import_warning *warnings;
    size_t warning_count;
    uint32_t uv_set_count;
    uint32_t draw_count;
    /* Channel-major flat UV storage, only allocated when uv_set_count > 1:
       uvs[(channel * vertex_count + vertex) * 2 + {0,1}]. NULL otherwise
       (single-set models carry channel 0 in review_import_vertex::uv). */
    float *uvs;
    size_t uv_value_count;
    /* The file's authored world unit in meters per source unit (e.g. 0.01 for a
       centimeter file), captured before normalizing the scene to meters. 0.0 if
       the file declared no unit. */
    float source_unit_meters;
    /* The model's UV-set names in source-file order (e.g. "UVMap",
       "UVMap.001"), one per UV set, captured from the mesh that defines
       `uv_set_count`. Each entry is an owned, NUL-terminated string; an entry
       may be the empty string if the source set carried no name. NULL when the
       model has no UV sets. */
    char **uv_set_names;
    size_t uv_set_name_count;
    /* Full scene-graph node hierarchy (one entry per ufbx node), for the
       Outliner. NULL when the scene has no nodes. */
    review_import_node *nodes;
    size_t node_count;
    /* Per-triangle material slot, parallel to `tri_to_face` (same length /
       ordering). Each entry indexes `materials`, or UINT32_MAX for a triangle
       whose face carried no material. NULL when there are no triangles. */
    uint32_t *tri_material;
    size_t tri_material_count;
} review_import_scene;

typedef struct review_import_options {
    bool triangulate;
} review_import_options;

typedef struct review_import_error {
    char message[256];
} review_import_error;

int review_import_load_fbx(
    const char *path,
    const review_import_options *options,
    review_import_scene *out_scene,
    review_import_error *out_error
);

void review_import_free_scene(review_import_scene *scene);

#endif
