//! The Inspector's Aud-workspace bodies: a focused finding, the audit profile
//! editor, and — with neither — the report's summary.
//!
//! The profile editor follows the Opt inspector's shape: copy the profile out,
//! let the widgets edit the copy, and write it back **only if it differs**, so
//! an idle frame never bumps the profile revision (which would re-run the audit
//! every frame). Its text fields go one step further and write back only when
//! the edit is finished (see [`committed_text_edit`]), so typing a word is one
//! revision rather than one per character.

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
use crate::widgets::{
    Tip, grid_control, grid_label_tip, list_row, list_row_label, panel_grid, tip, value_row,
    wide_button,
};

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

/// One row of the Inspector's lists of objects and findings, drawn like the
/// Issues tab's: a full-width [`list_row`] carrying an optional severity glyph,
/// the name, and its count at the right edge. Returns the row's click.
fn count_row(
    ui: &mut egui::Ui,
    id: egui::Id,
    selected: bool,
    severity: Option<Severity>,
    name: String,
    count: u64,
) -> egui::Response {
    let (response, content, trailing) = list_row(
        ui,
        id,
        selected,
        size::AUD_COUNT_COLUMN,
        size::OUTLINER_ROW_HEIGHT,
    );
    let mut name_rect = content;
    if let Some(severity) = severity {
        let glyph = egui::Rect::from_min_size(
            content.left_top(),
            egui::vec2(size::AUD_GLYPH_COLUMN, content.height()),
        );
        crate::panels::outliner::paint_severity_glyph(ui.painter(), glyph, severity);
        name_rect.set_left(glyph.right());
    }
    let name_color = if selected {
        color::TEXT_PRIMARY
    } else {
        color::TEXT_BODY
    };
    list_row_label(ui, name_rect, egui::RichText::new(name).color(name_color));
    ui.painter().text(
        egui::pos2(trailing.right(), trailing.center().y),
        egui::Align2::RIGHT_CENTER,
        k::row_count(count as f64),
        egui::FontId::monospace(crate::theme::font::STATS),
        color::TEXT_MUTED,
    );
    response
}

/// Every finding of one object, each a row selecting that object's share of it.
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
    ui.scope(|ui| {
        // Rows abut, as the Issues tab's do, so the hover bands tile.
        ui.spacing_mut().item_spacing.y = 0.0;
        for result in report.results.iter().filter(|result| result.failed()) {
            let Some(offender) = result.offender(node as u32) else {
                continue;
            };
            let clicked = count_row(
                ui,
                egui::Id::new(("aud_inspector_finding", result.rule)),
                false,
                Some(result.severity),
                review_localization::tr(audit_labels::rule_name(result.rule)).into_owned(),
                offender.count,
            )
            .clicked();
            if clicked {
                picked = Some(AuditFocus::Offender(result.rule, node));
            }
        }
    });
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

    if let Some(view) = rule.see_also() {
        ui.add_space(size::PANEL_ROW_GAP);
        let (title, description) = audit_labels::view(view);
        let supported = state.aud.view_supported(view, &state.capabilities);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::from(k::SEE_ALSO).color(color::TEXT_MUTED));
            let response = ui.add_enabled(
                supported,
                egui::Button::selectable(state.shown_diagnostic() == Some(view), title),
            );
            if supported {
                if tip(
                    response,
                    Tip::new(title).describe(description).page(Page::AudViews),
                )
                .clicked()
                {
                    state.aud.view = Some(view);
                }
            } else {
                response.on_disabled_hover_ui(|ui| {
                    crate::widgets::tip_body(ui, title, k::VIEW_UNSUPPORTED);
                });
            }
        });
    }

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
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, candidate) in result
                .offenders
                .iter()
                .take(size::AUD_INSPECTOR_MAX_OBJECTS)
                .enumerate()
            {
                let selected = candidate.node.map(|node| node as usize) == node;
                let clicked = count_row(
                    ui,
                    egui::Id::new(("aud_inspector_object", index)),
                    selected,
                    None,
                    node_label(model, candidate.node),
                    candidate.count,
                )
                .clicked();
                if clicked {
                    picked = Some(match (candidate.node, selected) {
                        (Some(node), false) => AuditFocus::Offender(rule, node as usize),
                        _ => AuditFocus::Rule(rule),
                    });
                }
            }
        });
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
        category_editor(ui, &mut edited, category);
    }

    if edited != *original {
        state.aud.edit_profile(|profile| *profile = edited);
    }
    intent
}

