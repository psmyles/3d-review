//! The Inspector's Aud-workspace bodies: a focused finding, the audit profile
//! editor, and — with neither — the report's summary.
//!
//! The profile editor follows the Opt inspector's shape: copy the profile out,
//! let the widgets edit the copy, and write it back **only if it differs**, so
//! an idle frame never bumps the profile revision (which would re-run the audit
//! every frame).

use review_audit::{
    AuditProfile, AuditReport, Axis, Category, Engine, ParamKind, ParamValue, RuleId, RuleResult,
    Severity, Status, param_specs,
};
use review_model::ModelData;

use crate::aud_state::{AuditFocus, AuditIntent};
use crate::audit_labels;
use crate::docs::Page;
use crate::keys;
use crate::keys::ui_audit as k;
use crate::state::UiState;
use crate::theme::{color, size};
use crate::widgets::{Tip, grid_control, grid_label_tip, panel_grid, tip, value_row, wide_button};

/// What the Aud inspector emitted this frame.
#[derive(Debug, Clone, Default)]
pub(crate) struct AudInspectorOutput {
    pub intent: Option<AuditIntent>,
    /// A profile widget is being dragged, so `app` collapses the drag into one
    /// undo step.
    pub edit_active: bool,
}

/// Whether the Aud inspector has something to show — the profile or a focus —
/// rather than handing the Inspector to the scene selection.
pub(crate) fn wants_inspector(state: &UiState) -> bool {
    state.aud.profile_open || state.aud.focus.is_some() || !state.has_selection()
}

pub(crate) fn body(
    ui: &mut egui::Ui,
    state: &mut UiState,
    model: &ModelData,
) -> AudInspectorOutput {
    let mut out = AudInspectorOutput::default();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if state.aud.profile_open {
                out.intent = profile_editor(ui, state);
            } else if let Some(focus) = state.aud.focus {
                finding_body(ui, state, model, focus);
            } else {
                summary_body(ui, state);
            }
        });
    out.edit_active = ui.ctx().egui_is_using_pointer();
    out
}

fn muted_paragraph(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.add(egui::Label::new(egui::RichText::new(text.into()).color(color::TEXT_MUTED)).wrap());
}

fn section(ui: &mut egui::Ui, title: review_localization::Key, text: impl Into<String>) {
    ui.add_space(size::PANEL_ROW_GAP);
    ui.label(egui::RichText::from(title).color(color::TEXT_PRIMARY));
    muted_paragraph(ui, text);
}

/// A severity's glyph and name, as a heading line.
fn severity_line(ui: &mut egui::Ui, severity: Severity) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(size::AUD_GLYPH_COLUMN, size::OUTLINER_ROW_HEIGHT),
            egui::Sense::hover(),
        );
        crate::panels::outliner::paint_severity_glyph(ui.painter(), rect, severity);
        ui.label(
            egui::RichText::from(audit_labels::severity(severity))
                .color(crate::panels::outliner::severity_color(severity)),
        );
    });
}

/// The report's headline counts, with a hint to pick something.
fn summary_body(ui: &mut egui::Ui, state: &UiState) {
    ui.heading(k::SUMMARY_HEADING);
    ui.add_space(size::PANEL_ROW_GAP);
    let Some(report) = &state.aud.report else {
        ui.weak(if state.aud.running {
            k::RUNNING
        } else {
            k::NO_MODEL
        });
        return;
    };
    let summary = &report.summary;
    panel_grid(ui, "aud_summary", |ui| {
        value_row(
            ui,
            Tip::new(k::SUMMARY_ERRORS),
            summary.failed[Severity::Error.index()].to_string(),
        );
        value_row(
            ui,
            Tip::new(k::SUMMARY_WARNINGS),
            summary.failed[Severity::Warning.index()].to_string(),
        );
        value_row(
            ui,
            Tip::new(k::SUMMARY_INFO),
            summary.failed[Severity::Info.index()].to_string(),
        );
        value_row(ui, Tip::new(k::SUMMARY_PASSED), summary.passed.to_string());
        value_row(
            ui,
            Tip::new(k::SUMMARY_NOT_EVALUATED),
            summary.not_evaluated.to_string(),
        );
        value_row(
            ui,
            Tip::new(k::SUMMARY_TIME),
            k::value_milliseconds(format!("{:.0}", report.elapsed_ms)),
        );
        value_row(
            ui,
            Tip::new(k::PROFILE),
            audit_labels::profile_name(&state.aud.profile),
        );
    });
    ui.add_space(size::PANEL_ROW_GAP);
    muted_paragraph(ui, review_localization::tr(k::SUMMARY_HINT));
}

