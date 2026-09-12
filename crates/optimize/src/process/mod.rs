//! The processing pipeline: stack in, LOD chain out.
//!
//! ## Shape of a run
//!
//! 1. Split the model into per-(node, material) submeshes ([`crate::submesh`]).
//! 2. Apply every enabled operation *above* the LOD operation, once.
//! 3. Fan out: level 0 is the mesh as it now stands; each configured LOD level
//!    re-simplifies **from that same level-0 mesh**, not from the level above.
//! 4. Apply every enabled operation *below* the LOD operation to each level.
//! 5. Compact, reassemble a [`ModelData`] per level, and measure it.
//!
//! Only [`OpKind::SimplifyLod`] fans out. [`OpKind::Reduce`] runs the same
//! simplifier as an ordinary step of 2 or 4, so what it leaves behind *is* the
//! mesh — the one every later operation works on, every level starts from, and
//! the export writes in the source mesh's place.
//!
//! Step 3 is the quality-over-speed choice the project's locked decisions call
//! for: simplifying each level from the full-detail mesh avoids compounding one
//! level's error into the next, at the cost of doing more work per run. That
//! work happens on a background thread, so the interactive loop doesn't feel it.
//!
//! ## Failure policy
//!
//! A failure inside one operation on one submesh is recorded as a warning and
//! that submesh is left as it was; the rest of the model still processes. A
//! preview that shows most of the asset plus a clear warning is far more useful
//! while tweaking sliders than no preview at all. Only a whole-run failure
//! (meshoptimizer missing, empty model) returns `Err`.//!
//! ## Layout
//!
//! [`process`] is the run. [`carry`] defines what a level carries beside its
//! mesh, [`pipeline`] the steps a stack is walked in, [`assemble`] the rebuild
//! of one `ModelData` per level, and [`metrics`] the measuring of each.

use std::time::{Duration, Instant};

use review_model::{ModelData, SourceExtras};

use crate::meshopt;
use crate::stack::{OpInstance, OpKind, OptStack};
use crate::submesh;
use crate::{OptError, Warnings};

mod assemble;
mod carry;
mod metrics;
mod pipeline;

// The three types `lib.rs` re-exports publicly. The globs below carry the
// rest at crate visibility; an explicit import shadows a glob, so naming
// these twice is not a conflict.
pub use carry::ProcessedLod;
pub use metrics::{AnalysisMetrics, MeshCounts};

pub(crate) use assemble::*;
pub(crate) use carry::*;
pub(crate) use metrics::*;
pub(crate) use pipeline::*;

/// Everything a run needs. Borrowed rather than owned so the caller can hand in
/// an `Arc<ModelData>`'s contents without cloning the mesh.
#[derive(Debug, Clone, Copy)]
pub struct ProcessInput<'a> {
    pub model: &'a ModelData,
    pub stack: &'a OptStack,
    /// Size in bytes of one vertex *as the renderer uploads it*. Only the
    /// overfetch figure uses it, and using the real GPU vertex size is what
    /// makes that figure describe the actual draw rather than this crate's
    /// intermediate layout.
    pub render_vertex_size: usize,
    /// Outliner-hidden mesh nodes (sorted node indices). Only the AO bake
    /// consults these: a hidden object neither occludes nor receives the bake.
    /// Game assets routinely carry their whole LOD chain and a collision shell
    /// as co-located sibling nodes, and baking against those invisible,
    /// near-coincident surfaces shreds the result — what you see occluding is
    /// what occludes. Every other operation still processes hidden geometry
    /// (it is exported either way).
    pub hidden_nodes: &'a [u32],
    /// The source-property capture that accompanies `model`, when it has
    /// landed. Carried through processing for the exporter; no operation reads
    /// it. `None` in the moments between a load and its capture arriving.
    pub extras: Option<&'a SourceExtras>,
}

