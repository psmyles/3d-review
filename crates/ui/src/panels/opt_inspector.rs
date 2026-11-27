//! The Inspector's Opt-workspace bodies: parameters for the operation selected
//! in the stack pane, the export settings, and per-object overrides.
//!
//! Every editor follows the same shape — copy the stored parameters out, let the
//! widgets edit the copy, and write back **only if the copy differs**. Writing
//! unconditionally would bump the stack revision every frame and reprocess the
//! mesh continuously, which is both the wrong result and an easy mistake to make
//! invisibly.

use review_labels::opt::{op_description, op_kind};
use review_model::ModelData;
use review_optimize::{
    AoQuality, AoTarget, AttributeWeights, BakeAoParams, BakeSource, BakeWearParams,
    ExportFileType, ExportOptions, FbxFormat, Flattening, GlbTangents, HierarchyMode, HullParams,
    LodLevel, LodPackaging, LodParams, NormalMode, NormalParams, NormalWeighting, OpKind,
    ReduceParams, RemeshDensity, RemeshParams, RemeshTopology, SeamSource, ShrinkwrapMethod,
    ShrinkwrapParams, SimplifyAlgorithm, SimplifyFlags, SimplifySettings, TargetMode, UnwrapParams,
    UvLayout, VoxelTarget, WeldParams,
};
use review_render::Selection;

use crate::docs::Page;
use crate::keys;
use crate::labels;
use crate::opt_state::{OptIntent, StackItem};
use crate::state::{
    UiState,
    range::{
        AO_BAKE_DISTANCE_MAX, AO_BAKE_DISTANCE_MIN, AO_BAKE_INTENSITY_MAX, AO_BAKE_INTENSITY_MIN,
        ATTRIBUTE_WEIGHT_MAX, ATTRIBUTE_WEIGHT_MIN, DISTANCE_LIMIT_MAX, DISTANCE_LIMIT_MIN,
        LOD_ERROR_MAX, LOD_ERROR_MIN, LOD_RATIO_MAX, LOD_RATIO_MIN, LOD_TRIANGLES_MAX,
        LOD_TRIANGLES_MIN, MAX_LOD_LEVELS, NORMAL_CREASE_MAX, NORMAL_CREASE_MIN,
        NORMAL_SMOOTHING_MAX, NORMAL_SMOOTHING_MIN, OVERDRAW_THRESHOLD_MAX, OVERDRAW_THRESHOLD_MIN,
        PRUNE_THRESHOLD_MAX, PRUNE_THRESHOLD_MIN, REMESH_ADAPTIVE_MAX, REMESH_ADAPTIVE_MIN,
        REMESH_CREASE_MAX, REMESH_CREASE_MIN, REMESH_FACES_MAX, REMESH_FACES_MIN, REMESH_RATIO_MAX,
        REMESH_RATIO_MIN, REMESH_SMOOTH_MAX, SHRINKWRAP_OFFSET_MAX, SHRINKWRAP_OFFSET_MIN,
        SHRINKWRAP_RESOLUTION_MAX, SHRINKWRAP_RESOLUTION_MIN, SHRINKWRAP_TARGET_RATIO_MAX,
        SHRINKWRAP_TARGET_RATIO_MIN, SHRINKWRAP_TARGET_TRIANGLES_MAX,
        SHRINKWRAP_TARGET_TRIANGLES_MIN, SHRINKWRAP_VOXEL_RESOLUTION_MAX,
        SHRINKWRAP_VOXEL_RESOLUTION_MIN, UNWRAP_CREASE_MAX, UNWRAP_CREASE_MIN, UNWRAP_DENSITY_MAX,
        UNWRAP_DENSITY_MIN, UNWRAP_MAX_STRETCH_MAX, UNWRAP_MAX_STRETCH_MIN, UNWRAP_PADDING_MAX,
        UNWRAP_RESOLUTION_MAX, UNWRAP_RESOLUTION_MIN, UV_SET_MAX, VOLUME_CHANGE_MAX,
        VOLUME_CHANGE_MIN, WEAR_ANGLE_MAX, WEAR_ANGLE_MIN, WEAR_FALLOFF_MAX, WEAR_FALLOFF_MIN,
        WELD_TOLERANCE_MAX, WELD_TOLERANCE_MIN,
    },
};
use crate::theme;
use crate::theme::{color, size};
use crate::widgets::{
    Tip, labeled_checkbox, labeled_combo, labeled_log_slider_with_value, labeled_slider_with_value,
    panel_grid, tip, tip_body, wide_button,
};

/// What the Opt inspector emitted this frame.
#[derive(Debug, Clone, Default)]
pub(crate) struct OptInspectorOutput {
    pub intent: Option<OptIntent>,
    /// A parameter widget is being actively dragged, so `app` collapses the drag
    /// into one undo step.
    pub edit_active: bool,
}

pub(crate) fn body(
    ui: &mut egui::Ui,
    state: &mut UiState,
    model: &ModelData,
) -> OptInspectorOutput {
    let mut out = OptInspectorOutput::default();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match state.opt.selected {
            Some(StackItem::Op(id)) => operation_body(ui, state, id, model),
            Some(StackItem::ExportSettings) => out.intent = export_body(ui, state),
            None => node_body(ui, state, model),
        });

    // A slider handle being dragged keeps egui's pointer captured; the 3D
    // viewport is not an egui widget, so orbiting the camera never sets this.
    out.edit_active = ui.ctx().egui_is_using_pointer();
    out
}

/// Parameters for the selected operation.
fn operation_body(ui: &mut egui::Ui, state: &mut UiState, id: u64, model: &ModelData) {
    let Some(op) = state.opt.stack.op(id) else {
        ui.weak(keys::ui_opt::OPERATION_GONE);
        return;
    };
    let kind = op.kind.clone();

    ui.heading(egui::RichText::from(op_kind(&kind)));
    ui.add_space(size::PANEL_ROW_GAP);
    ui.label(egui::RichText::from(op_description(&kind)).color(color::TEXT_MUTED));
    ui.add_space(size::PANEL_ROW_GAP);
    ui.separator();

    let edited = match kind.clone() {
        OpKind::Weld(params) => weld_params(ui, params).map(OpKind::Weld),
        OpKind::PruneComponents { error } => prune_params(ui, error),
        OpKind::Overdraw { threshold } => overdraw_params(ui, threshold),
        OpKind::Reduce(params) => reduce_params(ui, params).map(OpKind::Reduce),
        OpKind::Remesh(params) => remesh_params(ui, params).map(OpKind::Remesh),
        OpKind::Shrinkwrap(params) => shrinkwrap_params(ui, params).map(OpKind::Shrinkwrap),
        OpKind::ConvexHull(params) => hull_params(ui, &params).map(OpKind::ConvexHull),
        OpKind::RecalculateNormals(params) => {
            recalculate_normals_params(ui, params).map(OpKind::RecalculateNormals)
        }
        OpKind::SimplifyLod(params) => lod_params(ui, params).map(OpKind::SimplifyLod),
        OpKind::BakeAo(params) => bake_ao_params(ui, params).map(OpKind::BakeAo),
        OpKind::BakeWear(params) => bake_wear_params(ui, params).map(OpKind::BakeWear),
        OpKind::Unwrap(params) => unwrap_params(ui, &params, model).map(OpKind::Unwrap),
        // These three have nothing to configure — meshoptimizer exposes no knobs
        // for them, and inventing some would be worse than an honest note.
        OpKind::FilterTriangles | OpKind::VertexCache | OpKind::VertexFetch => {
            ui.add_space(size::PANEL_ROW_GAP);
            ui.weak(keys::ui_opt::NO_SETTINGS);
            None
        }
    };

    if let Some(kind) = edited {
        state.opt.edit_stack(|stack| {
            if let Some(op) = stack.op_mut(id) {
                op.kind = kind;
            }
        });
    }
}

