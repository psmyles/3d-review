//! A cheap mesh from a partition, for showing the rebuild while it is still
//! running.
//!
//! The regions are Voronoi cells, and the dual of a Voronoi diagram is a
//! triangulation: wherever three cells meet, their three seeds make a triangle.
//! So an input face whose three corners carry three different labels names one
//! output triangle, and a single pass over the faces is the whole extraction.
//!
//! That is fast enough to run after every relaxation pass, which is what lets
//! the viewport show the rebuild converging instead of a progress bar.
//!
//! ## Why this is not the real extraction
//!
//! It is a *sample* of the answer, not the answer:
//!
//! * A region touching a border contributes nothing along it — the dual has no
//!   face where there is no third cell — so open edges come back short.
//! * Two cells meeting along a strip with no third cell between them leave a
//!   gap.
//! * Nothing checks that the result is manifold, and on a pinched or branching
//!   input it will not be.
//!
//! Every one of those is exactly what [`super::collapse`] exists to get right,
//! and it costs a great deal more. The trade is deliberate: this is what is
//! shown *during* a run, and the collapse is what the run produces.

use glam::Vec3;

use super::RemeshOutput;
use super::partition::{NO_REGION, Partition};
use super::seeds::Seeds;

/// Build the dual mesh of a partition.
///
/// Positions come from the seeds, so the result has exactly one vertex per
/// region that any face reached — which is the same vertex count the finished
/// rebuild will have, and why a preview reads as the real thing getting
/// steadily better rather than as a different mesh.
pub(crate) fn build(
    positions: &[f32],
    indices: &[u32],
    seeds: &Seeds,
    partition: &Partition,
) -> RemeshOutput {
    let _z = crate::prof::zone!("Remesh Preview");
    let mut output = RemeshOutput {
        positions: Vec::new(),
        face_offsets: vec![0],
        corners: Vec::new(),
    };
    if seeds.is_empty() {
        return output;
    }

    // Regions are numbered over every seed, but only the ones a face reaches
    // produce a vertex, so the output is renumbered over those.
    let mut slot_of = vec![u32::MAX; partition.regions];
    let mut seen: Vec<[u32; 3]> = Vec::new();
    for corners in indices.as_chunks::<3>().0 {
        let labels = [
            partition.label[corners[0] as usize],
            partition.label[corners[1] as usize],
            partition.label[corners[2] as usize],
        ];
        // Three distinct cells meeting is what makes a dual face; a face inside
        // one cell, or straddling two, is interior to the output rather than
        // part of it.
        if labels.contains(&NO_REGION)
            || labels[0] == labels[1]
            || labels[1] == labels[2]
            || labels[0] == labels[2]
        {
            continue;
        }
        seen.push(labels);
    }

    // Deduplicated by content: several input faces can meet the same three
    // cells, and each would otherwise emit the triangle again. Sorted on a
    // canonical key so the answer does not depend on which face reached it
    // first.
    let mut keys: Vec<([u32; 3], [u32; 3])> = seen
        .into_iter()
        .map(|labels| {
            let mut sorted = labels;
            sorted.sort_unstable();
            (sorted, labels)
        })
        .collect();
    keys.sort_unstable();
    keys.dedup_by(|a, b| a.0 == b.0);

    for (_, labels) in &keys {
        for &label in labels {
            if slot_of[label as usize] == u32::MAX {
                let base = seeds.vertices[label as usize] as usize * 3;
                slot_of[label as usize] = output.positions.len() as u32;
                output.positions.push(Vec3::new(
                    positions[base],
                    positions[base + 1],
                    positions[base + 2],
                ));
            }
            output.corners.push(slot_of[label as usize]);
        }
        output
            .face_offsets
            .push(output.corners.len().try_into().unwrap_or(u32::MAX));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remesh::surface::Surface;
    use crate::remesh::{features, partition, seeds, size_field, topology::Topology};

    fn grid(side: u32) -> (Vec<f32>, Vec<u32>) {
        let step = 1.0 / (side - 1) as f32;
        let mut positions = Vec::new();
        for row in 0..side {
            for column in 0..side {
                positions.extend_from_slice(&[column as f32 * step, row as f32 * step, 0.0]);
            }
        }
        let mut indices = Vec::new();
        for row in 0..side - 1 {
            for column in 0..side - 1 {
                let at = row * side + column;
                indices.extend_from_slice(&[at, at + 1, at + side]);
                indices.extend_from_slice(&[at + 1, at + side + 1, at + side]);
            }
        }
        (positions, indices)
    }

    fn preview_of(side: u32, faces: u32) -> (RemeshOutput, Seeds) {
        let (positions, indices) = grid(side);
        let topology = Topology::build(&indices, positions.len() / 3, 1, None);
        let sizes = vec![1.0 / side as f32; topology.vertex_count];
        let areas = size_field::dual_areas(&positions, &indices, &topology, 1);
        let surface = Surface {
            positions: &positions,
            indices: &indices,
            topology: &topology,
            sizes: &sizes,
            areas: &areas,
        };
        let features = features::build(surface, true, None, None);
        let seeds = seeds::place(surface, &features, faces, None);
        let partition = partition::build(surface, &features, &seeds, 1, None);
        (build(&positions, &indices, &seeds, &partition), seeds)
    }

    #[test]
    fn the_dual_of_a_grid_is_a_triangle_mesh() {
        let (output, _) = preview_of(24, 300);

        assert!(output.validate().is_ok(), "the dual is a well-formed soup");
        assert!(!output.corners.is_empty(), "a grid has cells that meet");
        for face in 0..output.face_count() {
            let start = output.face_offsets[face] as usize;
            let end = output.face_offsets[face + 1] as usize;
            assert_eq!(end - start, 3, "every dual face is a triangle");
        }
    }

    #[test]
    fn every_dual_vertex_sits_on_a_seed() {
        let (output, seeds) = preview_of(20, 200);

        let (positions, _) = grid(20);
        let seeded: Vec<Vec3> = seeds
            .vertices
            .iter()
            .map(|&vertex| {
                let base = vertex as usize * 3;
                Vec3::new(positions[base], positions[base + 1], positions[base + 2])
            })
            .collect();
        for point in &output.positions {
            assert!(seeded.contains(point), "a dual vertex that is not a seed");
        }
    }

    #[test]
    fn a_triangle_is_emitted_once_however_many_faces_meet_there() {
        let (output, _) = preview_of(16, 120);

        let mut faces: Vec<[u32; 3]> = (0..output.face_count())
            .map(|face| {
                let start = output.face_offsets[face] as usize;
                let mut corners = [
                    output.corners[start],
                    output.corners[start + 1],
                    output.corners[start + 2],
                ];
                corners.sort_unstable();
                corners
            })
            .collect();
        let before = faces.len();
        faces.sort_unstable();
        faces.dedup();
        assert_eq!(before, faces.len(), "no dual triangle is emitted twice");
    }

    #[test]
    fn an_empty_partition_previews_nothing() {
        let output = build(&[], &[], &Seeds::default(), &Partition::default());

        assert!(output.positions.is_empty());
        assert_eq!(output.face_count(), 0);
    }
}
