/*
 * The one translation unit that drives vendored Instant Meshes.
 *
 * It mirrors `batch.cpp`'s call order — the upstream command-line front end,
 * which is the reference for how the stages fit together — with three
 * deliberate differences:
 *
 *  * the mesh comes in and goes out as flat arrays rather than through the
 *    vendored `.obj` / `.ply` I/O, which is not vendored;
 *  * the field solve is driven directly instead of through `Optimizer`, which
 *    owns a worker thread and a condition variable so a GUI can watch it
 *    converge. We have nothing to watch and are already on a worker thread;
 *  * every exception becomes an error string. Instant Meshes throws
 *    `std::runtime_error` for a malformed mesh, and an exception must not cross
 *    into Rust.
 *
 * See `remesh_bridge.h` for the ownership story.
 */

#include "remesh_bridge.h"

#include "remesh_density.h"

#ifdef REVIEW_HAS_QUADRIFLOW
#include "remesh_quadriflow.h"
#endif

#include <cmath>
#include <cstring>
#include <exception>
#include <limits>
#include <memory>
#include <new>
#include <set>
#include <string>
#include <vector>

#include "common.h"
#include "dedge.h"
#include "extract.h"
#include "field.h"
#include "hierarchy.h"
#include "meshstats.h"
#include "normal.h"
#include "subdivide.h"
#include "bvh.h"
#include <tbb/tbb.h>

/* Defined in `field.cpp` but declared in no header. */
extern void freeze_ivars_orientations(MultiResolutionHierarchy &mRes, int level,
                                      bool extrinsic, int rosy);
extern void freeze_ivars_positions(MultiResolutionHierarchy &mRes, int level,
                                   bool extrinsic, int posy);

namespace {

/* Whether `a * b` would exceed `size_t`. Every destination size below is
 * checked with it before anything is allocated. */
bool mul_overflows(size_t a, size_t b) {
    return b != 0 && a > (std::numeric_limits<size_t>::max)() / b;
}

void set_error(char *error, size_t error_length, const char *message) {
    if (error == nullptr || error_length == 0) {
        return;
    }
    const size_t length = std::strlen(message);
    const size_t copied = length < error_length - 1 ? length : error_length - 1;
    std::memcpy(error, message, copied);
    error[copied] = '\0';
}

/* Releases the hierarchy's adjacency matrices, which are raw `new[]` arrays its
 * destructor does not touch. */
struct HierarchyGuard {
    MultiResolutionHierarchy &mRes;
    explicit HierarchyGuard(MultiResolutionHierarchy &hierarchy) : mRes(hierarchy) {}
    ~HierarchyGuard() { mRes.free(); }
    HierarchyGuard(const HierarchyGuard &) = delete;
    HierarchyGuard &operator=(const HierarchyGuard &) = delete;
};

struct BvhGuard {
    BVH *bvh = nullptr;
    ~BvhGuard() { delete bvh; }
    BvhGuard() = default;
    BvhGuard(const BvhGuard &) = delete;
    BvhGuard &operator=(const BvhGuard &) = delete;
};

/* Push one level of the hierarchy's solution down to the level below, exactly
 * as `Optimizer::run` does between its per-level iteration blocks: the coarse
 * value is copied to each finer vertex it covers and re-projected into that
 * vertex's tangent plane. */
void prolongate(MultiResolutionHierarchy &mRes, int level, bool orientations) {
    const MatrixXu &to_upper = mRes.toUpper(level - 1);
    const MatrixXf &N = mRes.N(level - 1);
    if (orientations) {
        const MatrixXf &source = mRes.Q(level);
        MatrixXf &destination = mRes.Q(level - 1);
        for (uint32_t j = 0; j < (uint32_t) source.cols(); ++j) {
            for (int k = 0; k < 2; ++k) {
                const uint32_t target = to_upper(k, j);
                if (target == INVALID) {
                    continue;
                }
                const Vector3f q = source.col(j);
                const Vector3f n = N.col(target);
                destination.col(target) = q - n * n.dot(q);
            }
        }
    } else {
        const MatrixXf &source = mRes.O(level);
        const MatrixXf &V = mRes.V(level - 1);
        MatrixXf &destination = mRes.O(level - 1);
        for (uint32_t j = 0; j < (uint32_t) source.cols(); ++j) {
            for (int k = 0; k < 2; ++k) {
                const uint32_t target = to_upper(k, j);
                if (target == INVALID) {
                    continue;
                }
                Vector3f o = source.col(j);
                const Vector3f n = N.col(target);
                const Vector3f v = V.col(target);
                o -= n * n.dot(o - v);
                destination.col(target) = o;
            }
        }
    }
}

/* The hierarchical solve `Optimizer::run` performs in its own thread: six
 * smoothing iterations per level, coarsest first, prolongating one level after
 * each block, then freezing the integer variables and (for orientations)
 * propagating the solution back through the hierarchy.
 *
 * The iteration count is upstream's `levelIterations`. It is not exposed as a
 * parameter because it is not a quality knob the user would know how to set:
 * fewer leaves the field unconverged and the extraction produces noise. */
void solve_field(MultiResolutionHierarchy &mRes, bool orientations, bool extrinsic,
                 int rosy, int posy) {
    const int level_iterations = 6;
    const std::function<void(uint32_t)> progress = [](uint32_t) {};

    if (orientations) {
        mRes.setFrozenQ(false);
    } else {
        mRes.setFrozenO(false);
    }

    for (int level = mRes.levels() - 1; level >= 0; --level) {
        for (int iteration = 0; iteration < level_iterations; ++iteration) {
            if (orientations) {
                optimize_orientations(mRes, level, extrinsic, rosy, progress);
            } else {
                optimize_positions(mRes, level, extrinsic, posy, progress);
            }
        }
        if (level > 0) {
            prolongate(mRes, level, orientations);
        }
    }

    if (orientations) {
        if (!mRes.frozenQ()) {
            freeze_ivars_orientations(mRes, 0, extrinsic, rosy);
        }
        mRes.propagateSolution(rosy);
    } else if (!mRes.frozenO()) {
        freeze_ivars_positions(mRes, 0, extrinsic, posy);
    }
}

} // namespace

