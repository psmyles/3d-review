/*
 * A flat-array bridge over vendored Instant Meshes (and, later, QuadriFlow),
 * mirroring the shape of `export_bridge.h`: one call takes the mesh as plain
 * arrays and hands back a polygon soup, and the C++ side owns every allocation
 * it makes.
 *
 * ## Why a result handle rather than count-then-fill
 *
 * Invariant 7's count→fill discipline fits a *parse*, whose counts are cheap to
 * establish twice. A field-guided retopology has no such counts: the output size
 * is known only once the whole solve has run, so counting first would mean
 * running the solve twice (tens of seconds) or parking its state inside the
 * bridge between two calls. Instead the run produces an opaque, bridge-owned
 * result the caller reads through the accessors below and releases with
 * `review_remesh_free`. Every allocation is made and freed by the same C++
 * runtime, which is also what keeps this safe across the CRT boundary on
 * Windows.
 *
 * ## The output
 *
 * A polygon soup in three arrays: `positions` (3 floats per vertex),
 * `face_offsets` (`face_count + 1` starts) and `corners` (vertex indices, one
 * per polygon corner). Faces are triangles or quads; the caller re-establishes
 * every other attribute by projecting onto the source surface, so nothing but
 * position crosses this boundary.
 */
#ifndef REVIEW_REMESH_BRIDGE_H
#define REVIEW_REMESH_BRIDGE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define RVO_REMESH_ERROR_LENGTH 512

/* Which retopology engine runs. QuadriFlow is only present when the build
 * found `third_party/quadriflow`; asking for it otherwise is an error the
 * caller turns into a warning and a fall back to Instant Meshes. */
enum {
    RVO_REMESH_ENGINE_INSTANT_MESHES = 0,
    RVO_REMESH_ENGINE_QUADRIFLOW = 1
};

/* The proxy to remesh: a plain indexed triangle mesh. `positions` holds
 * `vertex_count * 3` floats; `indices` holds `index_count` corner indices and
 * must be a whole number of triangles. */
typedef struct rvo_remesh_input {
    const float *positions;
    size_t vertex_count;
    const uint32_t *indices;
    size_t index_count;
} rvo_remesh_input;

/* How to remesh. Laid out to match `RemeshOptions` in `remesh_ffi.rs` field for
 * field. */
typedef struct rvo_remesh_options {
    /* One of the `RVO_REMESH_ENGINE_*` values. */
    uint32_t engine;
    /* Rotational symmetry: 2, 4 or 6. Triangles use 6, quads 4. */
    uint32_t rosy;
    /* Positional symmetry: 3 (triangles) or 4 (quads). */
    uint32_t posy;
    /* Target face count for this node. The engine hits it approximately. */
    uint32_t face_count;
    /* Crease-angle threshold in degrees; negative disables crease detection. */
    float crease_angle_deg;
    /* Solve the field in the extrinsic (rather than intrinsic) formulation. */
    int32_t extrinsic;
    /* Pin the field to open boundaries so a border stays a straight edge loop. */
    int32_t align_to_boundaries;
    /* Laplacian smoothing passes applied to the extracted mesh, projected back
     * onto the input surface each time. */
    uint32_t smooth_iterations;
    /* Quads only: subdivide the remaining triangles so the output is all quads.
     * (Instant Meshes' own "pure quad" pass, not QuadriFlow.) */
    int32_t pure_quad;
    /* Take the reproducible path through every order-sensitive stage. */
    int32_t deterministic;
    /* QuadriFlow only (phase B); ignored by Instant Meshes. */
    int32_t adaptive_scale;
    int32_t min_cost_flow;
} rvo_remesh_options;

/* The run's output. Opaque and owned by the bridge: read it through the
 * accessors, release it with `review_remesh_free`. */
typedef struct rvo_remesh_result rvo_remesh_result;

/* Remesh `input` into `*out`. Returns 1 on success, 0 on failure — in which
 * case `*out` is NULL and `error` holds a NUL-terminated message (truncated to
 * `error_length`). Never throws: every C++ exception is caught and turned into
 * that message. */
int review_remesh_run(const rvo_remesh_input *input, const rvo_remesh_options *options,
                      rvo_remesh_result **out, char *error, size_t error_length);

/* `vertex_count * 3` floats. NULL (with `*vertex_count` 0) for an empty
 * result. */
const float *review_remesh_positions(const rvo_remesh_result *result, size_t *vertex_count);

/* One vertex index per polygon corner. */
const uint32_t *review_remesh_corners(const rvo_remesh_result *result, size_t *corner_count);

/* `*face_count + 1` starts into the corner array. */
const uint32_t *review_remesh_face_offsets(const rvo_remesh_result *result, size_t *face_count);

/* Release a result. NULL is a no-op. */
void review_remesh_free(rvo_remesh_result *result);

#ifdef __cplusplus
}
#endif

#endif /* REVIEW_REMESH_BRIDGE_H */
