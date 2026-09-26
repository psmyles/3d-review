//! The position-only mesh the rebuild runs over.
//!
//! A node reaches this operation as one submesh per material, each with its own
//! vertex array — and the seam between two materials is a hard vertex split,
//! since import gives every face corner its own vertex. The rebuild needs the
//! opposite: one connected surface, or it treats each material as a separate
//! object and leaves a hole along every seam.
//!
//! So the proxy concatenates the node's pieces and welds by **position bits**
//! across all of them. That is lossy in every attribute — which is exactly right
//! here, because no attribute crosses this boundary: the rebuild is told where
//! the surface is and nothing else, and [`super::project`] puts the attributes
//! back afterwards.

use std::collections::HashMap;

use glam::Vec3;
use review_model::Bounds;

use crate::submesh::Submesh;

/// One node's surface as the rebuild sees it.
pub(crate) struct Proxy {
    /// Three floats per vertex, welded across the node's materials.
    pub positions: Vec<f32>,
    pub indices: Vec<u32>,
    /// Triangles across the node's pieces *before* the weld and the degenerate
    /// filter — what a ratio budget reads against, so "50%" is measured against
    /// the Tris figure the stats card shows rather than against an internal
    /// count the user never sees.
    pub source_triangles: usize,
    /// Surface area in world units squared, for the area-weighted split of an
    /// absolute face budget.
    pub area: f32,
    pub bounds: Bounds,
}

impl Proxy {
    pub(crate) fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Bounding-sphere radius, which is what the projection range is a fraction
    /// of. Zero for an empty proxy.
    pub(crate) fn radius(&self) -> f32 {
        if self.bounds.is_empty() {
            0.0
        } else {
            self.bounds.radius()
        }
    }
}

pub(crate) fn build(pieces: &[&Submesh]) -> Proxy {
    let _z = crate::prof::zone!("Remesh Proxy");

    let mut slot_of: HashMap<[u32; 3], u32> = HashMap::new();
    let mut positions: Vec<f32> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut source_triangles = 0usize;
    let mut area = 0.0f64;
    let mut bounds = Bounds::EMPTY;

    for piece in pieces {
        source_triangles += piece.triangle_count();
        // Each piece's vertices renumbered into the shared, welded array once,
        // so a triangle lookup below is an index rather than a hash probe.
        let mut local: Vec<u32> = Vec::with_capacity(piece.vertices.len());
        for vertex in &piece.vertices {
            let key = position_key(vertex.position);
            let next = (positions.len() / 3) as u32;
            let slot = *slot_of.entry(key).or_insert_with(|| {
                positions.extend_from_slice(&[
                    vertex.position.x,
                    vertex.position.y,
                    vertex.position.z,
                ]);
                next
            });
            local.push(slot);
            bounds.include_point(vertex.position);
        }

        for triangle in piece.indices.as_chunks::<3>().0 {
            let corners = [
                local.get(triangle[0] as usize).copied(),
                local.get(triangle[1] as usize).copied(),
                local.get(triangle[2] as usize).copied(),
            ];
            let (Some(a), Some(b), Some(c)) = (corners[0], corners[1], corners[2]) else {
                continue;
            };
            // A triangle whose corners weld together has no surface for the
            // rebuild to follow and makes the adjacency non-manifold.
            if a == b || b == c || a == c {
                continue;
            }
            let positions_of = |slot: u32| {
                let base = slot as usize * 3;
                Vec3::new(positions[base], positions[base + 1], positions[base + 2])
            };
            let (pa, pb, pc) = (positions_of(a), positions_of(b), positions_of(c));
            let cross = (pb - pa).cross(pc - pa);
            let triangle_area = cross.length() * 0.5;
            // A zero or NaN area is no surface; spelled out rather than left to
            // a negated comparison, which reads as the opposite of what it does.
            if triangle_area.is_nan() || triangle_area <= 0.0 {
                continue;
            }
            area += triangle_area as f64;
            indices.extend_from_slice(&[a, b, c]);
        }
    }

    Proxy {
        positions,
        indices,
        source_triangles,
        area: area as f32,
        bounds,
    }
}

/// A position as its exact bit pattern. Two vertices merge only when their
/// coordinates are byte-for-byte equal — which is what a corner split produces,
/// so it rejoins every seam import made and nothing the artist authored.
fn position_key(position: Vec3) -> [u32; 3] {
    [
        position.x.to_bits(),
        position.y.to_bits(),
        position.z.to_bits(),
    ]
}