/// Returns the edited parameters only when they actually changed.
fn weld_params(ui: &mut egui::Ui, params: WeldParams) -> Option<WeldParams> {
    let mut edited = params;
    panel_grid(ui, "opt_weld", |ui| {
        labeled_slider_with_value(
            ui,
            Tip::new(keys::ui_opt::TOLERANCE)
                .describe(keys::ui_opt::TOLERANCE_DESCRIPTION)
                .page(Page::OptOperations),
            &mut edited.attribute_tolerance,
            WELD_TOLERANCE_MIN..=WELD_TOLERANCE_MAX,
            4,
        );
        labeled_checkbox(
            ui,
            Tip::new(keys::ui_opt::COMPARE_NORMALS).page(Page::OptOperations),
            &mut edited.compare_normals,
        );
        labeled_checkbox(
            ui,
            Tip::new(keys::ui_opt::COMPARE_UVS).page(Page::OptOperations),
            &mut edited.compare_uvs,
        );
        labeled_checkbox(
            ui,
            Tip::new(keys::ui_opt::COMPARE_COLORS).page(Page::OptOperations),
            &mut edited.compare_colors,
        );
    });

    ui.add_space(size::PANEL_ROW_GAP);
    ui.label(egui::RichText::from(keys::ui_opt::WELD_EXPLAINED).color(color::TEXT_MUTED));

    (edited != params).then_some(edited)
}

fn prune_params(ui: &mut egui::Ui, error: f32) -> Option<OpKind> {
    let mut edited = error;
    panel_grid(ui, "opt_prune", |ui| {
        labeled_slider_with_value(
            ui,
            Tip::new(keys::ui_opt::SIZE_THRESHOLD)
                .describe(keys::ui_opt::SIZE_THRESHOLD_DESCRIPTION)
                .page(Page::OptOperations),
            &mut edited,
            PRUNE_THRESHOLD_MIN..=PRUNE_THRESHOLD_MAX,
            3,
        );
    });
    ui.add_space(size::PANEL_ROW_GAP);
    ui.label(egui::RichText::from(keys::ui_opt::PRUNE_EXPLAINED).color(color::TEXT_MUTED));

    (edited != error).then_some(OpKind::PruneComponents { error: edited })
}

fn overdraw_params(ui: &mut egui::Ui, threshold: f32) -> Option<OpKind> {
    let mut edited = threshold;
    panel_grid(ui, "opt_overdraw", |ui| {
        labeled_slider_with_value(
            ui,
            Tip::new(keys::ui_opt::CACHE_TOLERANCE)
                .describe(keys::ui_opt::CACHE_TOLERANCE_DESCRIPTION)
                .page(Page::OptOperations),
            &mut edited,
            OVERDRAW_THRESHOLD_MIN..=OVERDRAW_THRESHOLD_MAX,
            2,
        );
    });
    ui.add_space(size::PANEL_ROW_GAP);
    ui.label(egui::RichText::from(keys::ui_opt::OVERDRAW_EXPLAINED).color(color::TEXT_MUTED));

    (edited - threshold)
        .abs()
        .gt(&f32::EPSILON)
        .then_some(OpKind::Overdraw { threshold: edited })
}

/// Returns the edited parameters only when they actually changed.
fn recalculate_normals_params(ui: &mut egui::Ui, params: NormalParams) -> Option<NormalParams> {
    let mut edited = params;
    panel_grid(ui, "opt_recalculate_normals", |ui| {
        normal_params_rows(ui, &mut edited);
    });
    ui.add_space(size::PANEL_ROW_GAP);
    ui.label(
        egui::RichText::from(keys::ui_opt::RECALCULATE_NORMALS_EXPLAINED).color(color::TEXT_MUTED),
    );
    (edited != params).then_some(edited)
}

/// The crease angle and smoothing rows, inside the caller's grid: shared by
/// Recalculate Normals and by every rebuild told to generate its normals, so the
/// same two numbers read the same wherever they appear.
fn normal_params_rows(ui: &mut egui::Ui, params: &mut NormalParams) {
    labeled_combo(
        ui,
        Tip::new(keys::ui_opt::NORMAL_WEIGHTING)
            .describe(keys::ui_opt::NORMAL_WEIGHTING_DESCRIPTION)
            .page(Page::OptNormals),
        "opt_normal_weighting",
        labels::normal_weighting(params.weighting),
        |ui| {
            for weighting in NormalWeighting::ALL {
                ui.selectable_value(
                    &mut params.weighting,
                    weighting,
                    labels::normal_weighting(weighting),
                );
            }
        },
    );
    labeled_slider_with_value(
        ui,
        Tip::new(keys::ui_opt::NORMAL_CREASE_ANGLE)
            .describe(keys::ui_opt::NORMAL_CREASE_ANGLE_DESCRIPTION)
            .page(Page::OptNormals),
        &mut params.crease_angle,
        NORMAL_CREASE_MIN..=NORMAL_CREASE_MAX,
        0,
    );
    // Smoothing is meshoptimizer's, which only the default weighting uses.
    if params.weighting == NormalWeighting::AngleAndEdges {
        labeled_slider_with_value(
            ui,
            Tip::new(keys::ui_opt::NORMAL_SMOOTHING)
                .describe(keys::ui_opt::NORMAL_SMOOTHING_DESCRIPTION)
                .page(Page::OptNormals),
            &mut params.smoothing,
            NORMAL_SMOOTHING_MIN..=NORMAL_SMOOTHING_MAX,
            1,
        );
    }
}

/// The "Normals" choice a rebuilding operation offers — projected off the
/// original, or generated from the new surface — with the generation rows
/// shown only when they are what the choice reads.
fn normal_source_rows(
    ui: &mut egui::Ui,
    id: &str,
    mode: &mut NormalMode,
    params: &mut NormalParams,
) {
    labeled_combo(
        ui,
        Tip::new(keys::ui_opt::NORMAL_SOURCE)
            .describe(keys::ui_opt::NORMAL_SOURCE_DESCRIPTION)
            .page(Page::OptNormals),
        id,
        labels::normal_mode(*mode),
        |ui| {
            for option in NormalMode::ALL {
                ui.selectable_value(mode, option, labels::normal_mode(option));
            }
        },
    );
    if *mode == NormalMode::Generate {
        normal_params_rows(ui, params);
    }
}

/// Returns the edited parameters only when they actually changed.
///
/// The rows follow the method: each extraction reads its own settings, and
/// showing the other's would invite setting one and wondering why nothing
/// changed. Changing method goes through [`ShrinkwrapParams::set_method`], which
/// also resets the normals to that method's default.
fn shrinkwrap_params(ui: &mut egui::Ui, params: ShrinkwrapParams) -> Option<ShrinkwrapParams> {
    let mut edited = params;
    panel_grid(ui, "opt_shrinkwrap", |ui| {
        let mut method = edited.method;
        labeled_combo(
            ui,
            Tip::new(keys::ui_opt::SHRINKWRAP_METHOD)
                .describe(keys::ui_opt::SHRINKWRAP_METHOD_DESCRIPTION)
                .page(Page::OptShrinkwrap),
            "opt_shrinkwrap_method",
            labels::shrinkwrap_method(method),
            |ui| {
                for option in ShrinkwrapMethod::ALL {
                    ui.selectable_value(&mut method, option, labels::shrinkwrap_method(option));
                }
            },
        );
        edited.set_method(method);

        match edited.method {
            ShrinkwrapMethod::Winding => {
                labeled_slider_with_value(
                    ui,
                    Tip::new(keys::ui_opt::SHRINKWRAP_RESOLUTION)
                        .describe(keys::ui_opt::SHRINKWRAP_RESOLUTION_DESCRIPTION)
                        .page(Page::OptShrinkwrap),
                    &mut edited.resolution,
                    SHRINKWRAP_RESOLUTION_MIN..=SHRINKWRAP_RESOLUTION_MAX,
                    0,
                );
                labeled_slider_with_value(
                    ui,
                    Tip::new(keys::ui_opt::SHRINKWRAP_OFFSET)
                        .describe(keys::ui_opt::SHRINKWRAP_OFFSET_DESCRIPTION)
                        .page(Page::OptShrinkwrap),
                    &mut edited.offset,
                    SHRINKWRAP_OFFSET_MIN..=SHRINKWRAP_OFFSET_MAX,
                    3,
                );
            }
            ShrinkwrapMethod::Voxel => voxel_rows(ui, &mut edited),
        }
        labeled_checkbox(
            ui,
            Tip::new(keys::ui_opt::SHRINKWRAP_LARGEST_SHELL)
                .describe(keys::ui_opt::SHRINKWRAP_LARGEST_SHELL_DESCRIPTION)
                .page(Page::OptShrinkwrap),
            &mut edited.keep_largest_shell,
        );
        normal_source_rows(
            ui,
            "opt_shrinkwrap_normals",
            &mut edited.normals,
            &mut edited.normal_params,
        );
    });

    ui.add_space(size::PANEL_ROW_GAP);
    ui.label(egui::RichText::from(keys::ui_opt::SHRINKWRAP_EXPLAINED).color(color::TEXT_MUTED));

    (edited != params).then_some(edited)
}

