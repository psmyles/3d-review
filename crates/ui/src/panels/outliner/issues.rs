//! The Outliner's Issues tab — the Aud workspace's list of findings.
//!
//! From the top: the Profile row (which opens the profile in the Inspector),
//! Copy summary and Save report, the By check / By object switch and the Show
//! passed filter, then the findings themselves. Picking a row focuses it: the
//! Inspector explains it and the viewport highlights its offenders.
//!
//! The list is flattened every frame into the rows that are actually open and
//! drawn through `ScrollArea::show_rows`, so a file whose checks list thousands
//! of objects lays out only the rows on screen.

use review_audit::{AuditReport, Category, RuleId, RuleResult, Severity, Status};
use review_model::ModelData;

use crate::aud_state::{AudGrouping, AuditFocus, AuditIntent, IssueGroup};
use crate::audit_labels;
use crate::docs::Page;
use crate::keys;
use crate::state::UiState;
use crate::theme::{color, size};
use crate::widgets::{Tip, list_row, list_row_label, segment_button, tip, wide_button};

/// One drawn row of the flattened list.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Row {
    Category(Category),
    Rule(RuleId),
    /// One object (or, with `None`, the whole file) failing a rule — under the
    /// rule in By check, under the object in By object.
    Offender {
        rule: RuleId,
        node: Option<usize>,
        depth: u8,
    },
    Object(Option<usize>),
}

/// What a click on a row asked for, applied once the list has drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Click {
    Row(Row),
    Fold(IssueGroup),
}

/// What a severity looks like in the list: its colour and glyph.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Mark {
    Severity(Severity),
    Passed,
    Skipped,
}

pub(super) fn issues_tab(
    ui: &mut egui::Ui,
    state: &mut UiState,
    model: &ModelData,
) -> Option<AuditIntent> {
    let mut intent = None;
    profile_row(ui, state);
    ui.add_space(size::PANEL_ROW_GAP);

    let has_report = state.aud.report.is_some();
    ui.horizontal(|ui| {
        let width = (ui.available_width() - ui.spacing().item_spacing.x) * 0.5;
        let copy = ui.add_enabled(has_report, wide_button(keys::ui_audit::COPY_SUMMARY, width));
        if tip(
            copy,
            Tip::new(keys::ui_audit::COPY_SUMMARY)
                .describe(keys::ui_audit::COPY_SUMMARY_DESCRIPTION)
                .page(Page::AudReport),
        )
        .clicked()
            && let Some(report) = &state.aud.report
        {
            ui.ctx().copy_text(audit_labels::audit_summary_text(
                report,
                &state.aud.profile,
                &model.name,
            ));
            intent = Some(AuditIntent::SummaryCopied);
        }
        let save = ui.add_enabled(has_report, wide_button(keys::ui_audit::SAVE_REPORT, width));
        if tip(
            save,
            Tip::new(keys::ui_audit::SAVE_REPORT)
                .describe(keys::ui_audit::SAVE_REPORT_DESCRIPTION)
                .page(Page::AudReport),
        )
        .clicked()
        {
            intent = Some(AuditIntent::SaveReport);
        }
    });
    ui.add_space(size::PANEL_ROW_GAP);

    ui.horizontal(|ui| {
        for (grouping, label, description) in [
            (
                AudGrouping::ByCheck,
                keys::ui_audit::GROUP_BY_CHECK,
                keys::ui_audit::GROUP_BY_CHECK_DESCRIPTION,
            ),
            (
                AudGrouping::ByObject,
                keys::ui_audit::GROUP_BY_OBJECT,
                keys::ui_audit::GROUP_BY_OBJECT_DESCRIPTION,
            ),
        ] {
            let text = review_localization::tr(label);
            let width = segment_width(ui, &text);
            let response = segment_button(ui, &text, state.aud.grouping == grouping, width);
            if tip(
                response,
                Tip::new(label).describe(description).page(Page::AudIssues),
            )
            .clicked()
            {
                state.aud.grouping = grouping;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let response = ui.checkbox(&mut state.aud.show_passed, keys::ui_audit::SHOW_PASSED);
            tip(
                response,
                Tip::new(keys::ui_audit::SHOW_PASSED)
                    .describe(keys::ui_audit::SHOW_PASSED_DESCRIPTION)
                    .page(Page::AudIssues),
            );
        });
    });
    ui.add_space(size::PANEL_ROW_GAP);

    let Some(report) = state.aud.report.clone() else {
        ui.weak(if model.indices.is_empty() && model.nodes.is_empty() {
            keys::ui_audit::NO_MODEL
        } else {
            keys::ui_audit::RUNNING
        });
        return intent;
    };
    if state.aud.running {
        ui.weak(keys::ui_audit::RUNNING);
    }

    let rows = match state.aud.grouping {
        AudGrouping::ByCheck => rows_by_check(state, &report),
        AudGrouping::ByObject => rows_by_object(state, &report, model),
    };
    if rows.is_empty() {
        let response = ui.weak(keys::ui_audit::ALL_CLEAR);
        tip(
            response,
            Tip::new(keys::ui_audit::ALL_CLEAR).describe(keys::ui_audit::ALL_CLEAR_DESCRIPTION),
        );
        return intent;
    }

    let mut clicked = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, size::OUTLINER_ROW_HEIGHT, rows.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for index in range {
                if let Some(click) = draw_row(ui, state, &report, model, rows[index], index) {
                    clicked = Some(click);
                }
            }
        });
    match clicked {
        Some(Click::Row(row)) => apply_click(state, row),
        Some(Click::Fold(group)) => state.aud.toggle(group),
        None => {}
    }
    intent
}

