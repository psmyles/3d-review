//! `review-cli`: the Opt and Aud workspaces' work from a terminal, a script or
//! Launcher, without opening the viewer.
//!
//! ```text
//! review-cli optimize <models or folders>... --out <folder> [--preset <file>]
//!            [--lods <ratios>|none] [--reduce <ratio>]
//!            [--packaging single-file|file-per-lod] [--hierarchy rebuild|flat]
//!            [--format binary|ascii] [--suffix <text>] [--if-exists fail|skip|replace]
//! review-cli audit <models or folders>... [--profile unity|unreal|generic|<file>]
//!            [--fail-on error|warning|info|never]
//! review-cli info <models or folders>...
//!     every command: [--recurse] [--report <file.json>] [--progress-bar auto|on|off]
//! review-cli --version | --help | <command> --help
//! ```
//!
//! It keeps the contract every console tool Launcher drives keeps
//! (`docs/cli.md`):
//!
//! * long options only, no prompts, no standard input;
//! * exit status 2 for a command line it cannot follow, before anything runs
//!   (a report an earlier run left at its `--report` path is still removed);
//!   0 only when everything worked; 1 otherwise;
//! * `--report <file.json>`, an account of the run whose top-level fields a tool
//!   reads as results;
//! * plain lines, and `OSC 9;4` progress where it is shown.
//!
//! A batch never stops at the first model that fails: every model is tried,
//! each failure is said and recorded, and the exit status carries the verdict.

#![forbid(unsafe_code)]

mod args;
mod audit;
mod console;
mod info;
mod inputs;
mod load;
mod optimize;
mod report;

use std::ffi::OsString;
use std::process::ExitCode;

use args::{Command, PROGRAM, Parsed};

/// The typed message keys, generated from `cli.ftl` by this crate's build
/// script.
mod keys {
    include!(concat!(env!("OUT_DIR"), "/keys.rs"));
}

/// The product version, from `product.json` (see build.rs).
pub(crate) const VERSION: &str = env!("REVIEW_VERSION");

fn main() -> ExitCode {
    review_localization::init(sys_locale::get_locale().as_deref());
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    let parsed = match args::parse(arguments.clone()) {
        Ok(parsed) => parsed,
        Err(error) => {
            // Nothing runs and no report is written, but one an earlier run
            // left at the path this line names would read as this run's.
            if let Some(path) = args::report_path(&arguments) {
                report::discard_stale(&path);
            }
            console::warn(&error.message);
            console::warn(&error.hint());
            return ExitCode::from(2);
        }
    };
    let worked = match parsed {
        Parsed::Version => {
            console::say(&format!("{PROGRAM} {VERSION}"));
            true
        }
        Parsed::Help(topic) => {
            console::say(&args::help(topic));
            true
        }
        Parsed::Run(run) => match &run.command {
            Command::Optimize(args) => optimize::run(&run.common, args),
            Command::Audit(args) => audit::run(&run.common, args),
            Command::Info => info::run(&run.common),
        },
    };
    if worked {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
