//! `review-comments`: print the review comments stored in FBX files, without the
//! viewer — for scripts, CI and pipeline tools.
//!
//! ```text
//! review-comments <file-or-folder>... [--json | --report] [--status open|resolved|all]
//!                 [--baseline <listing.json> [--update-baseline]]
//! ```
//!
//! A folder is searched for `.fbx` files at any depth, skipping folders whose
//! name starts with `.` and not following symlinked folders. `--report` (the
//! default) prints Markdown grouped by file and object; `--json` prints every
//! thread with its number and the object it is on.
//!
//! `--baseline` lists only what changed since an earlier `--json` listing: new
//! threads, new replies, and threads resolved or reopened. `--update-baseline`
//! then rewrites that listing with everything just read (whatever `--status`
//! hid included), so the next run reports only what arrives after this one; a
//! baseline that does not exist yet is created, and everything counts as new.
//!
//! Exit status: 0 when at least one thread was listed, 1 when none were (after
//! the filter and the baseline), 2 on a usage error or when any file, folder or
//! the baseline could not be read — so a script can tell "nothing to see" from
//! "couldn't look". A run where a file could not be read leaves the baseline as
//! it was: rewriting it without that file's threads would report them all as
//! new the next time.

use std::collections::HashSet;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use review_annotate::baseline::Baseline;
use review_annotate::comments::{self, FileComments};
use review_annotate::fbx::Scan;
use review_annotate::report::{self, FileListing, Selection, StatusFilter, Unreadable};

const USAGE: &str = "usage: review-comments <file-or-folder>... [--json | --report] \
                     [--status open|resolved|all] [--baseline <listing.json> [--update-baseline]]";

enum Output {
    Report,
    Json,
}

struct Args {
    paths: Vec<PathBuf>,
    output: Output,
    filter: StatusFilter,
    baseline: Option<PathBuf>,
    update_baseline: bool,
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut paths = Vec::new();
    let mut output = Output::Report;
    let mut filter = StatusFilter::All;
    let mut baseline = None;
    let mut update_baseline = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => output = Output::Json,
            "--report" => output = Output::Report,
            "--status" => {
                filter = match args.next().as_deref() {
                    Some("open") => StatusFilter::Open,
                    Some("resolved") => StatusFilter::Resolved,
                    Some("all") => StatusFilter::All,
                    other => {
                        return Err(format!(
                            "--status takes open, resolved or all, not {other:?}"
                        ));
                    }
                };
            }
            "--baseline" => {
                baseline = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--baseline takes a file".to_owned())?,
                ));
            }
            "--update-baseline" => update_baseline = true,
            "-h" | "--help" => return Err(String::new()),
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    if paths.is_empty() {
        return Err("no file or folder given".to_owned());
    }
    if update_baseline && baseline.is_none() {
        return Err("--update-baseline needs --baseline <listing.json>".to_owned());
    }
    Ok(Args {
        paths,
        output,
        filter,
        baseline,
        update_baseline,
    })
}

fn is_fbx(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("fbx"))
}

fn unreadable(path: &Path, message: impl Into<String>) -> Unreadable {
    Unreadable {
        file: path.display().to_string(),
        message: message.into(),
    }
}

/// Every `.fbx` under `dir`, in name order, depth first.
fn walk(dir: &Path, files: &mut Vec<PathBuf>, errors: &mut Vec<Unreadable>) {
    let mut entries: Vec<_> = match std::fs::read_dir(dir) {
        Ok(entries) => entries.filter_map(Result::ok).collect(),
        Err(error) => {
            errors.push(unreadable(
                dir,
                format!("could not list the folder: {error}"),
            ));
            return;
        }
    };
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        // `file_type` does not follow symlinks, so a symlinked folder is never
        // entered — which is what keeps a link cycle from recursing forever.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if !entry.file_name().to_string_lossy().starts_with('.') {
                walk(&path, files, errors);
            }
        } else if is_fbx(&path) && path.is_file() {
            files.push(path);
        }
    }
}

