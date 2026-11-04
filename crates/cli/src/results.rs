//! `--results <file.json>`: Asset Check's results (AC10), the one format both
//! checkers write, as `asset-rules` defines and writes it.
//!
//! Each failing check of a model becomes one finding, in the artist's words
//! (AC11): a title, **what** was found with its figures, **why** it matters in
//! the engine, **how** to fix it, and the technical artist's note on the rule -
//! the text the viewer's Inspector shows beside a finding, from the same
//! catalog. The figures and the offending objects' names are there as values
//! too, for a tool that wants them.
//!
//! Written whenever the run got as far as knowing what it checks against, and
//! a results file an earlier run left at the path is removed first - the
//! contract `--report` keeps: after any run, the file is this run's or is not
//! there.

use std::path::PathBuf;

use asset_rules::Mode;
use asset_rules::results::{
    ContextInfo, FileResult, Finding, Results, RulesInfo, Severity as ResultSeverity, Status,
    ToolInfo,
};
use review_audit::{AuditProfile, AuditReport, Measured, RuleResult, Severity, Threshold};
use review_labels::audit::{finding_what, rule_fix, rule_name, rule_why};
use review_localization::tr;
use review_model::ModelData;
use serde_json::{Map, Value, json};

use crate::args::PROGRAM;
use crate::keys::cli as k;
use crate::report::discard_stale;

/// The type a model has in results written without `--rules`, where no
/// `rules.json` names one: what Asset Check's rules call it.
pub(crate) const MODEL_TYPE: &str = "model";

/// This tool, as results and `rules.json`'s `requires` name it.
pub(crate) fn tool() -> ToolInfo {
    ToolInfo {
        name: PROGRAM.to_owned(),
        version: crate::VERSION.to_owned(),
    }
}

/// An audit severity as results write it.
pub(crate) fn severity(severity: Severity) -> ResultSeverity {
    match severity {
        Severity::Info => ResultSeverity::Info,
        Severity::Warning => ResultSeverity::Warning,
        Severity::Error => ResultSeverity::Error,
    }
}

/// A results severity as the audit's own, which `--fail-on` and the run
/// report are written in.
pub(crate) fn audit_severity(severity: ResultSeverity) -> Severity {
    match severity {
        ResultSeverity::Info => Severity::Info,
        ResultSeverity::Warning => Severity::Warning,
        ResultSeverity::Error => Severity::Error,
    }
}

/// One finding per failing check of `report`, worst first, then in the order
/// the checks are listed.
pub(crate) fn findings(
    report: &AuditReport,
    model: &ModelData,
    profile: &AuditProfile,
) -> Vec<Finding> {
    let mut failing: Vec<&RuleResult> = report.results.iter().filter(|r| r.failed()).collect();
    failing.sort_by_key(|result| std::cmp::Reverse(result.severity));
    failing
        .into_iter()
        .map(|result| finding(result, model, profile))
        .collect()
}

fn finding(result: &RuleResult, model: &ModelData, profile: &AuditProfile) -> Finding {
    let rule = result.rule;
    let mut finding = Finding::new(
        rule.as_str(),
        severity(result.severity),
        tr(rule_name(rule)).into_owned(),
        finding_what(result),
        rule_why(rule, profile.base),
        tr(rule_fix(rule)).into_owned(),
    );
    finding.note = profile
        .rules
        .get(&rule)
        .and_then(|config| config.note.clone());
    finding.measured = Some(measured(result));
    finding.limit = limit(&result.threshold);
    finding.objects = result
        .offender_nodes()
        .filter_map(|node| model.nodes.get(node as usize))
        .map(|node| node.name.clone())
        .collect();
    finding
}

/// `{"value": 25000, "unit": "count", "total": 3}`: the figure stated beside
/// the limit, when there is one, and how many triangles, edges, vertices or
/// objects were found.
fn measured(result: &RuleResult) -> Map<String, Value> {
    let mut map = Map::new();
    if let Some(headline) = result.headline() {
        let (value, unit) = value_and_unit(headline);
        map.insert("value".into(), value);
        map.insert("unit".into(), unit);
    }
    map.insert("total".into(), json!(result.total));
    map
}

/// `{"max": 10000, "unit": "count"}`, `{"min": .., "max": .., "unit": ..}`,
/// `{"equals": "positive_y", "unit": "axis"}` or `{"patterns": [..]}`; `None`
/// for a check with no limit to state.
fn limit(threshold: &Threshold) -> Option<Map<String, Value>> {
    let mut map = Map::new();
    let mut bound = |key: &str, figure: &Measured| {
        let (value, unit) = value_and_unit(figure);
        map.insert(key.into(), value);
        map.insert("unit".into(), unit);
    };
    match threshold {
        Threshold::None => return None,
        Threshold::Max(figure) => bound("max", figure),
        Threshold::Min(figure) => bound("min", figure),
        Threshold::Range { min, max } => {
            bound("min", min);
            bound("max", max);
        }
        Threshold::Equals(figure) => bound("equals", figure),
        Threshold::Patterns(patterns) => {
            map.insert("patterns".into(), json!(patterns));
        }
    }
    Some(map)
}

/// A figure as the audit report writes it - its unit as a word (`count`,
/// `meters`, `axis`) and its value - split in two.
fn value_and_unit(figure: &Measured) -> (Value, Value) {
    let mut written = serde_json::to_value(figure).unwrap_or(Value::Null);
    (shortest(written["value"].take()), written["unit"].take())
}