/// The full result of a run.
#[derive(Debug, Clone)]
pub struct ProcessedResult {
    /// Level 0 first, then each configured LOD level in order. Never empty on
    /// success.
    pub lods: Vec<ProcessedLod>,
    /// The input mesh's counts **after the lossless index pass** — the baseline
    /// every level's change is measured against.
    ///
    /// Deliberately not the raw corner-split buffer import hands over: every
    /// engine importer performs the same lossless indexing, so quoting changes
    /// against the un-indexed mesh would credit the user's operations with an
    /// "-82%" any cooker gets for free, and describe a cost the asset never has.
    pub source: MeshCounts,
    /// The indexed input mesh's own cache / overdraw / fetch figures, measured
    /// exactly as each level's are so the overlay can show what an operation did
    /// to them. Same baseline rule as [`Self::source`]: the raw corner-split
    /// buffer always measures ACMR 3.0 (no vertex is ever shared), which is a
    /// property of the import path, not of the asset.
    ///
    /// Measured on every run even though the source never changes between them:
    /// caching it would mean carrying a keyed baseline across the thread boundary
    /// for a figure that costs one more pass over a mesh the run has already
    /// walked several times.
    pub source_metrics: AnalysisMetrics,
    /// Non-fatal problems worth telling the user about, already de-duplicated.
    pub warnings: Vec<String>,
    /// Wall-clock time the run took, for the "this is taking a while" notice and
    /// the stats overlay.
    pub elapsed: Duration,
}

impl ProcessedResult {
    /// The level the viewport should show, clamped so a stale selection from a
    /// longer chain can't index past the end.
    pub fn lod(&self, level: usize) -> Option<&ProcessedLod> {
        self.lods.get(level.min(self.lods.len().saturating_sub(1)))
    }
}

