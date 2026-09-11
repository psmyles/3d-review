/*
 * The source-property capture: everything an FBX carries that the viewer never
 * draws with but a faithful re-export must write back — authored properties,
 * node attributes, textures, mesh topology layers, extra skin layers, poses,
 * display layers, selection sets, scene metadata, and the authored animation
 * curves. Read by the same `review_import_load_fbx` call that fills the
 * geometry (invariant 7: one C extraction), into a *second* output struct the
 * Rust side marshals *after* it has published the drawable model — so the
 * viewport shows the mesh first and this streams in behind it.
 *
 * Layout: flat arrays with the same count→fill discipline as `ufbx_bridge.h`,
 * plus two growable arenas. Every string is a `(offset, length)` into
 * `strings`, every binary payload (embedded texture content, blob-typed
 * properties) a `(offset, length)` into `bytes`; both arenas are one
 * allocation each, so freeing the capture is a fixed list of `free`s and
 * nothing here owns a pointer into ufbx's scene. Enum fields carry ufbx's own
 * codes (`ufbx_prop_type`, `ufbx_rotation_order`, ...) verbatim; the Rust side
 * mirrors them into `review_model::extras`.
 *
 * Index spaces match `ModelData`'s: corners are the corner-split render vertex
 * indices, logical vertices the global DCC numbering the skin CSR uses, faces
 * the global `ModelData::faces` index, nodes the `nodes` index (ufbx's
 * `typed_id`), materials the deduplicated material table.
 */
#ifndef REVIEW_IMPORT_UFBX_EXTRAS_H
#define REVIEW_IMPORT_UFBX_EXTRAS_H

#include <stddef.h>
#include <stdint.h>

/* A string in `review_import_extras::strings`. */
typedef struct review_import_str {
    uint32_t offset;
    uint32_t length;
} review_import_str;

/* A byte range in `review_import_extras::bytes`. */
typedef struct review_import_bytes {
    size_t offset;
    size_t length;
} review_import_bytes;

/* A range of `review_import_extras::props`. */
typedef struct review_import_prop_range {
    uint32_t first;
    uint32_t count;
} review_import_prop_range;

/* One authored property, as ufbx read it: `type` is `ufbx_prop_type`, `flags`
   is `ufbx_prop_flags` (the `VALUE_*` bits say which value fields the file
   carried). Synthetic and value-less properties are not captured. */
typedef struct review_import_prop {
    review_import_str name;
    uint32_t type;
    uint32_t flags;
    int64_t value_int;
    double value_real[4];
    review_import_str value_str;
    review_import_bytes value_blob;
} review_import_prop;

/* What a node's attribute is; `review_import_node_extras::attribute_kind`. */
enum {
    REVIEW_IMPORT_ATTRIB_NONE = 0,
    REVIEW_IMPORT_ATTRIB_MESH = 1,
    REVIEW_IMPORT_ATTRIB_BONE = 2,
    REVIEW_IMPORT_ATTRIB_LIGHT = 3,
    REVIEW_IMPORT_ATTRIB_CAMERA = 4,
    REVIEW_IMPORT_ATTRIB_EMPTY = 5,
    REVIEW_IMPORT_ATTRIB_LOD_GROUP = 6,
    REVIEW_IMPORT_ATTRIB_OTHER = 7
};

/* Why a node exists that the file did not author; `synthetic`. */
enum {
    REVIEW_IMPORT_SYNTHETIC_NONE = 0,
    REVIEW_IMPORT_SYNTHETIC_SCALE_HELPER = 1,
    REVIEW_IMPORT_SYNTHETIC_GEOMETRY_TRANSFORM_HELPER = 2,
    REVIEW_IMPORT_SYNTHETIC_ROOT = 3
};

/* Parallel to the scene's `nodes`. */
typedef struct review_import_node_extras {
    review_import_prop_range props;
    uint32_t rotation_order;
    uint32_t inherit_mode;
    uint32_t original_inherit_mode;
    /* `ufbx_node.geometry_to_node`, column-major 4x4. */
    double geometry_to_node[16];
    uint32_t synthetic;
    uint32_t visible;
    uint32_t attribute_kind;
    /* For LIGHT / CAMERA / LOD_GROUP kinds, the index into the matching typed
       table below; -1 otherwise. */
    int32_t attribute_index;
    review_import_str attribute_name;
    review_import_prop_range attribute_props;
} review_import_node_extras;

typedef struct review_import_light_extras {
    double color[3];
    double intensity;
    double local_direction[3];
    uint32_t type;
    uint32_t decay;
    uint32_t area_shape;
    double inner_angle;
    double outer_angle;
    uint32_t cast_light;
    uint32_t cast_shadows;
} review_import_light_extras;

