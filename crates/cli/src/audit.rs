//! `3d-review-cli audit`: every model checked against its profile, the way the
//! Aud workspace checks the model it has open.
//!
//! The same engine, the same source-property capture and the same report: each
//! model's entry carries the document the viewer's Save report writes
//! (`3d-review.audit-report`), so a pipeline reads one schema whichever wrote
//! it. The profile is a built-in or a file, and never the viewer's last-used
//! one - a run is what its command line says, on any machine.
//!
//! Or, with `--rules`, Asset Check's (AC13): `rules.json` picks each model's
//! profile and stage from its name, its folder and the branch, through
//! `asset-rules`, so `fire-cli` resolves a file the same way. `--results`
//! writes what it found in Asset Check's format, what / why / how included.
//! `--staged` checks the models staged in the current folder's repository, as
//! staged; there, each result is cached against the staged content, the rules
//! file that applied, this tool's version and the stage, so checking an
//! unchanged commit again is all but instant.
//!
//! A run is two passes. The first, on this thread and in order, works out what
//! each model is checked against - resolving it, reading its rules file, asking
//! the cache - which is cheap. The second loads and audits what is left, a few
//! models at a time: loading a model is one thread's work, so a commit of
//! twenty models is checked in a fraction of the time it takes one after
//! another. Each model's lines are held until it is done and said in the
//! models' order, so the output, the report and the results are the same
//! whichever model finishes first.
//!
//! The audit itself gives no verdict (the viewer's findings are information);
//! `--fail-on` is where a run gets one. A model with findings at that severity
//! or worse is checked - its entry is `done` - but the run exits 1, so a tool
//! that stops on a failed step stops on it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use asset_rules::cache::{Cache, CacheKey, digest_file};
use asset_rules::git::{Repo, StagedFile};
use asset_rules::results::{FileResult, Finding, Results};
use asset_rules::{Context, Resolution, Resolved, Rules, StageFrom, resolve};
use review_audit::profile::envelope;
use review_audit::report::{ReportSource, to_value};
use review_audit::{AuditInput, AuditProfile, RuleId, RunOptions, Severity};
use review_labels::audit::{
    explain_audit_error, finding_line, profile_name, severity, summary_lines,
};
use review_localization::tr;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::args::{AuditArgs, CommandName, Common, FailOn, Judge, ProfileChoice};
use crate::console::{self, Bar};
use crate::inputs;
use crate::keys::cli as k;
use crate::load;
use crate::report::{Entry, Report, Status, generator, now_unix};
use crate::results::{self, ResultsOut, audit_severity, profile_entry, tool, unreadable};
use crate::staged;

/// The most models loaded and audited at once. Each holds a whole model in
/// memory, and past four the audits' own threads have the cores anyway.
const MAX_MODELS_AT_ONCE: usize = 4;

pub(crate) fn run(common: &Common, args: &AuditArgs) -> bool {
    let bar = Bar::new(common.progress_bar);
    let mut report = Report::start(common.report.clone(), CommandName::Audit);
    let mut results = ResultsOut::start(args.results.clone());
    let reached = work(common, args, &bar, &mut report, &mut results);
    bar.clear();
    let ok = !report.has_problem() && report.failed() == 0 && !reached;
    // Both are written, whatever the other did.
    let results_written = results.finish();
    let report_written = report.finish(ok && results_written);
    report_written && results_written && ok
}

/// One model to check: where its content is, and how lines and results name
/// it.
struct Subject {
    /// The path as reached, or the repository path of a staged model.
    label: String,
    source: Source,
}

enum Source {
    File(PathBuf),
    /// Staged in the run's repository: read through it, and cached by its
    /// blob.
    Staged(StagedFile),
}

/// What each model is judged against.
enum Judging {
    Profile {
        profile: Arc<AuditProfile>,
        /// The `--profile` value as given, which results name it by.
        id: String,
    },
    Rules {
        rules: Box<Rules>,
        /// Each rules file a profile named, loaded once.
        files: HashMap<PathBuf, Result<RulesFile, String>>,
    },
}

