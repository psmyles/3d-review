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

#include "remesh_density.h"

#include <algorithm>
#include <cmath>
#include <vector>
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

/// Put a curvature-driven spacing multiplier into the solver's own scale field.
///
/// QuadriFlow carries a per-vertex multiplier (`Hierarchy::mS`) that every later
/// stage honours — the position solve, the edge-difference subdivision and the
/// index map all read it — but nothing upstream ever fills it with anything
/// meaningful. Its data term is `rho`, and `Parametrizer::Initialize` leaves
/// `rho` at 1 with the `ComputeCurvature` call beside it commented out, so
/// upstream's "adaptive" path solves a conformal-smoothness system over a
/// uniform field and comes back uniform. That is why a flat slab and a tight
/// fillet used to come out at the same face size.
///
/// What goes in instead is [`review::density_field`], which the Instant Meshes
/// driver steers by too — so the two engines vary face size by one rule. The
/// solve upstream would have run is skipped rather than overwritten: it is a
/// *conformal* field, which asks where faces should stay square rather than
/// where they are needed, and running it first would only cost time.
///
/// Returns false when there is no field to apply, in which case the caller must
/// run the uniform path.
bool apply_density_field(qflow::Parametrizer &field, float strength) {
    qflow::Hierarchy &mRes = field.hierarchy;
    const Eigen::Index count = mRes.mV[0].cols();
    if (count == 0) {
        return false;
    }

    /* The shared field works in floats over flat arrays, and QuadriFlow works in
     * doubles over Eigen columns; this is the one copy between them. */
    std::vector<float> positions((std::size_t)count * 3);
    std::vector<float> normals((std::size_t)count * 3);
    std::vector<float> areas((std::size_t)count);
    for (Eigen::Index vertex = 0; vertex < count; ++vertex) {
        for (int axis = 0; axis < 3; ++axis) {
            positions[(std::size_t)vertex * 3 + axis] = (float)mRes.mV[0](axis, vertex);
            normals[(std::size_t)vertex * 3 + axis] = (float)mRes.mN[0](axis, vertex);
        }
        areas[(std::size_t)vertex] = (float)mRes.mA[0][vertex];
    }
    std::vector<std::uint32_t> indices((std::size_t)mRes.mF.cols() * 3);
    for (Eigen::Index face = 0; face < mRes.mF.cols(); ++face) {
        for (int corner = 0; corner < 3; ++corner) {
            indices[(std::size_t)face * 3 + corner] = (std::uint32_t)mRes.mF(corner, face);
        }
    }

    review::DensityInput request;
    request.positions = positions.data();
    request.normals = normals.data();
    request.areas = areas.data();
    request.vertex_count = (std::size_t)count;
    request.indices = indices.data();
    request.index_count = indices.size();

    std::vector<float> spacing;
    if (!review::density_field(request, strength, (float)mRes.mScale, spacing)) {
        return false;
    }

    Eigen::MatrixXd &S = mRes.mS[0];
    for (Eigen::Index vertex = 0; vertex < count; ++vertex) {
        S(0, vertex) = S(1, vertex) = (double)spacing[(std::size_t)vertex];
    }

    /* The tail of `Optimizer::optimize_scale`: every coarser level of the
     * hierarchy needs the field too, since the position solve starts there. */
    for (std::size_t level = 0; level + 1 < mRes.mS.size(); ++level) {
        const Eigen::MatrixXd &fine = mRes.mS[level];
        Eigen::MatrixXd &coarse = mRes.mS[level + 1];
        const Eigen::MatrixXi &toUpper = mRes.mToUpper[level];
        for (Eigen::Index vertex = 0; vertex < toUpper.cols(); ++vertex) {
            const Eigen::Vector2i upper = toUpper.col(vertex);
            qflow::Vector2d value = fine.col(upper[0]);
            if (upper[1] != -1) {
                value = 0.5 * (value + fine.col(upper[1]));
            }
            coarse.col(vertex) = value;
        }
    }
    return true;
}


} // namespace

bool review_quadriflow_solve(const rvo_quadriflow_request &request,
                             rvo_quadriflow_output &out, char *error,
                             std::size_t error_length) {
    try {
        qflow::Parametrizer field;
        field.flag_preserve_sharp = request.preserve_sharp ? 1 : 0;
        field.flag_preserve_boundary = request.preserve_boundary ? 1 : 0;
        field.flag_adaptive_scale = 1;
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

        /* `optimize_scale` with the flag clear fills the field with ones, which
         * is what every stage below expects when nothing varies. The adaptive
         * field then replaces it outright — see `apply_density_field`. */
        qflow::Optimizer::optimize_scale(field.hierarchy, field.rho, 0);
        const bool varies = apply_density_field(field, request.adaptive_strength);
        /* Upstream sets this *after* the scale solve whether or not it was asked
         * for: the position solve below reads it as "a scale field exists now",
         * which it does either way. */
        field.flag_adaptive_scale = 1;

        qflow::Optimizer::optimize_positions(field.hierarchy, field.flag_adaptive_scale);
        field.ComputePositionSingularities();

        /* `with_scale` defaults to 0, and passing it matters: the last stage of
         * the index map builds a target edge *vector* per output quad edge, and
         * with the flag clear it builds every one of them at the uniform
         * `mScale` — pulling the extracted quads back to one size and undoing
         * the field. Upstream never notices because upstream's scale field is
         * all ones (see `apply_density_field`). Measured at full strength on a
         * sculpted pedestal's slab, which is the shape this is for: face sizes
         * spread 1.5x with it clear and 7.2x with it set. */
        if (!field.ComputeIndexMap(varies ? 1 : 0)) {
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
