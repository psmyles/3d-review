//! Partitioning a [`ModelData`] into independently-optimizable submeshes, and
//! the buffer surgery every operation shares.
//!
//! ## Why (node, material) is the unit
//!
//! Import produces one flat triangle soup for the whole scene, with each
//! triangle tagged by its owning node and material slot. meshoptimizer works on
//! a single indexed mesh and — crucially — its simplifier returns a *new* index
//! buffer with no correspondence back to the input triangles. There is
//! therefore no way to carry a per-triangle material tag through a simplify
//! pass. Splitting on `(node, material)` gives every output triangle an
//! unambiguous tag by construction, and has two further benefits: geometry
//! never collapses across an object boundary (two props sitting next to each
//! other stay two props), and it matches the renderer's own draw grouping.
//!
//! The cost is that a collapse can never merge two materials' geometry along
//! their shared seam. That seam is already a hard split in the source data —
//! import gives each material's corners their own vertices — so little is lost.
//!
//! ## What rides along
//!
//! Beyond the vertices and the triangle list, a submesh carries what the source
//! authored *about* them and the export has to write back: extra vertex-color
//! sets and vertex creases (per vertex, remapped with the vertices), and the
//! polygon topology — the faces the triangles were cut from, the edge list and
//! the per-face / per-edge layers — as a [`PolygonCarry`]. The carry follows
//! four rules, one per class of operation:
//!
//! * a **vertex remap** (weld, vertex fetch, compaction) renumbers its corners
//!   and edge endpoints, dropping a face that collapses below three distinct
//!   vertices;
//! * an **index-buffer rewrite that keeps triangles whole** (filter, prune, the
//!   cache and overdraw reorders) is reconciled by content — every triangle is
//!   found again in the new buffer, a face survives only when all its triangles
//!   did ([`Submesh::reconcile_triangles`]);
//! * a **simplify** produces triangles with no correspondence to the input, so
//!   the carry is cleared ([`Submesh::clear_polygons`]) and that piece goes out
//!   as triangles;
//! * a **rebuild** ([`crate::remesh`]) replaces the surface outright and writes
//!   a carry of its own, flagged [`PolygonCarry::rebuilt`]. That is the one kind
//!   the *renderer* can be shown — the others describe corner runs in a vertex
//!   array welding has destroyed — so it is what puts real quads on screen and
//!   in the exported file. The first three rules still apply to it afterwards: a
//!   Weld below a Remesh renumbers its corners like any other carry's.//!
//! ## Layout
//!
//! [`mesh`] is the `Submesh` itself, [`rows`] the per-vertex rows a remap must
//! follow, [`polygons`] the carried faces and edges, and [`partition`] the split
//! that produces them.

mod mesh;
mod partition;
mod polygons;
mod rows;

pub use mesh::*;
pub use partition::*;
pub use polygons::*;
pub use rows::*;

/// The no-material sentinel `ModelData::triangles.material` uses.
pub const NO_MATERIAL: u32 = u32::MAX;

/// A triangle that belongs to no carried face.
pub const NO_FACE: u32 = u32::MAX;

/// Which per-triangle tag arrays the source model carried. Reassembly mirrors
/// this rather than always emitting both: a model with no node hierarchy should
/// come out of processing still having none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TagPresence {
    pub node: bool,
    pub material: bool,
}

#[cfg(test)]
mod tests {
    use glam::Vec4;
    use review_model::Vertex;

    use super::*;
    use review_model::demo_cube_model;

    #[test]
    fn cube_partitions_into_one_submesh_with_all_corners() {
        let model = demo_cube_model();
        let (submeshes, _tags) = partition(&model, None);

        assert_eq!(
            submeshes.len(),
            1,
            "the demo cube is one node, one material"
        );
        let cube = &submeshes[0];
        assert_eq!(cube.triangle_count(), model.indices.len() / 3);
        assert_eq!(
            cube.vertices.len(),
            model.vertices.len(),
            "every source vertex is referenced, so none is dropped"
        );
    }

    #[test]
    fn positions_stream_is_tightly_packed() {
        let model = demo_cube_model();
        let (submeshes, _) = partition(&model, None);
        let positions = submeshes[0].positions();

        assert_eq!(positions.len(), submeshes[0].vertices.len() * 3);
        assert_eq!(positions[0], submeshes[0].vertices[0].position.x);
        assert_eq!(positions[2], submeshes[0].vertices[0].position.z);
    }

    #[test]
    fn compaction_drops_vertices_nothing_references() {
        let model = demo_cube_model();
        let (mut submeshes, _) = partition(&model, None);
        let cube = &mut submeshes[0];

        // Keep only the first triangle; the other 22 vertices become orphans.
        cube.indices.truncate(3);
        let kept: Vec<_> = cube
            .indices
            .iter()
            .map(|&i| cube.vertices[i as usize])
            .collect();
        cube.compact_unreferenced();

        assert_eq!(cube.vertices.len(), 3);
        assert_eq!(cube.indices, vec![0, 1, 2]);
        assert_eq!(cube.vertices, kept, "surviving vertices keep their data");
    }