/// The voxel method's own rows, inside the Shrinkwrap grid. The target's
/// amount row shows a ratio *or* a count, never both, as Remesh's density does.
fn voxel_rows(ui: &mut egui::Ui, edited: &mut ShrinkwrapParams) {
    labeled_slider_with_value(
        ui,
        Tip::new(keys::ui_opt::SHRINKWRAP_VOXEL_RESOLUTION)
            .describe(keys::ui_opt::SHRINKWRAP_VOXEL_RESOLUTION_DESCRIPTION)
            .page(Page::OptShrinkwrap),
        &mut edited.voxel_resolution,
        SHRINKWRAP_VOXEL_RESOLUTION_MIN..=SHRINKWRAP_VOXEL_RESOLUTION_MAX,
        0,
    );
    labeled_checkbox(
        ui,
        Tip::new(keys::ui_opt::SHRINKWRAP_FIT_SURFACE)
            .describe(keys::ui_opt::SHRINKWRAP_FIT_SURFACE_DESCRIPTION)
            .page(Page::OptShrinkwrap),
        &mut edited.voxel_solve,
    );
    labeled_checkbox(
        ui,
        Tip::new(keys::ui_opt::SHRINKWRAP_TWO_SIDED)
            .describe(keys::ui_opt::SHRINKWRAP_TWO_SIDED_DESCRIPTION)
            .page(Page::OptShrinkwrap),
        &mut edited.voxel_shell,
    );
    labeled_combo(
        ui,
        Tip::new(keys::ui_opt::SHRINKWRAP_TARGET)
            .describe(keys::ui_opt::SHRINKWRAP_TARGET_DESCRIPTION)
            .page(Page::OptShrinkwrap),
        "opt_shrinkwrap_target",
        labels::voxel_target(edited.voxel_target),
        |ui| {
            for target in VoxelTarget::ALL {
                ui.selectable_value(
                    &mut edited.voxel_target,
                    target,
                    labels::voxel_target(target),
                );
            }
        },
    );
    match edited.voxel_target {
        VoxelTarget::Keep => {}
        VoxelTarget::Ratio => {
            labeled_slider_with_value(
                ui,
                Tip::new(keys::ui_opt::SHRINKWRAP_TARGET_RATIO)
                    .describe(keys::ui_opt::SHRINKWRAP_TARGET_RATIO_DESCRIPTION)
                    .page(Page::OptShrinkwrap),
                &mut edited.voxel_ratio,
                SHRINKWRAP_TARGET_RATIO_MIN..=SHRINKWRAP_TARGET_RATIO_MAX,
                2,
            );
        }
        VoxelTarget::Triangles => {
            labeled_slider_with_value(
                ui,
                Tip::new(keys::ui_opt::SHRINKWRAP_TARGET_TRIANGLES)
                    .describe(keys::ui_opt::SHRINKWRAP_TARGET_TRIANGLES_DESCRIPTION)
                    .page(Page::OptShrinkwrap),
                &mut edited.voxel_triangles,
                SHRINKWRAP_TARGET_TRIANGLES_MIN..=SHRINKWRAP_TARGET_TRIANGLES_MAX,
                0,
            );
        }
    }
    if edited.voxel_target != VoxelTarget::Keep {
        labeled_checkbox(
            ui,
            Tip::new(keys::ui_opt::SHRINKWRAP_EVEN_TRIANGLES)
                .describe(keys::ui_opt::SHRINKWRAP_EVEN_TRIANGLES_DESCRIPTION)
                .page(Page::OptShrinkwrap),
            &mut edited.voxel_regularize,
        );
    }
}

/// Returns the edited parameters only when they actually changed.
///
/// Two rows are conditional rather than always present: the density row shows a
/// ratio *or* a face count (they are alternatives, and showing both would invite
/// setting one and reading the other), and the crease angle only appears with
/// sharp-edge detection on, which is the only thing that reads it.
fn remesh_params(ui: &mut egui::Ui, params: RemeshParams) -> Option<RemeshParams> {
    let mut edited = params;
    panel_grid(ui, "opt_remesh", |ui| {
        labeled_combo(
            ui,
            Tip::new(keys::ui_opt::REMESH_TOPOLOGY)
                .describe(keys::ui_opt::REMESH_TOPOLOGY_DESCRIPTION)
                .page(Page::OptRemesh),
            "opt_remesh_topology",
            labels::remesh_topology(edited.topology),
            |ui| {
                for topology in RemeshTopology::ALL {
                    ui.selectable_value(
                        &mut edited.topology,
                        topology,
                        labels::remesh_topology(topology),
                    );
                }
            },
        );
        labeled_combo(
            ui,
            Tip::new(keys::ui_opt::REMESH_DENSITY)
                .describe(keys::ui_opt::REMESH_DENSITY_DESCRIPTION)
                .page(Page::OptRemesh),
            "opt_remesh_density",
            labels::remesh_density(edited.density),
            |ui| {
                for density in RemeshDensity::ALL {
                    ui.selectable_value(
                        &mut edited.density,
                        density,
                        labels::remesh_density(density),
                    );
                }
            },
        );
        match edited.density {
            RemeshDensity::Ratio => {
                labeled_slider_with_value(
                    ui,
                    Tip::new(keys::ui_opt::REMESH_RATIO)
                        .describe(keys::ui_opt::REMESH_RATIO_DESCRIPTION)
                        .page(Page::OptRemesh),
                    &mut edited.ratio,
                    REMESH_RATIO_MIN..=REMESH_RATIO_MAX,
                    2,
                );
            }
            RemeshDensity::Absolute => {
                labeled_slider_with_value(
                    ui,
                    Tip::new(keys::ui_opt::REMESH_FACES)
                        .describe(keys::ui_opt::REMESH_FACES_DESCRIPTION)
                        .page(Page::OptRemesh),
                    &mut edited.faces,
                    REMESH_FACES_MIN..=REMESH_FACES_MAX,
                    0,
                );
            }
        }
        labeled_checkbox(
            ui,
            Tip::new(keys::ui_opt::REMESH_SHARP_EDGES)
                .describe(keys::ui_opt::REMESH_SHARP_EDGES_DESCRIPTION)
                .page(Page::OptRemesh),
            &mut edited.sharp_edges,
        );
        if edited.sharp_edges {
            labeled_slider_with_value(
                ui,
                Tip::new(keys::ui_opt::REMESH_CREASE_ANGLE)
                    .describe(keys::ui_opt::REMESH_CREASE_ANGLE_DESCRIPTION)
                    .page(Page::OptRemesh),
                &mut edited.crease_angle,
                REMESH_CREASE_MIN..=REMESH_CREASE_MAX,
                0,
            );
        }
        labeled_checkbox(
            ui,
            Tip::new(keys::ui_opt::REMESH_ALIGN_BOUNDARIES)
                .describe(keys::ui_opt::REMESH_ALIGN_BOUNDARIES_DESCRIPTION)
                .page(Page::OptRemesh),
            &mut edited.align_to_boundaries,
        );
        labeled_slider_with_value(
            ui,
            Tip::new(keys::ui_opt::REMESH_SMOOTHING)
                .describe(keys::ui_opt::REMESH_SMOOTHING_DESCRIPTION)
                .page(Page::OptRemesh),
            &mut edited.smooth_iterations,
            0..=REMESH_SMOOTH_MAX,
            0,
        );
        labeled_slider_with_value(
            ui,
            Tip::new(keys::ui_opt::REMESH_ADAPTIVE)
                .describe(keys::ui_opt::REMESH_ADAPTIVE_DESCRIPTION)
                .page(Page::OptRemesh),
            &mut edited.adaptive_strength,
            REMESH_ADAPTIVE_MIN..=REMESH_ADAPTIVE_MAX,
            2,
        );
        normal_source_rows(
            ui,
            "opt_remesh_normals",
            &mut edited.normals,
            &mut edited.normal_params,
        );
    });

    ui.add_space(size::PANEL_ROW_GAP);
    ui.label(egui::RichText::from(keys::ui_opt::REMESH_EXPLAINED).color(color::TEXT_MUTED));

    (edited != params).then_some(edited)
}

