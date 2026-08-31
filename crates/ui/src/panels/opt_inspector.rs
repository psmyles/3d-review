//! The Inspector's Opt-workspace bodies: parameters for the operation selected
//! in the stack pane, the export settings, and per-object overrides.
//!
//! Every editor follows the same shape — copy the stored parameters out, let the
//! widgets edit the copy, and write back **only if the copy differs**. Writing
//! unconditionally would bump the stack revision every frame and reprocess the
//! mesh continuously, which is both the wrong result and an easy mistake to make
//! invisibly.

use review_model::ModelData;
use review_optimize::{
    AoQuality, AoTarget, AttributeWeights, BakeAoParams, ExportOptions, FbxFormat, HierarchyMode,
    LodLevel, LodPackaging, LodParams, OpKind, ReduceParams, SimplifyAlgorithm, SimplifyFlags,
    SimplifySettings, WeldParams,
};
use review_render::Selection;

use crate::opt_state::{OptIntent, StackItem};
use crate::state::{
    UiState,
    range::{
        AO_BAKE_DISTANCE_MAX, AO_BAKE_DISTANCE_MIN, AO_BAKE_INTENSITY_MAX, AO_BAKE_INTENSITY_MIN,
        ATTRIBUTE_WEIGHT_MAX, ATTRIBUTE_WEIGHT_MIN, LOD_ERROR_MAX, LOD_ERROR_MIN, LOD_RATIO_MAX,
        LOD_RATIO_MIN, OVERDRAW_THRESHOLD_MAX, OVERDRAW_THRESHOLD_MIN, PRUNE_THRESHOLD_MAX,
        PRUNE_THRESHOLD_MIN, WELD_TOLERANCE_MAX, WELD_TOLERANCE_MIN,
    },
};
use crate::theme::{color, size};
use crate::widgets::{
    labeled_checkbox, labeled_combo, labeled_slider_with_value, panel_grid, wide_button,
};

/// The most levels a LOD chain may hold. Past this the chain stops being a
/// pipeline decision and starts being an experiment; the cost is one full
/// simplify of the whole model per level, per edit.
const MAX_LOD_LEVELS: usize = 8;

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
            Some(StackItem::Op(id)) => operation_body(ui, state, id),
            Some(StackItem::ExportSettings) => out.intent = export_body(ui, state),
            None => node_body(ui, state, model),
        });

    // A slider handle being dragged keeps egui's pointer captured; the 3D
    // viewport is not an egui widget, so orbiting the camera never sets this.
    out.edit_active = ui.ctx().is_using_pointer();
    out
}

/// Parameters for the selected operation.
fn operation_body(ui: &mut egui::Ui, state: &mut UiState, id: u64) {
    let Some(op) = state.opt.stack.op(id) else {
        ui.weak("This operation no longer exists.");
        return;
    };
    let kind = op.kind.clone();

    ui.heading(kind.label());
    ui.add_space(size::PANEL_ROW_GAP);
    ui.label(egui::RichText::new(kind.description()).color(color::TEXT_MUTED));
    ui.add_space(size::PANEL_ROW_GAP);
    ui.separator();

    let edited = match kind.clone() {
        OpKind::Weld(params) => weld_params(ui, params).map(OpKind::Weld),
        OpKind::PruneComponents { error } => prune_params(ui, error),
        OpKind::Overdraw { threshold } => overdraw_params(ui, threshold),
        OpKind::Reduce(params) => reduce_params(ui, params).map(OpKind::Reduce),
        OpKind::SimplifyLod(params) => lod_params(ui, params).map(OpKind::SimplifyLod),
        OpKind::BakeAo(params) => bake_ao_params(ui, params).map(OpKind::BakeAo),
        // These three have nothing to configure — meshoptimizer exposes no knobs
        // for them, and inventing some would be worse than an honest note.
        OpKind::FilterTriangles | OpKind::VertexCache | OpKind::VertexFetch => {
            ui.add_space(size::PANEL_ROW_GAP);
            ui.weak("This operation has no settings.");
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
            "Tolerance",
            &mut edited.attribute_tolerance,
            WELD_TOLERANCE_MIN..=WELD_TOLERANCE_MAX,
            4,
        );
        labeled_checkbox(ui, "Compare normals", &mut edited.compare_normals);
        labeled_checkbox(ui, "Compare UVs", &mut edited.compare_uvs);
        labeled_checkbox(ui, "Compare colors", &mut edited.compare_colors);
    });

    ui.add_space(size::PANEL_ROW_GAP);
    ui.label(
        egui::RichText::new(
            "Positions must always match exactly; the tolerance applies to the \
             compared attributes. Unchecking one merges across that kind of seam - \
             unchecking normals, for instance, welds a flat-shaded mesh's hard edges \
             and flattens their shading.",
        )
        .color(color::TEXT_MUTED),
    );

    (edited != params).then_some(edited)
}

