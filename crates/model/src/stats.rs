//! What the Model Stats panel reads.
//!
//! Every figure here is a **measured** value (invariant 5). The source DCC
//! counts are carried through import in [`ModelStats`]; the engine-cost figures
//! are measured off the buffers. The viewer's corner-split vertex buffer is an
//! internal layout, not a stat, and is surfaced nowhere.

use crate::*;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ModelStats {
    pub polygon_count: usize,
    pub triangle_count: usize,
    /// The source file's own logical vertex count (control points), as the
    /// artist's DCC reports it.
    pub vertex_count: usize,
    /// What the mesh costs on the GPU: unique vertices per draw group, as
    /// measured by [`ModelData::count_gpu_vertices`] — the buffer any engine
    /// importer's lossless indexing would build. Neither the DCC count above nor
    /// this viewer's internal corner-split buffer, which is an implementation
    /// detail and reported nowhere.
    pub gpu_vertex_count: usize,
    pub uv_set_count: usize,
    pub material_count: usize,
    pub draw_count: usize,
    /// Number of [`NodeKind::Bone`] nodes in the scene graph — a measured count
    /// (invariant 5), shown in the stats panel only when non-zero.
    pub bone_count: usize,
    /// Number of animation clips (FBX animation stacks) the file carries — a
    /// measured count (invariant 5), shown in the stats panel only when non-zero.
    pub clip_count: usize,
    /// The model's authored world unit, in meters per source unit, as recorded
    /// in the file (e.g. `0.01` for a centimeter file like a Maya export). This
    /// is the *original* unit before import normalizes everything to meters, so
    /// the stats panel can show what the file claimed. `0.0` means the source
    /// declared no unit (e.g. the built-in demo, or a file missing the metadata).
    pub source_unit_meters: f32,
}

/// Measured counts for one draw group — the triangles sharing a
/// (scene node, material slot) pair, which is the finest unit the renderer
/// actually draws and the unit every scope the stats overlay reports is a union
/// of.
///
/// Built once per model by [`ModelData::mesh_group_stats`] so the overlay's
/// per-scope columns are sums of measured values, not a per-frame rescan: the
/// GPU-vertex measurement alone is an O(corners) hash walk that must never run
/// on the redraw path (invariant 6).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MeshGroupStats {
    /// Index into [`ModelData::nodes`] (`0` for a model with no node tags).
    pub node: u32,
    /// Index into [`ModelData::materials`], or `u32::MAX` for untagged triangles
    /// — the same sentinel [`TriangleData::material`] uses.
    pub material: u32,
    /// Source polygons whose triangles land in this group.
    pub polygon_count: usize,
    pub triangle_count: usize,
    /// This group's share of [`ModelStats::gpu_vertex_count`]. Groups never share
    /// a vertex (the count keys on the group), so the shares sum to the total.
    pub gpu_vertex_count: usize,
}

/// Which slice of the model a [`ScopeStats`] covers.
///
/// There is deliberately no "whole model" variant: the file's own figures are
/// what [`ModelStats`] already carries, straight from import, and re-deriving
/// them here would be a second answer to a question that has one (invariant 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatsScope<'a> {
    /// The nodes marked in a per-node inclusion mask (indexed by node index; a
    /// mask shorter than the node table excludes the rest).
    Nodes(&'a [bool]),
    /// Every triangle of one material slot, whichever node carries it.
    Material(u32),
}

/// A scope's measured totals, summed from the [`MeshGroupStats`] table.
///
/// Every field is a real measured count (invariant 5) — the sums of counts taken
/// off the mesh itself, never a proportional share of a whole-model figure.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScopeStats {
    pub polygon_count: usize,
    pub triangle_count: usize,
    /// The DCC control-point count, `None` when the scope has no honest one: a
    /// material slot is not a set of meshes, and the count is authored per mesh
    /// (it is likewise `None` for a model whose nodes carry no
    /// [`SceneNode::source_vertex_count`]).
    pub vertex_count: Option<usize>,
    pub gpu_vertex_count: usize,
    /// Distinct material slots in the scope — one draw call each, matching
    /// [`ModelData::material_draw_count`]'s grouping.
    pub draw_count: usize,
}