fn node_label(model: &ModelData, node: Option<u32>) -> String {
    match node.and_then(|node| model.nodes.get(node as usize).map(|n| (node, n))) {
        Some((_, node)) if !node.name.is_empty() => node.name.clone(),
        Some((index, _)) => keys::ui_outliner::unnamed_node(f64::from(index)),
        None => review_localization::tr(k::SCENE_WIDE).into_owned(),
    }
}

fn finding_body(ui: &mut egui::Ui, state: &mut UiState, model: &ModelData, focus: AuditFocus) {
    let Some(report) = state.aud.report.clone() else {
        return;
    };
    match focus {
        AuditFocus::Object(node) => object_body(ui, state, &report, model, node),
        AuditFocus::Rule(rule) | AuditFocus::Offender(rule, _) => {
            let Some(result) = report.result(rule) else {
                return;
            };
            rule_body(ui, state, result, model, focus.node());
        }
    }
}

/// Every finding of one object, each a link to that object's share of it.
fn object_body(
    ui: &mut egui::Ui,
    state: &mut UiState,
    report: &AuditReport,
    model: &ModelData,
    node: usize,
) {
    ui.heading(node_label(model, Some(node as u32)));
    ui.add_space(size::PANEL_ROW_GAP);
    let mut picked = None;
    for result in report.results.iter().filter(|result| result.failed()) {
        let Some(offender) = result.offender(node as u32) else {
            continue;
        };
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(size::AUD_GLYPH_COLUMN, size::OUTLINER_ROW_HEIGHT),
                egui::Sense::hover(),
            );
            crate::panels::outliner::paint_severity_glyph(ui.painter(), rect, result.severity);
            let text = format!(
                "{}  {}",
                review_localization::tr(audit_labels::rule_name(result.rule)),
                k::row_count(offender.count as f64)
            );
            if ui.link(text).clicked() {
                picked = Some(AuditFocus::Offender(result.rule, node));
            }
        });
    }
    if let Some(focus) = picked {
        state.select_audit_focus(Some(focus));
    }
}

fn rule_body(
    ui: &mut egui::Ui,
    state: &mut UiState,
    result: &RuleResult,
    model: &ModelData,
    node: Option<usize>,
) {
    let rule = result.rule;
    ui.heading(audit_labels::rule_name(rule));
    match result.status {
        Status::Fail => severity_line(ui, result.severity),
        Status::Pass => {
            ui.label(egui::RichText::from(k::STATUS_PASSED).color(color::AUDIT_PASSED));
        }
        Status::NotEvaluated(reason) => {
            ui.label(egui::RichText::from(k::NOT_EVALUATED).color(color::AUDIT_SKIPPED));
            muted_paragraph(ui, review_localization::tr(audit_labels::skip(reason)));
        }
    }
    ui.add_space(size::PANEL_ROW_GAP);

    // The narrowed object's own figure when there is one, else the rule's.
    let offender = node.and_then(|node| result.offender(node as u32));
    let measured = offender
        .and_then(|offender| offender.measured.as_ref())
        .or(result.measured.as_ref());
    panel_grid(ui, "aud_finding", |ui| {
        if let Some(measured) = measured {
            value_row(ui, Tip::new(k::MEASURED), audit_labels::measured(measured));
        }
        if let Some(detail) = offender.and_then(|offender| offender.detail.as_ref()) {
            value_row(ui, Tip::new(k::MEASURED), audit_labels::measured(detail));
        }
        if let Some(limit) = audit_labels::threshold(&result.threshold) {
            value_row(ui, Tip::new(k::LIMIT), limit);
        }
        value_row(
            ui,
            Tip::new(k::PROFILE),
            audit_labels::profile_name(&state.aud.profile),
        );
    });

    section(
        ui,
        k::WHAT,
        review_localization::tr(audit_labels::rule_what(rule)),
    );
    section(
        ui,
        k::WHY,
        audit_labels::rule_why(rule, state.aud.profile.base),
    );
    section(
        ui,
        k::FIX,
        review_localization::tr(audit_labels::rule_fix(rule)),
    );

    if result.failed() && !result.offenders.is_empty() {
        ui.add_space(size::PANEL_ROW_GAP);
        let heading = ui.label(egui::RichText::from(k::OBJECTS).color(color::TEXT_PRIMARY));
        tip(
            heading,
            Tip::new(k::OBJECTS)
                .describe(k::OBJECTS_DESCRIPTION)
                .page(Page::AudIssues),
        );
        let mut picked = None;
        for candidate in result
            .offenders
            .iter()
            .take(size::AUD_INSPECTOR_MAX_OBJECTS)
        {
            let selected = candidate.node.map(|node| node as usize) == node;
            let text = format!(
                "{}  {}",
                node_label(model, candidate.node),
                k::row_count(candidate.count as f64)
            );
            if ui.selectable_label(selected, text).clicked() {
                picked = Some(match (candidate.node, selected) {
                    (Some(node), false) => AuditFocus::Offender(rule, node as usize),
                    _ => AuditFocus::Rule(rule),
                });
            }
        }
        if result.offenders.len() > size::AUD_INSPECTOR_MAX_OBJECTS {
            ui.weak(k::truncated(
                size::AUD_INSPECTOR_MAX_OBJECTS as f64,
                result.offenders.len() as f64,
            ));
        }
        if let Some(focus) = picked {
            state.select_audit_focus(Some(focus));
        }
    }
}