/// Run `stack` over `model`, producing one mesh per LOD level.
pub fn process(input: ProcessInput<'_>) -> Result<ProcessedResult, OptError> {
    let _z = crate::prof::zone!("Process Stack");
    let started = Instant::now();

    if !meshopt::available() {
        return Err(OptError::Unavailable);
    }
    if input.model.indices.is_empty() || input.model.vertices.is_empty() {
        return Err(OptError::EmptyMesh);
    }

    let mut warnings = Warnings::default();

    let (mut submeshes, tags) = submesh::partition(input.model, input.extras);
    if submeshes.is_empty() {
        return Err(OptError::EmptyMesh);
    }

    index_mesh(&mut submeshes, input.stack, &mut warnings);

    // The baseline is the mesh as it stands *now* — partitioned and losslessly
    // indexed, before any of the user's operations touch it (they mutate the
    // submeshes in place below). See the field docs on [`ProcessedResult`] for
    // why the raw corner-split buffer would be the wrong thing to quote against.
    let source = MeshCounts::of_submeshes(&submeshes);
    let source_metrics =
        measure_submeshes(&submeshes, input.render_vertex_size, 0.0, &mut warnings);

    // Nothing enabled: no mesh is produced, but the baseline above still comes
    // back — a user reads the mesh's overdraw and cache behaviour *before*
    // deciding what to add, and every later run's change is quoted against it.
    if !input.stack.ops.iter().any(|op| op.enabled) {
        return Ok(ProcessedResult {
            lods: Vec::new(),
            source,
            source_metrics,
            warnings: warnings.into_vec(),
            elapsed: started.elapsed(),
        });
    }

    // Operations split around the LOD operation: those above it shape the mesh
    // every level starts from, those below it tidy up each generated level.
    let lod_position = input
        .stack
        .ops
        .iter()
        .position(|op| op.enabled && matches!(op.kind, OpKind::SimplifyLod(_)));
    let (pre_ops, lod_op, post_ops) = match lod_position {
        Some(position) => (
            &input.stack.ops[..position],
            match &input.stack.ops[position].kind {
                OpKind::SimplifyLod(params) => Some(params),
                _ => None,
            },
            &input.stack.ops[position + 1..],
        ),
        None => (&input.stack.ops[..], None, &input.stack.ops[0..0]),
    };

    // A Reduce among the pre-operations simplifies the mesh every level then
    // starts from, so its error is part of level 0's — and of every level below
    // it, which inherits the reduced mesh.
    let mut base_error = 0.0f32;
    for op in pre_ops {
        base_error = base_error.max(apply_op(
            &mut submeshes,
            op,
            input.stack,
            input.model,
            input.hidden_nodes,
            &mut warnings,
        ));
    }

    // Level 0 is the mesh as the pre-operations left it; every LOD level
    // re-simplifies from this same snapshot.
    let mut levels: Vec<LevelState> = vec![LevelState {
        submeshes: submeshes.clone(),
        simplify_error: base_error,
    }];

    if let Some(params) = lod_op {
        for level in &params.levels {
            let mut state = build_lod_level(&submeshes, params, level, input.stack, &mut warnings);
            state.simplify_error = state.simplify_error.max(base_error);
            levels.push(state);
        }
    }

    for level in &mut levels {
        for op in post_ops {
            let error = apply_op(
                &mut level.submeshes,
                op,
                input.stack,
                input.model,
                input.hidden_nodes,
                &mut warnings,
            );
            level.simplify_error = level.simplify_error.max(error);
        }
        for piece in &mut level.submeshes {
            piece.compact_unreferenced();
        }
    }

    let geometry_changed = input
        .stack
        .ops
        .iter()
        .any(|op| op.enabled && op.kind.alters_geometry());

    // A weld that ignores normals merges vertices whose normals differ, and the
    // survivor keeps one of them arbitrarily — it then describes one incident face
    // rather than the surface, and the mesh shades as noise. Simplification needs
    // no such fix-up: it only ever *removes* vertices, so the ones that survive
    // still carry their authored normals.
    let normals_invalidated = input.stack.ops.iter().any(|op| {
        op.enabled && matches!(&op.kind, OpKind::Weld(params) if !params.compare_normals)
    });

    // A bake above a simplifier isn't wrong — surviving vertices keep their
    // colors — but the AO then describes the pre-simplified geometry, which is
    // rarely what the user meant. Advise rather than reorder.
    let first_bake = input
        .stack
        .ops
        .iter()
        .position(|op| op.enabled && matches!(op.kind, OpKind::BakeAo(_)));
    let last_simplify = input.stack.ops.iter().rposition(|op| {
        op.enabled && matches!(op.kind, OpKind::Reduce(_) | OpKind::SimplifyLod(_))
    });
    if let (Some(bake), Some(simplify)) = (first_bake, last_simplify)
        && bake < simplify
    {
        warnings.push(
            "Bake AO runs before a simplifier, so the baked occlusion describes \
             the pre-simplified geometry. Move the bake below the simplifier — or \
             below Generate LODs to bake every level.",
        );
    }

    let mut lods = Vec::with_capacity(levels.len());
    for (index, level) in levels.into_iter().enumerate() {
        let (model, carry) = assemble(
            &level.submeshes,
            input.model,
            tags,
            index,
            Rebuild {
                normals: normals_invalidated,
                tangents: geometry_changed,
            },
            &mut warnings,
        );
        // A level can legitimately collapse to nothing when the target ratio and
        // error budget are aggressive enough. That is a real result, not a bug —
        // but it renders as an empty viewport, so say why rather than let the
        // user wonder whether processing failed.
        if model.indices.is_empty() {
            warnings.push(&format!(
                "LOD {index} simplified away completely. Raise its target ratio, or \
                 lower its error limit so the simplifier stops sooner."
            ));
        }
        let metrics = measure(
            &model,
            input.render_vertex_size,
            level.simplify_error,
            &mut warnings,
        );
        lods.push(ProcessedLod {
            level: index,
            model,
            metrics,
            carry,
        });
    }

    Ok(ProcessedResult {
        lods,
        source,
        source_metrics,
        warnings: warnings.into_vec(),
        elapsed: started.elapsed(),
    })
}

