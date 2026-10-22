//! `review-cli audit`: every model checked against one profile, the way the
//! Aud workspace checks the model it has open.
//!
//! The same engine, the same source-property capture and the same report: each
//! model's entry carries the document the viewer's Save report writes
//! (`3d-review.audit-report`), so a pipeline reads one schema whichever wrote
//! it. The profile is a built-in or a file, and never the viewer's last-used
//! one - a run is what its command line says, on any machine.
//!
//! The audit itself gives no verdict (the viewer's findings are information);
//! `--fail-on` is where a run gets one. A model with findings at that severity
//! or worse is checked - its entry is `done` - but the run exits 1, so a tool
//! that stops on a failed step stops on it.

use review_audit::profile::envelope;
use review_audit::report::{ReportSource, to_value};
use review_audit::{AuditInput, AuditProfile, RunOptions, Severity};
use review_labels::audit::{explain_audit_error, profile_name, severity, summary_lines};
use review_localization::tr;
use serde_json::json;

use crate::args::{AuditArgs, CommandName, Common, FailOn, ProfileChoice};
use crate::console::{self, Bar};
use crate::inputs;
use crate::keys::cli as k;
use crate::load;
use crate::report::{Entry, Report, Status, generator, now_unix};

/// A model's share of the progress bar that loading it takes.
const LOADED: f32 = 0.5;

pub(crate) fn run(common: &Common, args: &AuditArgs) -> bool {
    let bar = Bar::new(common.progress_bar);
    let mut report = Report::start(common.report.clone(), CommandName::Audit);
    let flagged = work(common, args, &bar, &mut report);
    bar.clear();
    let ok = !report.has_problem() && report.failed() == 0 && flagged == 0;
    report.finish(ok) && ok
}

/// Check every model; how many reached `--fail-on`.
fn work(common: &Common, args: &AuditArgs, bar: &Bar, report: &mut Report) -> usize {
    let profile = match resolve(&args.profile) {
        Ok(profile) => profile,
        Err(problem) => {
            console::warn(&problem);
            report.problem(problem);
            return 0;
        }
    };
    report.total(
        "profile",
        json!(match &args.profile {
            ProfileChoice::Builtin(engine) => engine.as_str().to_owned(),
            ProfileChoice::File(path) => path.display().to_string(),
        }),
    );
    report.total(
        "fail_on",
        json!(match args.fail_on {
            FailOn::At(severity) => Some(severity),
            FailOn::Never => None,
        }),
    );

    let models = inputs::gather(common, report);
    if models.is_empty() {
        return 0;
    }

    let count = models.len();
    console::say(&k::audit_start(count as f64, profile_name(&profile)));
    // Two cores left free, as the viewer leaves them.
    let threads = std::thread::available_parallelism()
        .map_or(1, |cores| cores.get().saturating_sub(2).max(1));
    let mut found = [0u32; 3];
    let (mut done, mut flagged) = (0usize, 0usize);
    for (index, model) in models.iter().enumerate() {
        console::heading(&format!("[{}/{count}] {}", index + 1, model.label()));
        let progress = |fraction: f32| bar.set((index as f32 + fraction) / count as f32);
        let mut entry = Entry::new(&model.label());
        let failed = |entry: Entry, problem: String, report: &mut Report| {
            console::warn(&format!(
                "  {}",
                k::audit_failed(model.label(), problem.clone())
            ));
            report.add(Status::Failed, entry, Some(problem));
        };

        let loaded = match load::load(&model.path, &|fraction| progress(fraction * LOADED)) {
            Ok(loaded) => loaded,
            Err(problem) => {
                failed(entry, problem, report);
                continue;
            }
        };
        for warning in &loaded.warnings {
            console::warn(&format!("  {}", k::warning(warning.clone())));
            entry.notice(warning.clone());
        }
        let audit = review_audit::run(
            AuditInput {
                model: &loaded.model,
                extras: loaded.extras.as_ref(),
            },
            &profile,
            None,
            RunOptions {
                cancel: None,
                threads,
            },
        );
        let audit = match audit {
            Ok(audit) => audit,
            Err(error) => {
                failed(entry, explain_audit_error(&error), report);
                continue;
            }
        };
        let document = to_value(
            &audit,
            &loaded.model,
            loaded.extras.as_ref(),
            &profile,
            &ReportSource {
                file: model.label(),
                generator: generator(),
                created_unix: now_unix(),
            },
        );
        let document = match document {
            Ok(document) => document,
            Err(error) => {
                failed(entry, explain_audit_error(&error), report);
                continue;
            }
        };

        for line in summary_lines(&audit) {
            console::say(&format!("  {line}"));
        }
        let summary = &audit.summary;
        let reached = match args.fail_on {
            FailOn::At(least) => summary.worst.is_some_and(|worst| worst >= least),
            FailOn::Never => false,
        };
        for severity in Severity::ALL {
            found[severity.index()] += summary.failed[severity.index()];
        }
        entry.set("flagged", json!(reached));
        entry.set("worst", json!(summary.worst));
        entry.set("errors", json!(summary.failed[Severity::Error.index()]));
        entry.set("warnings", json!(summary.failed[Severity::Warning.index()]));
        entry.set("infos", json!(summary.failed[Severity::Info.index()]));
        entry.set("passed", json!(summary.passed));
        entry.set("not_evaluated", json!(summary.not_evaluated));
        entry.set("audit", document);
        report.add(Status::Done, entry, None);
        done += 1;
        if reached {
            flagged += 1;
        }
        progress(1.0);
    }

    console::say(&k::audit_done(done as f64, report.failed() as f64));
    if let FailOn::At(least) = args.fail_on
        && flagged > 0
    {
        console::warn(&k::audit_flagged(
            flagged as f64,
            tr(severity(least)).into_owned(),
        ));
    }
    report.total("flagged", json!(flagged));
    report.total("errors", json!(found[Severity::Error.index()]));
    report.total("warnings", json!(found[Severity::Warning.index()]));
    report.total("infos", json!(found[Severity::Info.index()]));
    flagged
}

/// The profile `--profile` names.
fn resolve(choice: &ProfileChoice) -> Result<AuditProfile, String> {
    match choice {
        ProfileChoice::Builtin(engine) => Ok(AuditProfile::builtin(*engine)),
        ProfileChoice::File(path) => {
            let unreadable =
                |reason: String| k::audit_profile_unreadable(path.display().to_string(), reason);
            let json =
                std::fs::read_to_string(path).map_err(|error| unreadable(error.to_string()))?;
            envelope::from_json(&json).map_err(|error| unreadable(explain_audit_error(&error)))
        }
    }
}
