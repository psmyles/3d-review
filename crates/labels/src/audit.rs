//! Catalog text for the audit's typed values: categories, severities, skip
//! reasons and measured figures, and the rules' own texts.
//!
//! Every map is an exhaustive `match` with no fall-through arm, the same rule
//! `ui`'s `labels.rs` keeps: a value added to `review-audit` is a compile error
//! here rather than English on a translated screen, and a catalog message
//! nothing names is `dead_code`.
//!
//! A rule's own texts (its name; what it looks for, why it matters and how to
//! fix it; its parameters' labels) are not Fluent messages: they are in the
//! rules catalog, `rules-catalog.toml`, with the rest of the rule, in English.
//! Translations, if ever needed, key on the rule's id.

use review_audit::{
    AuditError, AuditProfile, AuditReport, Axis, Category, DiagnosticView, ElementKind, Engine,
    Measured, RuleId, RuleResult, Severity, Skip, Status, Threshold,
};
use review_localization::{Key, tr};

use crate::keys::labels_audit as k;

pub fn category(category: Category) -> Key {
    match category {
        Category::Geometry => k::CATEGORY_GEOMETRY,
        Category::Transforms => k::CATEGORY_TRANSFORMS,
        Category::Uv => k::CATEGORY_UV,
        Category::Skin => k::CATEGORY_SKIN,
        Category::Density => k::CATEGORY_DENSITY,
        Category::Naming => k::CATEGORY_NAMING,
        Category::Hierarchy => k::CATEGORY_HIERARCHY,
    }
}

pub fn severity(severity: Severity) -> Key {
    match severity {
        Severity::Error => k::SEVERITY_ERROR,
        Severity::Warning => k::SEVERITY_WARNING,
        Severity::Info => k::SEVERITY_INFO,
    }
}

pub fn skip(skip: Skip) -> Key {
    match skip {
        Skip::Disabled => k::SKIP_DISABLED,
        Skip::SourcePropertiesUnavailable => k::SKIP_NO_PROPERTIES,
        Skip::NoSkin => k::SKIP_NO_SKIN,
        Skip::NoUvSet => k::SKIP_NO_UV_SET,
        Skip::InvalidPattern => k::SKIP_INVALID_PATTERN,
        Skip::EmptyModel => k::SKIP_EMPTY,
        Skip::Cancelled => k::SKIP_CANCELLED,
    }
}

/// A diagnostic view's name and its one-paragraph description.
pub fn view(view: DiagnosticView) -> (Key, Key) {
    match view {
        DiagnosticView::Issues => (k::VIEW_ISSUES, k::VIEW_ISSUES_DESCRIPTION),
        DiagnosticView::TexelDensity => (k::VIEW_TEXEL_DENSITY, k::VIEW_TEXEL_DENSITY_DESCRIPTION),
        DiagnosticView::TriangleDensity => (
            k::VIEW_TRIANGLE_DENSITY,
            k::VIEW_TRIANGLE_DENSITY_DESCRIPTION,
        ),
        DiagnosticView::Overdraw => (k::VIEW_OVERDRAW, k::VIEW_OVERDRAW_DESCRIPTION),
        DiagnosticView::QuadOverdraw => (k::VIEW_QUAD_OVERDRAW, k::VIEW_QUAD_OVERDRAW_DESCRIPTION),
    }
}

pub fn engine(engine: Engine) -> Key {
    match engine {
        Engine::Unity => k::ENGINE_UNITY,
        Engine::Unreal => k::ENGINE_UNREAL,
        Engine::Generic => k::ENGINE_GENERIC,
    }
}

pub fn axis(axis: Axis) -> Key {
    match axis {
        Axis::PositiveX => k::AXIS_X,
        Axis::NegativeX => k::AXIS_NEGATIVE_X,
        Axis::PositiveY => k::AXIS_Y,
        Axis::NegativeY => k::AXIS_NEGATIVE_Y,
        Axis::PositiveZ => k::AXIS_Z,
        Axis::NegativeZ => k::AXIS_NEGATIVE_Z,
    }
}

/// A check's short name, as listed.
pub fn rule_name(rule: RuleId) -> String {
    rule.rule().title.clone()
}

/// What a check looks for, in one sentence.
pub fn rule_what(rule: RuleId) -> String {
    rule.rule().what.clone()
}

