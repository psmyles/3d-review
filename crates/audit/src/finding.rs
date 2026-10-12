//! What a run produces: one [`RuleResult`] per rule, collected in an
//! [`AuditReport`].
//!
//! Everything here is typed data — no prose. The UI and the report writer turn
//! it into text through the catalog, and a later verdict is a pure function
//! over these fields.

use serde::{Deserialize, Serialize};

use crate::rule::{RuleId, Severity};

/// Whether a rule ran, and what it found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status", content = "reason")]
pub enum Status {
    Pass,
    Fail,
    NotEvaluated(Skip),
}

/// Why a rule did not run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Skip {
    /// Switched off in the profile.
    Disabled,
    /// It needs the file's source properties, which this load has none of.
    SourcePropertiesUnavailable,
    /// A skin check on a model with no skin.
    NoSkin,
    /// A UV check on a model without the UV set it reads.
    NoUvSet,
    /// A naming pattern in the profile could not be parsed.
    InvalidPattern,
    /// Nothing to check: the model has no geometry or no nodes.
    EmptyModel,
    /// The run was cancelled before reaching this rule.
    Cancelled,
}

/// The coordinate axis a scene declares as up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    PositiveX,
    NegativeX,
    PositiveY,
    NegativeY,
    PositiveZ,
    NegativeZ,
}

/// A measured figure, with its unit carried in the variant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "unit", content = "value")]
pub enum Measured {
    Count(u64),
    /// A unitless ratio in `0..=1` (shown as a percentage).
    Ratio(f64),
    Meters(f64),
    Degrees(f64),
    PerSquareMeter(f64),
    PxPerMeter(f64),
    /// Square pixels (an on-screen area).
    SquarePixels(f64),
    /// A screen size: the object's bounding-sphere diameter over the screen
    /// height.
    ScreenSize(f64),
    /// A scene unit, as meters per unit.
    UnitMeters(f64),
    Axis(Axis),
    Scale([f32; 3]),
    /// A name or pattern.
    Text(String),
}

/// The bound a measured figure was judged against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum Threshold {
    None,
    Max(Measured),
    Min(Measured),
    Range { min: Measured, max: Measured },
    Equals(Measured),
    Patterns(Vec<String>),
}

/// A sorted set of `u32`s stored as half-open runs. Triangles of one polygon,
/// and of one object, are contiguous, so offender triangle sets compress well.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RangeSet {
    runs: Vec<(u32, u32)>,
}

impl RangeSet {
    /// Build from ascending values (duplicates allowed).
    pub fn from_sorted(values: impl IntoIterator<Item = u32>) -> Self {
        let mut runs: Vec<(u32, u32)> = Vec::new();
        for value in values {
            match runs.last_mut() {
                Some((_, end)) if value < *end => {}
                Some((_, end)) if value == *end => *end += 1,
                _ => runs.push((value, value + 1)),
            }
        }
        Self { runs }
    }

    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// How many values the set holds.
    pub fn len(&self) -> u64 {
        self.runs
            .iter()
            .map(|&(start, end)| u64::from(end - start))
            .sum()
    }

    pub fn contains(&self, value: u32) -> bool {
        let index = self.runs.partition_point(|&(_, end)| end <= value);
        self.runs
            .get(index)
            .is_some_and(|&(start, _)| start <= value)
    }

    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.runs.iter().flat_map(|&(start, end)| start..end)
    }

    pub fn runs(&self) -> &[(u32, u32)] {
        &self.runs
    }

    /// Keep at most `cap` values, returning whether any were dropped.
    pub fn truncate(&mut self, cap: u64) -> bool {
        let mut kept = 0_u64;
        for index in 0..self.runs.len() {
            let (start, end) = self.runs[index];
            let length = u64::from(end - start);
            if kept + length > cap {
                let keep = (cap - kept) as u32;
                self.runs[index].1 = start + keep;
                let cut = if keep == 0 { index } else { index + 1 };
                self.runs.truncate(cut);
                return true;
            }
            kept += length;
        }
        false
    }
}

/// The elements of one offender — what to highlight.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "items")]
pub enum ElementSet {
    /// Object- or scene-level: nothing finer than the node.
    #[default]
    None,
    /// Global triangle indices.
    Triangles(RangeSet),
    /// Render-corner pairs, the `wire` program's edge-list format.
    Edges(Vec<[u32; 2]>),
    /// One representative render corner per logical vertex.
    Vertices(Vec<u32>),
    /// Points with no render corner of their own (unused control points, a
    /// pivot, a bone): world position plus an id — the logical vertex or the
    /// node.
    Points(Vec<([f32; 3], u32)>),
}

impl ElementSet {
    pub fn len(&self) -> u64 {
        match self {
            ElementSet::None => 0,
            ElementSet::Triangles(set) => set.len(),
            ElementSet::Edges(edges) => edges.len() as u64,
            ElementSet::Vertices(vertices) => vertices.len() as u64,
            ElementSet::Points(points) => points.len() as u64,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Keep at most `cap` elements, returning whether any were dropped.
    pub fn truncate(&mut self, cap: u64) -> bool {
        let cap_usize = usize::try_from(cap).unwrap_or(usize::MAX);
        match self {
            ElementSet::None => false,
            ElementSet::Triangles(set) => set.truncate(cap),
            ElementSet::Edges(items) => truncate_vec(items, cap_usize),
            ElementSet::Vertices(items) => truncate_vec(items, cap_usize),
            ElementSet::Points(items) => truncate_vec(items, cap_usize),
        }
    }
}

fn truncate_vec<T>(items: &mut Vec<T>, cap: usize) -> bool {
    let dropped = items.len() > cap;
    items.truncate(cap);
    dropped
}

/// One object (or the scene) failing one rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Offender {
    /// The node, indexing `ModelData::nodes`; `None` for a scene-wide finding.
    pub node: Option<u32>,
    /// How many elements of this object offend — exact even when `elements`
    /// was capped.
    pub count: u64,
    /// The object's own figure, where the rule measures one per object.
    pub measured: Option<Measured>,
    /// A second figure that explains the first (a density beside the screen
    /// size it implies).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Measured>,
    pub elements: ElementSet,
}

impl Offender {
    /// An object-level offender with no finer elements.
    pub fn node(node: usize, measured: Option<Measured>) -> Self {
        Self {
            node: Some(node as u32),
            count: 1,
            measured,
            detail: None,
            elements: ElementSet::None,
        }
    }