/// The measured figures the stats panel reads (invariant 5).
impl ModelData {
    /// Number of per-material draw ranges the renderer issues for this mesh —
    /// one per distinct [`TriangleData::material`] slot, in first-seen order.
    /// Mirrors the grouping in the renderer's `model_mesh`, so the Draws stat
    /// reflects the real draw-call count (invariant 5). `0` when the mesh has no
    /// triangles; `1` when triangles exist but carry no per-triangle material.
    pub fn material_draw_count(&self) -> usize {
        let triangle_count = self.indices.len() / 3;
        if triangle_count == 0 {
            return 0;
        }
        if self.triangles.material.len() != triangle_count {
            return 1;
        }
        let mut seen: Vec<u32> = Vec::new();
        for &slot in &self.triangles.material {
            if !seen.contains(&slot) {
                seen.push(slot);
            }
        }
        seen.len()
    }

    /// The number of vertices an engine's GPU vertex buffer would hold for this
    /// mesh: unique attribute tuples per draw group, counted over the referenced
    /// vertices.
    ///
    /// Import expands every face corner into its own vertex (FBX indexes
    /// normals/UVs per corner, and the polygon-topology views need the corner-run
    /// layout), so `vertices.len()` describes this viewer's internal buffer, not
    /// the asset. What the asset actually *costs* is what any engine importer's
    /// lossless indexing produces: one vertex per distinct
    /// (position, normal, UVs, color) tuple, per (node, material) draw group —
    /// duplicates across groups stay separate, exactly as separate draws keep
    /// separate buffers. This is the figure the stats panel reports as the GPU
    /// vertex count, and it matches the Opt workspace's post-index baseline.
    ///
    /// Equality is bit-exact after folding `-0.0` to `+0.0` and every NaN to one
    /// pattern — the same canonical form the Opt weld uses, so the two counts
    /// can never disagree. The tangent is deliberately excluded: it is derived
    /// from position/normal/UV, so identical inputs carry identical tangents and
    /// including it would only let floating-point noise split a vertex.
    pub fn count_gpu_vertices(&self) -> usize {
        self.mesh_group_stats()
            .iter()
            .map(|group| group.gpu_vertex_count)
            .sum()
    }

    /// Measure every draw group in the mesh: one [`MeshGroupStats`] per
    /// (node, material) pair the triangles carry, in first-seen order.
    ///
    /// This is the one walk behind every count the stats overlay reports, whole
    /// model or scoped — [`count_gpu_vertices`] is its total. Run it once per
    /// model and sum the table with [`scope_stats`]: it is O(corners) with a hash
    /// per distinct vertex, far too heavy for the redraw path (invariant 6).
    ///
    /// Untagged models fold into a single group (node 0, material 0), matching
    /// the fallbacks [`material_draw_count`] and the renderer already take.
    ///
    /// [`count_gpu_vertices`]: ModelData::count_gpu_vertices
    /// [`scope_stats`]: ModelData::scope_stats
    /// [`material_draw_count`]: ModelData::material_draw_count
    pub fn mesh_group_stats(&self) -> Vec<MeshGroupStats> {
        use std::collections::{HashMap, HashSet};

        let triangle_count = self.indices.len() / 3;
        if triangle_count == 0 || self.vertices.is_empty() {
            return Vec::new();
        }
        let node_tags =
            (self.triangles.node.len() == triangle_count).then_some(self.triangles.node.as_slice());
        let material_tags = (self.triangles.material.len() == triangle_count)
            .then_some(self.triangles.material.as_slice());
        let face_tags = (self.triangles.to_face.len() == triangle_count)
            .then_some(self.triangles.to_face.as_slice());

        // Fold a component to the canonical bit pattern (`-0.0` -> `+0.0`, any
        // NaN -> the one `f32::NAN`), so equality is by value, not encoding.
        let canonical = |value: f32| -> [u8; 4] {
            let folded = if value.is_nan() {
                f32::NAN
            } else if value == 0.0 {
                0.0
            } else {
                value
            };
            folded.to_ne_bytes()
        };

        let mut groups: Vec<MeshGroupStats> = Vec::new();
        let mut group_of: HashMap<(u32, u32), usize> = HashMap::new();
        // A vertex index is keyed at most once per group; a group's set holds
        // the distinct attribute tuples among them.
        let mut seen: HashSet<(u32, u32, u32)> = HashSet::new();
        let mut unique: HashSet<(u32, u32, Vec<u8>)> = HashSet::new();
        // A source polygon's triangles all carry its own node and material, so a
        // face falls in exactly one group; this mask is what keeps a triangulated
        // n-gon from counting its polygon once per triangle.
        let mut counted_face = vec![false; self.faces.len()];

        for (triangle, corners) in self.indices.as_chunks::<3>().0.iter().enumerate() {
            let node = node_tags.map_or(0, |tags| tags[triangle]);
            let material = material_tags.map_or(0, |tags| tags[triangle]);
            let group_index = *group_of.entry((node, material)).or_insert_with(|| {
                groups.push(MeshGroupStats {
                    node,
                    material,
                    ..MeshGroupStats::default()
                });
                groups.len() - 1
            });
            let group = &mut groups[group_index];
            group.triangle_count += 1;
            match face_tags.and_then(|tags| counted_face.get_mut(tags[triangle] as usize)) {
                Some(counted) => {
                    if !*counted {
                        *counted = true;
                        group.polygon_count += 1;
                    }
                }
                // No topology to attribute the triangle to — a processed mesh is
                // pure triangles and carries none — so each triangle is its own
                // polygon, the same fallback every `faces` consumer takes.
                None => group.polygon_count += 1,
            }
            for &index in corners {
                let Some(vertex) = self.vertices.get(index as usize) else {
                    continue;
                };
                if !seen.insert((node, material, index)) {
                    continue;
                }
                let mut key = Vec::with_capacity((12 + self.uv_channels.len() * 2) * 4);
                for component in [vertex.position.x, vertex.position.y, vertex.position.z] {
                    key.extend_from_slice(&canonical(component));
                }
                for component in [vertex.normal.x, vertex.normal.y, vertex.normal.z] {
                    key.extend_from_slice(&canonical(component));
                }
                // Every UV set participates; a single-set model carries its UVs
                // only on the vertex itself.
                if self.uv_channels.is_empty() {
                    key.extend_from_slice(&canonical(vertex.uv.x));
                    key.extend_from_slice(&canonical(vertex.uv.y));
                } else {
                    for channel in &self.uv_channels {
                        let uv = channel.get(index as usize).copied().unwrap_or_default();
                        key.extend_from_slice(&canonical(uv.x));
                        key.extend_from_slice(&canonical(uv.y));
                    }
                }
                for component in [
                    vertex.vertex_color.x,
                    vertex.vertex_color.y,
                    vertex.vertex_color.z,
                    vertex.vertex_color.w,
                ] {
                    key.extend_from_slice(&canonical(component));
                }
                if unique.insert((node, material, key)) {
                    group.gpu_vertex_count += 1;
                }
            }
        }
        groups
    }