/// The files `paths` name: each file as given, whatever its extension, and the
/// `.fbx` files under each folder. A file reached twice is read once.
fn collect(paths: &[PathBuf], errors: &mut Vec<Unreadable>) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for path in paths {
        match std::fs::metadata(path) {
            Ok(meta) if meta.is_dir() => walk(path, &mut found, errors),
            Ok(_) => found.push(path.clone()),
            Err(error) => errors.push(unreadable(path, error.to_string())),
        }
    }
    let mut seen = HashSet::new();
    found.retain(|path| seen.insert(std::fs::canonicalize(path).unwrap_or_else(|_| path.clone())));
    found
}

/// Why `path` could not be read, with the likely cause when it is a Git LFS
/// pointer — the usual reason an `.fbx` in a fresh checkout is not an FBX.
fn read_error(path: &Path, error: impl std::fmt::Display) -> String {
    let mut head = [0u8; 64];
    let len = std::fs::File::open(path)
        .and_then(|mut file| file.read(&mut head))
        .unwrap_or(0);
    if head[..len].starts_with(b"version https://git-lfs.github.com/spec/") {
        "a Git LFS pointer, not the file itself (run `git lfs pull`)".to_owned()
    } else {
        error.to_string()
    }
}

fn load_baseline(path: &Path, create: bool) -> Result<Baseline, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Baseline::from_json(&text).map_err(|error| {
            format!(
                "{}: not a `review-comments --json` listing: {error}",
                path.display()
            )
        }),
        Err(error) if error.kind() == ErrorKind::NotFound && create => {
            eprintln!(
                "review-comments: no baseline at {} yet, so every thread is new",
                path.display()
            );
            Ok(Baseline::default())
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Err(format!(
            "no baseline at {} (add --update-baseline to create it)",
            path.display()
        )),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

/// Replace `path` with `contents` through a sibling temporary file, so an
/// interrupted run leaves the old baseline rather than half of a new one.
fn write_replacing(path: &Path, contents: &str) -> std::io::Result<()> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, contents)?;
    std::fs::rename(&temp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            if !message.is_empty() {
                eprintln!("review-comments: {message}");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let baseline = match &args.baseline {
        Some(path) => match load_baseline(path, args.update_baseline) {
            Ok(baseline) => Some(baseline),
            Err(message) => {
                eprintln!("review-comments: {message}");
                return ExitCode::from(2);
            }
        },
        None => None,
    };

    let mut errors = Vec::new();
    let paths = collect(&args.paths, &mut errors);
    let mut read: Vec<(String, Scan, FileComments)> = Vec::new();
    for path in &paths {
        match comments::read_file(path) {
            Ok((scan, found)) => read.push((path.display().to_string(), scan, found)),
            Err(error) => errors.push(unreadable(path, read_error(path, error))),
        }
    }
    for error in &errors {
        eprintln!("review-comments: {}: {}", error.file, error.message);
    }
    if paths.is_empty() && errors.is_empty() {
        eprintln!("review-comments: no .fbx files found");
    }

    let files: Vec<FileListing> = read
        .iter()
        .map(|(file, scan, comments)| FileListing {
            file,
            scan,
            comments,
        })
        .collect();
    let selection = Selection {
        status: args.filter,
        since: baseline.as_ref(),
    };
    // Nothing could be read at all: the errors above are the whole answer.
    if !(files.is_empty() && !errors.is_empty()) {
        match args.output {
            Output::Json => println!("{}", report::json(&files, &errors, selection)),
            Output::Report => print!("{}", report::markdown(&files, &errors, selection)),
        }
    }

    if let Some(path) = args.baseline.as_ref().filter(|_| args.update_baseline) {
        if !errors.is_empty() {
            eprintln!("review-comments: baseline not updated, since not everything could be read");
            return ExitCode::from(2);
        }
        let everything = report::json(&files, &[], Selection::default());
        if let Err(error) = write_replacing(path, &everything) {
            eprintln!(
                "review-comments: could not write the baseline {}: {error}",
                path.display()
            );
            return ExitCode::from(2);
        }
    }

    let listed: usize = files
        .iter()
        .map(|listing| report::numbered(listing.comments, selection).len())
        .sum();
    if !errors.is_empty() {
        ExitCode::from(2)
    } else if listed == 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
