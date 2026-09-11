/*
 * A flat-array bridge over ufbx_write, mirroring the shape of
 * `crates/import/src/ufbx_bridge.h`: the whole scene crosses the FFI boundary
 * as plain `repr(C)` arrays in one call, and the C side does all the
 * handle-and-setter work.
 *
 * The alternative — declaring the ~60 `ufbxw_*` entry points an export needs and
 * driving them from Rust — would spread `unsafe` across a hundred call sites and
 * leak ufbx_write's handle lifetimes into Rust. This keeps the whole write in
 * one reviewable C function with a single ownership story: the scene is created
 * and freed here, on every path.
 *
 * All coordinates are `double`, because `ufbxw_real` is `double`.
 *
 * Authored properties travel as one table (`rvo_export_scene::props`) that every
 * element references by range. The bridge applies each by name: a property the
 * element's FBX template declares is *set* (its declared type wins); any other
 * is *added* with a type derived from what the source file declared, and its
 * authored flags (`U`, `A`, `H`). That is what carries user properties, pivots,
 * rotation orders, texture wrap modes and every other authored value through
 * without the bridge knowing each one by name.
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

/* One authored property. `type` is the source reader's property type code
 * (`review_model::extras::PropType`), `flags` its flag bits (the same bit
 * values as `ufbxw_prop_flag` for `A` / `U` / `H`, plus the reader's `VALUE_*`
 * bits saying which value fields the file carried). */
typedef struct rvo_export_prop {
    /* NUL-terminated, never NULL. */
    const char *name;
    uint32_t type;
    uint32_t flags;
    int64_t value_int;
    double value_real[4];
    /* NUL-terminated, never NULL (empty when the file carried none). */
    const char *value_str;
    /* NULL when `blob_length` is 0. */
    const uint8_t *blob;
    size_t blob_length;
} rvo_export_prop;

/* A range of `rvo_export_scene::props`. */
typedef struct rvo_export_prop_range {
    uint32_t first;
    uint32_t count;
} rvo_export_prop_range;

/* What node attribute a node carries; `rvo_export_node::attribute_kind`. */
enum {
    RVO_ATTRIB_NONE = 0,
    RVO_ATTRIB_BONE = 1,
    RVO_ATTRIB_LIGHT = 2,
    RVO_ATTRIB_CAMERA = 3,
    RVO_ATTRIB_NULL = 4,
    RVO_ATTRIB_LOD_GROUP = 5
};

/* One scene-graph node.
 *
 * With `authored_transform` set, `props` carries the node's authored properties
 * (`Lcl Translation` / `Rotation` / `Scaling`, `RotationOrder`, pre/post
 * rotations, pivots, offsets, `Geometric*`, `InheritType`, `Visibility`, user
 * properties, ...) and they alone define its transform. Otherwise the
 * translation / rotation / scaling below are written as a plain XYZ-ordered
 * local transform. */
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
    int32_t authored_transform;
    rvo_export_prop_range props;

    /* One of RVO_ATTRIB_*. The attribute element gets `attribute_name` and its
     * own authored properties (a bone's `Size`, a light's `Intensity`, a
     * camera's `FocalLength`, a LOD group's `Thresholds|Level<n>`, ...). */
    uint32_t attribute_kind;
    const char *attribute_name;
    rvo_export_prop_range attribute_props;
} rvo_export_node;

/* `rvo_export_material::shader`. */
enum {
    RVO_SHADER_LAMBERT = 0,
    RVO_SHADER_PHONG = 1,
    RVO_SHADER_CUSTOM = 2
};

/* One texture connection of a material: `prop` is the material property the
 * texture feeds (`DiffuseColor`, ...), `texture` an index into
 * `rvo_export_scene::textures`. */
typedef struct rvo_export_material_texture {
    const char *prop;
    int32_t texture;
} rvo_export_material_texture;

/* One material. With `props.count > 0` the authored properties define it (on top
 * of the template `shader` selects; `shading_model` names a custom one);
 * otherwise the viewer's PBR figures are written onto a Phong. */