/* The polygon soup the caller reads back. Vectors rather than raw buffers so
 * every path out of `review_remesh_run` frees them. */
struct rvo_remesh_result {
    std::vector<float> positions;
    std::vector<uint32_t> corners;
    /* `faces + 1` entries; always starts at 0. */
    std::vector<uint32_t> face_offsets;
};

extern "C" int review_remesh_run(const rvo_remesh_input *input,
                                 const rvo_remesh_options *options,
                                 rvo_remesh_result **out, char *error,
                                 size_t error_length) {
    if (out == nullptr) {
        return 0;
    }
    *out = nullptr;
    if (input == nullptr || options == nullptr) {
        set_error(error, error_length, "no mesh or options were given");
        return 0;
    }
    if (input->positions == nullptr || input->indices == nullptr) {
        set_error(error, error_length, "the mesh has no vertex or index buffer");
        return 0;
    }
    if (input->vertex_count == 0 || input->index_count == 0) {
        set_error(error, error_length, "the mesh is empty");
        return 0;
    }
    if (input->index_count % 3 != 0) {
        set_error(error, error_length,
                  "the index buffer is not a whole number of triangles");
        return 0;
    }
    if (mul_overflows(input->vertex_count, 3)) {
        set_error(error, error_length, "the mesh is too large to remesh");
        return 0;
    }
    if (options->posy != 3 && options->posy != 4) {
        set_error(error, error_length, "unsupported positional symmetry");
        return 0;
    }
    if (options->rosy != 2 && options->rosy != 4 && options->rosy != 6) {
        set_error(error, error_length, "unsupported rotational symmetry");
        return 0;
    }
    if (options->engine != RVO_REMESH_ENGINE_INSTANT_MESHES &&
        options->engine != RVO_REMESH_ENGINE_QUADRIFLOW) {
        set_error(error, error_length, "unknown remesh engine");
        return 0;
    }
#ifndef REVIEW_HAS_QUADRIFLOW
    if (options->engine == RVO_REMESH_ENGINE_QUADRIFLOW) {
        set_error(error, error_length,
                  "this build has no vendored QuadriFlow (third_party/quadriflow)");
        return 0;
    }
#endif

    for (size_t corner = 0; corner < input->index_count; ++corner) {
        if (input->indices[corner] >= input->vertex_count) {
            set_error(error, error_length, "an index addresses no vertex");
            return 0;
        }
    }
    for (size_t value = 0; value < input->vertex_count * 3; ++value) {
        if (!std::isfinite(input->positions[value])) {
            set_error(error, error_length, "a vertex position is not a finite number");
            return 0;
        }
    }

#ifdef REVIEW_HAS_QUADRIFLOW
    if (options->engine == RVO_REMESH_ENGINE_QUADRIFLOW) {
        rvo_quadriflow_request request;
        request.positions = input->positions;
        request.vertex_count = input->vertex_count;
        request.indices = input->indices;
        request.index_count = input->index_count;
        request.face_count = options->face_count < 4u ? 4u : options->face_count;
        request.preserve_sharp = options->crease_angle_deg >= 0.0f;
        request.preserve_boundary = options->align_to_boundaries;
        request.adaptive_strength = options->adaptive_strength;
        request.min_cost_flow = options->min_cost_flow;

        rvo_quadriflow_output solved;
        if (!review_quadriflow_solve(request, solved, error, error_length)) {
            return 0;
        }
        std::unique_ptr<rvo_remesh_result> result(new rvo_remesh_result());
        result->positions = std::move(solved.positions);
        result->corners = std::move(solved.corners);
        result->face_offsets = std::move(solved.face_offsets);
        *out = result.release();
        return 1;
    }
#endif

    try {
        const int rosy = (int) options->rosy;
        const int posy = (int) options->posy;
        const bool deterministic = options->deterministic != 0;
        const bool extrinsic = options->extrinsic != 0;

        /* Instant Meshes' own `deterministic` flag reaches the extraction and
         * the graph colouring but not the field solve, which runs in parallel
         * either way on the assumption that the colouring makes each phase's
         * writes disjoint — and it does not. Measured on one mesh: 1848 faces
         * at one thread, 1550 at two, and two runs in one process disagreeing
         * at eight. So when the caller asks for a reproducible result, the
         * shim is told to run this thread's parallel work in order; that, and
         * not the vendored flag, is what makes the setting true. */
        const tbb::shim::SerialScope ordered(deterministic);

        /* Instant Meshes lays vertices out column-major: three rows, one column
         * per vertex. */
        MatrixXf V(3, (Eigen::Index) input->vertex_count);
        for (size_t vertex = 0; vertex < input->vertex_count; ++vertex) {
            V(0, (Eigen::Index) vertex) = input->positions[vertex * 3 + 0];
            V(1, (Eigen::Index) vertex) = input->positions[vertex * 3 + 1];
            V(2, (Eigen::Index) vertex) = input->positions[vertex * 3 + 2];
        }
        const size_t triangle_count = input->index_count / 3;
        MatrixXu F(3, (Eigen::Index) triangle_count);
        for (size_t triangle = 0; triangle < triangle_count; ++triangle) {
            F(0, (Eigen::Index) triangle) = input->indices[triangle * 3 + 0];
            F(1, (Eigen::Index) triangle) = input->indices[triangle * 3 + 1];
            F(2, (Eigen::Index) triangle) = input->indices[triangle * 3 + 2];
        }

        const MeshStats stats = compute_mesh_stats(F, V, deterministic);
        if (!(stats.mSurfaceArea > 0.0)) {
            set_error(error, error_length, "the mesh has no surface area");
            return 0;
        }

        /* The target face count becomes a target edge length, the quantity the
         * field solve is actually parameterized by. Straight from `batch.cpp`;
         * the two formulas differ because a quad of side `s` covers `s^2` while
         * an equilateral triangle of side `s` covers `sqrt(3)/4 * s^2`. */
        const uint32_t requested = options->face_count < 4u ? 4u : options->face_count;
        const Float face_area = (Float) (stats.mSurfaceArea / (double) requested);
        const Float scale = posy == 4
                                ? std::sqrt(face_area)
                                : (Float) (2.0 * std::sqrt(face_area * std::sqrt(1.0f / 3.0f)));
        if (!(scale > 0) || !std::isfinite(scale)) {
            set_error(error, error_length, "the requested density is out of range");
            return 0;
        }

        VectorXu V2E, E2E;
        VectorXb boundary, nonManifold;
        VectorXf A;
        MatrixXf N;
        std::set<uint32_t> crease_in, crease_out;

        /* A mesh whose triangles are far larger than the target edge length
         * cannot carry a field at that resolution; upstream subdivides first. */
        if (stats.mMaximumEdgeLength * 2 > scale ||
            stats.mMaximumEdgeLength > stats.mAverageEdgeLength * 2) {
            build_dedge(F, V, V2E, E2E, boundary, nonManifold);
            subdivide(F, V, V2E, E2E, boundary, nonManifold,
                      std::min(scale / 2, (Float) stats.mAverageEdgeLength * 2),
                      deterministic);
        }

        build_dedge(F, V, V2E, E2E, boundary, nonManifold);

        AdjacencyMatrix adj = generate_adjacency_matrix_uniform(F, V2E, E2E, nonManifold);

        if (options->crease_angle_deg >= 0.0f) {
            generate_crease_normals(F, V, V2E, E2E, boundary, nonManifold,
                                    (Float) options->crease_angle_deg, N, crease_in);
        } else {
            generate_smooth_normals(F, V, V2E, E2E, nonManifold, N);
        }
        compute_dual_vertex_areas(F, V, V2E, E2E, nonManifold, A);

        MultiResolutionHierarchy mRes;
        HierarchyGuard hierarchy_guard(mRes);
        mRes.setE2E(std::move(E2E));
        mRes.setAdj(std::move(adj));
        mRes.setF(std::move(F));
        mRes.setV(std::move(V));
        mRes.setA(std::move(A));
        mRes.setN(std::move(N));
        mRes.setScale(scale);
        mRes.build(deterministic);
        mRes.resetSolution();

        /* Vary face size by curvature, when asked. The hierarchy carries an
         * optional per-vertex multiplier on `scale` (a change to the vendored
         * tree — see its review.patch), and `review::density_field` is what
         * decides where the small faces go; the QuadriFlow driver steers by the
         * same field, so the two engines behave alike. With nothing installed
         * every site in the solve reads the uniform scale exactly as before. */
        {
            review::DensityInput request;
            request.positions = mRes.V().data();
            request.normals = mRes.N().data();
            request.areas = mRes.A().data();
            request.vertex_count = (size_t) mRes.V().cols();
            request.indices = mRes.F().data();
            request.index_count = (size_t) mRes.F().size();

            std::vector<float> spacing;
            if (review::density_field(request, options->adaptive_strength, scale, spacing)) {
                VectorXf field(spacing.size());
                for (size_t vertex = 0; vertex < spacing.size(); ++vertex) {
                    field[(Eigen::Index) vertex] = spacing[vertex];
                }
                mRes.setScaleField(field);
            }
        }

        /* Pin the field to open borders so a boundary comes back as one straight
         * edge loop rather than a ragged fringe. */
        if (options->align_to_boundaries != 0) {
            mRes.clearConstraints();
            for (uint32_t corner = 0; corner < 3 * (uint32_t) mRes.F().cols(); ++corner) {
                if (mRes.E2E()[corner] != INVALID) {
                    continue;
                }
                const uint32_t i0 = mRes.F()(corner % 3, corner / 3);
                const uint32_t i1 = mRes.F()((corner + 1) % 3, corner / 3);
                const Vector3f p0 = mRes.V().col(i0);
                const Vector3f p1 = mRes.V().col(i1);
                Vector3f edge = p1 - p0;
                if (edge.squaredNorm() <= 0) {
                    continue;
                }
                edge.normalize();
                mRes.CO().col(i0) = p0;
                mRes.CO().col(i1) = p1;
                mRes.CQ().col(i0) = mRes.CQ().col(i1) = edge;
                mRes.CQw()[i0] = mRes.CQw()[i1] = mRes.COw()[i0] = mRes.COw()[i1] = 1.0f;
            }
            mRes.propagateConstraints(rosy, posy);
        }

        /* The extraction's smoothing passes re-project each moved vertex onto
         * the input surface, which needs a hierarchy over it. */
        BvhGuard bvh_guard;
        if (options->smooth_iterations > 0) {
            bvh_guard.bvh = new BVH(&mRes.F(), &mRes.V(), &mRes.N(), stats.mAABB);
            bvh_guard.bvh->build();
        }

        solve_field(mRes, true, extrinsic, rosy, posy);
        solve_field(mRes, false, extrinsic, rosy, posy);

        MatrixXf O_extracted, N_extracted, Nf_extracted;
        std::vector<std::vector<TaggedLink>> adj_extracted;
        extract_graph(mRes, extrinsic, rosy, posy, adj_extracted, O_extracted,
                      N_extracted, crease_in, crease_out, deterministic);

        MatrixXu F_extracted;
        extract_faces(adj_extracted, O_extracted, N_extracted, Nf_extracted,
                      F_extracted, posy, mRes.scale(), crease_out, true,
                      options->pure_quad != 0, bvh_guard.bvh,
                      (int) options->smooth_iterations);

        std::unique_ptr<rvo_remesh_result> result(new rvo_remesh_result());
        const size_t out_vertices = (size_t) O_extracted.cols();
        if (mul_overflows(out_vertices, 3)) {
            set_error(error, error_length, "the remesh produced more geometry than fits");
            return 0;
        }
        result->positions.resize(out_vertices * 3);
        for (size_t vertex = 0; vertex < out_vertices; ++vertex) {
            result->positions[vertex * 3 + 0] = (float) O_extracted(0, (Eigen::Index) vertex);
            result->positions[vertex * 3 + 1] = (float) O_extracted(1, (Eigen::Index) vertex);
            result->positions[vertex * 3 + 2] = (float) O_extracted(2, (Eigen::Index) vertex);
        }

        const size_t out_faces = (size_t) F_extracted.cols();
        const size_t degree = (size_t) F_extracted.rows();
        result->face_offsets.reserve(out_faces + 1);
        result->face_offsets.push_back(0);
        std::vector<uint32_t> face;
        for (size_t index = 0; index < out_faces; ++index) {
            face.clear();
            for (size_t corner = 0; corner < degree; ++corner) {
                const uint32_t vertex = F_extracted((Eigen::Index) corner, (Eigen::Index) index);
                if (vertex >= out_vertices) {
                    face.clear();
                    break;
                }
                /* A quad-slot face that is really a triangle repeats its last
                 * corner; any other repeat is a degenerate the extraction left
                 * behind. Either way, a corner equal to one already placed is
                 * not a corner. */
                bool repeated = false;
                for (size_t placed = 0; placed < face.size(); ++placed) {
                    if (face[placed] == vertex) {
                        repeated = true;
                        break;
                    }
                }
                if (!repeated) {
                    face.push_back(vertex);
                }
            }
            if (face.size() < 3) {
                continue;
            }
            result->corners.insert(result->corners.end(), face.begin(), face.end());
            result->face_offsets.push_back((uint32_t) result->corners.size());
        }

        *out = result.release();
        return 1;
    } catch (const std::exception &failure) {
        set_error(error, error_length, failure.what());
        return 0;
    } catch (...) {
        set_error(error, error_length, "the remesher failed for an unknown reason");
        return 0;
    }
}

extern "C" const float *review_remesh_positions(const rvo_remesh_result *result,
                                                size_t *vertex_count) {
    if (vertex_count != nullptr) {
        *vertex_count = result == nullptr ? 0 : result->positions.size() / 3;
    }
    if (result == nullptr || result->positions.empty()) {
        return nullptr;
    }
    return result->positions.data();
}

extern "C" const uint32_t *review_remesh_corners(const rvo_remesh_result *result,
                                                 size_t *corner_count) {
    if (corner_count != nullptr) {
        *corner_count = result == nullptr ? 0 : result->corners.size();
    }
    if (result == nullptr || result->corners.empty()) {
        return nullptr;
    }
    return result->corners.data();
}

extern "C" const uint32_t *review_remesh_face_offsets(const rvo_remesh_result *result,
                                                      size_t *face_count) {
    if (face_count != nullptr) {
        *face_count = (result == nullptr || result->face_offsets.empty())
                          ? 0
                          : result->face_offsets.size() - 1;
    }
    if (result == nullptr || result->face_offsets.empty()) {
        return nullptr;
    }
    return result->face_offsets.data();
}

extern "C" void review_remesh_free(rvo_remesh_result *result) { delete result; }
