/*
 * See `remesh_density.h` for what this is and why it is its own file.
 *
 * The rule, in one line: a chord of length `h` across a surface of curvature
 * `k` misses it by about `k * h^2 / 8`, so spending the same error everywhere
 * means `h` proportional to `1 / sqrt(k)`. Everything below is that, plus the
 * three guards that stop it from being useless in practice — a floor under
 * "flat", a dilation so a one-ring-wide fillet survives, and a floor under how
 * fine a face may be asked to be.
 */

#include "remesh_density.h"

#include <algorithm>
#include <cmath>

namespace review {

namespace {

/// A surface counts as flat once its radius of curvature passes this many times
/// the object's own diagonal. Expressed against the object rather than against
/// a length, because it has to mean the same thing whether the engine handed us
/// a mesh in meters or one normalized into a unit box.
const double FLAT_RADII = 100.0;

/// How far the normals are averaged before curvature is read off them, as a
/// multiple of the target face size.
///
/// This is the whole difference between a field that helps and one that makes
/// the result worse. Curvature taken across one triangle answers "how bumpy is
/// this surface", and a sculpted or scanned asset is bumpy *everywhere* at that
/// scale — so a flat slab reads as detailed, the field asks for small faces
/// over all of it, and what comes back is worse than the even layout it
/// replaced. Measured on a sculpted pedestal: the slab's median face went from
/// 12mm to 7mm and the quad grid broke up into a triangle mess.
///
/// Averaging the normals first asks the question that actually matters: does
/// the surface *turn* over the distance one face spans. Bumps finer than a face
/// cancel — no face size can represent them, so nothing is lost by ignoring
/// them — while a fillet or a rim, which turns through most of a right angle
/// over its width, survives untouched.
const double NORMAL_RADII = 1.0;

/// A ceiling on the normal averaging, in passes. Only reached when the input is
/// far finer than the target face size, where the cost of the exact answer is
/// out of proportion to a field that is then clamped and smoothed anyway.
const int MAX_NORMAL_PASSES = 256;

/// Passes that spread the *dense* requirement outward before anything is
/// averaged. A fillet one ring wide is exactly the feature this is for, and
/// smoothing it against its flat neighbours is what would lose it.
const int DILATE_PASSES = 2;

/// Laplacian passes over the dilated field, to take the last of the noise out.
/// A scale field that is not smooth is one the position solve fights rather
/// than follows.
const int SMOOTH_PASSES = 6;

/// How far either side of the object's own average face size the field may
/// reach at full strength, as a ratio. Four means the largest face is sixteen
/// times the *area* of the smallest. Past that the layout starts to come apart:
/// the quad grid has to resolve the transition somewhere, and an integer layout
/// resolves a steep one with singularities rather than with a gradient.
const double MAX_RANGE = 4.0;

/// How much finer than the triangles under it a face may be asked to be — or
/// rather, how much it may not. The extraction walks the input mesh; it cannot
/// cut an edge finer than the mesh it is walking, so a field that asks for it
/// does not get those faces, it just loses the ones the flat side gave up. This
/// is the single biggest reason a strong field can come back with fewer faces
/// than a uniform one.
const double SUPPORT = 1.2;

/// Vertex neighbours, as one flat array with a start index per vertex.
struct Adjacency {
    std::vector<std::uint32_t> neighbours;
    std::vector<std::uint32_t> start;
};

Adjacency build_adjacency(const DensityInput &input) {
    const std::size_t count = input.vertex_count;
    const std::size_t triangles = input.index_count / 3;
    Adjacency adjacency;
    adjacency.start.assign(count + 1, 0);

    /* Counting pass, then a fill pass, so the neighbour array is allocated once
     * rather than grown per vertex. Duplicates are left in: a neighbour reached
     * from two triangles is simply weighted twice, and every use below is an
     * average or a minimum, where that changes nothing. */
    for (std::size_t triangle = 0; triangle < triangles; ++triangle) {
        for (int corner = 0; corner < 3; ++corner) {
            const std::uint32_t i = input.indices[triangle * 3 + corner];
            const std::uint32_t j = input.indices[triangle * 3 + (corner + 1) % 3];
            if (i < count && j < count) {
                adjacency.start[i + 1] += 1;
                adjacency.start[j + 1] += 1;
            }
        }
    }
    for (std::size_t vertex = 0; vertex < count; ++vertex) {
        adjacency.start[vertex + 1] += adjacency.start[vertex];
    }
    adjacency.neighbours.assign(adjacency.start[count], 0);

    std::vector<std::uint32_t> cursor(adjacency.start.begin(), adjacency.start.end() - 1);
    for (std::size_t triangle = 0; triangle < triangles; ++triangle) {
        for (int corner = 0; corner < 3; ++corner) {
            const std::uint32_t i = input.indices[triangle * 3 + corner];
            const std::uint32_t j = input.indices[triangle * 3 + (corner + 1) % 3];
            if (i < count && j < count) {
                adjacency.neighbours[cursor[i]++] = j;
                adjacency.neighbours[cursor[j]++] = i;
            }
        }
    }
    return adjacency;
}

double diagonal_of(const DensityInput &input) {
    double lo[3] = {1.0e30, 1.0e30, 1.0e30};
    double hi[3] = {-1.0e30, -1.0e30, -1.0e30};
    for (std::size_t vertex = 0; vertex < input.vertex_count; ++vertex) {
        for (int axis = 0; axis < 3; ++axis) {
            const double value = input.positions[vertex * 3 + axis];
            lo[axis] = std::fmin(lo[axis], value);
            hi[axis] = std::fmax(hi[axis], value);
        }
    }
    double sum = 0.0;
    for (int axis = 0; axis < 3; ++axis) {
        const double extent = hi[axis] - lo[axis];
        sum += extent * extent;
    }
    return std::sqrt(sum);
}

} // namespace

bool density_field(const DensityInput &input, float strength, float target_edge,
                   std::vector<float> &out) {
    if (!(strength > 0.0f) || input.positions == nullptr || input.normals == nullptr ||
        input.indices == nullptr || input.vertex_count == 0 || input.index_count < 3 ||
        !(target_edge > 0.0f)) {
        return false;
    }
    const std::size_t count = input.vertex_count;
    const std::size_t triangles = input.index_count / 3;

    const double diagonal = diagonal_of(input);
    if (!(diagonal > 0.0)) {
        return false;
    }
    const double flat = 1.0 / (FLAT_RADII * diagonal);

    const Adjacency adjacency = build_adjacency(input);

    /* Every vertex's incident edge lengths, which two later steps want: the
     * averaging radius below is set against them, and so is the floor on how
     * fine a face may be asked to be. */
    std::vector<double> span(count, 0.0);
    std::vector<int> degree(count, 0);
    double total_length = 0.0;
    for (std::size_t triangle = 0; triangle < triangles; ++triangle) {
        for (int corner = 0; corner < 3; ++corner) {
            const std::uint32_t i = input.indices[triangle * 3 + corner];
            const std::uint32_t j = input.indices[triangle * 3 + (corner + 1) % 3];
            if (i >= count || j >= count) {
                continue;
            }
            double delta = 0.0;
            for (int axis = 0; axis < 3; ++axis) {
                const double d = (double)input.positions[i * 3 + axis] -
                                 (double)input.positions[j * 3 + axis];
                delta += d * d;
            }
            const double length = std::sqrt(delta);
            if (!(length > 0.0)) {
                continue;
            }
            span[i] += length;
            span[j] += length;
            degree[i] += 1;
            degree[j] += 1;
            total_length += length;
        }
    }
    const double mean_edge = total_length > 0.0 ? total_length / (double)(3 * triangles) : 0.0;
    if (!(mean_edge > 0.0)) {
        return false;
    }

    /* Average the normals out to the radius a face will span (see
     * NORMAL_RADII). One Laplacian pass diffuses about half an edge length, so
     * reaching a radius takes its square in passes. */
    const double radius = NORMAL_RADII * (double)target_edge;
    const int normal_passes =
        std::min(MAX_NORMAL_PASSES, (int)std::ceil(4.0 * radius * radius / (mean_edge * mean_edge)));
    std::vector<double> normals((std::size_t)count * 3);
    for (std::size_t vertex = 0; vertex < count * 3; ++vertex) {
        normals[vertex] = (double)input.normals[vertex];
    }
    {
        std::vector<double> next(normals.size(), 0.0);
        for (int pass = 0; pass < normal_passes; ++pass) {
            for (std::size_t vertex = 0; vertex < count; ++vertex) {
                double sum[3] = {normals[vertex * 3], normals[vertex * 3 + 1],
                                 normals[vertex * 3 + 2]};
                for (std::uint32_t at = adjacency.start[vertex]; at < adjacency.start[vertex + 1];
                     ++at) {
                    const std::uint32_t other = adjacency.neighbours[at];
                    for (int axis = 0; axis < 3; ++axis) {
                        sum[axis] += normals[(std::size_t)other * 3 + axis];
                    }
                }
                /* Renormalized rather than averaged: what is wanted is the
                 * direction the surface faces around here, and two opposed
                 * normals a fold apart must not average to a shorter vector
                 * that then reads as low curvature. */
                const double length =
                    std::sqrt(sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]);
                for (int axis = 0; axis < 3; ++axis) {
                    next[vertex * 3 + axis] = length > 0.0 ? sum[axis] / length
                                                           : normals[vertex * 3 + axis];
                }
            }
            normals.swap(next);
        }
    }