/// The Unwrap operation's settings, in the order an unwrap runs and the way
/// Blender's and Maya's unwrap options read: which UV map to write and which
/// existing UVs to learn from, then where to cut (**Seams**), how to flatten
/// (**Unfold**) and where the islands go (**Layout**). The attribute choice
/// lists what `model` carries — its custom attributes and its UV sets — plus
/// whatever the settings name that it does not (a preset made for another
/// model).
fn unwrap_params(
    ui: &mut egui::Ui,
    params: &UnwrapParams,
    model: &ModelData,
) -> Option<UnwrapParams> {
    let mut edited = params.clone();
    let uv_sets = model.stats.uv_set_count as u32;
    let mut seams: Vec<SeamSource> = vec![SeamSource::None];
    seams.extend(
        model
            .custom_attributes
            .iter()
            .map(|attribute| SeamSource::Attribute(attribute.name.clone())),
    );
    seams.extend((0..uv_sets).map(SeamSource::UvSetU));
    if !seams.contains(&edited.seams) {
        seams.push(edited.seams.clone());
    }

    panel_grid(ui, "opt_unwrap", |ui| {
        uv_set_row(
            ui,
            Tip::new(keys::ui_opt::UNWRAP_UV_MAP)
                .describe(keys::ui_opt::UNWRAP_UV_MAP_DESCRIPTION)
                .page(Page::OptUnwrap),
            "opt_unwrap_target_set",
            &mut edited.target_set,
            model,
            true,
        );
        existing_uvs_row(ui, &mut edited.reference_set, model);
    });
    let existing = edited.reference_set.is_some();

    unwrap_section(
        ui,
        keys::ui_opt::UNWRAP_SECTION_SEAMS,
        "opt_unwrap_seams_section",
        |ui| {
            panel_grid(ui, "opt_unwrap_seams", |ui| {
                if existing {
                    labeled_checkbox(
                        ui,
                        Tip::new(keys::ui_opt::UNWRAP_KEEP_SEAMS)
                            .describe(keys::ui_opt::UNWRAP_KEEP_SEAMS_DESCRIPTION)
                            .page(Page::OptUnwrap),
                        &mut edited.split_at_reference_seams,
                    );
                }
                labeled_checkbox(
                    ui,
                    Tip::new(keys::ui_opt::UNWRAP_SHARP_EDGES)
                        .describe(keys::ui_opt::UNWRAP_SHARP_EDGES_DESCRIPTION)
                        .page(Page::OptUnwrap),
                    &mut edited.split_at_creases,
                );
                if edited.split_at_creases {
                    labeled_slider_with_value(
                        ui,
                        Tip::new(keys::ui_opt::UNWRAP_SHARP_ANGLE)
                            .describe(keys::ui_opt::UNWRAP_SHARP_ANGLE_DESCRIPTION)
                            .page(Page::OptUnwrap),
                        &mut edited.crease_angle,
                        UNWRAP_CREASE_MIN..=UNWRAP_CREASE_MAX,
                        0,
                    );
                }
                labeled_combo(
                    ui,
                    Tip::new(keys::ui_opt::UNWRAP_ATTRIBUTE)
                        .describe(keys::ui_opt::UNWRAP_ATTRIBUTE_DESCRIPTION)
                        .page(Page::OptUnwrap),
                    "opt_unwrap_seams",
                    labels::seam_source(&edited.seams),
                    |ui| {
                        for seam in &seams {
                            ui.selectable_value(
                                &mut edited.seams,
                                seam.clone(),
                                labels::seam_source(seam),
                            );
                        }
                    },
                );
                labeled_slider_with_value(
                    ui,
                    Tip::new(keys::ui_opt::UNWRAP_MAX_STRETCH)
                        .describe(keys::ui_opt::UNWRAP_MAX_STRETCH_DESCRIPTION)
                        .page(Page::OptUnwrap),
                    &mut edited.max_stretch,
                    UNWRAP_MAX_STRETCH_MIN..=UNWRAP_MAX_STRETCH_MAX,
                    2,
                );
            });
        },
    );

    unwrap_section(
        ui,
        keys::ui_opt::UNWRAP_SECTION_UNFOLD,
        "opt_unwrap_unfold_section",
        |ui| {
            panel_grid(ui, "opt_unwrap_unfold", |ui| {
                labeled_combo(
                    ui,
                    Tip::new(keys::ui_opt::UNWRAP_METHOD)
                        .describe(keys::ui_opt::UNWRAP_METHOD_DESCRIPTION)
                        .page(Page::OptUnwrap),
                    "opt_unwrap_flattening",
                    labels::flattening(edited.flattening),
                    |ui| {
                        for flattening in Flattening::ALL {
                            ui.selectable_value(
                                &mut edited.flattening,
                                flattening,
                                labels::flattening(flattening),
                            );
                        }
                    },
                );
            });
        },
    );

    unwrap_section(
        ui,
        keys::ui_opt::UNWRAP_SECTION_LAYOUT,
        "opt_unwrap_layout_section",
        |ui| {
            panel_grid(ui, "opt_unwrap_layout", |ui| {
                labeled_combo(
                    ui,
                    Tip::new(keys::ui_opt::UNWRAP_LAYOUT)
                        .describe(keys::ui_opt::UNWRAP_LAYOUT_DESCRIPTION)
                        .page(Page::OptUnwrap),
                    "opt_unwrap_layout",
                    labels::uv_layout(edited.layout),
                    |ui| {
                        for layout in UvLayout::ALL {
                            ui.selectable_value(
                                &mut edited.layout,
                                layout,
                                labels::uv_layout(layout),
                            );
                        }
                    },
                );
                match edited.layout {
                    UvLayout::Atlas => {
                        labeled_log_slider_with_value(
                            ui,
                            Tip::new(keys::ui_opt::UNWRAP_TEXTURE_SIZE)
                                .describe(keys::ui_opt::UNWRAP_TEXTURE_SIZE_DESCRIPTION)
                                .page(Page::OptUnwrap),
                            &mut edited.resolution,
                            UNWRAP_RESOLUTION_MIN..=UNWRAP_RESOLUTION_MAX,
                            0,
                        );
                        labeled_slider_with_value(
                            ui,
                            Tip::new(keys::ui_opt::UNWRAP_PADDING)
                                .describe(keys::ui_opt::UNWRAP_PADDING_DESCRIPTION)
                                .page(Page::OptUnwrap),
                            &mut edited.padding,
                            0..=UNWRAP_PADDING_MAX,
                            0,
                        );
                        rotate_row(ui, &mut edited.rotate);
                    }
                    UvLayout::Tiled => {
                        labeled_log_slider_with_value(
                            ui,
                            Tip::new(keys::ui_opt::UNWRAP_DENSITY)
                                .describe(keys::ui_opt::UNWRAP_DENSITY_DESCRIPTION)
                                .page(Page::OptUnwrap),
                            &mut edited.density,
                            UNWRAP_DENSITY_MIN..=UNWRAP_DENSITY_MAX,
                            3,
                        );
                        if existing {
                            labeled_checkbox(
                                ui,
                                Tip::new(keys::ui_opt::UNWRAP_ALIGN)
                                    .describe(keys::ui_opt::UNWRAP_ALIGN_DESCRIPTION)
                                    .page(Page::OptUnwrap),
                                &mut edited.align_to_reference,
                            );
                        }
                        if existing && edited.align_to_reference {
                            labeled_checkbox(
                                ui,
                                Tip::new(keys::ui_opt::UNWRAP_KEEP_BETTER)
                                    .describe(keys::ui_opt::UNWRAP_KEEP_BETTER_DESCRIPTION)
                                    .page(Page::OptUnwrap),
                                &mut edited.keep_better,
                            );
                        } else {
                            rotate_row(ui, &mut edited.rotate);
                        }
                    }
                }
            });
        },
    );
    (edited != *params).then_some(edited)
}

