/*
 * The curvature-driven density field both retopology engines are steered by.
 *
 * Left alone, a field-guided remesher spreads one face size over the whole
 * object: a flat panel and a tight fillet come out at the same resolution,
 * which is the wrong place to spend a budget. Both vendored engines can carry a
 * per-vertex spacing multiplier — QuadriFlow ships one (`Hierarchy::mS`) and
 * Instant Meshes has been given one — but neither *decides* what should go in
 * it. That decision is here, once, so the two engines vary face size by the same
 * rule and a user moving between them sees the same behaviour.
 *
 * It is a separate translation unit from either driver for the reason the two
 * drivers are separate from each other: it must not include either engine's
 * headers, since `common.h` and `field-math.hpp` define the same names at
 * different scopes. It takes flat arrays and builds its own adjacency.
 */

#ifndef REVIEW_REMESH_DENSITY_H
#define REVIEW_REMESH_DENSITY_H

#include <cstddef>
#include <cstdint>
#include <vector>

namespace review {

/// The working mesh a field is wanted over — whatever the engine actually
/// solves on, which is not the caller's mesh: both engines subdivide first.
struct DensityInput {
    /// Three per vertex.
    const float *positions = nullptr;
    /// Three per vertex, unit length. The engine's own smoothed or creased
    /// normals, so a crease the user asked to keep reads as curvature.
    const float *normals = nullptr;
    /// One per vertex — the dual area the engine already computed. May be null,
    /// in which case every vertex weighs the same, which biases the budget
    /// toward whichever part of the surface is finely triangulated.
    const float *areas = nullptr;
    std::size_t vertex_count = 0;
    /// Three per triangle.
    const std::uint32_t *indices = nullptr;
    std::size_t index_count = 0;
};

/// Fill `out` with one spacing multiplier per vertex: the factor the engine's
/// uniform target edge length is scaled by there. Below 1 is denser than the
/// uniform result, above 1 is coarser, and the field is normalized so the *face
/// count* comes out where it would have — the budget is redistributed, not
/// raised.
///
/// `strength` runs 0 (a field of exactly 1, i.e. what the engine did before) to
/// 1 (as far as the layout will take it). `target_edge` is the uniform spacing
/// the engine derived from the requested face count, in the same units as
/// `positions`; it is what keeps the field from asking for faces smaller than
/// the triangles underneath them, which the extraction cannot deliver.
///
/// Returns false and leaves `out` untouched when there is nothing to do —
/// `strength` at zero, an empty mesh, or a surface with no area. A caller that
/// gets false must run its uniform path, not a field of ones: the two are the
/// same answer, but only the first is the *untouched* code path.
bool density_field(const DensityInput &input, float strength, float target_edge,
                   std::vector<float> &out);

} // namespace review

#endif /* REVIEW_REMESH_DENSITY_H */
