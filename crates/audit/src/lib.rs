//! The model audit: checks a loaded model against a rule profile and reports
//! what it finds.
//!
//! Host-agnostic and text-free, like `review-optimize`: it takes a
//! [`ModelData`](review_model::ModelData) (plus the source-property capture
//! when there is one) and returns typed results. The viewer's Aud workspace,
//! and later a headless batch run, turn those into text through their own
//! catalogs.
//!
//! A run is per rule: each enabled rule measures the model and judges what it
//! measured against its profile parameters. A re-run passed the previous report
//! ([`run`]'s `reuse`) keeps every rule whose settings did not change, so
//! editing one threshold re-measures one rule.

#![forbid(unsafe_code)]

mod checks;
mod context;
pub mod finding;
pub mod glob;
pub mod profile;
pub mod report;
pub mod rule;

use std::time::Instant;

pub use context::AuditInput;
pub use finding::{
    AuditReport, AuditSummary, Axis, ElementSet, Measured, Offender, RangeSet, RuleResult, Skip,
    Status, Threshold,
};
pub use profile::{
    AuditProfile, Engine, ParamKind, ParamSpec, ParamValue, RuleConfig, param_specs,
};
pub use rule::{Category, DiagnosticView, ElementKind, Marker, RuleId, Severity};

use context::Context;
use review_model::CancelToken;

/// Element lists are capped per rule so a pathological model cannot hold
/// gigabytes of offenders. Counts stay exact.
pub const ELEMENT_CAP: u64 = 1 << 20;

/// Why a run or a profile operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuditError {
    #[error("the audit was cancelled")]
    Cancelled,
    #[error("audit profile could not be read: {0}")]
    Profile(String),
    #[error("audit profile is version {found}; this build reads up to {supported}")]
    ProfileVersion { found: u32, supported: u32 },
    #[error("audit report could not be written: {0}")]
    Report(String),
}

/// How a run is carried out.
#[derive(Debug, Clone, Copy, Default)]
pub struct RunOptions<'a> {
    pub cancel: Option<&'a CancelToken>,
    /// Worker threads for the per-object checks; 0 or 1 runs serially.
    pub threads: usize,
}

/// Audit `input` against `profile`.
///
/// `reuse` is the previous report *for the same model*: every rule whose
/// settings are unchanged keeps its result (re-labelled with its current
/// severity), so editing one threshold re-measures one rule. Passing a report
/// for a different model gives wrong results — the caller drops it on a load.
pub fn run(
    input: AuditInput<'_>,
    profile: &AuditProfile,
    reuse: Option<&AuditReport>,
    options: RunOptions<'_>,
) -> Result<AuditReport, AuditError> {
    let started = Instant::now();
    let ctx = Context::new(input, options.cancel, options.threads);
    let mut results = Vec::with_capacity(RuleId::ALL.len());

    for &rule in RuleId::ALL {
        if ctx.cancelled() {
            return Err(AuditError::Cancelled);
        }
        let config = profile.rule(rule);
        let key = finding_key(profile, rule, &config);
        if let Some(previous) = reuse
            .and_then(|report| report.result(rule))
            .filter(|previous| previous.measure_key == key)
        {
            let mut result = previous.clone();
            result.severity = config.severity;
            results.push(result);
            continue;
        }

        let _zone = review_prof::zone!("Audit Rule");
        let outcome = if config.enabled {
            checks::evaluate(rule, &ctx, &config, profile)
        } else {
            checks::Outcome::skip(Skip::Disabled)
        };
        if outcome.status == Status::NotEvaluated(Skip::Cancelled) || ctx.cancelled() {
            return Err(AuditError::Cancelled);
        }
        results.push(outcome.into_result(rule, &config, key));
    }

    let mut report = AuditReport {
        profile_digest: profile.digest(),
        results,
        summary: AuditSummary::default(),
        elapsed_ms: 0.0,
    };
    report.summarize(input.model.nodes.len());
    report.elapsed_ms = started.elapsed().as_secs_f32() * 1000.0;
    Ok(report)
}

/// What decides whether a rule's previous result still holds: its own
/// settings, plus — for the reduce rule, which reads them — the LOD rule's
/// parameters.
fn finding_key(profile: &AuditProfile, rule: RuleId, config: &RuleConfig) -> u64 {
    let own = config.finding_key();
    match rule {
        RuleId::TriangleReduce => {
            own ^ profile
                .rule(RuleId::TriangleLod)
                .finding_key()
                .rotate_left(17)
        }
        _ => own,
    }
}