/// Every number in `value` that is exactly an `f32` - most figures were
/// measured as one - as the shortest number that reads back as it: `0.01` for
/// a centimetre rather than the `0.009999999776482582` its widening to `f64`
/// spells. The audit report's own rule ([`review_audit::report::figure`]).
fn shortest(value: Value) -> Value {
    match value {
        Value::Number(number) if !number.is_i64() && !number.is_u64() => match number.as_f64() {
            Some(wide) if f64::from(wide as f32) == wide => {
                review_audit::report::figure(wide as f32)
            }
            _ => Value::Number(number),
        },
        Value::Array(items) => Value::Array(items.into_iter().map(shortest).collect()),
        other => other,
    }
}

/// Where a run's results go, and the results so far.
pub(crate) struct ResultsOut {
    path: Option<PathBuf>,
    results: Option<Results>,
}

impl ResultsOut {
    /// Results to be written to `path`, if any. A file an earlier run left
    /// there is removed now, so it can never be read as this run's.
    pub(crate) fn start(path: Option<PathBuf>) -> Self {
        if let Some(path) = &path {
            discard_stale(path);
        }
        Self {
            path,
            results: None,
        }
    }

    /// The run knows what it checks against: from here on there are results
    /// to write.
    pub(crate) fn begin(&mut self, results: Results) {
        self.results = Some(results);
    }

    /// Results for a run with no `rules.json` to record: one against a
    /// `--profile`, whose `digest` it is, or one whose `rules.json` could not be
    /// read. `branch` is the current folder's, if it is in a repository. Advise
    /// mode, since there are no rules to say otherwise.
    pub(crate) fn begin_bare(&mut self, digest: String, branch: Option<String>) {
        self.begin(Results {
            tool: tool(),
            rules: RulesInfo {
                commit: None,
                digest,
            },
            context: ContextInfo {
                branch,
                mode: Mode::Advise,
            },
            findings: Vec::new(),
            files: Vec::new(),
        });
    }

    /// A finding about the run rather than any one model.
    pub(crate) fn run_finding(&mut self, finding: Finding) {
        if let Some(results) = &mut self.results {
            results.findings.push(finding);
        }
    }

    pub(crate) fn file(&mut self, file: FileResult) {
        if let Some(results) = &mut self.results {
            results.files.push(file);
        }
    }

    /// Write the results, when they were asked for and the run got far enough
    /// to have them. `false` when they could not be written, which fails the
    /// run: a hook waiting on the file would otherwise read nothing and be told
    /// everything worked.
    pub(crate) fn finish(self) -> bool {
        let (Some(path), Some(results)) = (self.path, self.results) else {
            return true;
        };
        match results.write(&path) {
            Ok(()) => true,
            Err(error) => {
                crate::console::warn(&k::results_failed(
                    path.display().to_string(),
                    error.to_string(),
                ));
                false
            }
        }
    }
}

/// A model that could not be checked, as results record it.
pub(crate) fn unreadable(mut file: FileResult, problem: String) -> FileResult {
    file.status = Status::Unreadable;
    file.problem = Some(problem);
    file.findings.clear();
    file
}

/// A checked model's entry when it was judged against one `--profile`: no
/// `rules.json` named its type, profile or stage, so the profile is the one
/// `--profile` gave (`id` as given, `label` as shown) and there is no stage.
pub(crate) fn profile_entry(path: String, id: &str, label: String) -> FileResult {
    FileResult {
        path,
        kind: Some(MODEL_TYPE.to_owned()),
        profile: Some(id.to_owned()),
        profile_label: Some(label),
        stage: None,
        stage_from: None,
        status: Status::Checked,
        problem: None,
        findings: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_limit_carries_its_bound_and_unit() {
        assert_eq!(
            Value::Object(limit(&Threshold::Max(Measured::Count(10_000))).unwrap()),
            json!({ "max": 10_000, "unit": "count" })
        );
        assert_eq!(
            Value::Object(
                limit(&Threshold::Range {
                    min: Measured::PxPerMeter(256.0),
                    max: Measured::PxPerMeter(1024.0),
                })
                .unwrap()
            ),
            json!({ "min": 256.0, "max": 1024.0, "unit": "px_per_meter" })
        );
        assert_eq!(
            Value::Object(
                limit(&Threshold::Equals(Measured::Axis(
                    review_audit::Axis::PositiveY
                )))
                .unwrap()
            ),
            json!({ "equals": "positive_y", "unit": "axis" })
        );
        assert_eq!(limit(&Threshold::None), None);
    }

    /// A centimetre measured as an `f32` reads `0.01`, not the seventeen
    /// digits of its widening; a figure that was never an `f32` keeps its own.
    #[test]
    fn figures_measured_as_f32_are_written_shortest() {
        let centimetre = f64::from(0.01_f32);
        assert_eq!(
            value_and_unit(&Measured::UnitMeters(centimetre)).0,
            json!(0.01)
        );
        assert_eq!(value_and_unit(&Measured::Ratio(0.1)).0, json!(0.1));
        assert_eq!(value_and_unit(&Measured::Count(4)).0, json!(4));
        assert_eq!(
            value_and_unit(&Measured::Scale([0.01, 1.0, 2.5])).0,
            json!([0.01, 1.0, 2.5])
        );
    }
}