typedef struct review_import_camera_extras {
    uint32_t projection_mode;
    uint32_t resolution_is_pixels;
    double resolution[2];
    double field_of_view_deg[2];
    double orthographic_extent;
    double aspect_ratio;
    double near_plane;
    double far_plane;
    uint32_t aspect_mode;
    uint32_t aperture_mode;
    uint32_t gate_fit;
    uint32_t aperture_format;
    double focal_length_mm;
    double film_size_inch[2];
    double aperture_size_inch[2];
    double squeeze_ratio;
} review_import_camera_extras;

typedef struct review_import_lod_group_extras {
    uint32_t relative_distances;
    uint32_t ignore_parent_transform;
    uint32_t use_distance_limit;
    double distance_limit_min;
    double distance_limit_max;
    /* Range of `lod_levels`. */
    uint32_t level_first;
    uint32_t level_count;
} review_import_lod_group_extras;

typedef struct review_import_lod_level {
    double distance;
    uint32_t display;
} review_import_lod_level;

/* Parallel to the scene's deduplicated `materials`. */
typedef struct review_import_material_extras {
    review_import_prop_range props;
    uint32_t shader_type;
    review_import_str shading_model;
    /* Range of `material_textures`. */
    uint32_t texture_first;
    uint32_t texture_count;
} review_import_material_extras;

typedef struct review_import_material_texture {
    review_import_str material_prop;
    review_import_str shader_prop;
    /* Indexes `textures`. */
    uint32_t texture;
} review_import_material_texture;

/* One entry per `ufbx_texture`, in `typed_id` order. */
typedef struct review_import_texture_extras {
    review_import_str name;
    uint32_t type;
    review_import_str filename;
    review_import_str absolute_filename;
    review_import_str relative_filename;
    review_import_str uv_set;
    uint32_t wrap_u;
    uint32_t wrap_v;
    uint32_t has_uv_transform;
    double uv_translation[3];
    /* x, y, z, w */
    double uv_rotation[4];
    double uv_scale[3];
    review_import_bytes content;
    /* Indexes `videos`, or -1. */
    int32_t video;
    /* Range of `texture_layers` (layered textures only). */
    uint32_t layer_first;
    uint32_t layer_count;
    review_import_prop_range props;
} review_import_texture_extras;

typedef struct review_import_texture_layer {
    uint32_t texture;
    uint32_t blend_mode;
    double alpha;
} review_import_texture_layer;

typedef struct review_import_video_extras {
    review_import_str name;
    review_import_str filename;
    review_import_str absolute_filename;
    review_import_str relative_filename;
    review_import_bytes content;
    review_import_prop_range props;
} review_import_video_extras;

typedef struct review_import_color_set {
    review_import_str name;
    uint32_t index;
    /* Start of this set's `4 * corner_count` doubles in `color_values`, or
       UINT32_MAX for set 0, whose values are the geometry's own vertex colors. */
    uint32_t value_first;
} review_import_color_set;

typedef struct review_import_face_group {
    int32_t id;
    review_import_str name;
} review_import_face_group;

typedef struct review_import_extra_skin {
    uint32_t method;
    uint32_t max_weights_per_vertex;
    uint32_t cluster_first;
    uint32_t cluster_count;
    /* `logical_count + 1` CSR row starts in `extra_skin_offsets`, relative to
       `influence_first`. */
    uint32_t offset_first;
    uint32_t influence_first;
    uint32_t influence_count;
} review_import_extra_skin;

typedef struct review_import_extra_cluster {
    uint32_t bone;
    review_import_str name;
    double mesh_node_to_bone[16];
    double bind_to_world[16];
} review_import_extra_cluster;

typedef struct review_import_extra_influence {
    /* Indexes the owning skin's clusters (relative to `cluster_first`). */
    uint32_t cluster;
    float weight;
} review_import_extra_influence;

typedef struct review_import_dq_weight {
    /* Global logical vertex. */
    uint32_t logical_vertex;
    double weight;
} review_import_dq_weight;