typedef struct rvo_export_material {
    const char *name;
    uint32_t shader;
    /* NUL-terminated; only read for RVO_SHADER_CUSTOM. */
    const char *shading_model;
    rvo_export_prop_range props;
    double base_color[3];
    double emissive[3];
    double shininess_exponent;
    double reflection_factor;
    const rvo_export_material_texture *textures;
    size_t texture_count;
} rvo_export_material;

typedef struct rvo_export_texture_layer {
    /* Index into `rvo_export_scene::textures`. */
    int32_t texture;
    /* FBX `BlendModes` enum value. */
    int32_t blend_mode;
    double alpha;
} rvo_export_texture_layer;

/* One texture. `layered` selects a `LayeredTexture` composed of `layers`; a file
 * texture carries its two authored file names, optional embedded bytes
 * (`content`) and / or a `video` (an index into `rvo_export_scene::videos`, or
 * -1). Wrap modes, UV set, UV transform and the rest are authored properties. */
typedef struct rvo_export_texture {
    const char *name;
    int32_t layered;
    const char *filename;
    const char *relative_filename;
    const uint8_t *content;
    size_t content_length;
    int32_t video;
    rvo_export_prop_range props;
    const rvo_export_texture_layer *layers;
    size_t layer_count;
} rvo_export_texture;

typedef struct rvo_export_video {
    const char *name;
    const char *filename;
    const char *relative_filename;
    const uint8_t *content;
    size_t content_length;
    rvo_export_prop_range props;
} rvo_export_video;

/* One skin cluster: a bone's binding to the mesh, with its authored matrices
 * (`Transform` = mesh node → bone, `TransformLink` = bone → world at bind, both
 * in the file's units) and the vertices it weights. */
typedef struct rvo_export_cluster {
    /* Index into `rvo_export_scene::nodes`. */
    int32_t bone;
    const char *name;
    /* Column-major 4x4. */
    double transform[16];
    double transform_link[16];
    /* `weight_count` mesh vertex indices and weights; may be empty. */
    const int32_t *vertices;
    const double *weights;
    size_t weight_count;
} rvo_export_cluster;

/* One skin deformer of a mesh. `skinning_type` is `ufbxw_skinning_type`
 * (0 rigid, 1 linear, 2 dual quaternion, 3 blend); the dual-quaternion blend
 * weights apply to the blend type. */
typedef struct rvo_export_skin {
    uint32_t skinning_type;
    const rvo_export_cluster *clusters;
    size_t cluster_count;
    const int32_t *dq_vertices;
    const double *dq_weights;
    size_t dq_count;
    /* Index into `rvo_export_scene::poses` of the bind pose this skin binds
     * against, or -1 (the writer then synthesizes one). */
    int32_t bind_pose;
} rvo_export_skin;

/* One blend shape: sparse per-vertex offsets (and normal deltas, parallel, or
 * NULL) at the channel weight `target_weight` (percent). */
typedef struct rvo_export_blend_shape {
    const char *name;
    const int32_t *vertices;
    /* 3 * offset_count. */
    const double *offsets;
    const double *normals;
    size_t offset_count;
    double target_weight;
} rvo_export_blend_shape;

/* One blend channel of a mesh, with its shapes in ascending target weight. */
typedef struct rvo_export_blend_channel {
    const char *name;
    /* The rest weight, percent. */
    double weight;
    const rvo_export_blend_shape *shapes;
    size_t shape_count;
} rvo_export_blend_channel;

typedef struct rvo_export_pose_node {
    /* Index into `rvo_export_scene::nodes`. */
    int32_t node;
    /* Column-major 4x4, the node's world matrix at bind in the file's units. */
    double matrix[16];
} rvo_export_pose_node;

/* An authored bind pose. */
typedef struct rvo_export_pose {
    const char *name;
    const rvo_export_pose_node *nodes;
    size_t node_count;
} rvo_export_pose;

/* One authored keyframe. `time` is FBX ktime (`UFBXW_KTIME_SECOND` per
 * second); `flags` are `ufbxw_keyframe_flags`; slopes are dy/dx and weights the
 * fraction of the neighbouring interval, read only under the matching flags. */