/// A model rules file: the profile in it, and the digest the cache keys on.
struct RulesFile {
    profile: Arc<AuditProfile>,
    digest: String,
}

/// What working out each model's rules needs.
struct Run {
    judging: Judging,
    /// The repository the current folder is in, and its branch.
    ctx: Context,
    /// The repository a `--staged` run reads, and its cache.
    staged: Option<(Repo, Cache)>,
}

/// What the run has found so far, over every model.
struct Tally {
    fail_on: FailOn,
    /// Failing checks per severity, `[Info, Warning, Error]`.
    found: [u32; 3],
    done: usize,
    /// Models with a finding at `--fail-on` or worse.
    flagged: usize,
}

/// Check every model; whether any finding, a model's or the run's, reached
/// `--fail-on`.
fn work(
    common: &Common,
    args: &AuditArgs,
    bar: &Bar,
    report: &mut Report,
    results: &mut ResultsOut,
) -> bool {
    report.total(
        "fail_on",
        json!(match args.fail_on {
            FailOn::At(severity) => Some(severity),
            FailOn::Never => None,
        }),
    );
    // Asking git costs two processes, so only a run that names models by their
    // repository path, or records the branch, asks.
    let ctx = if matches!(args.judge, Judge::Rules(_)) || args.results.is_some() || args.staged {
        crate::rules::context()
    } else {
        Context::default()
    };
    let (judging, opening, too_old) = match &args.judge {
        Judge::Profile(choice) => {
            let id = match choice {
                ProfileChoice::Builtin(engine) => engine.as_str().to_owned(),
                ProfileChoice::File(path) => path.display().to_string(),
            };
            report.total("profile", json!(id));
            let (profile, digest) = match resolve_profile(choice) {
                Ok(resolved) => resolved,
                Err(problem) => {
                    console::warn(&problem);
                    report.problem(problem);
                    return false;
                }
            };
            results.begin_bare(digest, ctx.branch.clone());
            let opening = Opening::Profile(profile_name(&profile));
            let profile = Arc::new(profile);
            (Judging::Profile { profile, id }, opening, None)
        }
        Judge::Rules(path) => {
            report.total("profile", Value::Null);
            report.total("rules", json!(path.display().to_string()));
            let rules = match Rules::load(path) {
                Ok(rules) => rules,
                Err(error) => {
                    let problem =
                        k::rules_unreadable(path.display().to_string(), error.to_string());
                    console::warn(&problem);
                    report.problem(problem);
                    // A rules.json too new to read can still say what to update.
                    if let Some(finding) = error.too_old(&tool()) {
                        say_finding("", &finding);
                        results.begin_bare(digest_file(path).unwrap_or_default(), ctx.branch);
                        results.run_finding(finding);
                    }
                    return false;
                }
            };
            let commit = Repo::discover(rules.dir())
                .and_then(|repo| repo.head())
                .ok()
                .flatten();
            let mut header = Results::new(tool(), &rules, commit, &ctx);
            let too_old = rules.too_old(&tool());
            header.findings.extend(too_old.clone());
            results.begin(header);
            let judging = Judging::Rules {
                rules: Box::new(rules),
                files: HashMap::new(),
            };
            (judging, Opening::Rules(path.display().to_string()), too_old)
        }
    };

    let mut run = Run {
        judging,
        ctx,
        staged: None,
    };
    let subjects: Vec<Subject> = if args.staged {
        let staged = match staged::gather() {
            Ok(staged) => staged,
            Err(problem) => {
                console::warn(&problem);
                report.problem(problem);
                return false;
            }
        };
        report.expect(staged.models.len());
        let subjects = staged
            .models
            .into_iter()
            .map(|file| Subject {
                label: file.path.clone(),
                source: Source::Staged(file),
            })
            .collect();
        // The rules' context is the staged repository's.
        run.ctx = staged.repo.context().unwrap_or(run.ctx);
        let cache = Cache::open(&staged.repo);
        // Entries nobody has used for a month. Nothing depends on this.
        let _ = cache.prune();
        run.staged = Some((staged.repo, cache));
        subjects
    } else {
        inputs::gather(common, report)
            .into_iter()
            .map(|model| Subject {
                label: model.label(),
                source: Source::File(model.path),
            })
            .collect()
    };

    let count = subjects.len();
    if count > 0 {
        console::say(&match opening {
            Opening::Profile(name) => k::audit_start(count as f64, name),
            Opening::Rules(file) => k::audit_start_rules(count as f64, file),
        });
    } else if args.staged {
        console::say(&tr(k::STAGED_NONE));
    }
    // A finding about the run - this tool older than the rules ask - fails it
    // at --fail-on as any model's finding would, models or none. It is the
    // results' to record; the report counts models.
    let run_reached = too_old.as_ref().is_some_and(|finding| {
        say_finding("", finding);
        reaches(args.fail_on, audit_severity(finding.severity))
    });

    // The first pass: what each model is checked against.
    let rules_mode = matches!(run.judging, Judging::Rules { .. });
    let mut outcomes: Vec<Option<Outcome>> = Vec::with_capacity(count);
    let mut jobs = Vec::new();
    for (index, subject) in subjects.iter().enumerate() {
        match verdict(&mut run, subject) {
            Verdict::Check {
                profile,
                entry,
                key,
            } => {
                outcomes.push(None);
                jobs.push(Job {
                    index,
                    profile,
                    file: entry,
                    key,
                });
            }
            decided => outcomes.push(Some(Outcome::decided(decided, subject, rules_mode))),
        }
    }

    // The second: load and audit the rest, saying each model in order as soon
    // as it and every model before it is done.
    let mut tally = Tally {
        fail_on: args.fail_on,
        found: [0; 3],
        done: 0,
        flagged: 0,
    };
    let cores = std::thread::available_parallelism().map_or(1, |cores| cores.get());
    let workers = jobs.len().min(MAX_MODELS_AT_ONCE).min((cores / 4).max(1));
    // Two cores left free, as the viewer leaves them, shared by the models
    // being audited at once.
    let threads = (cores.saturating_sub(2) / workers.max(1)).max(1);
    let staged = run.staged.as_ref();
    let mut said = 0;
    let mut finished = outcomes.iter().filter(|outcome| outcome.is_some()).count();
    let mut say_ready = |outcomes: &mut [Option<Outcome>]| {
        while let Some(outcome) = outcomes.get_mut(said).and_then(Option::take) {
            let subject = &subjects[said];
            console::heading(&format!("[{}/{count}] {}", said + 1, subject.label));
            outcome.record(&mut tally, report, results);
            said += 1;
        }
    };
    say_ready(&mut outcomes);
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        for _ in 0..workers {
            let sender = sender.clone();
            let (jobs, subjects, next) = (&jobs, &subjects, &next);
            scope.spawn(move || {
                while let Some(job) = jobs.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let subject = &subjects[job.index];
                    let outcome = check(job, subject, staged, threads, rules_mode);
                    if sender.send((job.index, outcome)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);
        bar.set(finished as f32 / count.max(1) as f32);
        for (index, outcome) in receiver {
            outcomes[index] = Some(outcome);
            finished += 1;
            bar.set(finished as f32 / count as f32);
            say_ready(&mut outcomes);
        }
    });

    if count > 0 {
        console::say(&k::audit_done(tally.done as f64, report.failed() as f64));
    }
    if let FailOn::At(least) = args.fail_on
        && tally.flagged > 0
    {
        console::warn(&k::audit_flagged(
            tally.flagged as f64,
            tr(severity(least)).into_owned(),
        ));
    }
    report.total("flagged", json!(tally.flagged));
    report.total("errors", json!(tally.found[Severity::Error.index()]));
    report.total("warnings", json!(tally.found[Severity::Warning.index()]));
    report.total("infos", json!(tally.found[Severity::Info.index()]));
    tally.flagged > 0 || run_reached
}

/// What the run's first line names: the profile, or the rules file.
enum Opening {
    Profile(String),
    Rules(String),
}

/// Whether a finding at `severity` fails a run that fails on `fail_on`.
fn reaches(fail_on: FailOn, severity: Severity) -> bool {
    match fail_on {
        FailOn::At(least) => severity >= least,
        FailOn::Never => false,
    }
}

/// A finding said in a line or two: its severity and what, then how.
fn say_finding(indent: &str, finding: &Finding) {
    for line in finding_lines(indent, finding) {
        line.say();
    }
}

fn finding_lines(indent: &str, finding: &Finding) -> [Line; 2] {
    let line = k::audit_finding(
        tr(severity(audit_severity(finding.severity))).into_owned(),
        finding.what.clone(),
    );
    [
        Line::Warn(format!("{indent}{line}")),
        Line::Warn(format!("{indent}{}", finding.how)),
    ]
}

/// The profile `--profile` names, and the digest results record it by: the
/// file's, or a built-in's saved form.
fn resolve_profile(choice: &ProfileChoice) -> Result<(AuditProfile, String), String> {
    match choice {
        ProfileChoice::Builtin(engine) => {
            let profile = AuditProfile::builtin(*engine);
            let saved = envelope::to_json(&profile).unwrap_or_default();
            Ok((profile, sha256(saved.as_bytes())))
        }
        ProfileChoice::File(path) => {
            let unreadable =
                |reason: String| k::audit_profile_unreadable(path.display().to_string(), reason);
            let bytes = std::fs::read(path).map_err(|error| unreadable(error.to_string()))?;
            let json = String::from_utf8_lossy(&bytes);
            let profile = envelope::from_json(&json)
                .map_err(|error| unreadable(explain_audit_error(&error)))?;
            Ok((profile, sha256(&bytes)))
        }
    }
}

/// `sha256:<hex>`, as `asset-rules` writes a digest.
fn sha256(bytes: &[u8]) -> String {
    let hex: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("sha256:{hex}")
}

/// What one model is checked against, once its rules are worked out.
enum Verdict {
    /// Check it against this profile; `entry` is its results entry so far.
    Check {
        profile: Arc<AuditProfile>,
        entry: FileResult,
        /// Where a `--staged` run caches it.
        key: Option<CacheKey>,
    },
    /// Already decided: no profile matched, or the cache has it.
    Decided { entry: FileResult, cached: bool },
    /// It could not be checked: its results entry, and why.
    Unreadable(FileResult, String),
    /// Not this tool's to check (another type's, or no type's): why.
    LeftOut(String),
}

/// A model the second pass loads and audits.
struct Job {
    /// Its place among the run's models.
    index: usize,
    profile: Arc<AuditProfile>,
    /// Its results entry so far: path, type, profile and stage.
    file: FileResult,
    key: Option<CacheKey>,
}

/// One line held back until its model's turn to be said.
enum Line {
    /// A result, on standard output.
    Say(String),
    /// A problem, on standard error.
    Warn(String),
}

impl Line {
    fn say(&self) {
        match self {
            Line::Say(text) => console::say(text),
            Line::Warn(text) => console::warn(text),
        }
    }
}

/// What one model came to: what to say about it, and what to record.
struct Outcome {
    lines: Vec<Line>,
    /// Its report entry, everything but the counts the run's tally adds.
    entry: Entry,
    end: End,
}

enum End {
    /// Checked, or decided without a check; its results entry is complete.
    Done(FileResult),
    /// It could not be checked: its results entry so far, and why.
    Failed(FileResult, String),
    /// Not this tool's to check: why.
    LeftOut(String),
}

impl Outcome {
    fn new(label: &str) -> Self {
        Self {
            lines: Vec::new(),
            entry: Entry::new(label),
            end: End::LeftOut(String::new()),
        }
    }

    fn say(&mut self, text: String) {
        self.lines.push(Line::Say(format!("  {text}")));
    }

    /// The outcome of a model the first pass already decided.
    fn decided(verdict: Verdict, subject: &Subject, rules_mode: bool) -> Self {
        let mut outcome = Outcome::new(&subject.label);
        match verdict {
            Verdict::LeftOut(reason) => {
                outcome.say(reason.clone());
                outcome.end = End::LeftOut(reason);
            }
            Verdict::Unreadable(file, problem) => {
                if file.stage.is_some() {
                    outcome.describe_resolution(&file);
                }
                outcome.fail(subject, file, problem);
            }
            Verdict::Decided {
                entry: file,
                cached,
            } => {
                if rules_mode {
                    outcome.describe(&file);
                }
                if cached {
                    outcome.say(tr(k::AUDIT_CACHED).into_owned());
                    outcome.entry.set("cached", json!(true));
                    // The lines a fresh check prints, from what the cache kept.
                    for finding in &file.findings {
                        outcome.say(cached_line(finding));
                    }
                } else {
                    for finding in &file.findings {
                        outcome.lines.extend(finding_lines("  ", finding));
                    }
                }
                outcome.end = End::Done(file);
            }
            Verdict::Check { .. } => unreachable!("a model to check is the second pass's"),
        }
        outcome
    }

    /// It could not be checked: say so, and end there.
    fn fail(&mut self, subject: &Subject, file: FileResult, problem: String) {
        self.lines.push(Line::Warn(format!(
            "  {}",
            k::audit_failed(subject.label.clone(), problem.clone())
        )));
        self.end = End::Failed(file, problem);
    }

    /// Say which profile and stage the rules gave a model, and record them in
    /// its report entry.
    fn describe(&mut self, file: &FileResult) {
        self.describe_resolution(file);
        let (Some(label), Some(id), Some(stage), Some(from)) = (
            &file.profile_label,
            &file.profile,
            &file.stage,
            file.stage_from,
        ) else {
            return;
        };
        let (label, id, stage) = (label.clone(), id.clone(), stage.clone());
        self.say(match from {
            StageFrom::Branch => k::audit_resolved_branch(id, label, stage),
            StageFrom::Profile => k::audit_resolved_profile(id, label, stage),
            StageFrom::Project => k::audit_resolved_project(id, label, stage),
            StageFrom::Asset => k::audit_resolved_asset(id, label, stage),
        });
    }

    fn describe_resolution(&mut self, file: &FileResult) {
        self.entry.set("profile", json!(file.profile));
        self.entry.set("profile_label", json!(file.profile_label));
        self.entry.set("stage", json!(file.stage));
        self.entry.set("stage_from", json!(file.stage_from));
    }

    /// Say it, and record it in the tally, the report and the results.
    fn record(self, tally: &mut Tally, report: &mut Report, results: &mut ResultsOut) {
        for line in &self.lines {
            line.say();
        }
        let mut entry = self.entry;
        match self.end {
            End::Done(file) => {
                tally.add(&mut entry, &file.findings);
                report.add(Status::Done, entry, None);
                results.file(file);
            }
            End::Failed(file, problem) => {
                results.file(unreadable(file, problem.clone()));
                report.add(Status::Failed, entry, Some(problem));
            }
            End::LeftOut(reason) => report.add(Status::Skipped, entry, Some(reason)),
        }
    }
}

/// A cached finding as [`summary_lines`] would have listed it.
fn cached_line(finding: &Finding) -> String {
    let level = audit_severity(finding.severity);
    match RuleId::from_wire(&finding.rule) {
        Some(rule) => {
            let found = finding
                .measured
                .as_ref()
                .and_then(|measured| measured.get("total"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            finding_line(rule, level, found)
        }
        None => k::audit_finding(tr(severity(level)).into_owned(), finding.title.clone()),
    }
}

impl Tally {
    /// Count a checked model's findings into its report entry and the run's
    /// totals, and flag it if any reaches `--fail-on`.
    fn add(&mut self, entry: &mut Entry, findings: &[Finding]) {
        let mut counts = [0u32; 3];
        let mut worst: Option<Severity> = None;
        for finding in findings {
            let level = audit_severity(finding.severity);
            counts[level.index()] += 1;
            worst = worst.max(Some(level));
        }
        for level in Severity::ALL {
            self.found[level.index()] += counts[level.index()];
        }
        let reached = worst.is_some_and(|worst| reaches(self.fail_on, worst));
        self.flagged += usize::from(reached);
        self.done += 1;
        entry.set("flagged", json!(reached));
        entry.set("worst", json!(worst));
        entry.set("errors", json!(counts[Severity::Error.index()]));
        entry.set("warnings", json!(counts[Severity::Warning.index()]));
        entry.set("infos", json!(counts[Severity::Info.index()]));
    }
}

/// Load and audit one model: the second pass's work, on a worker thread.
/// `threads` is this audit's share of the cores.
fn check(
    job: &Job,
    subject: &Subject,
    staged: Option<&(Repo, Cache)>,
    threads: usize,
    rules_mode: bool,
) -> Outcome {
    let mut outcome = Outcome::new(&subject.label);
    if rules_mode {
        outcome.describe(&job.file);
    }
    let failed = |mut outcome: Outcome, problem: String| {
        outcome.fail(subject, job.file.clone(), problem);
        outcome
    };

    // The content: the file, or what is staged of it. A temporary copy is
    // removed when `_copy` goes, after the load.
    let content = match (&subject.source, staged) {
        (Source::File(path), _) => Ok((path.clone(), None)),
        (Source::Staged(file), Some((repo, _))) => repo
            .content(file)
            .map(|checked| (checked.path().to_path_buf(), Some(checked)))
            .map_err(|error| k::staged_unreadable(error.to_string())),
        (Source::Staged(_), None) => unreachable!("a staged model comes with its repository"),
    };
    let (path, _copy) = match content {
        Ok(content) => content,
        Err(problem) => return failed(outcome, problem),
    };
    let loaded = match load::load(&path, &|_| {}) {
        Ok(loaded) => loaded,
        Err(problem) => return failed(outcome, problem),
    };
    for warning in &loaded.warnings {
        outcome
            .lines
            .push(Line::Warn(format!("  {}", k::warning(warning.clone()))));
        outcome.entry.notice(warning.clone());
    }

    let profile = &job.profile;
    let audit = review_audit::run(
        AuditInput {
            model: &loaded.model,
            extras: loaded.extras.as_ref(),
        },
        profile,
        None,
        RunOptions {
            cancel: None,
            threads,
        },
    );
    let audit = match audit {
        Ok(audit) => audit,
        Err(error) => return failed(outcome, explain_audit_error(&error)),
    };
    let document = to_value(
        &audit,
        &loaded.model,
        loaded.extras.as_ref(),
        profile,
        &ReportSource {
            file: subject.label.clone(),
            generator: generator(),
            created_unix: now_unix(),
        },
    );
    let document = match document {
        Ok(document) => document,
        Err(error) => return failed(outcome, explain_audit_error(&error)),
    };

    for line in summary_lines(&audit) {
        outcome.say(line);
    }
    let mut file = job.file.clone();
    file.findings = results::findings(&audit, &loaded.model, profile);
    if let (Some(key), Some((_, cache))) = (&job.key, staged) {
        // A cache that cannot be written costs the next run time, not this
        // one its answer.
        let _ = cache.put(key, &file);
    }
    outcome.entry.set("passed", json!(audit.summary.passed));
    outcome
        .entry
        .set("not_evaluated", json!(audit.summary.not_evaluated));
    outcome.entry.set("audit", document);
    outcome.end = End::Done(file);
    outcome
}

/// Work out what `subject` is checked against: its profile, or why it is not
/// checked - and, in a `--staged` run, whether the cache already has it.
fn verdict(run: &mut Run, subject: &Subject) -> Verdict {
    let path = results_path(&run.ctx, subject);
    let (rules, files) = match &mut run.judging {
        Judging::Profile { profile, id } => {
            let entry = profile_entry(path, id, profile_name(profile));
            return Verdict::Check {
                profile: profile.clone(),
                entry,
                key: None,
            };
        }
        Judging::Rules { rules, files } => (&*rules, files),
    };
    // A staged model resolves by its repository path; one on the line by
    // where it is, which `resolve` makes repository-relative itself.
    let resolution = match &subject.source {
        Source::Staged(file) => resolve(rules, &run.ctx, Path::new(&file.path)),
        Source::File(file) => {
            let absolute = std::path::absolute(file).unwrap_or_else(|_| file.clone());
            resolve(rules, &run.ctx, &absolute)
        }
    };
    let ours = |kind: &str| rules.types()[kind].tool == crate::args::PROGRAM;
    let resolved: Resolved = match resolution {
        Resolution::Ignored => return Verdict::LeftOut(tr(k::AUDIT_IGNORED).into_owned()),
        Resolution::Unmatched { kind } | Resolution::Resolved(Resolved { kind, .. })
            if !ours(&kind) =>
        {
            return Verdict::LeftOut(k::audit_other_tool(rules.types()[&kind].tool.clone()));
        }
        Resolution::Unmatched { kind } => {
            return Verdict::Decided {
                entry: FileResult::unmatched(path, &kind, rules),
                cached: false,
            };
        }
        Resolution::Resolved(resolved) => resolved,
    };
    let entry = FileResult::resolved(path.clone(), &resolved);
    let Some(rules_file) = resolved.rules_file.clone() else {
        let problem = k::audit_no_rules_file(resolved.profile_id.clone(), resolved.stage.clone());
        return Verdict::Unreadable(entry, problem);
    };
    let loaded = files
        .entry(rules_file.clone())
        .or_insert_with(|| load_rules_file(&rules_file));
    let (profile, digest) = match loaded {
        Ok(file) => (file.profile.clone(), file.digest.clone()),
        Err(problem) => return Verdict::Unreadable(entry, problem.clone()),
    };

    let key = match (&subject.source, &run.staged) {
        (Source::Staged(file), Some((_, cache))) => {
            let key = CacheKey::new(&file.path, &file.blob, &resolved, &digest, &tool());
            if let Some(cached) = cache.get(&key) {
                return Verdict::Decided {
                    entry: cached,
                    cached: true,
                };
            }
            Some(key)
        }
        _ => None,
    };
    Verdict::Check {
        profile,
        entry,
        key,
    }
}

/// A profile's rules file: the audit profile in it, read as `--profile` reads
/// one, and the digest of its bytes.
fn load_rules_file(path: &Path) -> Result<RulesFile, String> {
    let unreadable =
        |reason: String| k::audit_profile_unreadable(path.display().to_string(), reason);
    let json = std::fs::read_to_string(path).map_err(|error| unreadable(error.to_string()))?;
    let profile =
        envelope::from_json(&json).map_err(|error| unreadable(explain_audit_error(&error)))?;
    let digest = digest_file(path).map_err(|error| unreadable(error.to_string()))?;
    Ok(RulesFile {
        profile: Arc::new(profile),
        digest,
    })
}

/// How results name a model: by its repository path, with `/` separators, or
/// - outside the repository - by the path it was reached by.
fn results_path(ctx: &Context, subject: &Subject) -> String {
    match &subject.source {
        Source::Staged(file) => file.path.clone(),
        Source::File(path) => {
            let absolute = std::path::absolute(path).unwrap_or_else(|_| path.clone());
            ctx.repo_path(&absolute)
                .unwrap_or_else(|| subject.label.replace('\\', "/"))
        }
    }
}