    /// A quad cut into two triangles: the carry names the face, and survives a
    /// weld, a reorder and a compaction — but not the loss of one triangle.
    fn quad() -> Submesh {
        let vertex = |x: f32, y: f32| Vertex {
            position: glam::Vec3::new(x, y, 0.0),
            ..Vertex::default()
        };
        Submesh {
            node: 0,
            material: NO_MATERIAL,
            vertices: vec![
                vertex(0.0, 0.0),
                vertex(1.0, 0.0),
                vertex(1.0, 1.0),
                vertex(0.0, 1.0),
                // A duplicate of vertex 2, as a corner-split source would have.
                vertex(1.0, 1.0),
            ],
            uv_channels: Vec::new(),
            color_channels: vec![vec![Vec4::splat(0.5); 5]],
            vertex_crease: vec![0.0, 0.1, 0.2, 0.3, 0.2],
            indices: vec![0, 1, 2, 0, 4, 3],
            skin: VertexRows::default(),
            extra_skins: Vec::new(),
            dq_weight: VertexRows::default(),
            morph: VertexRows::default(),
            source_corner: Vec::new(),
            polygons: Some(PolygonCarry {
                face_offsets: vec![0, 4],
                corners: vec![0, 1, 2, 3],
                source_face: vec![7],
                triangle_face: vec![0, 0],
                face_smoothing: vec![true],
                face_hole: Vec::new(),
                face_group: vec![3],
                edges: vec![[0, 1], [1, 2], [2, 3], [3, 0]],
                source_edge: vec![0, 1, 2, 3],
                edge_smoothing: Vec::new(),
                edge_crease: vec![0.0, 0.5, 0.0, 0.5],
                edge_visibility: Vec::new(),
                rebuilt: false,
            }),
        }
    }

    #[test]
    fn a_vertex_remap_renumbers_the_carry_and_gathers_per_vertex_data() {
        let mut piece = quad();
        // Weld vertex 4 onto vertex 2.
        let remap = [0, 1, 2, 3, 2];
        piece.indices = vec![0, 1, 2, 0, 2, 3];
        piece.apply_vertex_remap(&remap, 4);

        assert_eq!(piece.vertices.len(), 4);
        assert_eq!(piece.vertex_crease, vec![0.0, 0.1, 0.2, 0.3]);
        assert_eq!(piece.color_channels[0].len(), 4);
        let polygons = piece.polygons.as_ref().unwrap();
        assert_eq!(polygons.corners, vec![0, 1, 2, 3]);
        assert_eq!(polygons.edges.len(), 4);
        assert_eq!(polygons.edge_crease, vec![0.0, 0.5, 0.0, 0.5]);
    }

    #[test]
    fn a_reorder_keeps_the_face_and_a_dropped_triangle_breaks_it() {
        let mut piece = quad();
        let before = piece.indices.clone();
        // The two triangles swapped, one of them rotated.
        piece.indices = vec![4, 3, 0, 0, 1, 2];
        piece.reconcile_triangles(&before);
        let polygons = piece.polygons.as_ref().unwrap();
        assert_eq!(polygons.face_count(), 1);
        assert_eq!(polygons.triangle_face, vec![0, 0]);

        let before = piece.indices.clone();
        piece.indices.truncate(3);
        piece.reconcile_triangles(&before);
        let polygons = piece.polygons.as_ref().unwrap();
        assert_eq!(
            polygons.face_count(),
            0,
            "a face missing a triangle is gone"
        );
        assert_eq!(polygons.triangle_face, vec![NO_FACE]);
        assert!(polygons.face_smoothing.is_empty() && polygons.face_group.is_empty());
    }

    #[test]
    fn an_invented_triangle_voids_the_carry() {
        let mut piece = quad();
        let before = piece.indices.clone();
        piece.indices = vec![0, 2, 1, 0, 4, 3];
        piece.reconcile_triangles(&before);
        assert!(
            piece.polygons.is_none(),
            "reversed winding is not the same triangle"
        );
    }
}

#[cfg(test)]
mod row_tests {
    use review_model::Vertex;

    use super::*;

    #[test]
    fn row_ids_name_equal_rows_and_survive_a_gather() {
        let mut rows = VertexRows::default();
        rows.push_row(&[(0u32, 1.0f32)]);
        rows.push_row(&[(1, 0.5), (2, 0.5)]);
        rows.push_row(&[(0, 1.0)]);
        let piece = Submesh {
            node: 0,
            material: NO_MATERIAL,
            vertices: vec![Vertex::default(); 3],
            uv_channels: Vec::new(),
            color_channels: Vec::new(),
            vertex_crease: Vec::new(),
            indices: vec![0, 1, 2],
            polygons: None,
            skin: rows.clone(),
            extra_skins: Vec::new(),
            dq_weight: VertexRows::default(),
            morph: VertexRows::default(),
            source_corner: Vec::new(),
        };
        let ids = piece.row_ids();
        assert_eq!(ids[0], ids[2], "identical rows share an id");
        assert_ne!(ids[0], ids[1]);

        let gathered = rows.gather(&[2, 1]);
        assert_eq!(gathered.row(0), &[(0, 1.0)]);
        assert_eq!(gathered.row(1), &[(1, 0.5), (2, 0.5)]);
        assert_eq!(gathered.offsets, vec![0, 1, 3]);
    }
}
