//! Turns the optimizer errors a user can act on into text that says what to do
//! about them.
//!
//! Everything else keeps its library diagnostic verbatim (invariant 12). That is
//! deliberate: an `OptError` is written for whoever debugs it, it is what a bug
//! report quotes, and `review-optimize` must not learn about catalogs to say it.
//! But some of its variants are not really diagnostics — they describe a
//! situation the reader can resolve, and "preset could not be read (version 4,
//! this build reads 3)" tells them less than "open it with the build that wrote
//! it" does.
//!
//! A further variant that turns out to need this is one more arm here, not a
//! trait spread across four crates.

use review_optimize::{ExportNote, OptError, OptWarning};

use crate::keys;

/// What to show the user for an optimizer error.
pub(crate) fn explain_opt_error(error: &OptError) -> String {
    match error {
        OptError::Unavailable => {
            review_localization::tr(keys::app_notifications::OPT_UNAVAILABLE).into_owned()
        }
        OptError::PresetVersion { found, supported } => {
            keys::app_notifications::preset_newer(f64::from(*found), f64::from(*supported))
        }
        // `ExportIncomplete` is deliberately absent: `handle_opt_exported` gives
        // it a report of its own that names every file this run did replace, and
        // a one-line summary here would say less.
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
pub(crate) fn explain_opt_warning(warning: &OptWarning) -> String {
    use keys::app_opt as k;
    let count = |value: usize| value as f64;
    match warning {
        OptWarning::DeformDropped { detail } => k::warning_deform_dropped(detail.clone()),
        OptWarning::MeasureFailed { detail } => k::warning_measure_failed(detail.clone()),
        OptWarning::IndexFailed { detail } => k::warning_index_failed(detail.clone()),
        OptWarning::OperationFailed { operation, detail } => {
            k::warning_operation_failed(review_ui::op_kind_name(operation), detail.clone())
        }
        OptWarning::BakeBeforeSimplify => {
            review_localization::tr(k::WARNING_BAKE_BEFORE_SIMPLIFY).into_owned()
        }
        OptWarning::QuadsLostToSimplify => {
            review_localization::tr(k::WARNING_QUADS_LOST).into_owned()
        }
        OptWarning::ShrinkwrapBelowShapeChange => {
            review_localization::tr(k::WARNING_SHRINKWRAP_BELOW_SHAPE_CHANGE).into_owned()
        }
        OptWarning::LodEmpty { level } => k::warning_lod_empty(count(*level)),
        OptWarning::SimplifierStalledOnSeams => {
            review_localization::tr(k::WARNING_SIMPLIFIER_STALLED).into_owned()
        }
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
pub(crate) fn explain_export_note(note: &ExportNote) -> String {
    use keys::app_opt as k;
    let count = |value: usize| value as f64;
    match note {
        ExportNote::CaptureNotLoaded => {
            review_localization::tr(k::NOTE_CAPTURE_NOT_LOADED).into_owned()
        }
        ExportNote::DeformNotWrittenFlat => {
            review_localization::tr(k::NOTE_DEFORM_NOT_WRITTEN_FLAT).into_owned()
        }
        ExportNote::AnimationNotWrittenFlat => {
            review_localization::tr(k::NOTE_ANIMATION_NOT_WRITTEN_FLAT).into_owned()
        }
        ExportNote::WrittenAsTriangles { level, mesh } => {
            k::note_written_as_triangles(count(*level), mesh.clone())
        }
        ExportNote::TrianglesRebuilt {
            level,
            mesh,
            count: triangles,
        } => k::note_triangles_rebuilt(count(*level), mesh.clone(), count(*triangles)),
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
        ExportNote::ParentChainLooped => {
            review_localization::tr(k::NOTE_PARENT_CHAIN_LOOPED).into_owned()
        }
        ExportNote::NonInvertibleTransform { node } => {
            k::note_non_invertible_transform(node.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
