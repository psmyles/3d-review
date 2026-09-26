//! Skinning: the CSR weight table over logical vertices, and the cluster table
//! carrying each (mesh node, bone) bind matrix.
//!
//! Rest pose is not bind pose. The buffers are world-baked in the bind pose, but
//! a skinned model is *displayed* in the file's default pose, so the palette at
//! rest is `bone_rest_world * world_to_bone_bind` per cluster, not identity.

use glam::Mat4;

/// Skin (skeletal binding) data: which bones move each vertex, and how strongly.
///
/// Stored **per logical source vertex** — the DCC's own vertex count, before the
/// importer expands each face corner into its own render vertex — in a compressed
/// sparse-row layout: vertex `v`'s influences are `bones[offsets[v]..offsets[v+1]]`
/// paired with `weights[..]` over the same range. Per-corner storage would multiply
/// every influence by the 3–6× corner expansion, so instead the model-wide
/// [`ModelData::corner_to_logical`](crate::ModelData::corner_to_logical) map (4 bytes per render vertex) projects the
/// render mesh back onto the logical vertices.
///
/// [`bones`] entries index [`ModelData::nodes`](crate::ModelData::nodes) directly (**not** a separate bone
/// table), so a skin influence and an Outliner row name the same thing with the
/// same number. The parallel [`influence_cluster`] names the *cluster* — the
/// (mesh node, bone) binding whose [`SkinCluster::world_to_bone_bind`] the GPU
/// skinning palette multiplies by — since one bone can bind several meshes with
/// different bind matrices.
///
/// [`bones`]: SkinData::bones
/// [`influence_cluster`]: SkinData::influence_cluster
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkinData {
    /// CSR row starts, `logical_vertex_count + 1` long: vertex `v`'s influences
    /// occupy `offsets[v]..offsets[v + 1]`. Monotonic non-decreasing, and the
    /// last entry equals the influence count (a vertex with no influences is a
    /// legal empty row).
    pub offsets: Vec<u32>,
    /// Flat influence bones, indexing [`ModelData::nodes`](crate::ModelData::nodes). Same length as
    /// [`SkinData::weights`].
    pub bones: Vec<u32>,
    /// Flat influence weights, parallel to [`SkinData::bones`]. Non-negative and
    /// finite, but **not** guaranteed to sum to 1 per vertex — FBX does not
    /// require normalized weights and the importer stores what was authored. Use
    /// [`SkinData::influence_fraction`] for display, which normalizes against the
    /// vertex's own total.
    pub weights: Vec<f32>,
    /// Per influence (parallel to [`SkinData::bones`]), the index into
    /// [`SkinData::clusters`] of the binding it belongs to. Always satisfies
    /// `clusters[influence_cluster[i]].bone == bones[i]`.
    pub influence_cluster: Vec<u32>,
    /// Every (mesh node, bone) binding the skin deformers declare, with the bind
    /// matrix the palette needs. One entry per cluster per mesh-bearing *node*
    /// (an instanced mesh under two nodes gets two, since their
    /// `geometry_to_world` differ).
    pub clusters: Vec<SkinCluster>,
    /// One entry per skinned mesh node: the skinning method the file declared
    /// and its weight cap — display metadata for the Inspector.
    pub deformers: Vec<SkinDeformerInfo>,
}