/// One category's collapsible group of rules. Its header carries a tick box at
/// the right edge that switches every rule in it at once — ticked when all are
/// on, a dash when only some are.
fn category_editor(ui: &mut egui::Ui, profile: &mut AuditProfile, category: Category) {
    let rules: Vec<RuleId> = RuleId::in_category(category).collect();
    let enabled = rules
        .iter()
        .filter(|rule| profile.rules.get(rule).is_some_and(|config| config.enabled))
        .count();
    let mut all = enabled == rules.len();

    let (mut name_clicked, mut switched) = (false, false);
    let id = ui.make_persistent_id(("aud_profile_category", category));
    let mut header =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
            .show_header(ui, |ui| {
                // The name opens the group as a stock header's does; only the arrow
                // would otherwise.
                name_clicked = ui
                    .add(
                        egui::Label::new(audit_labels::category(category))
                            .selectable(false)
                            .sense(egui::Sense::click()),
                    )
                    .clicked();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let checkbox = ui.add(
                        egui::Checkbox::without_text(&mut all)
                            .indeterminate(enabled > 0 && enabled < rules.len()),
                    );
                    switched = tip(
                        checkbox,
                        Tip::new(k::CATEGORY_ENABLED)
                            .describe(k::CATEGORY_ENABLED_DESCRIPTION)
                            .page(Page::AudChecks),
                    )
                    .changed();
                });
            });
    if name_clicked {
        header.toggle();
    }
    header.body(|ui| {
        for &rule in &rules {
            rule_editor(ui, profile, rule);
        }
    });

    if switched {
        for rule in &rules {
            if let Some(config) = profile.rules.get_mut(rule) {
                config.enabled = all;
            }
        }
    }
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
            let id = egui::Id::new(("aud_param_patterns", salt));
            let width = ui.available_width();
            let mut valid = true;
            let (response, finished) = committed_text_edit(ui, id, &patterns.join(", "), |text| {
                // Judged by what the field holds now rather than by the list
                // last committed, so a bad pattern shows as it is typed.
                valid = review_audit::glob::GlobSet::new(&parse_patterns(text)).is_ok();
                egui::TextEdit::singleline(text)
                    .hint_text(k::PATTERN_HINT)
                    .desired_width(width)
                    .text_color(if valid {
                        color::TEXT_BODY
                    } else {
                        color::SEVERITY_ERROR
                    })
            });
            if !valid {
                let _ = response.on_hover_text(k::PATTERN_INVALID);
            }
            if let Some(text) = finished {
                *patterns = parse_patterns(&text);
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

/// A patterns field's text as the list it names: comma-separated, each one
/// trimmed, the empty ones dropped.
fn parse_patterns(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|pattern| !pattern.is_empty())
        .map(str::to_owned)
        .collect()
}

/// What a [`committed_text_edit`] is holding while it is being typed in: the
/// text so far, and the value it was begun from.
#[derive(Debug, Clone)]
struct Draft {
    base: String,
    text: String,
}