/// Whether the user excluded `node` from processing entirely.
pub(crate) fn is_excluded(stack: &OptStack, node: u32) -> bool {
    stack
        .node_override(node as usize)
        .is_some_and(|entry| entry.exclude)
}

/// The settings to use for `op` on `node`: the node's override when it has one
/// for this operation, otherwise the global settings.
pub(crate) fn resolve_op<'a>(stack: &'a OptStack, op: &'a OpInstance, node: u32) -> &'a OpKind {
    stack
        .node_override(node as usize)
        .and_then(|entry| entry.ops.iter().find(|candidate| candidate.id == op.id))
        .map_or(&op.kind, |candidate| &candidate.kind)
}

#[cfg(all(test, has_meshopt))]
mod tests {
    use review_model::demo_cube_model;

    use super::*;
    use crate::stack::{
        LodLevel, LodParams, OpKind, ReduceParams, SimplifyAlgorithm, SimplifySettings, WeldParams,
    };

    /// A stand-in for the renderer's `SceneVertex` size in tests.
    const VERTEX_SIZE: usize = 48;

    fn run(stack: &OptStack) -> ProcessedResult {
        let model = demo_cube_model();
        process(ProcessInput {
            model: &model,
            stack,
            render_vertex_size: VERTEX_SIZE,
            hidden_nodes: &[],
            extras: None,
        })
        .expect("the demo cube always processes")
    }

    /// Nothing enabled produces no mesh — but the source is still measured, so
    /// the workspace can show what it costs before anything is asked of it.
    #[test]
    fn an_empty_stack_measures_the_source_and_produces_no_levels() {
        let result = run(&OptStack::default());

        assert!(result.lods.is_empty(), "nothing was asked for");
        assert_eq!(result.source.triangles, 12);
        assert_eq!(result.source.vertices, 24);
        assert!(
            result.source_metrics.acmr > 0.0,
            "the source is measured anyway: {:?}",
            result.source_metrics
        );
    }