/// One of the Unwrap's sections: a header that folds, open to start with.
fn unwrap_section(
    ui: &mut egui::Ui,
    title: review_localization::Key,
    salt: &str,
    contents: impl FnOnce(&mut egui::Ui),
) {
    ui.add_space(size::PANEL_ROW_GAP);
    egui::CollapsingHeader::new(title)
        .id_salt(salt)
        .default_open(true)
        .show(ui, contents);
}

fn rotate_row(ui: &mut egui::Ui, rotate: &mut bool) {
    labeled_checkbox(
        ui,
        Tip::new(keys::ui_opt::UNWRAP_ROTATE)
            .describe(keys::ui_opt::UNWRAP_ROTATE_DESCRIPTION)
            .page(Page::OptUnwrap),
        rotate,
    );
}

/// The existing UVs the unwrap learns from: none, or one of the model's sets
/// (or one the stack names that the model lacks, so the dropdown never hides
/// what the operation will do).
fn existing_uvs_row(ui: &mut egui::Ui, reference: &mut Option<u32>, model: &ModelData) {
    let count = (model.stats.uv_set_count as u32).min(UV_SET_MAX + 1);
    let name = |set: Option<u32>| -> String {
        match set {
            None => review_localization::tr(keys::ui_opt::UNWRAP_EXISTING_NONE).into_owned(),
            Some(set) => {
                keys::ui_opt::unwrap_uv_set(f64::from(set), model.uv_set_label(set as usize))
            }
        }
    };
    let mut choices: Vec<Option<u32>> = std::iter::once(None).chain((0..count).map(Some)).collect();
    if !choices.contains(reference) {
        choices.push(*reference);
    }
    labeled_combo(
        ui,
        Tip::new(keys::ui_opt::UNWRAP_EXISTING)
            .describe(keys::ui_opt::UNWRAP_EXISTING_DESCRIPTION)
            .page(Page::OptUnwrap),
        "opt_unwrap_reference_set",
        name(*reference),
        |ui| {
            for choice in choices {
                ui.selectable_value(reference, choice, name(choice));
            }
        },
    );
}

/// A UV set dropdown: each of the model's sets by its name, and, where a set
/// can be written, a new one after its last. A set the stack names that the
/// model does not have (a preset made for another model) is listed too, so
/// the dropdown never hides what the operation will do.
fn uv_set_row(
    ui: &mut egui::Ui,
    label: Tip,
    id: &str,
    set: &mut u32,
    model: &ModelData,
    offer_new: bool,
) {
    let count = (model.stats.uv_set_count as u32).min(UV_SET_MAX + 1);
    let name = |set: u32| -> String {
        if set < count {
            keys::ui_opt::unwrap_uv_set(f64::from(set), model.uv_set_label(set as usize))
        } else {
            keys::ui_opt::unwrap_new_uv_set(f64::from(set))
        }
    };
    let mut choices: Vec<u32> = (0..count).collect();
    if offer_new && count <= UV_SET_MAX {
        choices.push(count);
    }
    if !choices.contains(set) {
        choices.push(*set);
    }
    labeled_combo(ui, label, id, name(*set), |ui| {
        for choice in choices {
            ui.selectable_value(set, choice, name(choice));
        }
    });
}

fn bake_ao_params(ui: &mut egui::Ui, params: BakeAoParams) -> Option<BakeAoParams> {
    let mut edited = params;
    panel_grid(ui, "opt_bake_ao", |ui| {
        bake_source_row(ui, "opt_bake_ao_source", &mut edited.source, Page::OptAo);
        labeled_combo(
            ui,
            Tip::new(keys::ui_opt::AO_QUALITY)
                .describe(keys::ui_opt::AO_QUALITY_DESCRIPTION)
                .page(Page::OptOperations),
            "opt_bake_ao_quality",
            labels::ao_quality(edited.quality),
            |ui| {
                for quality in AoQuality::ALL {
                    ui.selectable_value(&mut edited.quality, quality, labels::ao_quality(quality));
                }
            },
        );
        labeled_slider_with_value(
            ui,
            Tip::new(keys::ui_opt::AO_MAX_DISTANCE)
                .describe(keys::ui_opt::AO_MAX_DISTANCE_DESCRIPTION)
                .page(Page::OptOperations),
            &mut edited.max_distance,
            AO_BAKE_DISTANCE_MIN..=AO_BAKE_DISTANCE_MAX,
            2,
        );
        labeled_slider_with_value(
            ui,
            Tip::new(keys::ui_opt::AO_INTENSITY)
                .describe(keys::ui_opt::AO_INTENSITY_DESCRIPTION)
                .page(Page::OptOperations),
            &mut edited.intensity,
            AO_BAKE_INTENSITY_MIN..=AO_BAKE_INTENSITY_MAX,
            2,
        );
        labeled_combo(
            ui,
            Tip::new(keys::ui_opt::AO_WRITE_TO)
                .describe(keys::ui_opt::AO_WRITE_TO_DESCRIPTION)
                .page(Page::OptOperations),
            "opt_bake_ao_target",
            labels::ao_target(edited.target),
            |ui| {
                for target in AoTarget::ALL {
                    ui.selectable_value(&mut edited.target, target, labels::ao_target(target));
                }
            },
        );
        if edited.target.is_rgb() {
            labeled_checkbox(
                ui,
                Tip::new(keys::ui_opt::AO_SRGB)
                    .describe(keys::ui_opt::AO_SRGB_DESCRIPTION)
                    .page(Page::OptAo),
                &mut edited.srgb,
            );
        }
    });

    ui.add_space(size::PANEL_ROW_GAP);
    ui.label(egui::RichText::from(keys::ui_opt::BAKE_AO_EXPLAINED).color(color::TEXT_MUTED));

    (edited != params).then_some(edited)
}

/// Which mesh a bake measures: the level, or the model as it came in.
fn bake_source_row(ui: &mut egui::Ui, id: &str, source: &mut BakeSource, page: Page) {
    labeled_combo(
        ui,
        Tip::new(keys::ui_opt::BAKE_SOURCE)
            .describe(keys::ui_opt::BAKE_SOURCE_DESCRIPTION)
            .page(page),
        id,
        labels::bake_source(*source),
        |ui| {
            for option in BakeSource::ALL {
                ui.selectable_value(source, option, labels::bake_source(option));
            }
        },
    );
}