    /* Curvature, as the angle the surface turns through per unit length: every
     * incident edge contributes the angle between the two averaged normals
     * across it, against its own length. */
    std::vector<double> turn(count, 0.0);
    for (std::size_t triangle = 0; triangle < triangles; ++triangle) {
        for (int corner = 0; corner < 3; ++corner) {
            const std::uint32_t i = input.indices[triangle * 3 + corner];
            const std::uint32_t j = input.indices[triangle * 3 + (corner + 1) % 3];
            if (i >= count || j >= count) {
                continue;
            }
            double dot = 0.0;
            for (int axis = 0; axis < 3; ++axis) {
                dot += normals[(std::size_t)i * 3 + axis] * normals[(std::size_t)j * 3 + axis];
            }
            const double angle = std::acos(std::fmin(std::fmax(dot, -1.0), 1.0));
            turn[i] += angle;
            turn[j] += angle;
        }
    }

    /* In logs throughout: the smoothing, the averaging and the strength blend
     * are all multiplicative on a spacing, and a scale field is a ratio rather
     * than a difference. The additive constant drops out at the centring step,
     * so only the shape of the field matters here. */
    std::vector<double> spacing(count, 0.0);
    for (std::size_t vertex = 0; vertex < count; ++vertex) {
        const double curvature = span[vertex] > 0.0 ? turn[vertex] / span[vertex] : 0.0;
        spacing[vertex] = -0.5 * std::log(curvature + flat);
    }

