#ifndef REVIEW_IMPORT_UFBX_BRIDGE_H
#define REVIEW_IMPORT_UFBX_BRIDGE_H

#include <stddef.h>
#include <stdint.h>

#include "ufbx_extras.h"

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
    /* The ufbx material this deduplicated slot came from, for the extras
       capture (which needs its properties and texture connections). Opaque to
       the Rust side; valid only while the ufbx scene is. */
    const void *source;
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
    /* The node's rest transform relative to its parent (`ufbx_node.local_transform`),
       as translation / rotation quaternion (x, y, z, w) / scale — what an
       animation clip's baked keys replace channel by channel. The scene loads with
       helper-node inherit-mode handling, so `parent_world * TRS(local)` reproduces
       `transform` exactly. */
    float local_translation[3];
    float local_rotation[4];
    float local_scale[3];
} review_import_node;

/* One skin cluster: the binding of a bone to one mesh-bearing node. */
typedef struct review_import_skin_cluster {
    /* The bone node, indexing `nodes`. */
    uint32_t bone;
    /* The skinned mesh node, indexing `nodes`. */
    uint32_t mesh_node;
    /* `geometry_to_bone * inverse(mesh geometry_to_world)`, column-major 4x4:
       takes a *baked world-space* vertex of `mesh_node` into the bone's bind
       space, so `bone_world(pose) * this` is the cluster's skinning matrix at
       any pose. */
    float world_to_bone_bind[16];
    /* The authored cluster matrices, as the file wrote them and a re-export
       writes them back: `Transform` (mesh node to bone, `mesh_node_to_bone`)
       and `TransformLink` (bone to world at bind, `bind_to_world`) — the
       latter in the scene's normalized (meter) space. Column-major 4x4. */
    double mesh_node_to_bone[16];
    double bind_to_world[16];
    /* The cluster's own name (usually the bone's), owned, NUL-terminated. */
    char *name;
} review_import_skin_cluster;

/* What one skinned mesh node's deformer declared (Inspector metadata). */
typedef struct review_import_skin_deformer {
    uint32_t mesh_node;
    /* `ufbx_skinning_method`: 0 linear, 1 rigid, 2 dual quaternion, 3 blended. */
    uint32_t method;
    uint32_t max_weights_per_vertex;
} review_import_skin_deformer;

/* A blend-shape channel (the artist-facing slider) of one mesh node. Its
   keyframes are `morph_keyframes[keyframe_first .. keyframe_first + keyframe_count]`
   in ascending target-weight order. */
typedef struct review_import_morph_channel {
    char *name;
    uint32_t mesh_node;
    /* The channel's weight at the file's default pose, in [0,1]. */
    float rest_weight;
    uint32_t keyframe_first;
    uint32_t keyframe_count;
} review_import_morph_channel;

typedef struct review_import_morph_keyframe {
    /* Indexes `morph_shapes`. */
    uint32_t shape;
    float target_weight;
} review_import_morph_keyframe;

typedef struct review_import_morph_shape {
    char *name;
} review_import_morph_shape;

/* One sparse blend-shape offset: `shape` moves logical vertex `logical_vertex`
   by `position` (and its normal by `normal`), both already rotated into the
   baked world orientation of the owning mesh node. Unsorted; the Rust side
   builds the per-logical-vertex CSR. */
typedef struct review_import_morph_entry {
    uint32_t logical_vertex;
    uint32_t shape;
    float position[3];
    float normal[3];
} review_import_morph_entry;

/* One animation clip (an FBX animation stack), baked to keyframes. Its node
   tracks are `anim_node_tracks[node_track_first ..]` and its morph tracks
   `anim_morph_tracks[morph_track_first ..]`. */
typedef struct review_import_anim_stack {
    char *name;
    double time_begin;
    double time_end;
    uint32_t node_track_first;
    uint32_t node_track_count;
    uint32_t morph_track_first;
    uint32_t morph_track_count;
} review_import_anim_stack;

/* The baked transform keys of one node within one clip: ranges into
   `anim_vec3_keys` (translation, scale) and `anim_quat_keys` (rotation). A zero
   count means the clip does not animate that channel. */
typedef struct review_import_node_track {
    uint32_t node;
    uint32_t translation_first;
    uint32_t translation_count;
    uint32_t rotation_first;
    uint32_t rotation_count;
    uint32_t scale_first;
    uint32_t scale_count;
} review_import_node_track;

typedef struct review_import_vec3_key {
    double time;
    float value[3];
} review_import_vec3_key;