impl SkinData {
    /// Number of logical source vertices this skin covers.
    pub fn logical_vertex_count(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    /// Total number of (bone, weight) influences across every vertex.
    pub fn influence_count(&self) -> usize {
        self.bones.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bones.is_empty()
    }

    /// The slice range into [`SkinData::bones`] / [`SkinData::weights`] holding
    /// `logical`'s influences. Empty for an out-of-range vertex, so callers can
    /// index without a bounds check of their own.
    pub fn influence_range(&self, logical: usize) -> std::ops::Range<usize> {
        match (self.offsets.get(logical), self.offsets.get(logical + 1)) {
            (Some(&start), Some(&end)) if end >= start => start as usize..end as usize,
            _ => 0..0,
        }
    }

    /// The raw summed weight `logical` receives from `bones`. `bones` must be
    /// **sorted ascending** (it's binary-searched once per influence); callers get
    /// that from the UI's sorted/deduped selection set. Returns 0.0 for a vertex
    /// with no influence from any of them.
    pub fn summed_weight(&self, logical: usize, bones: &[u32]) -> f32 {
        if bones.is_empty() {
            return 0.0;
        }
        let range = self.influence_range(logical);
        self.bones[range.clone()]
            .iter()
            .zip(&self.weights[range])
            .filter(|(bone, _)| bones.binary_search(bone).is_ok())
            .map(|(_, weight)| *weight)
            .sum()
    }

    /// The share of `logical`'s total influence that `bones` account for, in
    /// `0..=1` — the value the skin-weight heat map paints.
    ///
    /// Normalizing against the vertex's *own* total (rather than assuming 1.0) is
    /// what makes the display honest on rigs whose weights weren't normalized at
    /// export: "this bone owns 40% of this vertex" stays true either way, whereas a
    /// raw sum would read as full influence on a rig whose weights sum to 0.5. For
    /// the normalized common case the two are identical. Returns 0.0 for a vertex
    /// with no influences at all.
    pub fn influence_fraction(&self, logical: usize, bones: &[u32]) -> f32 {
        if bones.is_empty() {
            return 0.0;
        }
        let range = self.influence_range(logical);
        let mut selected = 0.0_f32;
        let mut total = 0.0_f32;
        for (bone, weight) in self.bones[range.clone()].iter().zip(&self.weights[range]) {
            total += *weight;
            if bones.binary_search(bone).is_ok() {
                selected += *weight;
            }
        }
        if total <= 0.0 {
            return 0.0;
        }
        (selected / total).clamp(0.0, 1.0)
    }

    /// The lockstep guard, mirroring [`TriangleData::validate`](crate::TriangleData::validate): called once at the
    /// import funnel (invariant 7) so a drifted CSR is caught there rather than by
    /// every reader's ad-hoc bounds check. Verifies the row offsets are the right
    /// length, monotonic, and terminate at the influence count, that
    /// bones/weights/clusters are the same length, that every bone indexes a real
    /// node with a finite, non-negative weight, and that every cluster reference
    /// resolves to a cluster naming that same bone with a finite bind matrix.
    /// There is deliberately **no** upper bound on a weight: FBX does not require
    /// normalized weights, so rejecting `> 1.0` would refuse files that every other
    /// tool loads. (The corner map is validated by [`ModelData::validate_deform`](crate::ModelData::validate_deform),
    /// since blend shapes share it.)
    pub fn validate(&self, logical_count: usize, node_count: usize) -> Result<(), String> {
        if self.offsets.len() != logical_count + 1 {
            return Err(format!(
                "skin offsets has {} entries, expected {} (logical vertices + 1)",
                self.offsets.len(),
                logical_count + 1
            ));
        }
        if self.bones.len() != self.weights.len() {
            return Err(format!(
                "skin bones has {} entries but weights has {}",
                self.bones.len(),
                self.weights.len()
            ));
        }
        if let Some(window) = self.offsets.windows(2).find(|window| window[0] > window[1]) {
            return Err(format!(
                "skin offsets are not monotonic: {} then {}",
                window[0], window[1]
            ));
        }
        // `offsets` is non-empty here (length is `logical_count + 1`), so the last
        // entry exists and — given monotonicity — is the largest.
        let tail = self.offsets.last().copied().unwrap_or(0) as usize;
        if tail != self.bones.len() {
            return Err(format!(
                "skin offsets end at {tail} but there are {} influences",
                self.bones.len()
            ));
        }
        if let Some(&bone) = self.bones.iter().find(|&&bone| bone as usize >= node_count) {
            return Err(format!("skin bone references node {bone} of {node_count}"));
        }
        if let Some(&weight) = self
            .weights
            .iter()
            .find(|&&weight| !weight.is_finite() || weight < 0.0)
        {
            return Err(format!("skin weight {weight} is negative or not finite"));
        }
        if self.influence_cluster.len() != self.bones.len() {
            return Err(format!(
                "skin influence_cluster has {} entries but bones has {}",
                self.influence_cluster.len(),
                self.bones.len()
            ));
        }
        for (index, (&cluster, &bone)) in self.influence_cluster.iter().zip(&self.bones).enumerate()
        {
            let Some(entry) = self.clusters.get(cluster as usize) else {
                return Err(format!(
                    "skin influence {index} references cluster {cluster} of {}",
                    self.clusters.len()
                ));
            };
            if entry.bone != bone {
                return Err(format!(
                    "skin influence {index} names bone {bone} but its cluster {cluster} binds bone {}",
                    entry.bone
                ));
            }
        }
        for (index, cluster) in self.clusters.iter().enumerate() {
            if cluster.bone as usize >= node_count || cluster.mesh_node as usize >= node_count {
                return Err(format!(
                    "skin cluster {index} references node {} / {} of {node_count}",
                    cluster.bone, cluster.mesh_node
                ));
            }
            if !cluster.world_to_bone_bind.is_finite() {
                return Err(format!("skin cluster {index} has a non-finite bind matrix"));
            }
        }
        for (index, deformer) in self.deformers.iter().enumerate() {
            if deformer.mesh_node as usize >= node_count {
                return Err(format!(
                    "skin deformer {index} references node {} of {node_count}",
                    deformer.mesh_node
                ));
            }
        }
        Ok(())
    }
}

/// One skin cluster: the binding of a bone to a mesh node.
#[derive(Debug, Clone, PartialEq)]
pub struct SkinCluster {
    /// The bone, indexing [`ModelData::nodes`](crate::ModelData::nodes).
    pub bone: u32,
    /// The skinned mesh node this cluster deforms, indexing [`ModelData::nodes`](crate::ModelData::nodes).
    pub mesh_node: u32,
    /// Maps a *baked world-space* vertex of `mesh_node` into the bone's bind
    /// space: `geometry_to_bone × inverse(mesh.geometry_to_world)`. The palette
    /// entry for this cluster at any pose is `bone_world(pose) × this`, and the
    /// skinned position is the weighted sum of those applied to the baked vertex
    /// — exact for any pose, including the file's own default one.
    pub world_to_bone_bind: Mat4,
    /// The authored `Transform` of the cluster: mesh node → bone, as the file
    /// wrote it (in the file's own units). Carried for re-export; the palette
    /// above is what the viewer uses.
    pub mesh_node_to_bone: Mat4,
    /// The authored `TransformLink`: bone → world at bind time, in the scene's
    /// normalized (meter) space. Carried for re-export.
    pub bind_to_world: Mat4,
    /// The cluster's own name (usually the bone's).
    pub name: String,
}

/// The skinning method an FBX skin deformer declares. The viewer always
/// evaluates linear blend skinning; the other variants are surfaced so the
/// Inspector can say what the file asked for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SkinningMethod {
    #[default]
    Linear,
    Rigid,
    DualQuaternion,
    BlendedDqLinear,
}