fn hull_params(ui: &mut egui::Ui, params: &HullParams) -> Option<HullParams> {
    use super::aud_inspector::{committed_text_edit, parse_patterns};
    let mut edited = params.clone();
    panel_grid(ui, "opt_hull", |ui| {
        crate::widgets::grid_label_tip(
            ui,
            Tip::new(keys::ui_opt::HULL_OBJECTS)
                .describe(keys::ui_opt::HULL_OBJECTS_DESCRIPTION)
                .page(Page::OptHull),
        );
        crate::widgets::grid_control(ui, |ui| {
            let mut valid = true;
            let (response, finished) = committed_text_edit(
                ui,
                egui::Id::new("opt_hull_objects"),
                &edited.objects.join(", "),
                |text| {
                    valid = review_model::glob::GlobSet::new(&parse_patterns(text)).is_ok();
                    egui::TextEdit::singleline(text)
                        .hint_text(keys::ui_audit::PATTERN_HINT)
                        .desired_width(f32::INFINITY)
                        .text_color(if valid {
                            color::TEXT_BODY
                        } else {
                            color::SEVERITY_ERROR
                        })
                },
            );
            if !valid {
                let _ = response.on_hover_text(keys::ui_audit::PATTERN_INVALID);
            }
            if let Some(text) = finished {
                edited.objects = parse_patterns(&text);
            }
        });
        ui.end_row();
        labeled_slider_with_value(
            ui,
            Tip::new(keys::ui_opt::HULL_MAX_VERTICES)
                .describe(keys::ui_opt::HULL_MAX_VERTICES_DESCRIPTION)
                .page(Page::OptHull),
            &mut edited.max_vertices,
            review_optimize::stack::HULL_VERTICES_MIN..=review_optimize::stack::HULL_VERTICES_MAX,
            0,
        );
    });
    (edited != *params).then_some(edited)
}

fn bake_wear_params(ui: &mut egui::Ui, params: BakeWearParams) -> Option<BakeWearParams> {
    let mut edited = params;
    panel_grid(ui, "opt_bake_wear", |ui| {
        bake_source_row(
            ui,
            "opt_bake_wear_source",
            &mut edited.source,
            Page::OptWear,
        );
        labeled_slider_with_value(
            ui,
            Tip::new(keys::ui_opt::WEAR_ANGLE_START)
                .describe(keys::ui_opt::WEAR_ANGLE_START_DESCRIPTION)
                .page(Page::OptWear),
            &mut edited.angle_start,
            WEAR_ANGLE_MIN..=WEAR_ANGLE_MAX,
            0,
        );
        labeled_slider_with_value(
            ui,
            Tip::new(keys::ui_opt::WEAR_ANGLE_FULL)
                .describe(keys::ui_opt::WEAR_ANGLE_FULL_DESCRIPTION)
                .page(Page::OptWear),
            &mut edited.angle_full,
            WEAR_ANGLE_MIN..=WEAR_ANGLE_MAX,
            0,
        );
        labeled_slider_with_value(
            ui,
            Tip::new(keys::ui_opt::WEAR_FALLOFF)
                .describe(keys::ui_opt::WEAR_FALLOFF_DESCRIPTION)
                .page(Page::OptWear),
            &mut edited.falloff,
            WEAR_FALLOFF_MIN..=WEAR_FALLOFF_MAX,
            2,
        );
        labeled_checkbox(
            ui,
            Tip::new(keys::ui_opt::WEAR_CONCAVE)
                .describe(keys::ui_opt::WEAR_CONCAVE_DESCRIPTION)
                .page(Page::OptWear),
            &mut edited.concave,
        );
        labeled_combo(
            ui,
            Tip::new(keys::ui_opt::AO_WRITE_TO).page(Page::OptWear),
            "opt_bake_wear_target",
            labels::ao_target(edited.target),
            |ui| {
                for target in AoTarget::ALL {
                    ui.selectable_value(&mut edited.target, target, labels::ao_target(target));
                }
            },
        );
        if edited.target.is_rgb() {
            labeled_checkbox(
                ui,
                Tip::new(keys::ui_opt::AO_SRGB)
                    .describe(keys::ui_opt::AO_SRGB_DESCRIPTION)
                    .page(Page::OptWear),
                &mut edited.srgb,
            );
        }
    });
    // An angle range the wrong way round wears nothing, which is never meant.
    if edited.angle_full < edited.angle_start {
        if edited.angle_full != params.angle_full {
            edited.angle_start = edited.angle_full;
        } else {
            edited.angle_full = edited.angle_start;
        }
    }
    (edited != params).then_some(edited)
}

/// The simplifier configuration both simplifying operations share: algorithm,
/// attribute weights and the option flags. `id` prefixes the widget ids so the
/// two operations' grids never collide.
///
/// One editor rather than two copies, matching the single [`SimplifySettings`]
/// the two parameter structs embed — a setting added there shows up in both.
fn simplify_settings(ui: &mut egui::Ui, id: &str, edited: &mut SimplifySettings) {
    panel_grid(ui, &format!("{id}_algorithm"), |ui| {
        labeled_combo(
            ui,
            Tip::new(keys::ui_opt::ALGORITHM)
                .describe(keys::ui_opt::ALGORITHM_DESCRIPTION)
                .page(Page::OptOperations),
            &format!("{id}_algo"),
            labels::simplify_algorithm(edited.algorithm),
            |ui| {
                for algorithm in SimplifyAlgorithm::ALL {
                    ui.selectable_value(
                        &mut edited.algorithm,
                        algorithm,
                        labels::simplify_algorithm(algorithm),
                    );
                }
            },
        );
    });

    if edited.algorithm == SimplifyAlgorithm::WithAttributes {
        ui.add_space(size::PANEL_ROW_GAP);
        tip(
            ui.label(keys::ui_opt::ATTRIBUTE_WEIGHTS),
            Tip::new(keys::ui_opt::ATTRIBUTE_WEIGHTS)
                .describe(keys::ui_opt::ATTRIBUTE_WEIGHTS_DESCRIPTION)
                .page(Page::OptSimplify),
        );
        panel_grid(ui, &format!("{id}_weights"), |ui| {
            let AttributeWeights {
                normal,
                uv,
                color: vertex_color,
            } = &mut edited.attribute_weights;
            labeled_slider_with_value(
                ui,
                Tip::new(keys::ui_opt::NORMALS).page(Page::OptOperations),
                normal,
                ATTRIBUTE_WEIGHT_MIN..=ATTRIBUTE_WEIGHT_MAX,
                2,
            );
            labeled_slider_with_value(
                ui,
                Tip::new(keys::ui_opt::UVS).page(Page::OptOperations),
                uv,
                ATTRIBUTE_WEIGHT_MIN..=ATTRIBUTE_WEIGHT_MAX,
                2,
            );
            labeled_slider_with_value(
                ui,
                Tip::new(keys::ui_opt::COLORS).page(Page::OptOperations),
                vertex_color,
                ATTRIBUTE_WEIGHT_MIN..=ATTRIBUTE_WEIGHT_MAX,
                2,
            );
        });
        ui.label(
            egui::RichText::from(keys::ui_opt::ATTRIBUTE_WEIGHTS_EXPLAINED)
                .color(color::TEXT_MUTED),
        );
    }

    // The sloppy simplifier ignores topology by construction, which is what most
    // of these flags exist to modulate — showing them would imply an effect they
    // do not have.
    if edited.algorithm != SimplifyAlgorithm::Sloppy {
        ui.add_space(size::PANEL_ROW_GAP);
        ui.collapsing(keys::ui_opt::SIMPLIFIER_OPTIONS, |ui| {
            let SimplifyFlags {
                lock_border,
                error_absolute,
                prune,
                regularize,
                regularize_light,
                permissive,
                preserve_folds,
                clamp_attribute_error,
            } = &mut edited.flags;
            ui.checkbox(lock_border, keys::ui_opt::LOCK_BORDER)
                .on_hover_text(keys::ui_opt::LOCK_BORDER_DESCRIPTION);
            ui.checkbox(error_absolute, keys::ui_opt::ABSOLUTE_ERROR)
                .on_hover_text(keys::ui_opt::ABSOLUTE_ERROR_DESCRIPTION);
            ui.checkbox(prune, keys::ui_opt::PRUNE_WHILE_SIMPLIFYING)
                .on_hover_text(keys::ui_opt::PRUNE_WHILE_SIMPLIFYING_DESCRIPTION);
            ui.checkbox(regularize, keys::ui_opt::REGULARIZE)
                .on_hover_text(keys::ui_opt::REGULARIZE_DESCRIPTION);
            ui.checkbox(regularize_light, keys::ui_opt::REGULARIZE_LIGHT)
                .on_hover_text(keys::ui_opt::REGULARIZE_LIGHT_DESCRIPTION);
            ui.checkbox(permissive, keys::ui_opt::COLLAPSE_ACROSS_SEAMS)
                .on_hover_text(keys::ui_opt::COLLAPSE_ACROSS_SEAMS_DESCRIPTION);
            ui.checkbox(preserve_folds, keys::ui_opt::PRESERVE_FOLDS)
                .on_hover_text(keys::ui_opt::PRESERVE_FOLDS_DESCRIPTION);
            // Clamping bounds the *attribute* error, which only the
            // attribute-aware simplifier computes; elsewhere it would do nothing.
            if edited.algorithm == SimplifyAlgorithm::WithAttributes {
                ui.checkbox(clamp_attribute_error, keys::ui_opt::CLAMP_ATTRIBUTE_ERROR)
                    .on_hover_text(keys::ui_opt::CLAMP_ATTRIBUTE_ERROR_DESCRIPTION);
            }
        });
    }

    // Limits on how much an object may change, whatever its target: off unless
    // switched on, for every algorithm.
    ui.add_space(size::PANEL_ROW_GAP);
    ui.collapsing(keys::ui_opt::QUALITY_LIMITS, |ui| {
        panel_grid(ui, &format!("{id}_limits"), |ui| {
            quality_limit(
                ui,
                Tip::new(keys::ui_opt::LIMIT_VOLUME)
                    .describe(keys::ui_opt::LIMIT_VOLUME_DESCRIPTION)
                    .page(Page::OptSimplify),
                Tip::new(keys::ui_opt::MAX_VOLUME_CHANGE)
                    .describe(keys::ui_opt::MAX_VOLUME_CHANGE_DESCRIPTION)
                    .page(Page::OptSimplify),
                &mut edited.limits.max_volume_change,
                VOLUME_CHANGE_MIN..=VOLUME_CHANGE_MAX,
                0.05,
            );
            quality_limit(
                ui,
                Tip::new(keys::ui_opt::LIMIT_DISTANCE)
                    .describe(keys::ui_opt::LIMIT_DISTANCE_DESCRIPTION)
                    .page(Page::OptSimplify),
                Tip::new(keys::ui_opt::MAX_DISTANCE)
                    .describe(keys::ui_opt::MAX_DISTANCE_DESCRIPTION)
                    .page(Page::OptSimplify),
                &mut edited.limits.max_distance,
                DISTANCE_LIMIT_MIN..=DISTANCE_LIMIT_MAX,
                0.01,
            );
        });
    });
}