fn prune_params(ui: &mut egui::Ui, error: f32) -> Option<OpKind> {
    let mut edited = error;
    panel_grid(ui, "opt_prune", |ui| {
        labeled_slider_with_value(
            ui,
            "Size threshold",
            &mut edited,
            PRUNE_THRESHOLD_MIN..=PRUNE_THRESHOLD_MAX,
            3,
        );
    });
    ui.add_space(size::PANEL_ROW_GAP);
    ui.label(
        egui::RichText::new(
            "A fraction of the mesh's overall size. Disconnected pieces smaller \
             than this are removed.",
        )
        .color(color::TEXT_MUTED),
    );

    (edited != error).then_some(OpKind::PruneComponents { error: edited })
}

fn overdraw_params(ui: &mut egui::Ui, threshold: f32) -> Option<OpKind> {
    let mut edited = threshold;
    panel_grid(ui, "opt_overdraw", |ui| {
        labeled_slider_with_value(
            ui,
            "Cache tolerance",
            &mut edited,
            OVERDRAW_THRESHOLD_MIN..=OVERDRAW_THRESHOLD_MAX,
            2,
        );
    });
    ui.add_space(size::PANEL_ROW_GAP);
    ui.label(
        egui::RichText::new(
            "How much vertex-cache efficiency may be given up to reduce overdraw. \
             1.05 allows a 5% regression; 1.0 forbids any.",
        )
        .color(color::TEXT_MUTED),
    );

    (edited - threshold)
        .abs()
        .gt(&f32::EPSILON)
        .then_some(OpKind::Overdraw { threshold: edited })
}

