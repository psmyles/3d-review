//! Catalog text for the optimizer's typed values: an operation's name and what
//! it does, a run's warnings, an export's notes, and the errors a reader can act
//! on.
//!
//! Everything else keeps its library diagnostic verbatim (invariant 12). That is
//! deliberate: an `OptError` is written for whoever debugs it, it is what a bug
//! report quotes, and `review-optimize` must not learn about catalogs to say it.
//! But some of its variants are not really diagnostics - they describe a
//! situation the reader can resolve, and "preset could not be read (version 4,
//! this build reads 3)" tells them less than "open it with the build that wrote
//! it" does.
//!
//! A further variant that turns out to need this is one more arm here, not a
//! trait spread across four crates.

use review_localization::{Key, tr};
use review_optimize::{ExportNote, OpKind, OptError, OptWarning, RebindReport, StackRepairs};

use crate::keys::labels_opt as k;

/// An operation's name: the stack row, the "Add operation" menu, the heading
/// over its parameters, the optimizing card's stage line and a console line.
pub fn op_kind(kind: &OpKind) -> Key {
    match kind {
        OpKind::Weld(_) => k::OP_WELD,
        OpKind::FilterTriangles => k::OP_FILTER_TRIANGLES,
        OpKind::PruneComponents { .. } => k::OP_PRUNE_COMPONENTS,
        OpKind::Reduce(_) => k::OP_REDUCE,
        OpKind::Remesh(_) => k::OP_REMESH,
        OpKind::Shrinkwrap(_) => k::OP_SHRINKWRAP,
        OpKind::RecalculateNormals(_) => k::OP_RECALCULATE_NORMALS,
        OpKind::SimplifyLod(_) => k::OP_SIMPLIFY_LOD,
        OpKind::BakeAo(_) => k::OP_BAKE_AO,
        OpKind::VertexCache => k::OP_VERTEX_CACHE,
        OpKind::Overdraw { .. } => k::OP_OVERDRAW,
        OpKind::VertexFetch => k::OP_VERTEX_FETCH,
    }
}

/// [`op_kind`], resolved.
pub fn op_kind_name(kind: &OpKind) -> String {
    tr(op_kind(kind)).into_owned()
}

/// What an operation does, in a sentence - the stack row's tooltip and the
/// heading text over its parameters. Paired with [`op_kind`] so the name and the
/// explanation cannot drift apart.
pub fn op_description(kind: &OpKind) -> Key {
    match kind {
        OpKind::Weld(_) => k::OP_WELD_DESCRIPTION,
        OpKind::FilterTriangles => k::OP_FILTER_TRIANGLES_DESCRIPTION,
        OpKind::PruneComponents { .. } => k::OP_PRUNE_COMPONENTS_DESCRIPTION,
        OpKind::Reduce(_) => k::OP_REDUCE_DESCRIPTION,
        OpKind::Remesh(_) => k::OP_REMESH_DESCRIPTION,
        OpKind::Shrinkwrap(_) => k::OP_SHRINKWRAP_DESCRIPTION,
        OpKind::RecalculateNormals(_) => k::OP_RECALCULATE_NORMALS_DESCRIPTION,
        OpKind::SimplifyLod(_) => k::OP_SIMPLIFY_LOD_DESCRIPTION,
        OpKind::BakeAo(_) => k::OP_BAKE_AO_DESCRIPTION,
        OpKind::VertexCache => k::OP_VERTEX_CACHE_DESCRIPTION,
        OpKind::Overdraw { .. } => k::OP_OVERDRAW_DESCRIPTION,
        OpKind::VertexFetch => k::OP_VERTEX_FETCH_DESCRIPTION,
    }
}

/// What to show the user for an optimizer error.
pub fn explain_opt_error(error: &OptError) -> String {
    match error {
        OptError::Unavailable => tr(k::OPT_UNAVAILABLE).into_owned(),
        OptError::PresetVersion { found, supported } => {
            k::preset_newer(f64::from(*found), f64::from(*supported))
        }
        // `ExportIncomplete` is deliberately absent: the viewer gives it a report
        // of its own that names every file the run did replace, and a one-line
        // summary here would say less.
        //
        // Genuine diagnostics: the library's own wording, unchanged.
        other => other.to_string(),
    }
}

