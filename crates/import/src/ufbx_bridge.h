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
} review_import_vertex;

typedef struct review_import_face {
    uint32_t first_index;
    uint32_t index_count;
} review_import_face;

typedef struct review_import_material {
    char *name;
    uint32_t draw_count;
} review_import_material;

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