    #[test]
    fn an_operation_produces_a_mesh_carrying_no_polygon_topology() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::FilterTriangles);

        let output = &run(&stack).lods[0].model;
        assert_eq!(output.indices.len(), 36);
        assert!(
            output.faces.is_empty() && output.triangles.to_face.is_empty(),
            "processed meshes carry no polygon topology"
        );
    }

    #[test]
    fn weld_ignoring_split_attributes_collapses_the_cube_to_eight_corners() {
        // The demo cube is flat-shaded with per-face UVs, so all 24 corners are
        // genuinely distinct until normals and UVs are excluded from the test.
        let mut stack = OptStack::default();
        stack.push_op(OpKind::Weld(WeldParams {
            attribute_tolerance: 0.0,
            compare_normals: false,
            compare_uvs: false,
            compare_colors: false,
        }));

        let result = run(&stack);
        let output = &result.lods[0].model;
        assert_eq!(output.vertices.len(), 8, "a cube has eight corners");
        assert_eq!(output.indices.len(), 36, "welding removes no triangles");
        assert_eq!(output.stats.vertex_count, 8);
        assert_eq!(output.stats.triangle_count, 12);
    }

    #[test]
    fn weld_comparing_split_attributes_merges_nothing_on_a_flat_shaded_cube() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::Weld(WeldParams::default()));

        let result = run(&stack);
        assert_eq!(
            result.lods[0].model.vertices.len(),
            24,
            "every corner has a distinct normal and UV"
        );
    }

    #[test]
    fn tolerance_weld_reaches_the_same_eight_corners() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::Weld(WeldParams {
            attribute_tolerance: 1.0e-4,
            compare_normals: false,
            compare_uvs: false,
            compare_colors: false,
        }));

        assert_eq!(run(&stack).lods[0].model.vertices.len(), 8);
    }

    #[test]
    fn filter_triangles_leaves_a_clean_mesh_alone() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::FilterTriangles);

        assert_eq!(run(&stack).lods[0].model.indices.len(), 36);
    }

    #[test]
    fn filter_triangles_removes_an_injected_duplicate_and_degenerate() {
        let mut model = demo_cube_model();
        let [a, b, c] = [model.indices[0], model.indices[1], model.indices[2]];
        // A duplicate of triangle 0 (same winding) and a degenerate sliver.
        model.indices.extend_from_slice(&[a, b, c, a, a, b]);
        model.triangles.to_face.extend_from_slice(&[0, 0]);
        model.triangles.material.extend_from_slice(&[0, 0]);
        model.triangles.node.extend_from_slice(&[0, 0]);

        let mut stack = OptStack::default();
        stack.push_op(OpKind::FilterTriangles);
        let result = run_model(&model, &stack);

        assert_eq!(
            result.lods[0].model.indices.len(),
            36,
            "both added triangles are redundant"
        );
    }

    #[test]
    fn reorder_operations_preserve_the_triangle_set() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::VertexCache);
        stack.push_op(OpKind::Overdraw { threshold: 1.05 });
        stack.push_op(OpKind::VertexFetch);

        let result = run(&stack);
        let output = &result.lods[0].model;
        assert_eq!(output.indices.len(), 36, "reordering removes no triangles");
        assert_eq!(output.vertices.len(), 24, "and drops no referenced vertex");
    }

    #[test]
    fn a_lod_chain_produces_one_model_per_level_with_shrinking_triangle_counts() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::Weld(WeldParams {
            attribute_tolerance: 0.0,
            compare_normals: false,
            compare_uvs: false,
            compare_colors: false,
        }));
        stack.push_op(OpKind::SimplifyLod(LodParams {
            simplify: SimplifySettings {
                algorithm: SimplifyAlgorithm::Sloppy,
                ..SimplifySettings::default()
            },
            levels: vec![
                LodLevel {
                    target_ratio: 0.5,
                    target_error: 1.0,
                },
                LodLevel {
                    target_ratio: 0.25,
                    target_error: 1.0,
                },
            ],
        }));

        let result = run(&stack);
        assert_eq!(result.lods.len(), 3, "base plus two levels");

        let counts: Vec<usize> = result
            .lods
            .iter()
            .map(|lod| lod.model.stats.triangle_count)
            .collect();
        assert_eq!(counts[0], 12);
        assert!(
            counts.windows(2).all(|pair| pair[1] <= pair[0]),
            "triangle counts never grow along the chain: {counts:?}"
        );
    }

    #[test]
    fn every_level_is_named_for_its_lod() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::SimplifyLod(LodParams {
            simplify: SimplifySettings {
                algorithm: SimplifyAlgorithm::Sloppy,
                ..SimplifySettings::default()
            },
            levels: vec![LodLevel {
                target_ratio: 0.5,
                target_error: 1.0,
            }],
        }));

        let result = run(&stack);
        assert_eq!(result.lods[0].model.name, "Demo Cube");
        assert_eq!(result.lods[1].model.name, "Demo Cube_LOD1");
    }

    /// A Reduce with no LOD operation leaves exactly one mesh — the source one,
    /// simplified. That single-mesh output is the whole point of the operation:
    /// it stands in for the source asset rather than adding levels beside it.
    #[test]
    fn reduce_replaces_the_base_mesh_instead_of_adding_a_level() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::Weld(WeldParams {
            attribute_tolerance: 0.0,
            compare_normals: false,
            compare_uvs: false,
            compare_colors: false,
        }));
        stack.push_op(OpKind::Reduce(ReduceParams {
            simplify: SimplifySettings {
                algorithm: SimplifyAlgorithm::Sloppy,
                ..SimplifySettings::default()
            },
            target: LodLevel {
                target_ratio: 0.5,
                target_error: 1.0,
            },
        }));

        let result = run(&stack);
        assert_eq!(result.lods.len(), 1, "no chain, just the reduced mesh");
        assert_eq!(result.lods[0].level, 0);
        assert_eq!(
            result.lods[0].model.name, "Demo Cube",
            "the reduced mesh keeps the source name, carrying no LOD suffix"
        );
        assert!(
            result.lods[0].model.indices.len() / 3 < 12,
            "the cube was actually simplified: {} triangles",
            result.lods[0].model.indices.len() / 3
        );
        assert!(
            result.lods[0].metrics.simplify_error > 0.0,
            "level 0 reports the error the reduce cost: {:?}",
            result.lods[0].metrics
        );
    }

    /// A Reduce above the LOD operation reshapes the mesh the whole chain is
    /// built from — level 0 included, which is what separates it from a level.
    #[test]
    fn reduce_above_the_lod_operation_feeds_every_level() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::Weld(WeldParams {
            attribute_tolerance: 0.0,
            compare_normals: false,
            compare_uvs: false,
            compare_colors: false,
        }));
        stack.push_op(OpKind::Reduce(ReduceParams {
            simplify: SimplifySettings {
                algorithm: SimplifyAlgorithm::Sloppy,
                ..SimplifySettings::default()
            },
            target: LodLevel {
                target_ratio: 0.5,
                target_error: 1.0,
            },
        }));
        stack.push_op(OpKind::SimplifyLod(LodParams {
            levels: vec![LodLevel {
                target_ratio: 0.5,
                target_error: 0.01,
            }],
            ..LodParams::default()
        }));

        let result = run(&stack);
        assert_eq!(result.lods.len(), 2, "base plus one level");
        let base_triangles = result.lods[0].model.indices.len() / 3;
        assert!(
            base_triangles < 12,
            "level 0 is the reduced mesh, not the source: {base_triangles} triangles"
        );
        assert!(
            result.lods[1].model.indices.len() / 3 <= base_triangles,
            "the level starts from the reduced mesh"
        );
    }

    /// The per-object overrides that apply to every other operation apply to a
    /// Reduce's target too, so one object can be reduced harder than the rest.
    #[test]
    fn a_node_override_retargets_a_reduce() {
        let untouched = ReduceParams {
            simplify: SimplifySettings {
                algorithm: SimplifyAlgorithm::Sloppy,
                ..SimplifySettings::default()
            },
            target: LodLevel {
                target_ratio: 1.0,
                target_error: 0.0,
            },
        };
        let mut stack = OptStack::default();
        stack.push_op(OpKind::Weld(WeldParams {
            attribute_tolerance: 0.0,
            compare_normals: false,
            compare_uvs: false,
            compare_colors: false,
        }));
        let id = stack.push_op(OpKind::Reduce(untouched));
        assert_eq!(
            run(&stack).lods[0].model.indices.len() / 3,
            12,
            "a full-ratio reduce is the control: it removes nothing"
        );

        stack.node_override_mut(0).ops.push(OpInstance {
            id,
            enabled: true,
            kind: OpKind::Reduce(ReduceParams {
                target: LodLevel {
                    target_ratio: 0.5,
                    target_error: 1.0,
                },
                ..untouched
            }),
        });

        assert!(
            run(&stack).lods[0].model.indices.len() / 3 < 12,
            "the only node's override is what reduced it"
        );
    }

    #[test]
    fn an_excluded_node_keeps_its_full_detail() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::Weld(WeldParams {
            attribute_tolerance: 0.0,
            compare_normals: false,
            compare_uvs: false,
            compare_colors: false,
        }));
        stack.node_override_mut(0).exclude = true;

        assert_eq!(
            run(&stack).lods[0].model.vertices.len(),
            24,
            "the only node is excluded, so nothing is welded"
        );
    }

    #[test]
    fn a_disabled_operation_does_nothing() {
        let mut stack = OptStack::default();
        let id = stack.push_op(OpKind::Weld(WeldParams {
            attribute_tolerance: 0.0,
            compare_normals: false,
            compare_uvs: false,
            compare_colors: false,
        }));
        stack.op_mut(id).expect("just pushed").enabled = false;

        assert!(
            run(&stack).lods.is_empty(),
            "a stack with nothing enabled is an empty stack"
        );
    }

    fn run_model(model: &ModelData, stack: &OptStack) -> ProcessedResult {
        process(ProcessInput {
            model,
            stack,
            render_vertex_size: VERTEX_SIZE,
            hidden_nodes: &[],
            extras: None,
        })
        .expect("processing succeeds")
    }

    #[test]
    fn processed_models_stay_internally_consistent() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::Weld(WeldParams::default()));
        // A tight error budget on the topology-preserving simplifier: a cube has
        // nothing it can collapse without visible deformation, so every level
        // keeps real geometry and the consistency checks have something to check.
        stack.push_op(OpKind::SimplifyLod(LodParams {
            simplify: SimplifySettings {
                algorithm: SimplifyAlgorithm::Standard,
                ..SimplifySettings::default()
            },
            levels: vec![LodLevel {
                target_ratio: 0.5,
                target_error: 0.01,
            }],
        }));
        stack.push_op(OpKind::VertexFetch);

        for lod in run(&stack).lods {
            let model = &lod.model;
            let triangle_count = model.indices.len() / 3;
            model
                .triangles
                .validate(
                    triangle_count,
                    model.faces.len(),
                    model.materials.len(),
                    model.nodes.len(),
                )
                .expect("per-triangle arrays stay in lockstep");
            assert!(
                model
                    .indices
                    .iter()
                    .all(|&i| (i as usize) < model.vertices.len()),
                "every index addresses a real vertex"
            );
            assert!(!model.vertices.is_empty(), "the level keeps geometry");
            assert!(model.bounds.is_some(), "bounds are recomputed");
        }
    }

    #[test]
    fn a_level_that_simplifies_away_completely_says_so() {
        let mut stack = OptStack::default();
        // Sloppy simplification with a 100%-of-extent error budget and a hard
        // triangle target is free to collapse the cube to nothing.
        stack.push_op(OpKind::SimplifyLod(LodParams {
            simplify: SimplifySettings {
                algorithm: SimplifyAlgorithm::Sloppy,
                ..SimplifySettings::default()
            },
            levels: vec![LodLevel {
                target_ratio: 0.05,
                target_error: 1.0,
            }],
        }));

        let result = run(&stack);
        let last = result.lods.last().expect("the chain has levels");
        if last.model.indices.is_empty() {
            assert!(
                result
                    .warnings
                    .iter()
                    .any(|warning| warning.contains("simplified away")),
                "an empty level is explained: {:?}",
                result.warnings
            );
        }
        assert_eq!(
            result.lods[0].model.indices.len(),
            36,
            "level 0 is never simplified"
        );
    }

    #[test]
    fn multi_channel_uvs_stay_parallel_through_a_weld() {
        let mut model = demo_cube_model();
        let count = model.vertices.len();
        model.uv_channels = vec![
            model.vertices.iter().map(|vertex| vertex.uv).collect(),
            (0..count)
                .map(|index| glam::Vec2::splat(index as f32 * 0.01))
                .collect(),
        ];
        model.uv_set_names = vec!["UVMap".to_owned(), "Lightmap".to_owned()];
        model.stats.uv_set_count = 2;

        let mut stack = OptStack::default();
        stack.push_op(OpKind::Weld(WeldParams {
            attribute_tolerance: 0.0,
            compare_normals: false,
            compare_uvs: false,
            compare_colors: false,
        }));

        let result = run_model(&model, &stack);

        let output = &result.lods[0].model;
        assert_eq!(output.uv_channels.len(), 2);
        for channel in &output.uv_channels {
            assert_eq!(
                channel.len(),
                output.vertices.len(),
                "each UV channel stays parallel to the vertex array"
            );
        }
    }

    /// A skinned source keeps its skin through the stack: every level vertex
    /// carries the influences of the corner it came from, over the source's
    /// clusters, and the clips ride along.
    #[test]
    fn a_skinned_source_keeps_its_skin_and_clips() {
        let mut model = demo_cube_model();
        let corners = model.vertices.len();
        model.corner_to_logical = (0..corners as u32).collect();
        model.stats.vertex_count = corners;
        model.nodes = vec![
            review_model::SceneNode::default(),
            review_model::SceneNode {
                name: "Bone".to_owned(),
                kind: review_model::NodeKind::Bone,
                ..Default::default()
            },
        ];
        let cluster = review_model::SkinCluster {
            bone: 1,
            mesh_node: 0,
            world_to_bone_bind: glam::Mat4::IDENTITY,
            mesh_node_to_bone: glam::Mat4::IDENTITY,
            bind_to_world: glam::Mat4::IDENTITY,
            name: "Bone".to_owned(),
        };
        model.skin = Some(review_model::SkinData {
            offsets: (0..=corners as u32).collect(),
            bones: vec![1; corners],
            weights: (0..corners).map(|i| 0.25 + (i % 4) as f32 * 0.25).collect(),
            influence_cluster: vec![0; corners],
            clusters: vec![cluster],
            deformers: vec![review_model::SkinDeformerInfo {
                mesh_node: 0,
                method: review_model::SkinningMethod::Linear,
                max_weights_per_vertex: 1,
            }],
        });
        model.animations = vec![review_model::AnimationClip {
            name: "Idle".to_owned(),
            time_begin: 0.0,
            time_end: 1.0,
            ..Default::default()
        }];
        let mut stack = OptStack::default();
        stack.push_op(OpKind::Weld(WeldParams::default()));
        stack.push_op(OpKind::FilterTriangles);

        let result = run_model(&model, &stack);
        let level = &result.lods[0].model;
        let skin = level.skin.as_ref().expect("the skin survives");
        assert_eq!(skin.logical_vertex_count(), level.vertices.len());
        assert_eq!(level.corner_to_logical.len(), level.vertices.len());
        // Every surviving vertex kept an influence with a weight the source had.
        for vertex in 0..level.vertices.len() {
            let range = skin.influence_range(vertex);
            assert_eq!(range.len(), 1, "one influence per vertex");
            assert!(
                model
                    .skin
                    .as_ref()
                    .unwrap()
                    .weights
                    .contains(&skin.weights[range.start])
            );
        }
        assert_eq!(level.animations.len(), 1);
        assert_eq!(level.stats.clip_count, 1);
        assert!(level.validate_deform().is_ok());
        assert!(
            !result
                .warnings
                .iter()
                .any(|warning| warning.contains("skin")),
            "nothing was dropped: {:?}",
            result.warnings
        );
    }

    #[test]
    fn metrics_are_measured_for_every_level() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::VertexCache);

        for lod in run(&stack).lods {
            assert!(lod.metrics.acmr > 0.0, "ACMR is measured");
            assert!(lod.metrics.atvr > 0.0, "ATVR is measured");
            assert!(
                lod.metrics.overfetch >= 1.0,
                "overfetch is at best 1.0, got {}",
                lod.metrics.overfetch
            );
        }
    }

    #[test]
    fn vertex_cache_optimization_does_not_worsen_acmr() {
        // The source's own figure, which is what the overlay quotes the change
        // against — measured by the same run that produces the optimized one.
        let mut stack = OptStack::default();
        stack.push_op(OpKind::VertexCache);
        let result = run(&stack);
        let plain = result.source_metrics.acmr;
        let optimized = result.lods[0].metrics.acmr;

        assert!(
            optimized <= plain + f32::EPSILON,
            "ACMR should not regress: {plain} -> {optimized}"
        );
    }
}