/// The profile editor. Returns the file action a button asked for.
fn profile_editor(ui: &mut egui::Ui, state: &mut UiState) -> Option<AuditIntent> {
    let mut intent = None;
    let original = state.aud.profile.clone();
    let mut edited: AuditProfile = (*original).clone();

    ui.heading(k::PROFILE_HEADING);
    ui.add_space(size::PANEL_ROW_GAP);
    ui.horizontal(|ui| {
        let width = (ui.available_width() - ui.spacing().item_spacing.x) * 0.5;
        let save = ui.add(wide_button(k::PROFILE_SAVE, width));
        if tip(
            save,
            Tip::new(k::PROFILE_SAVE)
                .describe(k::PROFILE_SAVE_DESCRIPTION)
                .page(Page::AudProfiles),
        )
        .clicked()
        {
            intent = Some(AuditIntent::SaveProfile);
        }
        let load = ui.add(wide_button(k::PROFILE_LOAD, width));
        if tip(
            load,
            Tip::new(k::PROFILE_LOAD)
                .describe(k::PROFILE_LOAD_DESCRIPTION)
                .page(Page::AudProfiles),
        )
        .clicked()
        {
            intent = Some(AuditIntent::LoadProfile);
        }
    });
    ui.add_space(size::PANEL_ROW_GAP);

    panel_grid(ui, "aud_profile", |ui| {
        grid_label_tip(
            ui,
            Tip::new(k::PROFILE_NAME)
                .describe(k::PROFILE_NAME_DESCRIPTION)
                .page(Page::AudProfiles),
        );
        grid_control(ui, |ui| {
            ui.add(egui::TextEdit::singleline(&mut edited.name).desired_width(f32::INFINITY));
        });
        ui.end_row();

        grid_label_tip(
            ui,
            Tip::new(k::PROFILE_ENGINE)
                .describe(k::PROFILE_ENGINE_DESCRIPTION)
                .page(Page::AudProfiles),
        );
        grid_control(ui, |ui| {
            egui::ComboBox::from_id_salt("aud_profile_engine")
                .selected_text(audit_labels::engine(edited.base))
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    crate::widgets::style_combo_popup(ui);
                    for engine in Engine::ALL {
                        ui.selectable_value(&mut edited.base, engine, audit_labels::engine(engine));
                    }
                });
        });
        ui.end_row();
    });
    ui.add_space(size::PANEL_ROW_GAP);
    let engine_name = review_localization::tr(audit_labels::engine(edited.base)).into_owned();
    let reset = ui.add(wide_button(
        k::profile_reset(engine_name),
        ui.available_width(),
    ));
    if tip(
        reset,
        Tip::new(k::PROFILE_HEADING)
            .describe(k::PROFILE_RESET_DESCRIPTION)
            .page(Page::AudProfiles),
    )
    .clicked()
    {
        let name = edited.name.clone();
        edited = AuditProfile::builtin(edited.base);
        edited.name = name;
    }
    ui.add_space(size::PANEL_ROW_GAP);

    for category in Category::ALL {
        egui::CollapsingHeader::new(audit_labels::category(category))
            .id_salt(("aud_profile_category", category))
            .default_open(false)
            .show(ui, |ui| {
                for rule in RuleId::in_category(category) {
                    rule_editor(ui, &mut edited, rule);
                }
            });
    }

    if edited != *original {
        state.aud.edit_profile(|profile| *profile = edited);
    }
    intent
}