/// What to show the user for an optimization warning.
///
/// Exhaustive, with no fall-through arm: a warning added to `review-optimize` is a
/// compile error here rather than English on a translated screen. The library's
/// `Display` stays the English diagnostic the log records.
pub fn explain_opt_warning(warning: &OptWarning) -> String {
    let count = |value: usize| value as f64;
    match warning {
        OptWarning::DeformDropped { detail } => k::warning_deform_dropped(detail.clone()),
        OptWarning::MeasureFailed { detail } => k::warning_measure_failed(detail.clone()),
        OptWarning::IndexFailed { detail } => k::warning_index_failed(detail.clone()),
        OptWarning::OperationFailed { operation, detail } => {
            k::warning_operation_failed(op_kind_name(operation), detail.clone())
        }
        OptWarning::BakeBeforeSimplify => tr(k::WARNING_BAKE_BEFORE_SIMPLIFY).into_owned(),
        OptWarning::QuadsLostToSimplify => tr(k::WARNING_QUADS_LOST).into_owned(),
        OptWarning::ShrinkwrapBelowShapeChange => {
            tr(k::WARNING_SHRINKWRAP_BELOW_SHAPE_CHANGE).into_owned()
        }
        OptWarning::LodEmpty { level } => k::warning_lod_empty(count(*level)),
        OptWarning::AuthoredLodsReplaced { objects } => {
            k::warning_authored_lods_replaced(count(*objects))
        }
        OptWarning::SimplifierStalledOnSeams => tr(k::WARNING_SIMPLIFIER_STALLED).into_owned(),
        OptWarning::TangentsFailed { detail } => k::warning_tangents_failed(detail.clone()),
        OptWarning::NormalsKeptForBlendShapes { object } => {
            k::warning_normals_kept_for_blend_shapes(object.clone())
        }
        OptWarning::NormalsFailed { object, detail } => {
            k::warning_normals_failed(object.clone(), detail.clone())
        }
        OptWarning::ShrinkwrapKeptDeforming { object } => {
            k::warning_shrinkwrap_kept_deforming(object.clone())
        }
        OptWarning::ShrinkwrapTooFewTriangles { object, triangles } => {
            k::warning_shrinkwrap_too_few_triangles(object.clone(), count(*triangles))
        }
        OptWarning::ShrinkwrapEmptyField { object, resolution } => {
            k::warning_shrinkwrap_empty_field(object.clone(), f64::from(*resolution))
        }
        OptWarning::ShrinkwrapEmptyVoxels { object, resolution } => {
            k::warning_shrinkwrap_empty_voxels(object.clone(), f64::from(*resolution))
        }
        OptWarning::ShrinkwrapVoxelFailed { object, detail } => {
            k::warning_shrinkwrap_voxel_failed(object.clone(), detail.clone())
        }
        OptWarning::ShrinkwrapFailed { object, detail } => {
            k::warning_shrinkwrap_failed(object.clone(), detail.clone())
        }
        OptWarning::ShrinkwrapResolutionLowered {
            object,
            requested,
            resolution,
        } => k::warning_shrinkwrap_resolution_lowered(
            object.clone(),
            f64::from(*requested),
            f64::from(*resolution),
        ),
        OptWarning::RemeshKeptDeforming { object } => {
            k::warning_remesh_kept_deforming(object.clone())
        }
        OptWarning::RemeshTooFewTriangles { object, triangles } => {
            k::warning_remesh_too_few_triangles(object.clone(), count(*triangles))
        }
        OptWarning::RemeshFailed { object, detail } => {
            k::warning_remesh_failed(object.clone(), detail.clone())
        }
        OptWarning::RemeshBudgetAboveInput {
            object,
            triangles,
            asked,
        } => k::warning_remesh_budget_above_input(
            object.clone(),
            count(*triangles),
            f64::from(*asked),
        ),
        OptWarning::RemeshEmpty { object } => k::warning_remesh_empty(object.clone()),
        OptWarning::RemeshStubborn { object, parts } => {
            k::warning_remesh_stubborn(object.clone(), count(*parts))
        }
        OptWarning::RemeshFewQuads {
            object,
            quads,
            faces,
        } => k::warning_remesh_few_quads(object.clone(), count(*quads), count(*faces)),
    }
}

/// What to show the user for an export note. Exhaustive for the reason
/// [`explain_opt_warning`] is.
pub fn explain_export_note(note: &ExportNote) -> String {
    let count = |value: usize| value as f64;
    match note {
        ExportNote::CaptureNotLoaded => tr(k::NOTE_CAPTURE_NOT_LOADED).into_owned(),
        ExportNote::DeformNotWrittenFlat => tr(k::NOTE_DEFORM_NOT_WRITTEN_FLAT).into_owned(),
        ExportNote::AnimationNotWrittenFlat => tr(k::NOTE_ANIMATION_NOT_WRITTEN_FLAT).into_owned(),
        ExportNote::OrphanedAnimationLayers { count: layers } => {
            k::note_orphaned_animation_layers(count(*layers))
        }
        ExportNote::UnmappedAnimatedProperties { count: properties } => {
            k::note_unmapped_animated_properties(count(*properties))
        }
        ExportNote::SelectionMembersLost { count: entries } => {
            k::note_selection_members_lost(count(*entries))
        }
        ExportNote::NonBindPosesSkipped { count: poses } => {
            k::note_non_bind_poses_skipped(count(*poses))
        }
        ExportNote::UnplacedClusters {
            mesh,
            count: clusters,
        } => k::note_unplaced_clusters(mesh.clone(), count(*clusters)),
        ExportNote::NestedLayeredTextures { count: textures } => {
            k::note_nested_layered_textures(count(*textures))
        }
        ExportNote::ParentChainLooped => tr(k::NOTE_PARENT_CHAIN_LOOPED).into_owned(),
        ExportNote::NonInvertibleTransform { node } => {
            k::note_non_invertible_transform(node.clone())
        }
    }
}