/// One quality limit as the Inspector edits it: a checkbox that switches it on at
/// `default`, and while it is on, its percentage on a log rail — a limit can be a
/// hundredth of a percent or a half.
///
/// The share is written back only when the slider moved: turning it into a
/// percentage and back every frame would nudge the `f32`, and the stack would
/// count as edited — and re-run — on every frame.
fn quality_limit(
    ui: &mut egui::Ui,
    switch: Tip,
    amount: Tip,
    limit: &mut Option<f32>,
    range: std::ops::RangeInclusive<f32>,
    default: f32,
) {
    let mut on = limit.is_some();
    labeled_checkbox(ui, switch, &mut on);
    match (on, *limit) {
        (false, Some(_)) => *limit = None,
        (true, None) => *limit = Some(default),
        _ => {}
    }
    if let Some(share) = limit {
        let mut percent = *share * 100.0;
        if labeled_log_slider_with_value(
            ui,
            amount,
            &mut percent,
            range.start() * 100.0..=range.end() * 100.0,
            2,
        ) {
            *share = percent / 100.0;
        }
    }
}

/// The target / error-limit pair one simplify run is aimed at — a LOD level's
/// target, and equally the Reduce operation's single one. The target is a share or
/// a count, both kept, so switching between them gives back what each was.
fn simplify_target(ui: &mut egui::Ui, id: &str, target: &mut LodLevel) {
    panel_grid(ui, id, |ui| {
        labeled_combo(
            ui,
            Tip::new(keys::ui_opt::TARGET_MODE)
                .describe(keys::ui_opt::TARGET_MODE_DESCRIPTION)
                .page(Page::OptSimplify),
            &format!("{id}_mode"),
            labels::target_mode(target.target_mode),
            |ui| {
                for mode in TargetMode::ALL {
                    ui.selectable_value(&mut target.target_mode, mode, labels::target_mode(mode));
                }
            },
        );
        match target.target_mode {
            TargetMode::Ratio => {
                labeled_slider_with_value(
                    ui,
                    Tip::new(keys::ui_opt::TRIANGLES)
                        .describe(keys::ui_opt::TRIANGLES_DESCRIPTION)
                        .page(Page::OptOperations),
                    &mut target.target_ratio,
                    LOD_RATIO_MIN..=LOD_RATIO_MAX,
                    3,
                );
            }
            // A log rail: a budget is 300 for a bolt and 30,000 for a hero, and on a
            // linear one every small part would sit in the first pixel. The box
            // beside it takes a typed number.
            TargetMode::Triangles => {
                labeled_log_slider_with_value(
                    ui,
                    Tip::new(keys::ui_opt::TRIANGLE_COUNT)
                        .describe(keys::ui_opt::TRIANGLE_COUNT_DESCRIPTION)
                        .page(Page::OptSimplify),
                    &mut target.target_triangles,
                    LOD_TRIANGLES_MIN..=LOD_TRIANGLES_MAX,
                    0,
                );
            }
        }
        labeled_slider_with_value(
            ui,
            Tip::new(keys::ui_opt::ERROR_LIMIT)
                .describe(keys::ui_opt::ERROR_LIMIT_DESCRIPTION)
                .page(Page::OptOperations),
            &mut target.target_error,
            LOD_ERROR_MIN..=LOD_ERROR_MAX,
            4,
        );
    });
}

/// The in-place Reduce editor: the shared simplifier settings against one target.
fn reduce_params(ui: &mut egui::Ui, params: ReduceParams) -> Option<ReduceParams> {
    let mut edited = params;

    simplify_settings(ui, "opt_reduce", &mut edited.simplify);

    ui.add_space(size::PANEL_ROW_GAP);
    ui.separator();
    ui.add_space(size::PANEL_ROW_GAP);
    simplify_target(ui, "opt_reduce_target", &mut edited.target);

    ui.add_space(size::PANEL_ROW_GAP);
    ui.label(egui::RichText::from(keys::ui_opt::REDUCE_EXPLAINED).color(color::TEXT_MUTED));

    (edited != params).then_some(edited)
}

/// The LOD chain editor: the shared simplifier settings, plus the per-level
/// ratio / error rows.
fn lod_params(ui: &mut egui::Ui, params: LodParams) -> Option<LodParams> {
    let mut edited = params.clone();

    simplify_settings(ui, "opt_lod", &mut edited.simplify);

    ui.add_space(size::PANEL_ROW_GAP);
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(keys::ui_opt::LEVELS);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(edited.levels.len() < MAX_LOD_LEVELS, egui::Button::new("+"))
                .on_hover_text(keys::ui_opt::ADD_LEVEL)
                .clicked()
            {
                // A new level continues the halving the defaults establish, so
                // adding one lands somewhere sensible rather than at a duplicate
                // of the level above it. It keeps that level's error limit and
                // kind of target: the defaults hold one limit down the chain.
                let previous = edited.levels.last().copied().unwrap_or(LodLevel::default());
                // Halving either kind of target: the level continues the chain in
                // whichever the one above it is given in.
                edited.levels.push(LodLevel {
                    target_ratio: (previous.target_ratio * 0.5).max(LOD_RATIO_MIN),
                    target_triangles: (previous.target_triangles / 2).max(LOD_TRIANGLES_MIN),
                    ..previous
                });
            }
        });
    });

    let mut remove: Option<usize> = None;
    for (index, level) in edited.levels.iter_mut().enumerate() {
        ui.add_space(size::PANEL_ROW_GAP);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(keys::ui_opt::lod_level((index + 1) as f64))
                    .color(color::TEXT_MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(theme::REMOVE_GLYPH)
                    .on_hover_text(keys::ui_opt::REMOVE_LEVEL)
                    .clicked()
                {
                    remove = Some(index);
                }
            });
        });
        simplify_target(ui, &format!("opt_lod_level_{index}"), level);
    }
    if let Some(index) = remove {
        edited.levels.remove(index);
    }

    ui.add_space(size::PANEL_ROW_GAP);
    ui.label(egui::RichText::from(keys::ui_opt::LOD_EXPLAINED).color(color::TEXT_MUTED));

    (edited != params).then_some(edited)
}