    /// A per-node inclusion mask for the subtree rooted at `root` (inclusive):
    /// a node is in it when walking its parent links reaches `root`.
    ///
    /// This is what "a node selection" means everywhere — the highlighted
    /// triangles, the selection bounding box, the scoped stats column — so the
    /// walk lives here rather than once per consumer. O(nodes x depth), and a
    /// guard bounds any malformed parent cycle.
    pub fn node_subtree_mask(&self, root: usize) -> Vec<bool> {
        let mut mask = vec![false; self.nodes.len()];
        if root >= self.nodes.len() {
            return mask;
        }
        let node_count = self.nodes.len();
        for (index, included) in mask.iter_mut().enumerate() {
            let mut current = Some(index);
            let mut guard = 0;
            while let Some(node) = current {
                if node == root {
                    *included = true;
                    break;
                }
                current = self.nodes.get(node).and_then(|node| node.parent);
                guard += 1;
                if guard > node_count {
                    break;
                }
            }
        }
        mask
    }

    /// Sum a [`mesh_group_stats`] table over the groups `scope` covers.
    ///
    /// The table is the expensive part and is built once per model; this is a
    /// walk over a handful of groups, so the stats overlay can re-scope it
    /// every time the selection or the hidden set changes.
    ///
    /// [`mesh_group_stats`]: ModelData::mesh_group_stats
    pub fn scope_stats(&self, groups: &[MeshGroupStats], scope: StatsScope<'_>) -> ScopeStats {
        let mut stats = ScopeStats::default();
        // One draw call per slot, so the count is small and a linear dedupe beats
        // standing a hash set up.
        let mut materials: Vec<u32> = Vec::new();

        for group in groups {
            let included = match scope {
                StatsScope::Nodes(mask) => mask.get(group.node as usize).copied().unwrap_or(false),
                StatsScope::Material(slot) => group.material == slot,
            };
            if !included {
                continue;
            }
            stats.polygon_count += group.polygon_count;
            stats.triangle_count += group.triangle_count;
            stats.gpu_vertex_count += group.gpu_vertex_count;
            if !materials.contains(&group.material) {
                materials.push(group.material);
            }
        }
        stats.draw_count = materials.len();

        // The DCC count is authored per mesh, so only a scope that *is* a set of
        // meshes has one. A model whose producer never recorded it reports none
        // at all rather than a zero that would read as "this selection holds no
        // vertices" (invariant 5).
        let tracked = self.nodes.iter().any(|node| node.source_vertex_count > 0);
        stats.vertex_count = match scope {
            StatsScope::Nodes(mask) if tracked => Some(
                self.nodes
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| mask.get(*index).copied().unwrap_or(false))
                    .map(|(_, node)| node.source_vertex_count)
                    .sum(),
            ),
            StatsScope::Nodes(_) | StatsScope::Material(_) => None,
        };
        stats
    }
}