/// The width of a grouping segment: its label plus the tile padding.
fn segment_width(ui: &egui::Ui, text: &str) -> f32 {
    let font = egui::FontId::proportional(crate::theme::font::MODE_SEGMENT);
    let width = ui.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(text.to_owned(), font, color::TEXT_PRIMARY)
            .size()
            .x
    });
    width + size::OUTLINER_ROW_PAD_X * 2.0
}

/// The Profile row: the active profile's name, opening it in the Inspector.
fn profile_row(ui: &mut egui::Ui, state: &mut UiState) {
    let (response, content, _) = list_row(
        ui,
        egui::Id::new("aud_profile_row"),
        state.aud.profile_open,
        0.0,
        size::OUTLINER_ROW_HEIGHT,
    );
    let name = audit_labels::profile_name(&state.aud.profile);
    list_row_label(
        ui,
        content,
        egui::RichText::new(keys::ui_audit::profile_row(name)).color(color::TEXT_BODY),
    );
    if tip(
        response,
        Tip::new(keys::ui_audit::PROFILE_HEADING)
            .describe(keys::ui_audit::PROFILE_ROW_DESCRIPTION)
            .page(Page::AudProfiles),
    )
    .clicked()
    {
        if state.aud.profile_open {
            state.aud.profile_open = false;
        } else {
            state.open_audit_profile();
        }
    }
}

/// Whether a result is listed: every failure, and the rest under Show passed.
fn listed(state: &UiState, result: &RuleResult) -> bool {
    result.status == Status::Fail || state.aud.show_passed
}

fn rows_by_check(state: &UiState, report: &AuditReport) -> Vec<Row> {
    let mut rows = Vec::new();
    for category in Category::ALL {
        let results: Vec<&RuleResult> = RuleId::in_category(category)
            .filter_map(|rule| report.result(rule))
            .filter(|result| listed(state, result))
            .collect();
        if results.is_empty() {
            continue;
        }
        rows.push(Row::Category(category));
        if !state.aud.is_open(IssueGroup::Category(category)) {
            continue;
        }
        for result in results {
            rows.push(Row::Rule(result.rule));
            if result.failed() && state.aud.is_open(IssueGroup::Rule(result.rule)) {
                rows.extend(result.offenders.iter().map(|offender| Row::Offender {
                    rule: result.rule,
                    node: offender.node.map(|node| node as usize),
                    depth: 2,
                }));
            }
        }
    }
    rows
}