/* One entry per mesh part, in the geometry fill's traversal order. */
typedef struct review_import_mesh_extras {
    uint32_t node;
    review_import_str name;
    review_import_prop_range props;
    uint32_t corner_first;
    uint32_t corner_count;
    uint32_t logical_first;
    uint32_t logical_count;
    uint32_t face_first;
    uint32_t face_count;
    uint32_t tangents_authored;
    uint32_t reversed_winding;
    uint32_t color_set_first;
    uint32_t color_set_count;
    /* Range of `edges` / `edge_*`. */
    uint32_t edge_first;
    uint32_t edge_count;
    uint32_t face_group_first;
    uint32_t face_group_count;
    /* Which per-face / per-edge / per-vertex layers the file carried; the flat
       arrays hold zeros elsewhere. */
    uint32_t has_face_smoothing;
    uint32_t has_face_hole;
    uint32_t has_face_group;
    uint32_t has_edge_smoothing;
    uint32_t has_edge_crease;
    uint32_t has_edge_visibility;
    uint32_t has_vertex_crease;
    uint32_t subdivision_preview_levels;
    uint32_t subdivision_render_levels;
    uint32_t subdivision_display_mode;
    uint32_t subdivision_boundary;
    uint32_t subdivision_uv_boundary;
    uint32_t extra_skin_first;
    uint32_t extra_skin_count;
    uint32_t dq_first;
    uint32_t dq_count;
} review_import_mesh_extras;

typedef struct review_import_pose_extras {
    review_import_str name;
    uint32_t is_bind_pose;
    uint32_t entry_first;
    uint32_t entry_count;
    review_import_prop_range props;
} review_import_pose_extras;

typedef struct review_import_pose_entry {
    uint32_t node;
    double bone_to_world[16];
} review_import_pose_entry;

typedef struct review_import_display_layer_extras {
    review_import_str name;
    uint32_t visible;
    uint32_t frozen;
    double ui_color[3];
    /* Range of `layer_nodes`. */
    uint32_t node_first;
    uint32_t node_count;
    review_import_prop_range props;
} review_import_display_layer_extras;

typedef struct review_import_selection_set_extras {
    review_import_str name;
    review_import_prop_range props;
    /* Range of `selection_nodes`. */
    uint32_t node_first;
    uint32_t node_count;
} review_import_selection_set_extras;

typedef struct review_import_selection_node {
    int32_t node;
    uint32_t include_node;
    /* Ranges of `selection_indices`: vertices are global logical vertices,
       edges index the target mesh part's edge range, faces are global faces.
       Entries the target mesh could not resolve are dropped. */
    uint32_t vertex_first;
    uint32_t vertex_count;
    uint32_t edge_first;
    uint32_t edge_count;
    uint32_t face_first;
    uint32_t face_count;
} review_import_selection_node;

/* One per `ufbx_anim_stack`, in `typed_id` order. */
typedef struct review_import_anim_stack_extras {
    review_import_str name;
    review_import_prop_range props;
    /* The clip this stack became in `anim_stacks`, or -1 when its bake failed. */
    int32_t clip;
    double time_begin;
    double time_end;
    /* Range of `stack_layers` (indices into `anim_layers`). */
    uint32_t layer_first;
    uint32_t layer_count;
} review_import_anim_stack_extras;

/* One per `ufbx_anim_layer`, in `typed_id` order. */
typedef struct review_import_anim_layer_extras {
    review_import_str name;
    double weight;
    uint32_t weight_is_animated;
    uint32_t blended;
    uint32_t additive;
    uint32_t compose_rotation;
    uint32_t compose_scale;
    review_import_prop_range props;
    uint32_t anim_prop_first;
    uint32_t anim_prop_count;
} review_import_anim_layer_extras;

/* The target of an animated property; `review_import_anim_prop_extras::target_kind`. */
enum {
    REVIEW_IMPORT_TARGET_UNMAPPED = 0,
    REVIEW_IMPORT_TARGET_NODE = 1,
    REVIEW_IMPORT_TARGET_NODE_ATTRIBUTE = 2,
    REVIEW_IMPORT_TARGET_MATERIAL = 3,
    REVIEW_IMPORT_TARGET_TEXTURE = 4,
    REVIEW_IMPORT_TARGET_VIDEO = 5,
    REVIEW_IMPORT_TARGET_BLEND_CHANNEL = 6,
    REVIEW_IMPORT_TARGET_DISPLAY_LAYER = 7,
    REVIEW_IMPORT_TARGET_ANIM_LAYER = 8
};

typedef struct review_import_anim_prop_extras {
    uint32_t target_kind;
    /* Index in the target kind's table (node, material, texture, video, morph
       channel, display layer, anim layer); unused when unmapped. */
    uint32_t target;
    /* ufbx's `ufbx_element_type` code and the element's name, kept for the
       unmapped case so the export can say what it dropped. */
    uint32_t element_type;
    review_import_str element_name;
    review_import_str prop_name;
    double default_value[3];
    /* Indexes `anim_curves`, or -1 per component. */
    int32_t curves[3];
} review_import_anim_prop_extras;