#[cfg(test)]
mod tests {
    use glam::Mat4;

    use super::*;
    use crate::{ModelData, NodeKind, SceneNode, TriangleData, Vertex, demo_cube_model};

    /// Two mesh nodes, two materials, one triangle each — enough to tell the
    /// four draw groups apart and to check a scope sums only its own.
    fn two_node_model() -> ModelData {
        let corner = |x: f32| Vertex {
            position: Vec3::new(x, 0.0, 0.0),
            ..Vertex::default()
        };
        let node = |name: &str, source_vertex_count: usize, mesh_part: usize| SceneNode {
            name: name.to_owned(),
            parent: None,
            mesh_part: Some(mesh_part),
            source_vertex_count,
            transform: Mat4::IDENTITY,
            rest_local: LocalTransform::IDENTITY,
            kind: NodeKind::Mesh,
            bone: None,
        };
        ModelData {
            // Four triangles, every corner a distinct position, so each triangle
            // contributes three unique GPU vertices to its own group.
            vertices: (0..12).map(|i| corner(i as f32)).collect(),
            indices: (0..12).collect(),
            faces: (0..4)
                .map(|face| TopologyFace {
                    first_index: face * 3,
                    index_count: 3,
                })
                .collect(),
            triangles: TriangleData {
                to_face: vec![0, 1, 2, 3],
                material: vec![0, 1, 0, 1],
                node: vec![0, 0, 1, 1],
            },
            nodes: vec![node("a", 40, 0), node("b", 60, 1)],
            stats: ModelStats {
                vertex_count: 100,
                ..ModelStats::default()
            },
            ..ModelData::default()
        }
    }

    #[test]
    fn mesh_group_stats_split_the_model_into_draw_groups() {
        let model = two_node_model();
        let groups = model.mesh_group_stats();

        // One group per (node, material) pair the triangles carry.
        assert_eq!(groups.len(), 4);
        for group in &groups {
            assert_eq!(group.triangle_count, 1);
            assert_eq!(group.polygon_count, 1);
            assert_eq!(group.gpu_vertex_count, 3);
        }

        // The groups partition the mesh, so their shares add back up to the
        // whole-model figures the stats panel already reports.
        let total: usize = groups.iter().map(|group| group.gpu_vertex_count).sum();
        assert_eq!(total, model.count_gpu_vertices());
        assert_eq!(
            groups
                .iter()
                .map(|group| group.triangle_count)
                .sum::<usize>(),
            model.indices.len() / 3
        );
    }

