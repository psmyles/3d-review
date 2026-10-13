//! How a simplified mesh relates to the mesh it was made from, vertex by vertex:
//! whether its vertices are the source's own or new positions, and whether its
//! normals are the source's normals at those points or something else.
//!
//! The image comparison says *that* two meshes shade differently; this says
//! what a tool did to get its result — which is how one reverse-engineers
//! another simplifier's choices from its output.

use std::collections::HashMap;

use glam::Vec3;
use review_model::Vertex;

use crate::inputs::Subject;

/// Print the relationship between `subject`'s visible vertices and
/// `reference`'s.
pub fn report(reference: &Subject, subject: &Subject) {
    for (label, who) in [("reference", reference), ("subject", subject)] {
        let groups: Vec<String> = who
            .model
            .mesh_group_stats()
            .iter()
            .filter(|group| who.hidden.binary_search(&group.node).is_err())
            .map(|group| {
                let material = who
                    .model
                    .materials
                    .get(group.material as usize)
                    .map_or("-", |material| material.name.as_str());
                format!("{material}: {} tris", group.triangle_count)
            })
            .collect();
        println!("    {label} draw groups: {}", groups.join(", "));
    }
    let source = visible_vertices(reference);
    let target = visible_vertices(subject);
    if source.is_empty() || target.is_empty() {
        println!("    nothing visible to inspect");
        return;
    }
    let extent = reference
        .bounds()
        .map_or(1.0, |bounds| bounds.size().length().max(1e-6));

    // Every normal the reference carries at each exact position.
    let mut by_position: HashMap<[u32; 3], Vec<Vec3>> = HashMap::new();
    for vertex in &source {
        by_position
            .entry(key(vertex.position))
            .or_default()
            .push(vertex.normal);
    }
    let grid = Grid::new(&source, extent * 0.01);

    let mut exact = 0usize;
    let mut normal_kept = Vec::new();
    let mut moved_distance = Vec::new();
    let mut moved_normal_to_nearest = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for vertex in &target {
        // One sample per distinct (position, normal) corner pair.
        if !seen.insert((key(vertex.position), key(vertex.normal))) {
            continue;
        }
        if let Some(normals) = by_position.get(&key(vertex.position)) {
            exact += 1;
            normal_kept.push(min_angle(vertex.normal, normals));
        } else if let Some((distance, nearest)) = grid.nearest(vertex.position, &source) {
            moved_distance.push(distance / extent);
            moved_normal_to_nearest.push(angle(vertex.normal, nearest.normal));
        }
    }
    let total = seen.len();
    println!(
        "    corners: {total}  on a source position: {:.1}%  moved: {:.1}%",
        100.0 * exact as f32 / total as f32,
        100.0 * (total - exact) as f32 / total as f32
    );
    if !normal_kept.is_empty() {
        println!(
            "    kept positions - normal vs the source's there (deg): {}",
            distribution(&mut normal_kept)
        );
    }
    if !moved_distance.is_empty() {
        println!(
            "    moved positions - distance to nearest source vertex (x extent): {}",
            distribution(&mut moved_distance)
        );
        println!(
            "    moved positions - normal vs that vertex's (deg): {}",
            distribution(&mut moved_normal_to_nearest)
        );
    }
    let mut recomputed = recomputed_normal_angles(subject);
    println!(
        "    normal vs its own area-weighted face normals (deg): {}",
        distribution(&mut recomputed)
    );
}

/// The vertices of every visible triangle, one per corner.
fn visible_vertices(subject: &Subject) -> Vec<Vertex> {
    let model = &subject.model;
    let tagged = model.triangles.node.len() * 3 == model.indices.len();
    model
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .enumerate()
        .filter(|(triangle, _)| {
            !tagged
                || subject
                    .hidden
                    .binary_search(&model.triangles.node[*triangle])
                    .is_err()
        })
        .flat_map(|(_, corners)| corners.iter().map(|&index| model.vertices[index as usize]))
        .collect()
}