/// The export settings and the Export action.
fn export_body(ui: &mut egui::Ui, state: &mut UiState) -> Option<OptIntent> {
    ui.heading(keys::ui_opt::EXPORT_SETTINGS);
    ui.add_space(size::PANEL_ROW_GAP);
    ui.label(egui::RichText::from(keys::ui_opt::EXPORT_EXPLAINED).color(color::TEXT_MUTED));
    ui.add_space(size::PANEL_ROW_GAP);
    ui.separator();

    let current = state.opt.stack.export;
    let mut edited = current;

    panel_grid(ui, "opt_export", |ui| {
        labeled_combo(
            ui,
            Tip::new(keys::ui_opt::EXPORT_FILE_TYPE).page(Page::OptOperations),
            "opt_export_file_type",
            labels::export_file_type(edited.file_type),
            |ui| {
                for file_type in ExportFileType::ALL {
                    ui.selectable_value(
                        &mut edited.file_type,
                        file_type,
                        labels::export_file_type(file_type),
                    );
                }
            },
        );
        labeled_combo(
            ui,
            Tip::new(keys::ui_opt::EXPORT_PACKAGING).page(Page::OptOperations),
            "opt_export_packaging",
            labels::lod_packaging(edited.packaging),
            |ui| {
                for packaging in LodPackaging::ALL {
                    ui.selectable_value(
                        &mut edited.packaging,
                        packaging,
                        labels::lod_packaging(packaging),
                    );
                }
            },
        );
        labeled_combo(
            ui,
            Tip::new(keys::ui_opt::EXPORT_HIERARCHY).page(Page::OptOperations),
            "opt_export_hierarchy",
            labels::hierarchy_mode(edited.hierarchy),
            |ui| {
                for hierarchy in HierarchyMode::ALL {
                    ui.selectable_value(
                        &mut edited.hierarchy,
                        hierarchy,
                        labels::hierarchy_mode(hierarchy),
                    );
                }
            },
        );
        // Each file type's own settings: FBX's encoding; glb's custom attributes.
        match edited.file_type {
            ExportFileType::Fbx => labeled_combo(
                ui,
                Tip::new(keys::ui_opt::EXPORT_FORMAT).page(Page::OptOperations),
                "opt_export_format",
                labels::fbx_format(edited.format),
                |ui| {
                    for format in FbxFormat::ALL {
                        ui.selectable_value(&mut edited.format, format, labels::fbx_format(format));
                    }
                },
            ),
            ExportFileType::Glb => {
                labeled_combo(
                    ui,
                    Tip::new(keys::ui_opt::EXPORT_TANGENTS)
                        .describe(keys::ui_opt::EXPORT_TANGENTS_DESCRIPTION)
                        .page(Page::OptOperations),
                    "opt_export_tangents",
                    labels::glb_tangents(edited.glb_tangents),
                    |ui| {
                        for tangents in GlbTangents::ALL {
                            ui.selectable_value(
                                &mut edited.glb_tangents,
                                tangents,
                                labels::glb_tangents(tangents),
                            );
                        }
                    },
                );
                let mut keep = !edited.strip_custom_attributes;
                labeled_checkbox(
                    ui,
                    Tip::new(keys::ui_opt::EXPORT_KEEP_CUSTOM)
                        .describe(keys::ui_opt::EXPORT_KEEP_CUSTOM_DESCRIPTION)
                        .page(Page::OptOperations),
                    &mut keep,
                );
                edited.strip_custom_attributes = !keep;
            }
        }
    });

    if edited != current {
        state.opt.edit_stack(|stack| stack.export = edited);
    }

    ui.add_space(size::PANEL_ROW_GAP);
    describe_export(ui, edited, state.opt.level_count());

    ui.add_space(size::PANEL_ROW_GAP);
    ui.separator();
    ui.add_space(size::PANEL_ROW_GAP);

    let ready = state.opt.has_result() && !state.opt.processing;
    let response = ui.add_enabled(
        ready,
        wide_button(keys::ui_opt::EXPORT_BUTTON, ui.available_width()),
    );
    let response = if state.opt.processing {
        response.on_disabled_hover_text(keys::ui_opt::EXPORT_WAITING)
    } else if !state.opt.has_result() {
        response.on_disabled_hover_ui(|ui| {
            tip_body(
                ui,
                keys::ui_opt::EXPORT_NOTHING,
                keys::ui_opt::EXPORT_NOTHING_DESCRIPTION,
            );
        })
    } else {
        response
    };
    response.clicked().then_some(OptIntent::Export)
}

/// Spell out what the chosen settings will actually produce, so the choice is
/// checkable before a file dialog opens rather than after.
fn describe_export(ui: &mut egui::Ui, options: ExportOptions, levels: usize) {
    let levels = levels.max(1);
    // With no LOD chain there is only the processed mesh, which stands in for the
    // source asset — so neither packaging has anything to suffix, and naming a
    // "_LOD0" would describe a file the export does not write.
    let packaging = match options.packaging {
        _ if levels == 1 => {
            review_localization::tr(keys::ui_opt::PACKAGING_SINGLE_ONE_LEVEL).into_owned()
        }
        LodPackaging::SingleFileSuffixed => {
            keys::ui_opt::packaging_single_explained((levels - 1) as f64)
        }
        LodPackaging::FilePerLod => keys::ui_opt::packaging_file_per_lod_explained(levels as f64),
    };
    let hierarchy = match options.hierarchy {
        HierarchyMode::Rebuild => keys::ui_opt::HIERARCHY_REBUILD_EXPLAINED,
        HierarchyMode::FlatBaked => keys::ui_opt::HIERARCHY_FLAT_EXPLAINED,
    };
    ui.label(egui::RichText::new(packaging).color(color::TEXT_MUTED));
    ui.label(egui::RichText::from(hierarchy).color(color::TEXT_MUTED));
}

/// With no stack row selected, the Inspector offers the selected object's
/// per-object overrides — the other half of what Opt does with a selection.
fn node_body(ui: &mut egui::Ui, state: &mut UiState, model: &ModelData) {
    let Selection::Node(index) = state.selection else {
        ui.weak(keys::ui_opt::SELECT_SOMETHING);
        return;
    };

    let name = model
        .nodes
        .get(index)
        .map(|node| node.name.clone())
        .unwrap_or_else(|| keys::ui_opt::unnamed_node(index as f64));
    ui.heading(name);
    ui.add_space(size::PANEL_ROW_GAP);
    ui.label(egui::RichText::from(keys::ui_opt::OBJECT_OVERRIDES).color(color::TEXT_MUTED));
    ui.add_space(size::PANEL_ROW_GAP);
    ui.separator();
    ui.add_space(size::PANEL_ROW_GAP);

    let mut exclude = state
        .opt
        .stack
        .node_override(index)
        .is_some_and(|entry| entry.exclude);
    if ui
        .checkbox(&mut exclude, keys::ui_opt::EXCLUDE)
        .on_hover_ui(|ui| tip_body(ui, keys::ui_opt::EXCLUDE, keys::ui_opt::EXCLUDE_DESCRIPTION))
        .changed()
    {
        state.opt.edit_stack(|stack| {
            stack.node_override_mut(index).exclude = exclude;
            stack.prune_overrides();
        });
    }
}