    std::vector<double> scratch(count, 0.0);
    for (int pass = 0; pass < DILATE_PASSES; ++pass) {
        for (std::size_t vertex = 0; vertex < count; ++vertex) {
            double smallest = spacing[vertex];
            for (std::uint32_t at = adjacency.start[vertex]; at < adjacency.start[vertex + 1];
                 ++at) {
                smallest = std::fmin(smallest, spacing[adjacency.neighbours[at]]);
            }
            scratch[vertex] = smallest;
        }
        spacing.swap(scratch);
    }
    for (int pass = 0; pass < SMOOTH_PASSES; ++pass) {
        for (std::size_t vertex = 0; vertex < count; ++vertex) {
            double sum = spacing[vertex];
            double weight = 1.0;
            for (std::uint32_t at = adjacency.start[vertex]; at < adjacency.start[vertex + 1];
                 ++at) {
                sum += spacing[adjacency.neighbours[at]];
                weight += 1.0;
            }
            scratch[vertex] = sum / weight;
        }
        spacing.swap(scratch);
    }

    double total_area = 0.0;
    double mean = 0.0;
    for (std::size_t vertex = 0; vertex < count; ++vertex) {
        const double area = input.areas != nullptr ? (double)input.areas[vertex] : 1.0;
        const double weight = area > 0.0 ? area : 0.0;
        total_area += weight;
        mean += weight * spacing[vertex];
    }
    if (!(total_area > 0.0)) {
        return false;
    }
    mean /= total_area;

    const double limit = std::log(MAX_RANGE);
    out.assign(count, 1.0f);
    std::vector<double> field(count, 1.0);
    for (std::size_t vertex = 0; vertex < count; ++vertex) {
        const double centred = (spacing[vertex] - mean) * (double)strength;
        field[vertex] = std::exp(std::fmin(std::fmax(centred, -limit), limit));
    }

    /* No face finer than the triangles under it (see SUPPORT). The floor takes
     * area out of the budget sum, so the normalization below is iterated rather
     * than solved: a handful of passes is enough for it to settle, and it is
     * monotone, so stopping early only leaves the field slightly coarse. */
    std::vector<double> floor_at(count, 0.0);
    for (std::size_t vertex = 0; vertex < count; ++vertex) {
        const double edge = degree[vertex] > 0 ? span[vertex] / degree[vertex] : 0.0;
        floor_at[vertex] = edge * SUPPORT / (double)target_edge;
    }

    for (int pass = 0; pass < 8; ++pass) {
        for (std::size_t vertex = 0; vertex < count; ++vertex) {
            field[vertex] = std::fmax(field[vertex], floor_at[vertex]);
        }
        /* A face covers `(target_edge * field)^2` of surface, so holding the
         * count means holding the area-weighted mean of `1 / field^2` at one. */
        double density = 0.0;
        for (std::size_t vertex = 0; vertex < count; ++vertex) {
            const double area = input.areas != nullptr ? (double)input.areas[vertex] : 1.0;
            const double weight = area > 0.0 ? area : 0.0;
            density += weight / (field[vertex] * field[vertex]);
        }
        const double correction = std::sqrt(density / total_area);
        if (!(correction > 0.0) || !std::isfinite(correction) ||
            std::fabs(correction - 1.0) < 1.0e-4) {
            break;
        }
        for (std::size_t vertex = 0; vertex < count; ++vertex) {
            field[vertex] *= correction;
        }
    }

    bool varies = false;
    for (std::size_t vertex = 0; vertex < count; ++vertex) {
        if (!std::isfinite(field[vertex]) || !(field[vertex] > 0.0)) {
            return false;
        }
        out[vertex] = (float)field[vertex];
        if (std::fabs(field[vertex] - 1.0) > 1.0e-3) {
            varies = true;
        }
    }
    return varies;
}

} // namespace review