/// A single-line text field that hands its text back once, when the edit is
/// finished, rather than on every keystroke.
///
/// Every write to the profile bumps its revision, and a revision is an audit run
/// and an undo step, so a field written back per keystroke re-ran the audit once
/// a character. A field rebuilt from the profile every frame also cannot hold
/// text the profile has no way to store: the patterns field re-joined the parsed
/// list, so the comma before a second pattern vanished as soon as it was typed.
///
/// So while the field has focus, the text being typed lives in egui's temporary
/// memory under the field's id and the profile is left alone. The first frame the
/// field is without focus — Enter, a click elsewhere, Tab — returns that text to
/// commit, and `Esc` returns nothing, putting the field back as it was. A draft
/// whose starting value has since changed underneath it (an undo, a loaded
/// profile) is dropped rather than written over the new value.
///
/// `build` makes the field from the text it edits, so a caller can style it by
/// what is being typed.
fn committed_text_edit(
    ui: &mut egui::Ui,
    id: egui::Id,
    committed: &str,
    build: impl FnOnce(&mut String) -> egui::TextEdit<'_>,
) -> (egui::Response, Option<String>) {
    let draft_id = id.with("draft");
    let stored: Option<Draft> = ui.data(|data| data.get_temp(draft_id));
    let had_draft = stored.is_some();
    let draft = stored.filter(|draft| draft.base == committed);
    let editing = draft.is_some();
    let mut text = draft.map_or_else(|| committed.to_owned(), |draft| draft.text);

    let response = ui.add(build(&mut text).id(id));
    if response.has_focus() {
        if response.changed() {
            // `build` styled the field by the text before this keystroke.
            ui.ctx().request_repaint();
        }
        let draft = Draft {
            base: committed.to_owned(),
            text,
        };
        ui.data_mut(|data| data.insert_temp(draft_id, draft));
        return (response, None);
    }
    if had_draft {
        ui.data_mut(|data| data.remove::<Draft>(draft_id));
    }
    let cancelled = ui.input(|input| input.key_pressed(egui::Key::Escape));
    (response, (editing && !cancelled).then_some(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A field driven a frame at a time, the way a user types into it.
    struct Field {
        ctx: egui::Context,
        id: egui::Id,
        /// The value the field writes back to, as the profile holds it.
        value: String,
        /// How many times the field handed a value back.
        commits: usize,
    }

    impl Field {
        fn new(value: &str) -> Self {
            Self {
                ctx: egui::Context::default(),
                id: egui::Id::new("committed_text_edit_test"),
                value: value.to_owned(),
                commits: 0,
            }
        }

        /// One frame delivering `events`.
        fn frame(&mut self, events: Vec<egui::Event>) {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 200.0),
                )),
                focused: true,
                events,
                ..Default::default()
            };
            let (id, value) = (self.id, self.value.clone());
            let mut finished = None;
            let output = self.ctx.run_ui(input, |ui| {
                finished =
                    committed_text_edit(ui, id, &value, |text| egui::TextEdit::singleline(text)).1;
            });
            output.drop_without_applying_deltas();
            if let Some(text) = finished {
                self.value = text;
                self.commits += 1;
            }
        }

        fn focus(&mut self) {
            self.ctx.memory_mut(|memory| memory.request_focus(self.id));
            self.frame(Vec::new());
        }

        fn type_text(&mut self, text: &str) {
            self.frame(vec![egui::Event::Text(text.to_owned())]);
        }

        fn press(&mut self, key: egui::Key) {
            let event = |pressed| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(vec![event(true), event(false)]);
            // The frame after, where a focus change made at the end of the last
            // pass shows.
            self.frame(Vec::new());
        }
    }

    /// The bug this field exists for: typed a character at a time, a second
    /// pattern used to lose the comma in front of it.
    #[test]
    fn a_second_pattern_can_be_typed_in_order() {
        let mut field = Field::new("*.fbx");
        field.focus();
        for text in [",", " ", "*", ".", "o", "b", "j"] {
            field.type_text(text);
        }
        assert_eq!(field.commits, 0, "nothing is written while typing");
        field.press(egui::Key::Enter);
        assert_eq!(field.commits, 1, "one edit is one write");
        assert_eq!(parse_patterns(&field.value), ["*.fbx", "*.obj"]);
    }

    #[test]
    fn escape_puts_the_field_back() {
        let mut field = Field::new("Studio");
        field.focus();
        field.type_text(" draft");
        field.press(egui::Key::Escape);
        assert_eq!(field.commits, 0);
        assert_eq!(field.value, "Studio");
    }

    /// An undo or a loaded profile replaces the value mid-edit: the stale draft
    /// must not be written over it.
    #[test]
    fn a_draft_is_dropped_when_its_value_changes_underneath_it() {
        let mut field = Field::new("Studio");
        field.focus();
        field.type_text(" draft");
        field.value = "Loaded".to_owned();
        field.press(egui::Key::Enter);
        assert_eq!(field.value, "Loaded");
    }

    #[test]
    fn patterns_split_on_commas_and_drop_the_empty_ones() {
        assert_eq!(parse_patterns(" *.fbx , ,*.obj,"), ["*.fbx", "*.obj"]);
        assert!(parse_patterns("  ").is_empty());
    }
}