/// The AO bake editor.
fn bake_ao_params(ui: &mut egui::Ui, params: BakeAoParams) -> Option<BakeAoParams> {
    let mut edited = params;
    panel_grid(ui, "opt_bake_ao", |ui| {
        labeled_combo(
            ui,
            "Quality",
            "opt_bake_ao_quality",
            edited.quality.label(),
            |ui| {
                for quality in AoQuality::ALL {
                    ui.selectable_value(&mut edited.quality, quality, quality.label());
                }
            },
        );
        labeled_slider_with_value(
            ui,
            "Max distance",
            &mut edited.max_distance,
            AO_BAKE_DISTANCE_MIN..=AO_BAKE_DISTANCE_MAX,
            2,
        );
        labeled_slider_with_value(
            ui,
            "Intensity",
            &mut edited.intensity,
            AO_BAKE_INTENSITY_MIN..=AO_BAKE_INTENSITY_MAX,
            2,
        );
        labeled_combo(
            ui,
            "Write to",
            "opt_bake_ao_target",
            edited.target.label(),
            |ui| {
                for target in AoTarget::ALL {
                    ui.selectable_value(&mut edited.target, target, target.label());
                }
            },
        );
        if edited.target.is_rgb() {
            labeled_checkbox(ui, "sRGB encode", &mut edited.srgb);
        }
    });

    ui.add_space(size::PANEL_ROW_GAP);
    ui.label(
        egui::RichText::new(
            "Objects named *_LOD<n> bake only against their own LOD's geometry \
             (plus objects with no LOD suffix), so a whole visible LOD chain bakes \
             in one run. Hidden objects neither occlude nor bake - hide collision \
             shells in the Outliner first. Max distance is how far a surface can \
             be and still occlude, in world meters; 0 is unlimited. Intensity is a \
             power on the visibility - above 1 darkens. The alpha channel is \
             always written linear. Preview via the Vertex Colors material. If a \
             Preserve Attributes simplify runs after this bake, raise its Colors \
             weight above zero or it will ignore the AO.",
        )
        .color(color::TEXT_MUTED),
    );

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
            "Algorithm",
            &format!("{id}_algo"),
            edited.algorithm.label(),
            |ui| {
                for algorithm in SimplifyAlgorithm::ALL {
                    ui.selectable_value(&mut edited.algorithm, algorithm, algorithm.label());
                }
            },
        );
    });

    if edited.algorithm == SimplifyAlgorithm::WithAttributes {
        ui.add_space(size::PANEL_ROW_GAP);
        ui.label("Attribute weights");
        panel_grid(ui, &format!("{id}_weights"), |ui| {
            let AttributeWeights {
                normal,
                uv,
                color: vertex_color,
            } = &mut edited.attribute_weights;
            labeled_slider_with_value(
                ui,
                "Normals",
                normal,
                ATTRIBUTE_WEIGHT_MIN..=ATTRIBUTE_WEIGHT_MAX,
                2,
            );
            labeled_slider_with_value(
                ui,
                "UVs",
                uv,
                ATTRIBUTE_WEIGHT_MIN..=ATTRIBUTE_WEIGHT_MAX,
                2,
            );
            labeled_slider_with_value(
                ui,
                "Colors",
                vertex_color,
                ATTRIBUTE_WEIGHT_MIN..=ATTRIBUTE_WEIGHT_MAX,
                2,
            );
        });
        ui.label(
            egui::RichText::new(
                "Higher weights protect that attribute at the cost of geometric \
                 accuracy. Zero ignores it entirely.",
            )
            .color(color::TEXT_MUTED),
        );
    }

    // The sloppy simplifier ignores topology by construction, which is what most
    // of these flags exist to modulate — showing them would imply an effect they
    // do not have.
    if edited.algorithm != SimplifyAlgorithm::Sloppy {
        ui.add_space(size::PANEL_ROW_GAP);
        ui.collapsing("Simplifier options", |ui| {
            let SimplifyFlags {
                lock_border,
                error_absolute,
                prune,
                regularize,
                regularize_light,
                permissive,
            } = &mut edited.flags;
            ui.checkbox(lock_border, "Lock border")
                .on_hover_text("Never move vertices on an open boundary");
            ui.checkbox(error_absolute, "Absolute error").on_hover_text(
                "Read the error limit in world units, not as a fraction of the mesh size",
            );
            ui.checkbox(prune, "Prune while simplifying")
                .on_hover_text("Let the simplifier delete disconnected parts as it goes");
            ui.checkbox(regularize, "Regularize")
                .on_hover_text("Even out triangle size and shape, at some cost to accuracy");
            ui.checkbox(regularize_light, "Regularize (light)")
                .on_hover_text("A gentler regularization");
            ui.checkbox(permissive, "Collapse across seams")
                .on_hover_text("Allow collapses across UV and normal discontinuities");
        });
    }
}