/// Why a check matters, worded for the profile's target engine where the
/// engines differ (the catalog's `[rule.base.<engine>]`).
pub fn rule_why(rule: RuleId, target: Engine) -> String {
    rule.rule().texts(Some(target.as_str()), None).why
}

/// How to fix a finding in the modelling program, in general terms.
pub fn rule_fix(rule: RuleId) -> String {
    rule.rule().how.clone()
}

/// A rule parameter's label and explanation, by its profile key.
pub fn param(rule: RuleId, key: &str) -> (String, String) {
    rule.rule().param(key).map_or_else(
        || (key.to_owned(), String::new()),
        |param| (param.label.clone(), param.help.clone().unwrap_or_default()),
    )
}

/// A number with `digits` decimals, trailing zeros trimmed.
fn number(value: f64, digits: usize) -> String {
    let text = format!("{value:.digits$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

/// A measured figure, with its unit.
pub fn measured(value: &Measured) -> String {
    match value {
        Measured::Count(count) => k::value_count(*count as f64),
        Measured::Ratio(ratio) => k::value_percent(number(ratio * 100.0, 1)),
        Measured::Meters(meters) => k::value_meters(number(*meters, 4)),
        Measured::Degrees(degrees) => k::value_degrees(number(*degrees, 1)),
        Measured::PerSquareMeter(value) => k::value_per_square_meter(number(*value, 0)),
        Measured::PxPerMeter(value) => k::value_px_per_meter(number(*value, 0)),
        Measured::Pixels(value) => k::value_pixels(number(*value, 1)),
        Measured::SquarePixels(value) => k::value_square_pixels(number(*value, 1)),
        Measured::ScreenSize(value) => k::value_screen_size(number(*value, 2)),
        Measured::UnitMeters(value) => match crate::units::match_known_unit(*value as f32) {
            Some(unit) => tr(unit.symbol).into_owned(),
            None => k::value_unit(number(*value, 4)),
        },
        Measured::Axis(value) => tr(axis(*value)).into_owned(),
        Measured::Scale([x, y, z]) => k::value_scale(
            number(f64::from(*x), 3),
            number(f64::from(*y), 3),
            number(f64::from(*z), 3),
        ),
        Measured::Text(text) => text.clone(),
    }
}

/// The bound a figure was judged against, or `None` for a check that has none
/// to state.
pub fn threshold(value: &Threshold) -> Option<String> {
    match value {
        Threshold::None => None,
        Threshold::Max(limit) => Some(k::value_at_most(measured(limit))),
        Threshold::Min(limit) => Some(k::value_at_least(measured(limit))),
        Threshold::Range { min, max } => Some(k::value_range(measured(max), measured(min))),
        Threshold::Equals(target) => Some(measured(target)),
        Threshold::Patterns(patterns) => Some(patterns.join(", ")),
    }
}

/// What a failing check found, in plain words with its figures - the "what" of
/// a finding in Asset Check's results (AC10, AC11), where the artist reads it
/// without the viewer beside them: the check's own sentence, how much of the
/// model it found, and the figure it measured against the profile's limit.
///
/// The figure is the rule's headline (its worst object's, or the scene's), or
/// the one offender's own when the rule states none. A limit is only stated
/// beside a figure, since "at most 4" read alone says nothing; the check's own
/// sentence already says what it is a limit on.
pub fn finding_what(result: &RuleResult) -> String {
    let mut sentences = vec![rule_what(result.rule)];
    let in_objects = result.offender_nodes().count();
    let objects = in_objects as f64;
    let count = result.total as f64;
    let found = match result.rule.element() {
        _ if in_objects == 0 => None,
        ElementKind::Triangle => Some(k::found_triangles(count, objects)),
        ElementKind::Edge => Some(k::found_edges(count, objects)),
        ElementKind::Vertex => Some(k::found_vertices(count, objects)),
        ElementKind::Node => Some(k::found_objects(objects)),
        ElementKind::Scene => None,
    };
    sentences.extend(found);
    sentences.extend(figure(result));
    sentences.join(" ")
}

/// [`finding_what`]'s last sentence: the figure against the limit.
fn figure(result: &RuleResult) -> Option<String> {
    match (&result.threshold, result.headline().map(measured)) {
        (Threshold::Patterns(patterns), _) => Some(k::figure_patterns(patterns.join(", "))),
        (_, None) => None,
        (Threshold::None, Some(value)) => Some(k::figure_measured(value)),
        (Threshold::Max(limit), Some(value)) => Some(k::figure_max(measured(limit), value)),
        (Threshold::Min(limit), Some(value)) => Some(k::figure_min(measured(limit), value)),
        (Threshold::Range { min, max }, Some(value)) => {
            Some(k::figure_range(measured(max), value, measured(min)))
        }
        (Threshold::Equals(target), Some(value)) => Some(k::figure_equals(value, measured(target))),
    }
}

/// The profile's display name: its own, or its engine's when it has none, with
/// a marker when it differs from that engine's built-in.
pub fn profile_name(profile: &AuditProfile) -> String {
    let name = if profile.name.trim().is_empty() {
        tr(engine(profile.base)).into_owned()
    } else {
        profile.name.clone()
    };
    if profile.is_builtin() {
        name
    } else {
        k::profile_modified(name)
    }
}

/// The plain-text summary "Copy summary" puts on the clipboard: a heading, the
/// counts, then one line per failing check, worst first.
pub fn audit_summary_text(report: &AuditReport, profile: &AuditProfile, file: &str) -> String {
    let mut lines = vec![k::text_heading(file.to_owned(), profile_name(profile))];
    lines.extend(summary_lines(report));
    lines.join("\n")
}

/// [`audit_summary_text`] without its heading: the counts, then one line per
/// failing check, worst first - what `review3d-cli audit` prints under each
/// model's name.
pub fn summary_lines(report: &AuditReport) -> Vec<String> {
    let mut lines = vec![k::text_counts(
        f64::from(report.summary.failed[Severity::Error.index()]),
        f64::from(report.summary.failed[Severity::Info.index()]),
        f64::from(report.summary.passed),
        f64::from(report.summary.failed[Severity::Warning.index()]),
    )];
    let mut failing: Vec<_> = report
        .results
        .iter()
        .filter(|result| result.status == Status::Fail)
        .collect();
    failing.sort_by_key(|result| std::cmp::Reverse(result.severity));
    if failing.is_empty() {
        lines.push(tr(k::TEXT_NONE).into_owned());
    }
    for result in failing {
        lines.push(finding_line(result.rule, result.severity, result.total));
    }
    lines
}

/// One failing check as [`summary_lines`] lists it: `Warning: Texel density
/// (1)`, where the count is the triangles, edges, vertices or objects found.
pub fn finding_line(rule: RuleId, level: Severity, count: u64) -> String {
    k::text_finding(
        count as f64,
        rule_name(rule),
        tr(severity(level)).into_owned(),
    )
}

/// What to show the user for an audit error: a newer profile says what to do
/// about it; everything else keeps the library's own diagnostic.
pub fn explain_audit_error(error: &AuditError) -> String {
    match error {
        AuditError::ProfileVersion { found, supported } => {
            k::profile_newer(f64::from(*found), f64::from(*supported))
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_trim_trailing_zeros() {
        assert_eq!(number(1.50, 2), "1.5");
        assert_eq!(number(2.0, 2), "2");
        assert_eq!(number(1024.0, 0), "1024");
    }

    #[test]
    fn every_rule_has_its_text() {
        for rule in RuleId::ALL {
            for target in Engine::ALL {
                assert!(!rule_why(rule, target).is_empty(), "{}", rule.as_str());
            }
            assert!(!rule_name(rule).is_empty());
            assert!(!rule_what(rule).is_empty());
            assert!(!rule_fix(rule).is_empty());
            for spec in review_audit::param_specs(rule) {
                let (label, description) = param(rule, spec.key);
                assert!(!label.is_empty() && !description.is_empty(), "{}", spec.key);
            }
        }
    }

    /// The engines' own reasons, where the catalog gives them.
    #[test]
    fn why_is_worded_for_the_engine() {
        assert!(rule_why(RuleId::Unit, Engine::Unity).starts_with("Unity works in meters."));
        assert!(rule_why(RuleId::Unit, Engine::Unreal).starts_with("Unreal works in centimeters."));
    }
}
