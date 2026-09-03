#ifndef REVIEW_IMPORT_UFBX_BRIDGE_H
#define REVIEW_IMPORT_UFBX_BRIDGE_H

#include <stddef.h>
#include <stdint.h>

typedef struct review_import_vertex {
    float position[3];
    float normal[3];
    float uv[2];
    float tangent[4];
    /* Per-vertex RGBA color from the mesh's vertex-color attribute (the DCC
       color set). White (1,1,1,1) when the mesh has no vertex-color layer. The
       resolved material base color / smoothness are no longer baked per vertex
       (Phase 1): they seed `review_import_material` and drive per-material draws. */
    float vertex_color[4];
} review_import_vertex;

typedef struct review_import_face {
    uint32_t first_index;
    uint32_t index_count;
} review_import_face;

typedef struct review_import_material {
    char *name;
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
/* What a node *is*, from its ufbx node attribute. Mirrored by
   `review_model::NodeKind` (and `node_kind_from_code` on the Rust side); an
   unrecognized code marshals to `Other` rather than failing, so adding an
   attribute type here is backward compatible. */
typedef enum review_import_node_kind {
    REVIEW_IMPORT_NODE_OTHER = 0,
    REVIEW_IMPORT_NODE_MESH = 1,
    REVIEW_IMPORT_NODE_BONE = 2,
    REVIEW_IMPORT_NODE_LIGHT = 3,
    REVIEW_IMPORT_NODE_CAMERA = 4,
    REVIEW_IMPORT_NODE_EMPTY = 5
} review_import_node_kind;

typedef struct review_import_node {
    char *name;
    /* Index into the scene's `nodes` array of this node's parent, or -1 for the
       root (and any node ufbx left parentless). */
    int32_t parent;
    /* Running index among mesh-bearing nodes, in the same order the geometry fill
       walks `scene->nodes`, or -1 when this node carries no renderable mesh. */
    int32_t mesh_part_index;
    /* This node's mesh's own logical (DCC control-point) vertex count, 0 for a
       node carrying no mesh. Summing it over the mesh-bearing nodes reproduces
       `source_vertex_count` exactly, which is what lets the stats overlay report
       a faithful Verts figure for a subset of the scene (invariant 5). */
    uint32_t source_vertex_count;
    /* node_to_world as a column-major 4x4 (16 floats), last row implicitly
       [0,0,0,1]. */
    float transform[16];
    /* One of `review_import_node_kind`, from the node's ufbx attribute type. */
    uint32_t kind;
    /* `ufbx_bone.radius` / `ufbx_bone.relative_length`, both 0 for a non-bone
       node (and for a bone whose file declared neither). Sizes the skeleton
       overlay's leaf/root joint markers. */
    float bone_radius;
    float bone_relative_length;
} review_import_node;

typedef struct review_import_scene {
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
    uint32_t uv_set_count;
    /* Channel-major flat UV storage, only allocated when uv_set_count > 1:
       uvs[(channel * vertex_count + vertex) * 2 + {0,1}]. NULL otherwise
       (single-set models carry channel 0 in review_import_vertex::uv). */
    float *uvs;
    size_t uv_value_count;
    /* The source DCC's logical vertex count (the sum of each mesh's
       `num_vertices`), *before* per-corner expansion — `vertex_count` above is
       the expanded corner count that sizes `vertices`. This is the faithful
       Verts stat (invariant 5). */
    size_t source_vertex_count;
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
    /* Per-triangle owning scene-graph node, parallel to `tri_to_face` (same
       length / ordering). Each entry indexes `nodes` (the node whose mesh the
       triangle came from), driving the Outliner's per-node selection / solo
       (Phase 2). NULL when there are no triangles. */
    uint32_t *tri_node;
    size_t tri_node_count;
    /* Per expanded corner (parallel to `vertices`, same length), the *logical*
       source vertex it came from, in a global numbering that concatenates each
       mesh-bearing node's `num_vertices` in the fill pass's traversal order —
       so `source_vertex_count` bounds it. Skin weights are indexed by logical
       vertex, so this is what projects them onto the render mesh. Always
       allocated (skinned or not); NULL only when there is no geometry. */
    uint32_t *corner_source_vertex;
    size_t corner_source_vertex_count;
    /* Skin weights in compressed sparse-row form over the logical vertices:
       vertex v's influences are skin_bones/skin_weights[skin_offsets[v] ..
       skin_offsets[v + 1]]. `skin_offsets` is `source_vertex_count + 1` long.
       All three are NULL (and `skin_influence_count` 0) for an unskinned scene.
       `skin_bones` entries index `nodes`, not a cluster table. */
    uint32_t *skin_offsets;
    size_t skin_offset_count;
    uint32_t *skin_bones;
    float *skin_weights;
    size_t skin_influence_count;
} review_import_scene;

typedef struct review_import_error {
    char message[256];
} review_import_error;

int review_import_load_fbx(
    const char *path,
    review_import_scene *out_scene,
    review_import_error *out_error
);

void review_import_free_scene(review_import_scene *scene);

#endif