    /// An offender listing `elements` of `node`.
    pub fn elements(node: Option<usize>, elements: ElementSet) -> Self {
        Self {
            node: node.map(|node| node as u32),
            count: elements.len(),
            measured: None,
            detail: None,
            elements,
        }
    }
}

/// One rule's outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleResult {
    pub rule: RuleId,
    pub status: Status,
    pub severity: Severity,
    /// The headline figure (the worst object's, or the scene's).
    pub measured: Option<Measured>,
    pub threshold: Threshold,
    pub offenders: Vec<Offender>,
    /// Offending elements across every object, exact.
    pub total: u64,
    /// Whether any offender's element list was capped.
    pub truncated: bool,
    /// A hash of the profile parameters that change what this rule measures.
    /// A re-run whose key matches can re-judge from the stored measurements.
    #[serde(skip)]
    pub measure_key: u64,
}

impl RuleResult {
    pub fn failed(&self) -> bool {
        self.status == Status::Fail
    }

    /// The offender for `node`, if that object fails this rule.
    pub fn offender(&self, node: u32) -> Option<&Offender> {
        self.offenders
            .iter()
            .find(|offender| offender.node == Some(node))
    }
}

/// Counts over a whole report.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AuditSummary {
    /// Failing rules per severity, `[Info, Warning, Error]`.
    pub failed: [u32; 3],
    pub passed: u32,
    pub not_evaluated: u32,
    pub worst: Option<Severity>,
    /// Per node, the worst severity of any rule it fails.
    pub node_worst: Vec<Option<Severity>>,
}

impl AuditSummary {
    /// Failing rules at Warning or Error — the count the toolbar shows.
    pub fn attention_count(&self) -> u32 {
        self.failed[Severity::Warning.index()] + self.failed[Severity::Error.index()]
    }

    /// The worst severity among Warning and Error findings.
    pub fn attention_worst(&self) -> Option<Severity> {
        self.worst.filter(|severity| *severity > Severity::Info)
    }
}

/// A whole run's outcome.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AuditReport {
    /// A digest of the resolved profile the report was judged against.
    pub profile_digest: u64,
    pub results: Vec<RuleResult>,
    pub summary: AuditSummary,
    pub elapsed_ms: f32,
}

impl AuditReport {
    pub fn result(&self, rule: RuleId) -> Option<&RuleResult> {
        self.results.iter().find(|result| result.rule == rule)
    }

    /// Recompute [`AuditReport::summary`] from the results.
    pub fn summarize(&mut self, node_count: usize) {
        let mut summary = AuditSummary {
            node_worst: vec![None; node_count],
            ..Default::default()
        };
        for result in &self.results {
            match result.status {
                Status::Pass => summary.passed += 1,
                Status::NotEvaluated(_) => summary.not_evaluated += 1,
                Status::Fail => {
                    summary.failed[result.severity.index()] += 1;
                    summary.worst = summary.worst.max(Some(result.severity));
                    for offender in &result.offenders {
                        if let Some(slot) = offender
                            .node
                            .and_then(|node| summary.node_worst.get_mut(node as usize))
                        {
                            *slot = (*slot).max(Some(result.severity));
                        }
                    }
                }
            }
        }
        self.summary = summary;
    }

    /// Approximate heap size of the report, for the log and the gate stamp.
    pub fn heap_bytes(&self) -> usize {
        let elements: usize = self
            .results
            .iter()
            .flat_map(|result| &result.offenders)
            .map(|offender| match &offender.elements {
                ElementSet::None => 0,
                ElementSet::Triangles(set) => set.runs().len() * 8,
                ElementSet::Edges(edges) => edges.len() * 8,
                ElementSet::Vertices(vertices) => vertices.len() * 4,
                ElementSet::Points(points) => points.len() * 16,
            })
            .sum();
        let offenders: usize = self
            .results
            .iter()
            .map(|result| result.offenders.len() * std::mem::size_of::<Offender>())
            .sum();
        elements
            + offenders
            + self.results.len() * std::mem::size_of::<RuleResult>()
            + self.summary.node_worst.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_sets_compress_runs_and_answer_membership() {
        let set = RangeSet::from_sorted([1, 2, 3, 7, 8, 20]);
        assert_eq!(set.runs(), &[(1, 4), (7, 9), (20, 21)]);
        assert_eq!(set.len(), 6);
        assert!(set.contains(2) && set.contains(8) && set.contains(20));
        assert!(!set.contains(0) && !set.contains(4) && !set.contains(19));
        assert_eq!(set.iter().collect::<Vec<_>>(), vec![1, 2, 3, 7, 8, 20]);
    }

    #[test]
    fn range_sets_truncate_mid_run() {
        let mut set = RangeSet::from_sorted([1, 2, 3, 7, 8]);
        assert!(set.truncate(4));
        assert_eq!(set.iter().collect::<Vec<_>>(), vec![1, 2, 3, 7]);
        assert!(!set.truncate(10));
    }
}