impl SkinningMethod {
    pub fn label(self) -> &'static str {
        match self {
            SkinningMethod::Linear => "Linear",
            SkinningMethod::Rigid => "Rigid",
            SkinningMethod::DualQuaternion => "Dual Quaternion (shown as linear)",
            SkinningMethod::BlendedDqLinear => "Blended DQ / linear (shown as linear)",
        }
    }
}

/// What one skinned mesh node's deformer declared — Inspector metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkinDeformerInfo {
    /// The skinned mesh node, indexing [`ModelData::nodes`](crate::ModelData::nodes).
    pub mesh_node: u32,
    pub method: SkinningMethod,
    /// The largest number of influences any vertex of this mesh carries, as the
    /// file reports it.
    pub max_weights_per_vertex: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    // The fixtures reach for types this module does not itself name.
    use crate::*;

    // ── SkinData ────────────────────────────────────────────────────────────
    //
    // A 2-logical-vertex skin over a 6-corner (2-triangle) mesh, bound to nodes
    // 1 and 2: vertex 0 is split 0.75/0.25 between them, vertex 1 rides node 2
    // alone.
    fn sample_skin() -> SkinData {
        let cluster = |bone: u32| SkinCluster {
            bone,
            mesh_node: 0,
            world_to_bone_bind: Mat4::IDENTITY,
            mesh_node_to_bone: Mat4::IDENTITY,
            bind_to_world: Mat4::IDENTITY,
            name: String::new(),
        };
        SkinData {
            offsets: vec![0, 2, 3],
            bones: vec![1, 2, 2],
            weights: vec![0.75, 0.25, 1.0],
            influence_cluster: vec![0, 1, 1],
            clusters: vec![cluster(1), cluster(2)],
            deformers: Vec::new(),
        }
    }

    /// The render mesh [`sample_skin`] projects onto: 6 corners over 2 logical
    /// vertices.
    fn sample_corner_map() -> Vec<u32> {
        vec![0, 0, 1, 1, 0, 1]
    }

    /// A model wrapping [`sample_skin`] so [`ModelData::validate_deform`](crate::ModelData::validate_deform) can be
    /// exercised end to end.
    fn skinned_model() -> ModelData {
        ModelData {
            vertices: vec![Vertex::default(); 6],
            nodes: (0..3).map(|_| SceneNode::default()).collect(),
            stats: ModelStats {
                vertex_count: 2,
                ..ModelStats::default()
            },
            corner_to_logical: sample_corner_map(),
            skin: Some(sample_skin()),
            ..ModelData::default()
        }
    }

    #[test]
    fn skin_validate_accepts_a_consistent_skin() {
        assert_eq!(sample_skin().validate(2, 3), Ok(()));
    }

    #[test]
    fn skin_influence_range_slices_each_vertex() {
        let skin = sample_skin();
        assert_eq!(skin.influence_range(0), 0..2);
        assert_eq!(skin.influence_range(1), 2..3);
        // Out of range reads as "no influences" rather than panicking.
        assert_eq!(skin.influence_range(7), 0..0);
        assert_eq!(skin.logical_vertex_count(), 2);
        assert_eq!(skin.influence_count(), 3);
    }

    #[test]
    fn skin_summed_weight_adds_only_the_selected_bones() {
        let skin = sample_skin();
        // `bones` must be sorted — that is what the UI hands us.
        assert_eq!(skin.summed_weight(0, &[1]), 0.75);
        assert_eq!(skin.summed_weight(0, &[2]), 0.25);
        // Both selected -> the influences sum (the multi-select heat map's core).
        assert_eq!(skin.summed_weight(0, &[1, 2]), 1.0);
        assert_eq!(skin.summed_weight(1, &[1]), 0.0);
        assert_eq!(skin.summed_weight(1, &[1, 2]), 1.0);
        // Nothing selected -> no weight anywhere.
        assert_eq!(skin.summed_weight(0, &[]), 0.0);
    }

    #[test]
    fn skin_validate_catches_a_cluster_naming_another_bone() {
        let mut skin = sample_skin();
        skin.influence_cluster[0] = 1;
        let error = skin.validate(2, 3).unwrap_err();
        assert!(
            error.contains("names bone 1 but its cluster 1 binds bone 2"),
            "{error}"
        );
    }

    #[test]
    fn skin_validate_catches_an_out_of_range_cluster() {
        let mut skin = sample_skin();
        skin.influence_cluster[2] = 5;
        let error = skin.validate(2, 3).unwrap_err();
        assert!(error.contains("references cluster 5 of 2"), "{error}");
        let mut skin = sample_skin();
        skin.clusters[0].mesh_node = 3;
        let error = skin.validate(2, 3).unwrap_err();
        assert!(error.contains("cluster 0 references node"), "{error}");
    }

    #[test]
    fn skin_validate_catches_a_drifted_offsets_length() {
        let mut skin = sample_skin();
        skin.offsets.push(3);
        let error = skin.validate(2, 3).unwrap_err();
        assert!(error.contains("offsets has 4"), "{error}");
    }

    #[test]
    fn skin_validate_catches_non_monotonic_offsets() {
        let mut skin = sample_skin();
        skin.offsets = vec![0, 3, 2];
        let error = skin.validate(2, 3).unwrap_err();
        assert!(error.contains("not monotonic"), "{error}");
    }

    #[test]
    fn skin_validate_catches_offsets_that_miss_the_influences() {
        let mut skin = sample_skin();
        skin.offsets = vec![0, 2, 2];
        let error = skin.validate(2, 3).unwrap_err();
        assert!(error.contains("end at 2 but there are 3"), "{error}");
    }

    #[test]
    fn skin_validate_catches_mismatched_bones_and_weights() {
        let mut skin = sample_skin();
        skin.weights.pop();
        let error = skin.validate(2, 3).unwrap_err();
        assert!(
            error.contains("bones has 3 entries but weights has 2"),
            "{error}"
        );
    }

    #[test]
    fn skin_validate_catches_an_out_of_range_bone() {
        let mut skin = sample_skin();
        skin.bones[1] = 7;
        let error = skin.validate(2, 3).unwrap_err();
        assert!(error.contains("bone references node 7 of 3"), "{error}");
    }

    #[test]
    fn skin_validate_catches_a_negative_or_non_finite_weight() {
        for bad in [-0.25_f32, f32::NAN, f32::INFINITY] {
            let mut skin = sample_skin();
            skin.weights[0] = bad;
            let error = skin.validate(2, 3).unwrap_err();
            assert!(error.contains("negative or not finite"), "{bad}: {error}");
        }
    }

    #[test]
    fn skin_validate_accepts_unnormalized_weights() {
        // FBX does not require normalized weights; a weight above 1.0 is sloppy
        // but loadable, and refusing it would reject files other tools open.
        let mut skin = sample_skin();
        skin.weights[0] = 1.75;
        assert_eq!(skin.validate(2, 3), Ok(()));
    }

    #[test]
    fn skin_influence_fraction_normalizes_against_the_vertex_total() {
        let skin = sample_skin();
        // Already normalized (0.75 + 0.25) -> the fraction equals the raw sum.
        assert_eq!(skin.influence_fraction(0, &[1]), 0.75);
        assert_eq!(skin.influence_fraction(0, &[1, 2]), 1.0);

        // An un-normalized vertex: raw weights 1.5 / 0.5 sum to 2.0, so bone 1
        // owns 75% of the vertex even though its raw weight exceeds 1.0.
        let unnormalized = SkinData {
            offsets: vec![0, 2],
            bones: vec![1, 2],
            weights: vec![1.5, 0.5],
            ..SkinData::default()
        };
        assert_eq!(unnormalized.summed_weight(0, &[1]), 1.5);
        assert_eq!(unnormalized.influence_fraction(0, &[1]), 0.75);
        assert_eq!(unnormalized.influence_fraction(0, &[1, 2]), 1.0);

        // A vertex with no influences at all reads as zero, not NaN.
        let empty = SkinData {
            offsets: vec![0, 0],
            ..SkinData::default()
        };
        assert_eq!(empty.influence_fraction(0, &[1]), 0.0);
    }

    #[test]
    fn validate_deform_accepts_a_consistent_skinned_model() {
        assert_eq!(skinned_model().validate_deform(), Ok(()));
    }

    #[test]
    fn validate_deform_catches_a_drifted_corner_map() {
        let mut model = skinned_model();
        model.corner_to_logical.pop();
        let error = model.validate_deform().unwrap_err();
        assert!(error.contains("corner_to_logical has 5"), "{error}");
    }

    #[test]
    fn validate_deform_catches_an_out_of_range_corner_map() {
        let mut model = skinned_model();
        model.corner_to_logical[3] = 9;
        let error = model.validate_deform().unwrap_err();
        assert!(error.contains("source vertex 9 of 2"), "{error}");
    }

    #[test]
    fn validate_deform_requires_the_corner_map_for_a_skin() {
        let mut model = skinned_model();
        model.corner_to_logical.clear();
        let error = model.validate_deform().unwrap_err();
        assert!(error.contains("corner_to_logical map"), "{error}");
    }
}
