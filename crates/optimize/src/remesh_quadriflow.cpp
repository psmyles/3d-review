/*
 * Driving vendored QuadriFlow: the "Only quads" topology.
 *
 * The stage order is upstream's own `src/main.cpp`, with the file I/O replaced
 * by the caller's arrays. Unlike Instant Meshes, QuadriFlow *solves* for an
 * integer quad layout — a mixed-integer program over the position field — which
 * is why every face it emits is a quad and why it needs a closed manifold to
 * run on at all. The caller checks that before calling; this only has to refuse
 * gracefully when it turns out not to hold.
 *
 * It is entirely serial as vendored: every TBB path in the tree is behind
 * `WITH_TBB`, which is deliberately not defined (see the NOTICE). So the result
 * is reproducible without any of the machinery Instant Meshes needs for it —
 * the one remaining source of variation is `Hierarchy::rng_seed`, pinned below.
 */

#include "remesh_quadriflow.h"

#include <cmath>
#include <cstring>
#include <exception>
#include <string>

#include "config.hpp"
#include "field-math.hpp"
#include "optimizer.hpp"
#include "parametrizer.hpp"

namespace {

void set_error(char *error, std::size_t error_length, const char *message) {
    if (error == nullptr || error_length == 0) {
        return;
    }
    const std::size_t length = std::strlen(message);
    const std::size_t copied = length < error_length - 1 ? length : error_length - 1;
    std::memcpy(error, message, copied);
    error[copied] = '\0';
}

/// Pin the field solve's shuffle so two runs of the same input agree.
///
/// QuadriFlow seeds `srand` from this in `Hierarchy::Initialize`. Upstream's
/// command line exposes it; there is no reason to, since a user asking for a
/// different arbitrary layout of the same mesh is not a thing anyone wants.
const int FIXED_RNG_SEED = 0;

} // namespace