fn rows_by_object(state: &UiState, report: &AuditReport, model: &ModelData) -> Vec<Row> {
    // Each object with the rules it fails, scene-wide findings first.
    let mut objects: std::collections::BTreeMap<Option<usize>, Vec<RuleId>> = Default::default();
    for result in report.results.iter().filter(|result| result.failed()) {
        for offender in &result.offenders {
            objects
                .entry(offender.node.map(|node| node as usize))
                .or_default()
                .push(result.rule);
        }
    }
    let mut rows = Vec::new();
    for (node, mut rules) in objects {
        if node.is_some_and(|node| node >= model.nodes.len()) {
            continue;
        }
        rules.sort_by_key(|rule| {
            std::cmp::Reverse(report.result(*rule).map(|result| result.severity))
        });
        rules.dedup();
        rows.push(Row::Object(node));
        if state.aud.is_open(IssueGroup::Object(node)) {
            rows.extend(rules.into_iter().map(|rule| Row::Offender {
                rule,
                node,
                depth: 1,
            }));
        }
    }
    rows
}

/// The mark a rule's row carries.
fn rule_mark(result: &RuleResult) -> Mark {
    match result.status {
        Status::Fail => Mark::Severity(result.severity),
        Status::Pass => Mark::Passed,
        Status::NotEvaluated(_) => Mark::Skipped,
    }
}

fn mark_color(mark: Mark) -> egui::Color32 {
    match mark {
        Mark::Severity(severity) => severity_color(severity),
        Mark::Passed => color::AUDIT_PASSED,
        Mark::Skipped => color::AUDIT_SKIPPED,
    }
}

pub(crate) fn severity_color(severity: Severity) -> egui::Color32 {
    match severity {
        Severity::Error => color::SEVERITY_ERROR,
        Severity::Warning => color::SEVERITY_WARNING,
        Severity::Info => color::SEVERITY_INFO,
    }
}

/// Paint a severity's glyph centred in `rect` — the Scene tab's dot in Aud.
pub(crate) fn paint_severity(painter: &egui::Painter, rect: egui::Rect, severity: Severity) {
    paint(painter, rect, Mark::Severity(severity));
}

/// Paint `mark` centred in `rect`: a disc for an error, a triangle for a
/// warning, a ring for info, a tick for passed and a dash for not run — shape
/// as well as colour, so severity reads without colour.
fn paint(painter: &egui::Painter, rect: egui::Rect, mark: Mark) {
    let color = mark_color(mark);
    let center = rect.center();
    let radius = size::AUD_SEVERITY_DOT * 0.5;
    let stroke = egui::Stroke::new(size::HAIRLINE * 1.5, color);
    match mark {
        Mark::Severity(Severity::Error) => {
            painter.circle_filled(center, radius, color);
        }
        Mark::Severity(Severity::Warning) => {
            let h = radius * 1.1;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    center + egui::vec2(0.0, -h),
                    center + egui::vec2(h, h * 0.8),
                    center + egui::vec2(-h, h * 0.8),
                ],
                color,
                egui::Stroke::NONE,
            ));
        }
        Mark::Severity(Severity::Info) => {
            painter.circle_stroke(center, radius - 0.5, stroke);
        }
        Mark::Passed => {
            painter.line_segment(
                [
                    center + egui::vec2(-radius, 0.0),
                    center + egui::vec2(-radius * 0.3, radius * 0.7),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    center + egui::vec2(-radius * 0.3, radius * 0.7),
                    center + egui::vec2(radius, -radius * 0.7),
                ],
                stroke,
            );
        }
        Mark::Skipped => {
            painter.line_segment(
                [
                    center + egui::vec2(-radius * 0.7, 0.0),
                    center + egui::vec2(radius * 0.7, 0.0),
                ],
                stroke,
            );
        }
    }
}