typedef struct review_import_anim_curve_extras {
    uint32_t key_first;
    uint32_t key_count;
    uint32_t pre_mode;
    int32_t pre_repeat;
    uint32_t post_mode;
    int32_t post_repeat;
} review_import_anim_curve_extras;

typedef struct review_import_anim_key {
    double time;
    double value;
    uint32_t interpolation;
    float left_dx;
    float left_dy;
    float right_dx;
    float right_dy;
} review_import_anim_key;

typedef struct review_import_scene_extras {
    review_import_str creator;
    review_import_str filename;
    review_import_str original_file_path;
    uint32_t version;
    uint32_t ascii;
    review_import_str original_vendor;
    review_import_str original_name;
    review_import_str original_version;
    review_import_str latest_vendor;
    review_import_str latest_name;
    review_import_str latest_version;
    review_import_prop_range scene_props;
    review_import_prop_range settings_props;
    uint32_t axis_right;
    uint32_t axis_up;
    uint32_t axis_front;
    uint32_t original_axis_up;
    double unit_meters;
    double original_unit_meters;
    double frames_per_second;
    double ambient_color[3];
    review_import_str default_camera;
    uint32_t time_mode;
    uint32_t time_protocol;
    uint32_t snap_mode;
} review_import_scene_extras;

typedef struct review_import_extras {
    char *strings;
    size_t string_count;
    size_t string_capacity;
    unsigned char *bytes;
    size_t byte_count;
    size_t byte_capacity;

    review_import_prop *props;
    size_t prop_count;

    review_import_scene_extras scene;

    review_import_node_extras *nodes;
    size_t node_count;
    review_import_light_extras *lights;
    size_t light_count;
    review_import_camera_extras *cameras;
    size_t camera_count;
    review_import_lod_group_extras *lod_groups;
    size_t lod_group_count;
    review_import_lod_level *lod_levels;
    size_t lod_level_count;

    review_import_material_extras *materials;
    size_t material_count;
    review_import_material_texture *material_textures;
    size_t material_texture_count;
    review_import_texture_extras *textures;
    size_t texture_count;
    review_import_texture_layer *texture_layers;
    size_t texture_layer_count;
    review_import_video_extras *videos;
    size_t video_count;

    review_import_mesh_extras *meshes;
    size_t mesh_count;
    review_import_color_set *color_sets;
    size_t color_set_count;
    double *color_values;
    size_t color_value_count;
    /* Global corner-split vertex pairs, `2 * edge_count`. */
    uint32_t *edges;
    uint8_t *edge_smoothing;
    double *edge_crease;
    uint8_t *edge_visibility;
    size_t edge_count;
    /* Indexed by global face. */
    uint8_t *face_smoothing;
    uint8_t *face_hole;
    uint32_t *face_group;
    size_t face_count;
    /* Indexed by global logical vertex. */
    double *vertex_crease;
    size_t vertex_crease_count;
    review_import_face_group *face_groups;
    size_t face_group_count;
    review_import_extra_skin *extra_skins;
    size_t extra_skin_count;
    review_import_extra_cluster *extra_clusters;
    size_t extra_cluster_count;
    uint32_t *extra_skin_offsets;
    size_t extra_skin_offset_count;
    review_import_extra_influence *extra_influences;
    size_t extra_influence_count;
    review_import_dq_weight *dq_weights;
    size_t dq_weight_count;

    review_import_pose_extras *poses;
    size_t pose_count;
    review_import_pose_entry *pose_entries;
    size_t pose_entry_count;

    review_import_display_layer_extras *display_layers;
    size_t display_layer_count;
    uint32_t *layer_nodes;
    size_t layer_node_count;

    review_import_selection_set_extras *selection_sets;
    size_t selection_set_count;
    review_import_selection_node *selection_nodes;
    size_t selection_node_count;
    uint32_t *selection_indices;
    size_t selection_index_count;

    review_import_anim_stack_extras *anim_stacks;
    size_t anim_stack_count;
    uint32_t *stack_layers;
    size_t stack_layer_count;
    review_import_anim_layer_extras *anim_layers;
    size_t anim_layer_count;
    review_import_anim_prop_extras *anim_props;
    size_t anim_prop_count;
    review_import_anim_curve_extras *anim_curves;
    size_t anim_curve_count;
    review_import_anim_key *anim_keys;
    size_t anim_key_count;
} review_import_extras;

void review_import_free_extras(review_import_extras *extras);

#endif