bool review_quadriflow_solve(const rvo_quadriflow_request &request,
                             rvo_quadriflow_output &out, char *error,
                             std::size_t error_length) {
    try {
        qflow::Parametrizer field;
        field.flag_preserve_sharp = request.preserve_sharp ? 1 : 0;
        field.flag_preserve_boundary = request.preserve_boundary ? 1 : 0;
        field.flag_adaptive_scale = request.adaptive_scale ? 1 : 0;
        field.flag_minimum_cost_flow = request.min_cost_flow ? 1 : 0;
        field.hierarchy.rng_seed = FIXED_RNG_SEED;

        /* QuadriFlow works in doubles, column-major: three rows, one column per
         * vertex, exactly as `loader.cpp` would have filled them. */
        field.V.resize(3, (Eigen::Index)request.vertex_count);
        for (std::size_t vertex = 0; vertex < request.vertex_count; ++vertex) {
            field.V(0, (Eigen::Index)vertex) = request.positions[vertex * 3 + 0];
            field.V(1, (Eigen::Index)vertex) = request.positions[vertex * 3 + 1];
            field.V(2, (Eigen::Index)vertex) = request.positions[vertex * 3 + 2];
        }
        const std::size_t triangle_count = request.index_count / 3;
        field.F.resize(3, (Eigen::Index)triangle_count);
        for (std::size_t triangle = 0; triangle < triangle_count; ++triangle) {
            field.F(0, (Eigen::Index)triangle) = (int)request.indices[triangle * 3 + 0];
            field.F(1, (Eigen::Index)triangle) = (int)request.indices[triangle * 3 + 1];
            field.F(2, (Eigen::Index)triangle) = (int)request.indices[triangle * 3 + 2];
        }

        /* Everything below operates in the normalized unit box `Load` would have
         * put the mesh in; `normalize_scale` / `normalize_offset` undo it at the
         * end, as `OutputMesh` does. */
        field.NormalizeMesh();
        field.Initialize((int)request.face_count);

        if (field.flag_preserve_boundary) {
            qflow::Hierarchy &mRes = field.hierarchy;
            mRes.clearConstraints();
            for (std::uint32_t corner = 0; corner < 3 * (std::uint32_t)mRes.mF.cols(); ++corner) {
                if (mRes.mE2E[corner] != -1) {
                    continue;
                }
                const std::uint32_t i0 = mRes.mF(corner % 3, corner / 3);
                const std::uint32_t i1 = mRes.mF((corner + 1) % 3, corner / 3);
                const qflow::Vector3d p0 = mRes.mV[0].col(i0);
                const qflow::Vector3d p1 = mRes.mV[0].col(i1);
                qflow::Vector3d edge = p1 - p0;
                if (edge.squaredNorm() <= 0) {
                    continue;
                }
                edge.normalize();
                mRes.mCO[0].col(i0) = p0;
                mRes.mCO[0].col(i1) = p1;
                mRes.mCQ[0].col(i0) = mRes.mCQ[0].col(i1) = edge;
                mRes.mCQw[0][i0] = mRes.mCQw[0][i1] = mRes.mCOw[0][i0] = mRes.mCOw[0][i1] = 1.0;
            }
            mRes.propagateConstraints();
        }

        qflow::Optimizer::optimize_orientations(field.hierarchy);
        field.ComputeOrientationSingularities();

        if (field.flag_adaptive_scale == 1) {
            field.EstimateSlope();
        }
        qflow::Optimizer::optimize_scale(field.hierarchy, field.rho, field.flag_adaptive_scale);
        /* Upstream sets this *after* the scale solve whether or not it was asked
         * for: the position solve below reads it as "a scale field exists now",
         * which it does either way. */
        field.flag_adaptive_scale = 1;

        qflow::Optimizer::optimize_positions(field.hierarchy, field.flag_adaptive_scale);
        field.ComputePositionSingularities();

        if (!field.ComputeIndexMap()) {
            set_error(error, error_length,
                      "the quad layout could not be solved for this object");
            return false;
        }

        const std::size_t out_vertices = field.O_compact.size();
        out.positions.resize(out_vertices * 3);
        for (std::size_t vertex = 0; vertex < out_vertices; ++vertex) {
            const qflow::Vector3d point =
                field.O_compact[vertex] * field.normalize_scale + field.normalize_offset;
            out.positions[vertex * 3 + 0] = (float)point[0];
            out.positions[vertex * 3 + 1] = (float)point[1];
            out.positions[vertex * 3 + 2] = (float)point[2];
        }

        out.face_offsets.clear();
        out.face_offsets.push_back(0);
        out.corners.clear();
        std::vector<std::uint32_t> face;
        for (std::size_t index = 0; index < field.F_compact.size(); ++index) {
            face.clear();
            for (int corner = 0; corner < 4; ++corner) {
                const int vertex = field.F_compact[index][corner];
                if (vertex < 0 || (std::size_t)vertex >= out_vertices) {
                    face.clear();
                    break;
                }
                /* A repeated corner is a degenerate the solve left behind, not a
                 * corner. Same rule as the Instant Meshes path. */
                bool repeated = false;
                for (std::size_t placed = 0; placed < face.size(); ++placed) {
                    if (face[placed] == (std::uint32_t)vertex) {
                        repeated = true;
                        break;
                    }
                }
                if (!repeated) {
                    face.push_back((std::uint32_t)vertex);
                }
            }
            if (face.size() < 3) {
                continue;
            }
            out.corners.insert(out.corners.end(), face.begin(), face.end());
            out.face_offsets.push_back((std::uint32_t)out.corners.size());
        }

        if (out.face_offsets.size() <= 1) {
            set_error(error, error_length, "the quad solve produced no faces");
            return false;
        }
        return true;
    } catch (const std::exception &failure) {
        set_error(error, error_length, failure.what());
        return false;
    } catch (...) {
        set_error(error, error_length, "the quad solver failed for an unknown reason");
        return false;
    }
}