typedef struct rvo_export_key {
    int64_t time;
    double value;
    uint32_t flags;
    double weight_left;
    double weight_right;
    double slope_left;
    double slope_right;
} rvo_export_key;

typedef struct rvo_export_curve {
    const rvo_export_key *keys;
    size_t key_count;
    /* `ufbxw_extrapolation_type` codes and repeat counts. */
    uint32_t pre_mode;
    int32_t pre_repeat;
    uint32_t post_mode;
    int32_t post_repeat;
} rvo_export_curve;

/* What an animated property belongs to; `rvo_export_anim_prop::target_kind`. */
enum {
    RVO_TARGET_NODE = 0,
    RVO_TARGET_NODE_ATTRIBUTE = 1,
    RVO_TARGET_MATERIAL = 2,
    RVO_TARGET_TEXTURE = 3,
    RVO_TARGET_VIDEO = 4,
    /* `target` is a mesh, `target2` the blend channel within it. */
    RVO_TARGET_BLEND_CHANNEL = 5,
    RVO_TARGET_DISPLAY_LAYER = 6,
    RVO_TARGET_ANIM_LAYER = 7
};

/* One animated property within a layer: up to three component curves (NULL
 * for a component the source did not animate). */
typedef struct rvo_export_anim_prop {
    uint32_t target_kind;
    int32_t target;
    int32_t target2;
    const char *prop_name;
    double default_value[3];
    const rvo_export_curve *curves[3];
} rvo_export_anim_prop;

typedef struct rvo_export_anim_layer {
    const char *name;
    /* Index into `rvo_export_scene::anim_stacks`. */
    int32_t stack;
    double weight;
    rvo_export_prop_range props;
    const rvo_export_anim_prop *anim_props;
    size_t anim_prop_count;
} rvo_export_anim_layer;

typedef struct rvo_export_anim_stack {
    const char *name;
    rvo_export_prop_range props;
    int64_t time_begin;
    int64_t time_end;
} rvo_export_anim_stack;

typedef struct rvo_export_display_layer {
    const char *name;
    rvo_export_prop_range props;
    const int32_t *nodes;
    size_t node_count;
} rvo_export_display_layer;

typedef struct rvo_export_selection_node {
    /* Index into `rvo_export_scene::nodes`. */
    int32_t node;
    int32_t include_node;
    /* Indices into the node's exported mesh: vertices, edges (positions in its
     * `Edges` array) and faces. May be empty. */
    const int32_t *vertices;
    size_t vertex_count;
    const int32_t *edges;
    size_t edge_count;
    const int32_t *faces;
    size_t face_count;
} rvo_export_selection_node;

typedef struct rvo_export_selection_set {
    const char *name;
    rvo_export_prop_range props;
    const rvo_export_selection_node *nodes;
    size_t node_count;
} rvo_export_selection_set;

/* One vertex-color set beyond the first. */
typedef struct rvo_export_color_set {
    const char *name;
    /* 4 * vertex_count. */
    const double *values;
} rvo_export_color_set;

/*
 * One mesh, attached to `node`.
 *
 * Vertex attributes are all *per vertex* (not per polygon-vertex): the
 * optimizer's output already splits a vertex wherever any attribute differs, so
 * a vertex-mapped attribute is exactly as expressive and half the data.
 *
 * Geometry is polygons: `indices` is the polygon-vertex stream and
 * `face_offsets` (`face_count + 1` entries, first 0, last `index_count`) cuts it
 * into faces of at least three corners — the source's own polygons where the
 * stack preserved them, triangles elsewhere. Edges are positions in that stream
 * (the corner an edge starts at, as FBX's `Edges` array names them).
 */
