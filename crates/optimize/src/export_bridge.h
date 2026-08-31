/*
 * A flat-array bridge over ufbx_write, mirroring the shape of
 * `crates/import/src/ufbx_bridge.h`: the whole scene crosses the FFI boundary
 * as plain `repr(C)` arrays in one call, and the C side does all the
 * handle-and-setter work.
 *
 * The alternative — declaring the ~20 `ufbxw_*` entry points an export needs and
 * driving them from Rust — would spread `unsafe` across a hundred call sites and
 * leak ufbx_write's handle lifetimes into Rust. This keeps the whole write in
 * one reviewable C function with a single ownership story: the scene is created
 * and freed here, on every path.
 *
 * All coordinates are `double`, because `ufbxw_real` is `double`.
 */
#ifndef REVIEW_EXPORT_BRIDGE_H
#define REVIEW_EXPORT_BRIDGE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define RVO_EXPORT_ERROR_LENGTH 512

/* No parent (a root node). */
#define RVO_NO_PARENT (-1)

/* One scene-graph node. Transforms are *local* to the parent. */
typedef struct rvo_export_node {
    /* NUL-terminated UTF-8. Never NULL; an unnamed node passes "". */
    const char *name;
    /* Index into `rvo_export_scene::nodes`, or RVO_NO_PARENT. Always less than
     * this node's own index, so one forward pass can create parents first. */
    int32_t parent;
    double translation[3];
    /* Local rotation as a quaternion (x, y, z, w). */
    double rotation[4];
    double scaling[3];
} rvo_export_node;

/* One material. Written as an FBX Lambert, which every importer understands;
 * the PBR values map onto its classic slots. */
typedef struct rvo_export_material {
    const char *name;
    double base_color[3];
    double emissive[3];
} rvo_export_material;

/*
 * One mesh, attached to `node`.
 *
 * Vertex attributes are all *per vertex* (not per polygon-vertex): the
 * optimizer's output already splits a vertex wherever any attribute differs, so
 * a vertex-mapped attribute is exactly as expressive and half the data.
 *
 * Geometry is always triangles: `indices` holds `3 * triangle_count` entries.
 */
typedef struct rvo_export_mesh {
    const char *name;
    /* Index into `rvo_export_scene::nodes`. */
    int32_t node;

    /* 3 * vertex_count. */
    const double *positions;
    size_t vertex_count;

    /* 3 * triangle_count, indexing the vertices. */
    const int32_t *indices;
    size_t triangle_count;

    /* 3 * vertex_count, or NULL. */
    const double *normals;
    /* 4 * vertex_count, or NULL. */
    const double *colors;

    /* `uv_set_count` pointers, each 2 * vertex_count; NULL when there are none.
     * `uv_set_names[i]` is NUL-terminated and never NULL when present. */
    const double *const *uv_sets;
    const char *const *uv_set_names;
    size_t uv_set_count;

    /* Global material indices this mesh uses, in the order its faces reference
     * them; NULL when the mesh has no material. */
    const int32_t *material_slots;
    size_t material_slot_count;
    /* One entry per triangle, indexing `material_slots` (not the global table).
     * NULL when `material_slot_count <= 1`. */
    const int32_t *face_materials;
} rvo_export_mesh;

typedef struct rvo_export_scene {
    /*
     * Centimeters per scene unit, written as FBX's `UnitScaleFactor`.
     *
     * FBX declares its unit rather than fixing one, and its conventional default
     * is centimeters. Import normalizes everything to meters, so the geometry
     * that reaches here is metric and this must say 100 — otherwise a reader
     * takes the coordinates as centimeters and the model comes back a hundred
     * times too small.
     */
    double unit_scale_cm;

    const rvo_export_node *nodes;
    size_t node_count;

    const rvo_export_material *materials;
    size_t material_count;

    const rvo_export_mesh *meshes;
    size_t mesh_count;
} rvo_export_scene;

/*
 * Write `scene` to `path`.
 *
 * `ascii` selects the FBX text format instead of binary. Returns 0 on success,
 * non-zero on failure with a NUL-terminated message in `error` (which must have
 * room for RVO_EXPORT_ERROR_LENGTH bytes).
 *
 * The ufbx_write scene is freed on every path, success or failure.
 */
int review_export_fbx(const rvo_export_scene *scene, const char *path, int ascii, char *error,
                      size_t error_length);

/* Non-zero when this build has the vendored ufbx_write compiled in. */
int review_export_available(void);

#ifdef __cplusplus
}
#endif

#endif /* REVIEW_EXPORT_BRIDGE_H */
