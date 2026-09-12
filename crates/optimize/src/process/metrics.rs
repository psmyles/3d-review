//! Measuring a level: the buffer counts, and the figures meshoptimizer reports.
//!
//! The run's baseline is measured **after** the lossless index pass, not before
//! it. Quoting changes against the corner-split buffer credited the user's
//! operations with a reduction any engine cooker gets for free, and made the
//! baseline ACMR a meaningless 3.0.

use review_model::{ModelData, ModelStats};

use crate::Warnings;
use crate::meshopt::{self, AnalysisCounters};
use crate::submesh::{self, Submesh};

use super::*;

/// Measured figures for one level. Every field is a real measurement of the
/// mesh in the same struct — nothing here is estimated or carried over from the
/// source (invariant 5).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AnalysisMetrics {
    /// Average cache misses per triangle; lower is better (best case ~0.5).
    pub acmr: f32,
    /// Transformed vertices per vertex; 1.0 is ideal.
    pub atvr: f32,
    /// Shaded pixels / covered pixels; 1.0 means no overdraw.
    pub overdraw: f32,
    /// Fetched bytes / vertex buffer size; 1.0 means each byte is read once.
    pub overfetch: f32,
    /// The largest simplification error any submesh in this level reported, in
    /// the units the simplifier was asked for (a fraction of the mesh extent, or
    /// world units under the absolute-error flag). `0.0` for level 0 unless the
    /// stack reduced the mesh in place, whose error every level inherits.
    pub simplify_error: f32,
}

/// The size of a mesh as the GPU sees it: what an engine's vertex/index buffers
/// would actually hold.
///
/// Not the same thing as [`ModelStats::vertex_count`] for the *source*, which
/// carries the file's own DCC count (invariant 5), and not the corner-split
/// buffer either — see the field docs on [`ProcessedResult::source`]. A run
/// measures the source and each level the same way, so the overlay's deltas
/// subtract like from like.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MeshCounts {
    pub triangles: usize,
    pub vertices: usize,
}

impl MeshCounts {
    pub(crate) fn of_submeshes(submeshes: &[Submesh]) -> Self {
        Self {
            triangles: submeshes.iter().map(Submesh::triangle_count).sum(),
            vertices: submeshes.iter().map(|piece| piece.vertices.len()).sum(),
        }
    }
}

/// `"Asset"` for level 0, `"Asset_LOD1"` and up for the rest — the same naming
/// the suffixed-siblings export uses, so what the viewport labels matches what
/// lands on disk.
pub(crate) fn level_name(source_name: &str, level: usize) -> String {
    if level == 0 {
        source_name.to_owned()
    } else {
        format!("{source_name}_LOD{level}")
    }
}

/// Stats measured off the processed mesh itself.
///
/// Two figures deliberately differ in meaning from their source counterparts:
/// `polygon_count` counts the source faces the stack preserved plus every
/// triangle no face survived for (a simplified piece is all triangles), and
/// `vertex_count` is the real length of the vertex buffer rather than the
/// source file's logical DCC count. Both are honest measurements of the mesh
/// in hand, which is what the overlay must show.
pub(crate) fn measured_stats(
    model: &ModelData,
    source: &ModelData,
    carry: &LevelCarry,
) -> ModelStats {
    let triangle_count = model.indices.len() / 3;
    ModelStats {
        polygon_count: carry.polygon_count(triangle_count),
        triangle_count,
        vertex_count: model.vertices.len(),
        gpu_vertex_count: model.count_gpu_vertices(),
        uv_set_count: source.stats.uv_set_count,
        material_count: model.materials.len(),
        draw_count: model.material_draw_count(),
        // The node graph is carried through unchanged, so its bone count still
        // describes this model — even though the skin binding itself is dropped.
        bone_count: source.stats.bone_count,
        clip_count: model.animations.len(),
        source_unit_meters: source.stats.source_unit_meters,
    }
}

/// Measure a finished level against the cache / overdraw / fetch models.
///
/// Submeshes are measured separately (they are what the GPU draws) and their raw
/// counters summed before the ratios are re-derived — averaging per-submesh
/// ratios would let a ten-triangle part outweigh a hundred-thousand-triangle one
/// and report a number the mesh never exhibits.
pub(crate) fn measure(
    model: &ModelData,
    render_vertex_size: usize,
    simplify_error: f32,
    warnings: &mut Warnings,
) -> AnalysisMetrics {
    let (submeshes, _) = submesh::partition(model, None);
    measure_submeshes(&submeshes, render_vertex_size, simplify_error, warnings)
}

/// [`measure`] over submeshes already in hand — the shape the pipeline holds
/// mid-run, so the baseline can be measured without assembling a `ModelData`.
///
/// A submesh that cannot be analyzed is left out of the totals and reported: the
/// figures would otherwise describe part of the mesh while being presented as
/// the whole of it (invariant 5).
pub(crate) fn measure_submeshes(
    submeshes: &[Submesh],
    render_vertex_size: usize,
    simplify_error: f32,
    warnings: &mut Warnings,
) -> AnalysisMetrics {
    let _z = crate::prof::zone!("Measure Level");

    let mut counters = AnalysisCounters::default();

    for piece in submeshes {
        if piece.is_empty() {
            continue;
        }
        let positions = piece.positions();
        match meshopt::analyze(
            &piece.indices,
            &positions,
            piece.vertices.len(),
            render_vertex_size,
        ) {
            Ok(measured) => counters.accumulate(measured),
            Err(error) => warnings.push(&format!(
                "Couldn't measure part of the mesh: {error}. The cache, overdraw \
                 and fetch figures cover only the parts that measured."
            )),
        }
    }

    AnalysisMetrics {
        acmr: counters.acmr(),
        atvr: counters.atvr(),
        overdraw: counters.overdraw(),
        overfetch: counters.overfetch(),
        simplify_error,
    }
}