typedef struct review_import_quat_key {
    double time;
    /* x, y, z, w */
    float value[4];
} review_import_quat_key;

/* The baked weight keys of one blend-shape channel within one clip: a range
   into `anim_scalar_keys`, values already divided from percent to [0,1]. */
typedef struct review_import_morph_track {
    uint32_t channel;
    uint32_t first;
    uint32_t count;
} review_import_morph_track;

typedef struct review_import_scalar_key {
    double time;
    float value;
} review_import_scalar_key;

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
       so `source_vertex_count` bounds it. Skin weights and blend-shape offsets
       are indexed by logical vertex, so this is what projects them onto the
       render mesh. Always allocated (skinned or not); NULL only when there is
       no geometry. */
    uint32_t *corner_source_vertex;
    size_t corner_source_vertex_count;
    /* Skin weights in compressed sparse-row form over the logical vertices:
       vertex v's influences are skin_bones/skin_weights[skin_offsets[v] ..
       skin_offsets[v + 1]]. `skin_offsets` is `source_vertex_count + 1` long.
       All three are NULL (and `skin_influence_count` 0) for an unskinned scene.
       `skin_bones` entries index `nodes`, not a cluster table; the parallel
       `skin_influence_cluster` indexes `skin_clusters`. */
    uint32_t *skin_offsets;
    size_t skin_offset_count;
    uint32_t *skin_bones;
    float *skin_weights;
    size_t skin_influence_count;
    uint32_t *skin_influence_cluster;
    /* One entry per (mesh node, bone) binding, in the fill pass's traversal
       order. NULL for an unskinned scene. */
    review_import_skin_cluster *skin_clusters;
    size_t skin_cluster_count;
    /* One entry per skinned mesh node. NULL for an unskinned scene. */
    review_import_skin_deformer *skin_deformers;
    size_t skin_deformer_count;
    /* Blend shapes; every array NULL (count 0) when no mesh carries a blend
       deformer with a usable offset. */
    review_import_morph_channel *morph_channels;
    size_t morph_channel_count;
    review_import_morph_keyframe *morph_keyframes;
    size_t morph_keyframe_count;
    review_import_morph_shape *morph_shapes;
    size_t morph_shape_count;
    review_import_morph_entry *morph_entries;
    size_t morph_entry_count;
    /* Animation clips; every array NULL (count 0) when the file carries no
       animation stack. */
    review_import_anim_stack *anim_stacks;
    size_t anim_stack_count;
    review_import_node_track *anim_node_tracks;
    size_t anim_node_track_count;
    review_import_vec3_key *anim_vec3_keys;
    size_t anim_vec3_key_count;
    review_import_quat_key *anim_quat_keys;
    size_t anim_quat_key_count;
    review_import_morph_track *anim_morph_tracks;
    size_t anim_morph_track_count;
    review_import_scalar_key *anim_scalar_keys;
    size_t anim_scalar_key_count;
    /* `ufbx_scene_settings.frames_per_second`; 0 when the file declared none. */
    double frames_per_second;
} review_import_scene;

typedef struct review_import_error {
    char message[256];
} review_import_error;

/* Called from inside the ufbx parse with the bytes read so far, so a large file
   can drive a progress indicator instead of a silent wait. `user` is passed
   through untouched; `bytes_total` is 0 when the size isn't known. Invoked on
   the calling thread only, never after `review_import_load_fbx` returns.

   Returns non-zero to carry on, or 0 to abandon the parse — which the viewer
   answers when the load has been superseded by a newer one. A cancelled load
   returns REVIEW_IMPORT_CANCELLED rather than an error, since nothing went
   wrong and the caller asked for it. */
typedef int (*review_import_progress_fn)(
    void *user,
    uint64_t bytes_read,
    uint64_t bytes_total
);

/* `review_import_load_fbx` return codes: any other non-zero value is success. */
#define REVIEW_IMPORT_FAILED 0
#define REVIEW_IMPORT_CANCELLED (-1)

/* `progress` may be NULL, in which case no progress is reported. `out_extras`
   may be NULL to skip the source-property capture; when given it is filled from
   the same ufbx scene in the same call (invariant 7) and must be released with
   `review_import_free_extras` — separately from `out_scene`, so the geometry
   can be freed the moment the model is built while the extras wait to be
   marshaled after it is on screen. */
int review_import_load_fbx(
    const char *path,
    review_import_scene *out_scene,
    review_import_extras *out_extras,
    review_import_error *out_error,
    review_import_progress_fn progress,
    void *progress_user
);

void review_import_free_scene(review_import_scene *scene);

#endif