/// What to tell the user about a loaded preset's per-object overrides, one line
/// per thing that happened. A preset that landed cleanly produces nothing.
///
/// The two halves are separate warnings because they mean different things: a
/// dropped override is a setting the user has lost, while one matched by
/// position is a setting that *may* have landed on the wrong object - only an
/// old preset can produce it, and only the user can tell whether it is right.
pub fn rebind_notes(report: RebindReport) -> Vec<String> {
    // Both notes read differently for one override than for several. That is a
    // plural rule, so it belongs in the catalog's own `{ $count -> }` selector
    // rather than in hand-picked `if count == 1` fragments here - English needs
    // two forms, and other languages need more or fewer.
    let mut notes = Vec::new();
    if report.dropped > 0 {
        notes.push(k::overrides_dropped(report.dropped as f64));
    }
    if report.by_position > 0 {
        notes.push(k::overrides_by_position(report.by_position as f64));
    }
    notes
}

/// What loading a preset had to repair, one line per kind of repair, shown the
/// way [`rebind_notes`] is. Empty for any preset the viewer saved itself.
pub fn repair_notes(repairs: StackRepairs) -> Vec<String> {
    let mut notes = Vec::new();
    if repairs.lods_removed > 0 {
        notes.push(k::preset_lods_removed(repairs.lods_removed as f64));
    }
    if repairs.overrides_dropped > 0 {
        notes.push(k::preset_overrides_repaired(
            repairs.overrides_dropped as f64,
        ));
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repaired preset says what was left out, one line per kind of repair, and
    /// a clean one says nothing.
    #[test]
    fn a_repaired_preset_says_what_was_left_out() {
        assert!(repair_notes(StackRepairs::default()).is_empty());
        let notes = repair_notes(StackRepairs {
            lods_removed: 1,
            overrides_dropped: 3,
        });
        assert_eq!(notes.len(), 2);
        assert!(notes.iter().all(|note| !note.is_empty()));
        assert!(
            notes[1].contains('3'),
            "the count is in the line: {}",
            notes[1]
        );
    }

    /// Every operation has a name and a description in the catalog.
    #[test]
    fn every_operation_has_its_text() {
        // `OpKind::ALL` is a table of builders, because several variants carry
        // parameters and there is no bare value to list.
        for make in OpKind::ALL {
            let kind = make();
            assert!(!tr(op_kind(&kind)).is_empty());
            assert!(!tr(op_description(&kind)).is_empty());
        }
    }

    /// The actionable variants must not fall through to the library's own
    /// `Display`, and everything else must.
    #[test]
    fn only_the_actionable_variants_are_rewritten() {
        let unavailable = explain_opt_error(&OptError::Unavailable);
        assert_ne!(unavailable, OptError::Unavailable.to_string());

        let versioned = OptError::PresetVersion {
            found: 4,
            supported: 3,
        };
        assert_ne!(explain_opt_error(&versioned), versioned.to_string());

        let diagnostic = OptError::IndexCount(7);
        assert_eq!(explain_opt_error(&diagnostic), diagnostic.to_string());
    }

    /// The catalog's words, not the library's, and every value handed over
    /// arrives - including the plural branch a count selects.
    #[test]
    fn warnings_and_notes_render_through_the_catalog() {
        let skipped = OptWarning::RemeshKeptDeforming {
            object: "Barrel".to_owned(),
        };
        assert!(explain_opt_warning(&skipped).contains("'Barrel'"));

        let one = explain_export_note(&ExportNote::NonBindPosesSkipped { count: 1 });
        let many = explain_export_note(&ExportNote::NonBindPosesSkipped { count: 3 });
        assert!(one.starts_with("One pose"), "{one}");
        assert!(many.starts_with("3 poses"), "{many}");

        // A catalogued message is typeable ASCII even where the diagnostic uses
        // a dash the keyboard does not have.
        let reordered = OptWarning::ShrinkwrapBelowShapeChange;
        assert!(reordered.to_string().contains('\u{2014}'));
        assert!(explain_opt_warning(&reordered).is_ascii());
    }
}
