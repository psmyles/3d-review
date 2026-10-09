//! `review-comments`: print the review comments stored in an FBX, without the
//! viewer — for scripts, CI and pipeline tools.
//!
//! ```text
//! review-comments <file.fbx> [--json | --report] [--status open|resolved|all]
//! ```
//!
//! `--report` (the default) prints Markdown grouped by object; `--json` prints
//! every thread with its number and the object it is on. Exit status: 0 when at
//! least one thread was listed, 1 when the file has none (after the filter), 2 on
//! a usage or read error — so a script can tell "no comments" from "couldn't
//! read".

use std::path::PathBuf;
use std::process::ExitCode;

use review_annotate::comments;
use review_annotate::report::{self, StatusFilter};

const USAGE: &str =
    "usage: review-comments <file.fbx> [--json | --report] [--status open|resolved|all]";

enum Output {
    Report,
    Json,
}

struct Args {
    path: PathBuf,
    output: Output,
    filter: StatusFilter,
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut path = None;
    let mut output = Output::Report;
    let mut filter = StatusFilter::All;
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
            "-h" | "--help" => return Err(String::new()),
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            _ if path.is_none() => path = Some(PathBuf::from(arg)),
            _ => return Err(format!("unexpected argument {arg}")),
        }
    }
    Ok(Args {
        path: path.ok_or_else(|| "no file given".to_owned())?,
        output,
        filter,
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
    let (scan, found) = match comments::read_file(&args.path) {
        Ok(read) => read,
        Err(error) => {
            eprintln!("review-comments: {}: {error}", args.path.display());
            return ExitCode::from(2);
        }
    };
    let file = args.path.display().to_string();
    let listed = report::numbered(&found, args.filter).len();
    match args.output {
        Output::Json => println!("{}", report::json(&file, &scan, &found, args.filter)),
        Output::Report => print!("{}", report::markdown(&file, &scan, &found, args.filter)),
    }
    if listed == 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