/// The ratio / error-limit pair one simplify run is aimed at — a LOD level's
/// target, and equally the Reduce operation's single one.
fn simplify_target(ui: &mut egui::Ui, id: &str, target: &mut LodLevel) {
    panel_grid(ui, id, |ui| {
        labeled_slider_with_value(
            ui,
            "Triangles",
            &mut target.target_ratio,
            LOD_RATIO_MIN..=LOD_RATIO_MAX,
            3,
        );
        labeled_slider_with_value(
            ui,
            "Error limit",
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
    ui.label(
        egui::RichText::new(
            "Triangles is a fraction of this object's count at this point in the \
             stack; the simplifier stops short of it rather than exceed the error \
             limit. Unlike Generate LODs this rewrites the mesh itself - every \
             operation below it works on the reduced geometry, and an export with \
             no LOD chain writes it in the source mesh's place.",
        )
        .color(color::TEXT_MUTED),
    );

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
        ui.label("Levels");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(edited.levels.len() < MAX_LOD_LEVELS, egui::Button::new("+"))
                .on_hover_text("Add a level")
                .clicked()
            {
                // A new level continues the halving the defaults establish, so
                // adding one lands somewhere sensible rather than at a duplicate
                // of the level above it.
                let previous = edited.levels.last().copied().unwrap_or(LodLevel::default());
                edited.levels.push(LodLevel {
                    target_ratio: (previous.target_ratio * 0.5).max(0.01),
                    target_error: (previous.target_error * 2.0).min(1.0),
                });
            }
        });
    });

    let mut remove: Option<usize> = None;
    for (index, level) in edited.levels.iter_mut().enumerate() {
        ui.add_space(size::PANEL_ROW_GAP);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("LOD {}", index + 1)).color(color::TEXT_MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("X").on_hover_text("Remove this level").clicked() {
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
    ui.label(
        egui::RichText::new(
            "Triangles is a fraction of this object's count at this point in the \
             stack. Every level is simplified from that same mesh, not from the \
             level above, so one level's error never compounds into the next. The \
             simplifier stops short of the target rather than exceed the error limit.",
        )
        .color(color::TEXT_MUTED),
    );

    (edited != params).then_some(edited)
}

/// The export settings and the Export action.
fn export_body(ui: &mut egui::Ui, state: &mut UiState) -> Option<OptIntent> {
    ui.heading("Export settings");
    ui.add_space(size::PANEL_ROW_GAP);
    ui.label(
        egui::RichText::new(
            "Where and how the processed mesh - and its LOD chain, if the stack \
             generates one - is written. Output is always triangulated, and carries \
             the materials as imported - material edits made in the viewer stay \
             previews.",
        )
        .color(color::TEXT_MUTED),
    );
    ui.add_space(size::PANEL_ROW_GAP);
    ui.separator();

    let current = state.opt.stack.export;
    let mut edited = current;

    panel_grid(ui, "opt_export", |ui| {
        labeled_combo(
            ui,
            "Packaging",
            "opt_export_packaging",
            edited.packaging.label(),
            |ui| {
                for packaging in LodPackaging::ALL {
                    ui.selectable_value(&mut edited.packaging, packaging, packaging.label());
                }
            },
        );
        labeled_combo(
            ui,
            "Hierarchy",
            "opt_export_hierarchy",
            edited.hierarchy.label(),
            |ui| {
                for hierarchy in HierarchyMode::ALL {
                    ui.selectable_value(&mut edited.hierarchy, hierarchy, hierarchy.label());
                }
            },
        );
        labeled_combo(
            ui,
            "Format",
            "opt_export_format",
            edited.format.label(),
            |ui| {
                for format in FbxFormat::ALL {
                    ui.selectable_value(&mut edited.format, format, format.label());
                }
            },
        );
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
    let response = ui.add_enabled(ready, wide_button("Export…", ui.available_width()));
    let response = if state.opt.processing {
        response.on_disabled_hover_text("Waiting for the current run to finish")
    } else if !state.opt.has_result() {
        response.on_disabled_hover_text("Nothing processed yet - add an operation first")
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
        _ if levels == 1 => "One file, with each mesh under its source name".to_owned(),
        LodPackaging::SingleFileSuffixed => {
            format!(
                "One file, with each mesh repeated as MeshName_LOD0 … _LOD{}",
                levels - 1
            )
        }
        LodPackaging::FilePerLod => format!("{levels} files, one per level (…_LOD0.fbx, …)"),
    };
    let hierarchy = match options.hierarchy {
        HierarchyMode::Rebuild => {
            "The source node hierarchy is rebuilt, geometry moved back into each node's local space."
        }
        HierarchyMode::FlatBaked => {
            "Each mesh becomes a root-level node with world-space geometry."
        }
    };
    ui.label(egui::RichText::new(packaging).color(color::TEXT_MUTED));
    ui.label(egui::RichText::new(hierarchy).color(color::TEXT_MUTED));
}

/// With no stack row selected, the Inspector offers the selected object's
/// per-object overrides — the other half of what Opt does with a selection.
fn node_body(ui: &mut egui::Ui, state: &mut UiState, model: &ModelData) {
    let Selection::Node(index) = state.selection else {
        ui.weak(
            "Select an operation in the stack to edit its settings, or a node in \
             the Outliner to give it its own.",
        );
        return;
    };

    let name = model
        .nodes
        .get(index)
        .map(|node| node.name.clone())
        .unwrap_or_else(|| format!("Node {index}"));
    ui.heading(name);
    ui.add_space(size::PANEL_ROW_GAP);
    ui.label(
        egui::RichText::new("How this object deviates from the global stack.")
            .color(color::TEXT_MUTED),
    );
    ui.add_space(size::PANEL_ROW_GAP);
    ui.separator();
    ui.add_space(size::PANEL_ROW_GAP);

    let mut exclude = state
        .opt
        .stack
        .node_override(index)
        .is_some_and(|entry| entry.exclude);
    if ui
        .checkbox(&mut exclude, "Exclude from optimization")
        .on_hover_text(
            "Pass this object through untouched. It still appears in every LOD \
             level, at full detail.",
        )
        .changed()
    {
        state.opt.edit_stack(|stack| {
            stack.node_override_mut(index).exclude = exclude;
            stack.prune_overrides();
        });
    }
}