/// The node's display name, or the whole file for a scene-wide finding.
fn object_label(model: &ModelData, node: Option<usize>) -> String {
    match node.and_then(|node| model.nodes.get(node).map(|n| (node, n))) {
        Some((index, node)) => super::display_name(node, index),
        None => review_localization::tr(keys::ui_audit::SCENE_GROUP).into_owned(),
    }
}

/// Draw one row, returning what a click on it asked for.
fn draw_row(
    ui: &mut egui::Ui,
    state: &UiState,
    report: &AuditReport,
    model: &ModelData,
    row: Row,
    index: usize,
) -> Option<Click> {
    let focus = state.aud.focus;
    let (selected, depth, mark, label, count, group) = match row {
        Row::Category(category) => {
            let worst = RuleId::in_category(category)
                .filter_map(|rule| report.result(rule))
                .filter(|result| result.failed())
                .map(|result| result.severity)
                .max();
            let failing = RuleId::in_category(category)
                .filter_map(|rule| report.result(rule))
                .filter(|result| result.failed())
                .count();
            (
                false,
                0,
                Some(worst.map_or(Mark::Passed, Mark::Severity)),
                review_localization::tr(audit_labels::category(category)).into_owned(),
                (failing > 0).then_some(failing as u64),
                Some(IssueGroup::Category(category)),
            )
        }
        Row::Rule(rule) => {
            let result = report.result(rule);
            (
                focus == Some(AuditFocus::Rule(rule)),
                1,
                result.map(rule_mark),
                review_localization::tr(audit_labels::rule_name(rule)).into_owned(),
                result
                    .filter(|result| result.failed())
                    .map(|result| result.total),
                result
                    .filter(|result| result.failed())
                    .map(|_| IssueGroup::Rule(rule)),
            )
        }
        Row::Offender { rule, node, depth } => {
            let result = report.result(rule);
            let count = result
                .and_then(|result| {
                    result
                        .offenders
                        .iter()
                        .find(|o| o.node.map(|n| n as usize) == node)
                })
                .map(|offender| offender.count);
            let label = if state.aud.grouping == AudGrouping::ByObject {
                review_localization::tr(audit_labels::rule_name(rule)).into_owned()
            } else {
                object_label(model, node)
            };
            let selected = match node {
                Some(node) => focus == Some(AuditFocus::Offender(rule, node)),
                None => focus == Some(AuditFocus::Rule(rule)),
            };
            (
                selected,
                depth,
                (state.aud.grouping == AudGrouping::ByObject)
                    .then(|| result.map(rule_mark))
                    .flatten(),
                label,
                count,
                None,
            )
        }
        Row::Object(node) => {
            let worst = report
                .summary
                .node_worst
                .get(node.unwrap_or(usize::MAX))
                .copied()
                .flatten();
            let worst = worst.or_else(|| {
                report
                    .results
                    .iter()
                    .filter(|result| {
                        result.failed() && result.offenders.iter().any(|o| o.node.is_none())
                    })
                    .map(|result| result.severity)
                    .max()
            });
            (
                node.is_some_and(|node| focus == Some(AuditFocus::Object(node))),
                0,
                worst.map(Mark::Severity),
                object_label(model, node),
                None,
                Some(IssueGroup::Object(node)),
            )
        }
    };

    let (response, content, trailing) = list_row(
        ui,
        egui::Id::new(("aud_issue_row", index)),
        selected,
        size::AUD_COUNT_COLUMN,
        size::OUTLINER_ROW_HEIGHT,
    );
    let painter = ui.painter().clone();
    let mut x = content.left() + f32::from(depth) * size::AUD_INDENT;

    // The disclosure arrow, for rows that open.
    let arrow_rect = egui::Rect::from_min_size(
        egui::pos2(x, content.top()),
        egui::vec2(size::AUD_GLYPH_COLUMN, content.height()),
    );
    let mut click = None;
    let mut on_arrow = false;
    if let Some(group) = group {
        let open = state.aud.is_open(group);
        let arrow = ui.interact(
            arrow_rect,
            egui::Id::new(("aud_issue_arrow", index)),
            egui::Sense::click(),
        );
        on_arrow = arrow.hovered();
        if arrow.clicked() {
            click = Some(Click::Fold(group));
        }
        let c = arrow_rect.center();
        let r = size::AUD_SEVERITY_DOT * 0.4;
        let points = if open {
            vec![
                c + egui::vec2(-r, -r * 0.5),
                c + egui::vec2(r, -r * 0.5),
                c + egui::vec2(0.0, r * 0.6),
            ]
        } else {
            vec![
                c + egui::vec2(-r * 0.5, -r),
                c + egui::vec2(-r * 0.5, r),
                c + egui::vec2(r * 0.6, 0.0),
            ]
        };
        painter.add(egui::Shape::convex_polygon(
            points,
            color::TEXT_MUTED,
            egui::Stroke::NONE,
        ));
    }
    x += size::AUD_GLYPH_COLUMN;

    if let Some(mark) = mark {
        let rect = egui::Rect::from_min_size(
            egui::pos2(x, content.top()),
            egui::vec2(size::AUD_GLYPH_COLUMN, content.height()),
        );
        paint(&painter, rect, mark);
        x += size::AUD_GLYPH_COLUMN;
    }

    let text_color = match (mark, selected) {
        (_, true) => color::TEXT_PRIMARY,
        (Some(Mark::Skipped | Mark::Passed), _) if !matches!(row, Row::Category(_)) => {
            color::TEXT_MUTED
        }
        _ => color::TEXT_BODY,
    };
    let name_rect = egui::Rect::from_min_max(egui::pos2(x, content.top()), content.max);
    list_row_label(ui, name_rect, egui::RichText::new(label).color(text_color));
    if let Some(count) = count {
        painter.text(
            egui::pos2(trailing.right(), trailing.center().y),
            egui::Align2::RIGHT_CENTER,
            keys::ui_audit::row_count(count as f64),
            egui::FontId::monospace(crate::theme::font::STATS),
            color::TEXT_MUTED,
        );
    }

    // A rule that did not run says why on hover.
    if let Row::Rule(rule) = row
        && let Some(Status::NotEvaluated(reason)) = report.result(rule).map(|result| result.status)
    {
        let _ = response.clone().on_hover_text(audit_labels::skip(reason));
    }

    if response.clicked() && !on_arrow {
        click = Some(Click::Row(row));
    }
    click
}

/// Apply a row click (and any pending fold) once the list has drawn.
fn apply_click(state: &mut UiState, row: Row) {
    match row {
        Row::Category(category) => state.aud.toggle(IssueGroup::Category(category)),
        Row::Rule(rule) => {
            let focus = AuditFocus::Rule(rule);
            let next = (state.aud.focus != Some(focus)).then_some(focus);
            state.select_audit_focus(next);
        }
        Row::Offender { rule, node, .. } => {
            let focus = match node {
                Some(node) => AuditFocus::Offender(rule, node),
                None => AuditFocus::Rule(rule),
            };
            let next = (state.aud.focus != Some(focus)).then_some(focus);
            state.select_audit_focus(next);
        }
        Row::Object(node) => match node {
            Some(node) => {
                let focus = AuditFocus::Object(node);
                let next = (state.aud.focus != Some(focus)).then_some(focus);
                state.select_audit_focus(next);
            }
            None => state.aud.toggle(IssueGroup::Object(None)),
        },
    }
}
