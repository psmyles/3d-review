//! The run report: `--report <file.json>`.
//!
//! One document for every command, shaped for a tool that turns a run into
//! results: the counts a step shows (`models`, `done`, `skipped`, `failed`, and
//! each command's own totals) sit flat at the top, beside `ok` - which is the
//! exit status as a field - and every model's own account is an entry in
//! `results`. Field names are ids, never prose; prose is only ever in a
//! `problem` or a `notices` line, already in the reader's language.
//!
//! A run that could read its command line always writes one, even when it
//! stops before the first model (`problem` says why), and a report left at the
//! path by an earlier run is removed first - so a tool reading the file after a
//! run that died part-way finds nothing rather than an earlier answer. A
//! command line refused outright writes none, but still removes that earlier
//! one when the path can be read off it.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use crate::args::{CommandName, PROGRAM};
use crate::keys::cli as k;

/// The report's schema id and version. Fields are only ever added within a
/// version; a change to what one means is a new version.
pub(crate) const SCHEMA: &str = "3d-review.cli-report";
pub(crate) const VERSION: u32 = 1;

/// What a model came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    Done,
    Skipped,
    Failed,
}

impl Status {
    fn id(self) -> &'static str {
        match self {
            Status::Done => "done",
            Status::Skipped => "skipped",
            Status::Failed => "failed",
        }
    }
}

/// One model's entry, as the command fills it in.
pub(crate) struct Entry {
    fields: Map<String, Value>,
    notices: Vec<String>,
}

impl Entry {
    pub(crate) fn new(file: &str) -> Self {
        let mut fields = Map::new();
        fields.insert("file".into(), json!(file));
        Self {
            fields,
            notices: Vec::new(),
        }
    }

    pub(crate) fn set(&mut self, key: &str, value: Value) {
        self.fields.insert(key.into(), value);
    }

    /// Something worth saying that did not stop the model: a warning from the
    /// load or the run, a note on the export.
    pub(crate) fn notice(&mut self, text: String) {
        self.notices.push(text);
    }
}

pub(crate) struct Report {
    path: Option<PathBuf>,
    command: CommandName,
    results: Vec<Value>,
    models: usize,
    done: usize,
    skipped: usize,
    failed: usize,
    problem: Option<String>,
    totals: Map<String, Value>,
}

/// Take away a report an earlier run left at `path`, so nobody reads it as this
/// run's. What every run does first - and what a refused command line does
/// instead of running, since it writes no report of its own.
pub(crate) fn discard_stale(path: &Path) {
    if path.is_file() {
        let _ = std::fs::remove_file(path);
    }
}

impl Report {
    pub(crate) fn start(path: Option<PathBuf>, command: CommandName) -> Self {
        if let Some(path) = &path {
            discard_stale(path);
        }
        Self {
            path,
            command,
            results: Vec::new(),
            models: 0,
            done: 0,
            skipped: 0,
            failed: 0,
            problem: None,
            totals: Map::new(),
        }
    }

    /// How many models the run set out to work on.
    pub(crate) fn expect(&mut self, models: usize) {
        self.models = models;
    }

    pub(crate) fn add(&mut self, status: Status, mut entry: Entry, problem: Option<String>) {
        match status {
            Status::Done => self.done += 1,
            Status::Skipped => self.skipped += 1,
            Status::Failed => self.failed += 1,
        }
        entry.set("status", json!(status.id()));
        if let Some(problem) = problem {
            entry.set("problem", json!(problem));
        }
        let notices = std::mem::take(&mut entry.notices);
        entry.set("notices", json!(notices));
        self.results.push(Value::Object(entry.fields));
    }

    /// Why the run stopped before (or instead of) working on any model.
    pub(crate) fn problem(&mut self, text: String) {
        self.problem = Some(text);
    }

    pub(crate) fn has_problem(&self) -> bool {
        self.problem.is_some()
    }

    pub(crate) fn failed(&self) -> usize {
        self.failed
    }

    /// A command's own run-wide figure: `written`, `errors`, `triangles`...
    pub(crate) fn total(&mut self, key: &str, value: Value) {
        self.totals.insert(key.into(), value);
    }

    /// Write the report, when one was asked for. `false` when it could not be
    /// written, which fails the run: a tool waiting on the file would otherwise
    /// read nothing and be told everything worked.
    pub(crate) fn finish(self, ok: bool) -> bool {
        let Some(path) = &self.path else {
            return true;
        };
        let mut document = Map::new();
        document.insert("schema".into(), json!(SCHEMA));
        document.insert("version".into(), json!(VERSION));
        document.insert("generator".into(), json!(generator()));
        document.insert("created_unix".into(), json!(now_unix()));
        document.insert("command".into(), json!(self.command.word()));
        document.insert("ok".into(), json!(ok));
        document.insert("problem".into(), json!(self.problem));
        document.insert("models".into(), json!(self.models));
        document.insert("done".into(), json!(self.done));
        document.insert("skipped".into(), json!(self.skipped));
        document.insert("failed".into(), json!(self.failed));
        document.extend(self.totals);
        document.insert("results".into(), Value::Array(self.results));

        let written = serde_json::to_string_pretty(&Value::Object(document))
            .map_err(|error| error.to_string())
            .and_then(|json| {
                if let Some(parent) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                // Replaced rather than overwritten, so a reader never sees half
                // of one.
                review_optimize::write_bytes_replacing(path, &json)
                    .map_err(|error| error.to_string())
            });
        match written {
            Ok(()) => true,
            Err(reason) => {
                crate::console::warn(&k::report_failed(path.display().to_string(), reason));
                false
            }
        }
    }
}

/// A measured `f32` figure as the shortest number that reads back as it, rather
/// than the seventeen digits its widening to `f64` would print - the audit
/// report's own rule, so a run report and the audit document inside it spell
/// one figure one way.
pub(crate) use review_audit::report::figure;

/// The program that wrote a report, as every report names it.
pub(crate) fn generator() -> String {
    format!("{PROGRAM} {}", crate::VERSION)
}

pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}