/// One rule's switch, severity and parameters.
fn rule_editor(ui: &mut egui::Ui, profile: &mut AuditProfile, rule: RuleId) {
    let Some(config) = profile.rules.get_mut(&rule) else {
        return;
    };
    ui.add_space(size::PANEL_ROW_GAP);
    ui.label(egui::RichText::from(audit_labels::rule_name(rule)).color(color::TEXT_PRIMARY));
    panel_grid(ui, &format!("aud_rule_{}", rule.as_str()), |ui| {
        grid_label_tip(
            ui,
            Tip::new(k::RULE_ENABLED)
                .describe(k::RULE_ENABLED_DESCRIPTION)
                .page(Page::AudChecks),
        );
        grid_control(ui, |ui| ui.checkbox(&mut config.enabled, ""));
        ui.end_row();

        grid_label_tip(
            ui,
            Tip::new(k::RULE_SEVERITY)
                .describe(k::RULE_SEVERITY_DESCRIPTION)
                .page(Page::AudChecks),
        );
        grid_control(ui, |ui| {
            egui::ComboBox::from_id_salt(("aud_rule_severity", rule))
                .selected_text(audit_labels::severity(config.severity))
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    crate::widgets::style_combo_popup(ui);
                    for severity in [Severity::Error, Severity::Warning, Severity::Info] {
                        ui.selectable_value(
                            &mut config.severity,
                            severity,
                            audit_labels::severity(severity),
                        );
                    }
                });
        });
        ui.end_row();

        for spec in param_specs(rule) {
            let (label, description) = audit_labels::param(spec.key);
            grid_label_tip(
                ui,
                Tip::new(label).describe(description).page(Page::AudChecks),
            );
            let value = config
                .params
                .entry(spec.key.to_owned())
                .or_insert(ParamValue::Number(0.0));
            grid_control(ui, |ui| {
                param_widget(ui, (rule, spec.key), spec.kind, value)
            });
            ui.end_row();
        }
    });
}

/// The widget for one parameter, by its kind.
fn param_widget(ui: &mut egui::Ui, salt: (RuleId, &str), kind: ParamKind, value: &mut ParamValue) {
    match (kind, value) {
        (ParamKind::Unit, ParamValue::Number(meters)) => {
            let current = crate::units::match_known_unit(*meters as f32).map_or_else(
                || meters.to_string(),
                |unit| review_localization::tr(unit.symbol).into_owned(),
            );
            egui::ComboBox::from_id_salt(("aud_param_unit", salt))
                .selected_text(current)
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    crate::widgets::style_combo_popup(ui);
                    for unit in crate::units::KNOWN_UNITS {
                        ui.selectable_value(meters, f64::from(unit.meters), unit.symbol);
                    }
                });
        }
        (ParamKind::Axis, ParamValue::Axis(axis)) => {
            egui::ComboBox::from_id_salt(("aud_param_axis", salt))
                .selected_text(audit_labels::axis(*axis))
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    crate::widgets::style_combo_popup(ui);
                    for candidate in [
                        Axis::PositiveY,
                        Axis::PositiveZ,
                        Axis::PositiveX,
                        Axis::NegativeY,
                        Axis::NegativeZ,
                        Axis::NegativeX,
                    ] {
                        ui.selectable_value(axis, candidate, audit_labels::axis(candidate));
                    }
                });
        }
        (ParamKind::Patterns, ParamValue::Patterns(patterns)) => {
            let mut text = patterns.join(", ");
            let valid = review_audit::glob::GlobSet::new(patterns).is_ok();
            let response = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .hint_text(k::PATTERN_HINT)
                    .desired_width(ui.available_width())
                    .text_color(if valid {
                        color::TEXT_BODY
                    } else {
                        color::SEVERITY_ERROR
                    }),
            );
            if !valid {
                let _ = response.clone().on_hover_text(k::PATTERN_INVALID);
            }
            if response.changed() {
                *patterns = text
                    .split(',')
                    .map(str::trim)
                    .filter(|pattern| !pattern.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
        }
        (kind, ParamValue::Number(number)) => {
            let Some((min, max)) = kind.range() else {
                return;
            };
            let integer = matches!(kind, ParamKind::Count { .. } | ParamKind::Pixels { .. });
            let speed = ((max - min) / 1000.0).max(if integer { 1.0 } else { 1.0e-5 });
            let mut drag = egui::DragValue::new(number).range(min..=max).speed(speed);
            if integer {
                drag = drag.fixed_decimals(0);
            } else {
                drag = drag.max_decimals(5);
            }
            ui.add(drag);
        }
        // A value of the wrong kind: `sanitize` repairs these on load, and the
        // editor never writes one.
        _ => {}
    }
}