/// For every visible corner, the angle between its normal and the
/// area-weighted average of the face normals at its position — what a plain
/// smooth "recalculate normals" would have given it.
fn recomputed_normal_angles(subject: &Subject) -> Vec<f32> {
    let corners = visible_vertices(subject);
    let mut sums: HashMap<[u32; 3], Vec3> = HashMap::new();
    for triangle in corners.as_chunks::<3>().0.iter() {
        let face = (triangle[1].position - triangle[0].position)
            .cross(triangle[2].position - triangle[0].position);
        for corner in triangle {
            *sums.entry(key(corner.position)).or_default() += face;
        }
    }
    corners
        .iter()
        .map(|corner| angle(corner.normal, sums[&key(corner.position)]))
        .collect()
}

fn key(v: Vec3) -> [u32; 3] {
    let canonical = |x: f32| {
        if x == 0.0 {
            0.0f32.to_bits()
        } else {
            x.to_bits()
        }
    };
    [canonical(v.x), canonical(v.y), canonical(v.z)]
}

fn angle(a: Vec3, b: Vec3) -> f32 {
    a.normalize_or_zero()
        .dot(b.normalize_or_zero())
        .clamp(-1.0, 1.0)
        .acos()
        .to_degrees()
}

fn min_angle(normal: Vec3, candidates: &[Vec3]) -> f32 {
    candidates
        .iter()
        .map(|&candidate| angle(normal, candidate))
        .fold(f32::INFINITY, f32::min)
}

fn distribution(values: &mut [f32]) -> String {
    values.sort_by(f32::total_cmp);
    let at = |q: f32| values[((values.len() - 1) as f32 * q) as usize];
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    format!(
        "n {}  mean {mean:.4}  median {:.4}  p90 {:.4}  p99 {:.4}  max {:.4}",
        values.len(),
        at(0.5),
        at(0.9),
        at(0.99),
        at(1.0)
    )
}

/// A uniform grid over the reference's vertices, for nearest-vertex queries.
struct Grid {
    cell: f32,
    cells: HashMap<[i32; 3], Vec<u32>>,
}

impl Grid {
    fn new(vertices: &[Vertex], cell: f32) -> Self {
        let mut cells: HashMap<[i32; 3], Vec<u32>> = HashMap::new();
        for (index, vertex) in vertices.iter().enumerate() {
            cells
                .entry(Self::cell_of(vertex.position, cell))
                .or_default()
                .push(index as u32);
        }
        Self { cell, cells }
    }

    fn cell_of(position: Vec3, cell: f32) -> [i32; 3] {
        let c = (position / cell).floor();
        [c.x as i32, c.y as i32, c.z as i32]
    }

    /// The nearest vertex within a few cells, with its distance.
    fn nearest<'a>(&self, position: Vec3, vertices: &'a [Vertex]) -> Option<(f32, &'a Vertex)> {
        let center = Self::cell_of(position, self.cell);
        let mut best: Option<(f32, &Vertex)> = None;
        for radius in 0..=3i32 {
            for dx in -radius..=radius {
                for dy in -radius..=radius {
                    for dz in -radius..=radius {
                        if dx.abs().max(dy.abs()).max(dz.abs()) != radius {
                            continue;
                        }
                        let Some(list) =
                            self.cells
                                .get(&[center[0] + dx, center[1] + dy, center[2] + dz])
                        else {
                            continue;
                        };
                        for &index in list {
                            let vertex = &vertices[index as usize];
                            let distance = vertex.position.distance(position);
                            if best.is_none_or(|(d, _)| distance < d) {
                                best = Some((distance, vertex));
                            }
                        }
                    }
                }
            }
            if best.is_some_and(|(d, _)| d <= radius as f32 * self.cell) {
                break;
            }
        }
        best
    }
}