    #[test]
    fn a_triangulated_polygon_is_counted_once_per_group() {
        // The demo cube is six quads, each triangulated into two triangles: the
        // Polys figure must stay 6, not become 12.
        let model = demo_cube_model();
        let groups = model.mesh_group_stats();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].polygon_count, 6);
        assert_eq!(groups[0].triangle_count, 12);
    }

    #[test]
    fn scope_stats_sum_only_the_scope() {
        let model = two_node_model();
        let groups = model.mesh_group_stats();

        // One node: its two draw groups, and its own authored vertex count.
        let first = model.scope_stats(&groups, StatsScope::Nodes(&[true, false]));
        assert_eq!(first.triangle_count, 2);
        assert_eq!(first.polygon_count, 2);
        assert_eq!(first.gpu_vertex_count, 6);
        assert_eq!(first.draw_count, 2);
        assert_eq!(first.vertex_count, Some(40));

        // Both nodes: the whole mesh, and the file's own vertex count back.
        let both = model.scope_stats(&groups, StatsScope::Nodes(&[true, true]));
        assert_eq!(both.triangle_count, 4);
        assert_eq!(both.gpu_vertex_count, model.count_gpu_vertices());
        assert_eq!(both.draw_count, model.material_draw_count());
        assert_eq!(both.vertex_count, Some(model.stats.vertex_count));

        // A material slot spans both nodes — one draw, and no authored vertex
        // count, since a material is not a set of meshes.
        let slot = model.scope_stats(&groups, StatsScope::Material(1));
        assert_eq!(slot.triangle_count, 2);
        assert_eq!(slot.draw_count, 1);
        assert_eq!(slot.vertex_count, None);

        // An empty mask measures nothing, rather than everything — a measured
        // zero, since the scope really is a (empty) set of meshes.
        let none = model.scope_stats(&groups, StatsScope::Nodes(&[false, false]));
        assert_eq!(none.triangle_count, 0);
        assert_eq!(none.gpu_vertex_count, 0);
        assert_eq!(none.draw_count, 0);
        assert_eq!(none.vertex_count, Some(0));
    }

    #[test]
    fn scope_stats_report_no_vertex_count_when_the_model_never_recorded_one() {
        // What a processed / procedurally-built mesh looks like: real geometry,
        // but no per-node authored count to apportion.
        let mut model = two_node_model();
        for node in &mut model.nodes {
            node.source_vertex_count = 0;
        }
        let groups = model.mesh_group_stats();
        let scope = model.scope_stats(&groups, StatsScope::Nodes(&[true, false]));
        assert_eq!(scope.triangle_count, 2);
        assert_eq!(scope.vertex_count, None);
    }

    #[test]
    fn node_subtree_mask_covers_descendants_only() {
        let mut model = demo_cube_model();
        let child = |parent: Option<usize>| SceneNode {
            name: "child".to_owned(),
            parent,
            mesh_part: None,
            source_vertex_count: 0,
            transform: Mat4::IDENTITY,
            rest_local: LocalTransform::IDENTITY,
            kind: NodeKind::Empty,
            bone: None,
        };
        // 0 is the cube; 1 hangs off it, 2 off 1, 3 is a separate root.
        model.nodes.push(child(Some(0)));
        model.nodes.push(child(Some(1)));
        model.nodes.push(child(None));

        assert_eq!(model.node_subtree_mask(0), [true, true, true, false]);
        assert_eq!(model.node_subtree_mask(1), [false, true, true, false]);
        assert_eq!(model.node_subtree_mask(3), [false, false, false, true]);
        // Out of range selects nothing rather than panicking.
        assert_eq!(model.node_subtree_mask(9), [false; 4]);
    }

    #[test]
    fn material_draw_count_counts_distinct_slots() {
        let mut model = demo_cube_model();
        // Single material across every triangle -> one draw.
        assert_eq!(model.material_draw_count(), 1);

        // Three distinct slots -> three draws, regardless of ordering/repeats.
        model.triangles.material = vec![0, 0, 1, 1, 2, 2, 0, 1, 2, 2, 1, 0];
        assert_eq!(model.material_draw_count(), 3);

        // No triangles -> no draws.
        model.indices.clear();
        assert_eq!(model.material_draw_count(), 0);
    }

    /// Every corner of the flat-shaded demo cube is genuinely unique (shared
    /// positions, but a different normal per face and different UVs per corner),
    /// so its GPU cost equals its corner count.
    #[test]
    fn gpu_vertex_count_of_the_demo_cube_is_every_corner() {
        let model = demo_cube_model();
        assert_eq!(model.count_gpu_vertices(), 24);
        assert_eq!(model.stats.gpu_vertex_count, 24);
    }

    /// Corner-split duplicates — identical in every attribute — collapse to one
    /// GPU vertex, and `-0.0` counts as `0.0` (an indexer compares values, not
    /// encodings).
    #[test]
    fn gpu_vertex_count_merges_bit_identical_corners() {
        let mut model = demo_cube_model();
        // Append exact copies of triangle 0's corners as three new vertices and
        // a triangle over them: the mesh grows, its GPU cost must not.
        let corners: Vec<Vertex> = model.indices[..3]
            .iter()
            .map(|&index| model.vertices[index as usize])
            .collect();
        let base = model.vertices.len() as u32;
        model.vertices.extend(corners);
        model.indices.extend_from_slice(&[base, base + 1, base + 2]);
        model.triangles.to_face.push(0);
        model.triangles.material.push(0);
        model.triangles.node.push(0);

        assert_eq!(model.count_gpu_vertices(), 24, "duplicates cost nothing");
    }

    /// A vertex shared by triangles of two materials is uploaded once per draw
    /// group, exactly as the Opt pipeline's per-(node, material) partition keeps
    /// it — the two counts must never disagree.
    #[test]
    fn gpu_vertex_count_keeps_material_boundaries_separate() {
        let mut model = demo_cube_model();
        assert_eq!(model.count_gpu_vertices(), 24);
        // A quad face is two triangles sharing two corners. Splitting the pair
        // across materials puts those shared corners on a draw-group boundary,
        // and each group uploads its own copy — exactly as the Opt pipeline's
        // per-(node, material) partition keeps them, so the counts can't drift.
        model.triangles.material[1] = 1;
        assert_eq!(
            model.count_gpu_vertices(),
            26,
            "the two shared corners are uploaded once per group"
        );
    }
}
