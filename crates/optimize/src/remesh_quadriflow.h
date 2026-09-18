/*
 * The QuadriFlow half of the remesh bridge, in its own translation unit.
 *
 * Split from `remesh_bridge.cpp` because the two vendored engines cannot share
 * one: Instant Meshes' `common.h` defines `Vector3f`, `MatrixXu` and friends at
 * *global* scope, and QuadriFlow's `field-math.hpp` brings Eigen's own names in
 * for `namespace qflow`. Including both headers in one file is a collision
 * waiting to happen, and each engine's own translation unit costs nothing.
 *
 * Only this project's C++ includes it; it is not part of the C ABI in
 * `remesh_bridge.h`, and nothing in Rust names it.
 */
#ifndef REVIEW_REMESH_QUADRIFLOW_H
#define REVIEW_REMESH_QUADRIFLOW_H

#include <cstddef>
#include <cstdint>
#include <vector>

/// Everything `review_remesh_run` passes through, flattened.
struct rvo_quadriflow_request {
    const float *positions;
    std::size_t vertex_count;
    const std::uint32_t *indices;
    std::size_t index_count;
    /// Target quad count. QuadriFlow hits this far more closely than a field
    /// extraction does, because it solves for it rather than for a face size.
    std::uint32_t face_count;
    int preserve_sharp;
    int preserve_boundary;
    /// How far face size may follow curvature, 0 (never) to 1 (as far as the
    /// layout will take it). See `remesh_density.h`.
    float adaptive_strength;
    int min_cost_flow;
};

/// The same three arrays `rvo_remesh_result` holds, filled in place.
struct rvo_quadriflow_output {
    std::vector<float> positions;
    std::vector<std::uint32_t> corners;
    std::vector<std::uint32_t> face_offsets;
};

/// Solve `request` into `out`. Returns true on success; on failure `error`
/// holds a NUL-terminated message and `out` is untouched.
///
/// Never throws: QuadriFlow raises `std::runtime_error` on geometry its
/// half-edge structure cannot describe, and that must not cross into Rust.
bool review_quadriflow_solve(const rvo_quadriflow_request &request,
                             rvo_quadriflow_output &out, char *error,
                             std::size_t error_length);

#endif /* REVIEW_REMESH_QUADRIFLOW_H */