typedef struct rvo_export_mesh {
    const char *name;
    /* Index into `rvo_export_scene::nodes`. */
    int32_t node;

    /* 3 * vertex_count. */
    const double *positions;
    size_t vertex_count;

    /* The polygon-vertex stream, indexing the vertices. */
    const int32_t *indices;
    size_t index_count;
    /* `face_count + 1` starts into `indices`. */
    const int32_t *face_offsets;
    size_t face_count;

    /* 3 * vertex_count, or NULL. */
    const double *normals;
    /* 4 * vertex_count, or NULL. */
    const double *colors;
    /* 4 * vertex_count (xyz + handedness), or NULL when the source authored no
     * tangent layer — synthesized tangents are never written. */
    const double *tangents;

    /* `uv_set_count` pointers, each 2 * vertex_count; NULL when there are none.
     * `uv_set_names[i]` is NUL-terminated and never NULL when present. */
    const double *const *uv_sets;
    const char *const *uv_set_names;
    size_t uv_set_count;

    /* Global material indices this mesh uses, in the order its faces reference
     * them; NULL when the mesh has no material. */
    const int32_t *material_slots;
    size_t material_slot_count;
    /* One entry per face, indexing `material_slots` (not the global table).
     * NULL when `material_slot_count <= 1`. */
    const int32_t *face_materials;

    /* The name of vertex-color set 0, or NULL for the default. */
    const char *color_set_name;
    /* Further vertex-color sets; NULL when there are none. */
    const rvo_export_color_set *color_sets;
    size_t color_set_count;

    /* Per-face layers, `face_count` each, or NULL when the source had none. */
    const uint8_t *face_smoothing;
    const uint8_t *face_hole;
    const int32_t *face_group;
    /* Edges as positions in `indices`, and their layers (`edge_count` each, or
     * NULL). NULL / 0 when the source had no edge list. */
    const int32_t *edges;
    size_t edge_count;
    const uint8_t *edge_smoothing;
    const double *edge_crease;
    const uint8_t *edge_visibility;
    /* Per vertex, or NULL. */
    const double *vertex_crease;
    /* The geometry element's authored properties (subdivision settings, ...). */
    rvo_export_prop_range props;

    /* Skin deformers (the primary first, then any further layers) and blend
     * channels; NULL / 0 when the mesh has none. */
    const rvo_export_skin *skins;
    size_t skin_count;
    const rvo_export_blend_channel *blend_channels;
    size_t blend_channel_count;
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

    /* Every authored property any element references. May be NULL when
     * `prop_count` is 0. */
    const rvo_export_prop *props;
    size_t prop_count;

    /* Coordinate axes as `ufbxw_coordinate_axis` codes (0..5), or -1 to leave
     * the writer's default. */
    int32_t axis_right;
    int32_t axis_up;
    int32_t axis_front;
    /* `ufbxw_time_mode` code, or -1 for the default; `frame_rate` is written
     * as the custom rate when the mode is custom (or when > 0 and no mode). */
    int32_t time_mode;
    double frame_rate;
    /* The remaining `GlobalSettings` properties (time span, ambient color,
     * default camera, original axis / unit, ...), applied by name. */
    rvo_export_prop_range settings_props;
    /* `SceneInfo` properties beyond the save-info block, applied by name. */
    rvo_export_prop_range scene_info_props;
    /* Save info: the source's original application and file name, carried as
     * FBX's `Original|*` block. Each NUL-terminated or NULL. */
    const char *original_application_vendor;
    const char *original_application_name;
    const char *original_application_version;
    const char *original_filename;
    /* This tool's own name and version, for the `LastSaved|*` block. */
    const char *application_name;
    const char *application_version;

    const rvo_export_node *nodes;
    size_t node_count;

    const rvo_export_material *materials;
    size_t material_count;

    const rvo_export_texture *textures;
    size_t texture_count;

    const rvo_export_video *videos;
    size_t video_count;

    const rvo_export_mesh *meshes;
    size_t mesh_count;

    const rvo_export_pose *poses;
    size_t pose_count;

    /* Animation: every stack, then every layer (each naming its stack), with
     * the authored curves. `active_stack` indexes `anim_stacks`, or -1. */
    const rvo_export_anim_stack *anim_stacks;
    size_t anim_stack_count;
    const rvo_export_anim_layer *anim_layers;
    size_t anim_layer_count;
    int32_t active_stack;

    const rvo_export_display_layer *display_layers;
    size_t display_layer_count;
    const rvo_export_selection_set *selection_sets;
    size_t selection_set_count;
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
