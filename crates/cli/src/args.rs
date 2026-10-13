//! The command line: what `review-cli` was asked to do, or why it cannot tell.
//!
//! One table, [`SPECS`], is both what is accepted and what `--help` lists, so
//! the two cannot disagree. Parsing is all or nothing: a command line with one
//! thing wrong in it runs nothing (exit status 2), because a batch that starts
//! on a misread option can write a folder full of the wrong files.
//!
//! Two rules make it easy to drive from a tool that fills a command line in
//! from a form:
//!
//! * an empty value is the same as leaving the option out, so a setting nobody
//!   filled in can still be passed (`--preset ""`);
//! * a list is one value, separated by commas (`--lods 0.5,0.25`), which is how
//!   a multiple choice arrives.

use std::ffi::OsString;
use std::path::PathBuf;

use review_audit::{Engine, Severity};
use review_localization::{Key, tr};
use review_optimize::stack::limits::{LOD_RATIO_MAX, LOD_RATIO_MIN, MAX_LOD_LEVELS};
use review_optimize::{FbxFormat, HierarchyMode, LodPackaging};

use crate::keys::cli as k;

/// The program's name, as a command line names it.
pub(crate) const PROGRAM: &str = "review-cli";

/// What the command line asked for.
#[derive(Debug)]
pub(crate) enum Parsed {
    Version,
    Help(Option<CommandName>),
    Run(Run),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandName {
    Optimize,
    Audit,
    Info,
}

impl CommandName {
    const ALL: [CommandName; 3] = [CommandName::Optimize, CommandName::Audit, CommandName::Info];

    /// The word that names it on a command line.
    pub(crate) fn word(self) -> &'static str {
        match self {
            CommandName::Optimize => "optimize",
            CommandName::Audit => "audit",
            CommandName::Info => "info",
        }
    }

    fn usage(self) -> Key {
        match self {
            CommandName::Optimize => k::HELP_USAGE_OPTIMIZE,
            CommandName::Audit => k::HELP_USAGE_AUDIT,
            CommandName::Info => k::HELP_USAGE_INFO,
        }
    }

    fn summary(self) -> Key {
        match self {
            CommandName::Optimize => k::HELP_COMMAND_OPTIMIZE,
            CommandName::Audit => k::HELP_COMMAND_AUDIT,
            CommandName::Info => k::HELP_COMMAND_INFO,
        }
    }
}

/// A run: the options every command shares, and the command's own.
#[derive(Debug)]
pub(crate) struct Run {
    pub(crate) common: Common,
    pub(crate) command: Command,
}

#[derive(Debug)]
pub(crate) struct Common {
    /// Models and folders, as given.
    pub(crate) inputs: Vec<PathBuf>,
    pub(crate) recurse: bool,
    pub(crate) report: Option<PathBuf>,
    pub(crate) progress_bar: BarMode,
}

#[derive(Debug)]
pub(crate) enum Command {
    Optimize(OptimizeArgs),
    Audit(AuditArgs),
    Info,
}

#[derive(Debug)]
pub(crate) struct OptimizeArgs {
    pub(crate) out: PathBuf,
    pub(crate) preset: Option<PathBuf>,
    pub(crate) lods: Option<Lods>,
    pub(crate) reduce: Option<f32>,
    pub(crate) packaging: Option<LodPackaging>,
    pub(crate) hierarchy: Option<HierarchyMode>,
    pub(crate) format: Option<FbxFormat>,
    pub(crate) suffix: String,
    pub(crate) if_exists: IfExists,
}

/// `--lods`: the chain to make, or none at all.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Lods {
    None,
    Ratios(Vec<f32>),
}

/// `--if-exists`: what to do about a file a model would write that is already
/// there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IfExists {
    Fail,
    Skip,
    Replace,
}

#[derive(Debug)]
pub(crate) struct AuditArgs {
    pub(crate) profile: ProfileChoice,
    pub(crate) fail_on: FailOn,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ProfileChoice {
    Builtin(Engine),
    File(PathBuf),
}

/// `--fail-on`: the least severe finding that fails the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailOn {
    At(Severity),
    Never,
}

/// `--progress-bar`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BarMode {
    Auto,
    On,
    Off,
}

/// A command line `review-cli` cannot follow. Nothing has run.
#[derive(Debug)]
pub(crate) struct UsageError {
    pub(crate) message: String,
    /// The command whose `--help` the hint points at.
    pub(crate) command: Option<CommandName>,
}

impl UsageError {
    fn new(message: String, command: Option<CommandName>) -> Self {
        Self { message, command }
    }

    /// Where to read what the command takes.
    pub(crate) fn hint(&self) -> String {
        let help = match self.command {
            Some(command) => format!("{PROGRAM} {} {HELP}", command.word()),
            None => format!("{PROGRAM} {HELP}"),
        };
        k::usage_hint(help)
    }
}

// The options, by name. Constants, so the parser, the table and the messages
// that mention an option all spell it one way.
const OUT: &str = "--out";
const PRESET: &str = "--preset";
const LODS: &str = "--lods";
const REDUCE: &str = "--reduce";
const PACKAGING: &str = "--packaging";
const HIERARCHY: &str = "--hierarchy";
const FORMAT: &str = "--format";
const SUFFIX: &str = "--suffix";
const IF_EXISTS: &str = "--if-exists";
const PROFILE: &str = "--profile";
const FAIL_ON: &str = "--fail-on";
const RECURSE: &str = "--recurse";
const REPORT: &str = "--report";
const PROGRESS_BAR: &str = "--progress-bar";
const HELP: &str = "--help";
const VERSION: &str = "--version";

// The words a choice takes. The order is the order `--help` shows them in.
const PACKAGING_CHOICES: &[&str] = &["single-file", "file-per-lod"];
const HIERARCHY_CHOICES: &[&str] = &["rebuild", "flat"];
const FORMAT_CHOICES: &[&str] = &["binary", "ascii"];
const IF_EXISTS_CHOICES: &[&str] = &["fail", "skip", "replace"];
const PROFILE_CHOICES: &[&str] = &["unity", "unreal", "generic"];
const FAIL_ON_CHOICES: &[&str] = &["error", "warning", "info", "never"];
const BAR_CHOICES: &[&str] = &["auto", "on", "off"];
const NO_LODS: &str = "none";

/// What an option takes after its name.
#[derive(Clone, Copy)]
enum Takes {
    Nothing,
    Value(Key),
    Choice(&'static [&'static str]),
    /// One of the words, or else a value (`--profile unity` or a file).
    ChoiceOrValue(&'static [&'static str], Key),
}

struct Spec {
    name: &'static str,
    takes: Takes,
    help: Key,
    on: &'static [CommandName],
}

const EVERY: &[CommandName] = &CommandName::ALL;
const OPTIMIZE: &[CommandName] = &[CommandName::Optimize];
const AUDIT: &[CommandName] = &[CommandName::Audit];

/// Every option, in the order `--help` lists them.
const SPECS: &[Spec] = &[
    Spec {
        name: OUT,
        takes: Takes::Value(k::HELP_VALUE_FOLDER),
        help: k::HELP_OPTION_OUT,
        on: OPTIMIZE,
    },
    Spec {
        name: PRESET,
        takes: Takes::Value(k::HELP_VALUE_FILE),
        help: k::HELP_OPTION_PRESET,
        on: OPTIMIZE,
    },
    Spec {
        name: LODS,
        takes: Takes::Value(k::HELP_VALUE_RATIOS),
        help: k::HELP_OPTION_LODS,
        on: OPTIMIZE,
    },
    Spec {
        name: REDUCE,
        takes: Takes::Value(k::HELP_VALUE_RATIO),
        help: k::HELP_OPTION_REDUCE,
        on: OPTIMIZE,
    },
    Spec {
        name: PACKAGING,
        takes: Takes::Choice(PACKAGING_CHOICES),
        help: k::HELP_OPTION_PACKAGING,
        on: OPTIMIZE,
    },
    Spec {
        name: HIERARCHY,
        takes: Takes::Choice(HIERARCHY_CHOICES),
        help: k::HELP_OPTION_HIERARCHY,
        on: OPTIMIZE,
    },
    Spec {
        name: FORMAT,
        takes: Takes::Choice(FORMAT_CHOICES),
        help: k::HELP_OPTION_FORMAT,
        on: OPTIMIZE,
    },
    Spec {
        name: SUFFIX,
        takes: Takes::Value(k::HELP_VALUE_TEXT),
        help: k::HELP_OPTION_SUFFIX,
        on: OPTIMIZE,
    },
    Spec {
        name: IF_EXISTS,
        takes: Takes::Choice(IF_EXISTS_CHOICES),
        help: k::HELP_OPTION_IF_EXISTS,
        on: OPTIMIZE,
    },
    Spec {
        name: PROFILE,
        takes: Takes::ChoiceOrValue(PROFILE_CHOICES, k::HELP_VALUE_FILE),
        help: k::HELP_OPTION_PROFILE,
        on: AUDIT,
    },
    Spec {
        name: FAIL_ON,
        takes: Takes::Choice(FAIL_ON_CHOICES),
        help: k::HELP_OPTION_FAIL_ON,
        on: AUDIT,
    },
    Spec {
        name: RECURSE,
        takes: Takes::Nothing,
        help: k::HELP_OPTION_RECURSE,
        on: EVERY,
    },
    Spec {
        name: REPORT,
        takes: Takes::Value(k::HELP_VALUE_FILE),
        help: k::HELP_OPTION_REPORT,
        on: EVERY,
    },
    Spec {
        name: PROGRESS_BAR,
        takes: Takes::Choice(BAR_CHOICES),
        help: k::HELP_OPTION_PROGRESS_BAR,
        on: EVERY,
    },
    Spec {
        name: HELP,
        takes: Takes::Nothing,
        help: k::HELP_OPTION_HELP,
        on: EVERY,
    },
];

/// Read the arguments after the program's name.
pub(crate) fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Parsed, UsageError> {
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return Err(UsageError::new(tr(k::USAGE_NO_COMMAND).into_owned(), None));
    };
    let first = first.to_string_lossy().into_owned();
    let rest: Vec<OsString> = args.collect();
    let command = match first.as_str() {
        VERSION | HELP | "-h" if !rest.is_empty() => {
            let extra = rest[0].to_string_lossy().into_owned();
            return Err(UsageError::new(k::usage_unknown_option(extra), None));
        }
        VERSION => return Ok(Parsed::Version),
        HELP | "-h" => return Ok(Parsed::Help(None)),
        word => match CommandName::ALL
            .into_iter()
            .find(|command| command.word() == word)
        {
            Some(command) => command,
            None if word.starts_with('-') => {
                return Err(UsageError::new(
                    k::usage_unknown_option(word.to_owned()),
                    None,
                ));
            }
            None => {
                return Err(UsageError::new(
                    k::usage_unknown_command(word.to_owned()),
                    None,
                ));
            }
        },
    };
    parse_command(command, rest)
}

/// The values an option was given, last one winning. A boolean is present or
/// not; an empty value never arrives here.
#[derive(Default)]
struct Given {
    values: Vec<(&'static str, Option<OsString>)>,
    inputs: Vec<PathBuf>,
}

impl Given {
    fn flag(&self, name: &str) -> bool {
        self.values.iter().any(|(given, _)| *given == name)
    }

    fn value(&self, name: &str) -> Option<&OsString> {
        self.values
            .iter()
            .rev()
            .find(|(given, _)| *given == name)
            .and_then(|(_, value)| value.as_ref())
    }

    fn text(&self, name: &str) -> Option<String> {
        self.value(name)
            .map(|value| value.to_string_lossy().trim().to_owned())
    }

    fn path(&self, name: &str) -> Option<PathBuf> {
        self.value(name).map(PathBuf::from)
    }
}

fn parse_command(command: CommandName, args: Vec<OsString>) -> Result<Parsed, UsageError> {
    // A help request anywhere wins over everything else on the line, mistakes
    // included: it is what someone with a mistake on the line is looking for.
    let asks_for_help = args
        .iter()
        .map(|arg| arg.to_string_lossy())
        .take_while(|arg| arg != "--")
        .any(|arg| arg == HELP || arg == "-h");
    if asks_for_help {
        return Ok(Parsed::Help(Some(command)));
    }

    let error = |message: String| UsageError::new(message, Some(command));
    let mut given = Given::default();
    let mut args = args.into_iter();
    let mut positional_only = false;

    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy().into_owned();
        if positional_only || !text.starts_with('-') {
            if !arg.is_empty() {
                given.inputs.push(PathBuf::from(arg));
            }
            continue;
        }
        if text == "--" {
            positional_only = true;
            continue;
        }
        let (name, inline) = match text.split_once('=') {
            Some((name, value)) => (name.to_owned(), Some(OsString::from(value))),
            None => (text.clone(), None),
        };
        let Some(spec) = SPECS
            .iter()
            .find(|spec| spec.name == name && spec.on.contains(&command))
        else {
            return Err(error(k::usage_unknown_option(name)));
        };
        match spec.takes {
            Takes::Nothing => {
                if inline.is_some() {
                    return Err(error(k::usage_takes_no_value(name)));
                }
                given.values.push((spec.name, None));
            }
            Takes::Value(_) | Takes::Choice(_) | Takes::ChoiceOrValue(..) => {
                let value = match inline {
                    Some(value) => value,
                    // The next argument, unless it is another option: `--out
                    // --recurse` is a value someone forgot, not a folder called
                    // "--recurse" (that one is written `--out=--recurse`).
                    None => match args.next() {
                        Some(value) if !value.to_string_lossy().starts_with("--") => value,
                        _ => return Err(error(k::usage_needs_value(name))),
                    },
                };
                if !value.to_string_lossy().trim().is_empty() {
                    given.values.push((spec.name, Some(value)));
                }
            }
        }
    }

    if given.inputs.is_empty() {
        return Err(error(tr(k::USAGE_NO_MODELS).into_owned()));
    }

    let common = Common {
        inputs: std::mem::take(&mut given.inputs),
        recurse: given.flag(RECURSE),
        report: given.path(REPORT),
        progress_bar: match choice(&given, PROGRESS_BAR, BAR_CHOICES).map_err(error)? {
            Some("on") => BarMode::On,
            Some("off") => BarMode::Off,
            _ => BarMode::Auto,
        },
    };
    let command = match command {
        CommandName::Optimize => Command::Optimize(optimize_args(&given).map_err(error)?),
        CommandName::Audit => Command::Audit(audit_args(&given).map_err(error)?),
        CommandName::Info => Command::Info,
    };
    Ok(Parsed::Run(Run { common, command }))
}

fn optimize_args(given: &Given) -> Result<OptimizeArgs, String> {
    let Some(out) = given.path(OUT) else {
        return Err(tr(k::USAGE_NEEDS_OUT).into_owned());
    };
    let lods = match given.text(LODS) {
        None => None,
        Some(value) if value.eq_ignore_ascii_case(NO_LODS) => Some(Lods::None),
        Some(value) => Some(Lods::Ratios(ratio_list(LODS, &value)?)),
    };
    let reduce = given
        .text(REDUCE)
        .map(|value| ratio(REDUCE, &value))
        .transpose()?;
    let preset = given.path(PRESET);
    let makes_lods = matches!(lods, Some(Lods::Ratios(_)));
    if preset.is_none() && !makes_lods && reduce.is_none() {
        return Err(tr(k::USAGE_NOTHING_TO_DO).into_owned());
    }
    Ok(OptimizeArgs {
        out,
        preset,
        lods,
        reduce,
        packaging: choice(given, PACKAGING, PACKAGING_CHOICES)?.map(|word| match word {
            "file-per-lod" => LodPackaging::FilePerLod,
            _ => LodPackaging::SingleFileSuffixed,
        }),
        hierarchy: choice(given, HIERARCHY, HIERARCHY_CHOICES)?.map(|word| match word {
            "flat" => HierarchyMode::FlatBaked,
            _ => HierarchyMode::Rebuild,
        }),
        format: choice(given, FORMAT, FORMAT_CHOICES)?.map(|word| match word {
            "ascii" => FbxFormat::Ascii,
            _ => FbxFormat::Binary,
        }),
        suffix: given.text(SUFFIX).unwrap_or_default(),
        if_exists: match choice(given, IF_EXISTS, IF_EXISTS_CHOICES)? {
            Some("skip") => IfExists::Skip,
            Some("replace") => IfExists::Replace,
            _ => IfExists::Fail,
        },
    })
}

fn audit_args(given: &Given) -> Result<AuditArgs, String> {
    let profile = match given.text(PROFILE) {
        None => ProfileChoice::Builtin(Engine::Generic),
        Some(value) => match PROFILE_CHOICES
            .iter()
            .position(|word| value.eq_ignore_ascii_case(word))
        {
            Some(0) => ProfileChoice::Builtin(Engine::Unity),
            Some(1) => ProfileChoice::Builtin(Engine::Unreal),
            Some(_) => ProfileChoice::Builtin(Engine::Generic),
            // Anything else is a profile file. A file that happens to be called
            // `unity` is reached as `./unity`.
            None => ProfileChoice::File(given.path(PROFILE).unwrap_or_default()),
        },
    };
    let fail_on = match choice(given, FAIL_ON, FAIL_ON_CHOICES)? {
        Some("warning") => FailOn::At(Severity::Warning),
        Some("info") => FailOn::At(Severity::Info),
        Some("never") => FailOn::Never,
        _ => FailOn::At(Severity::Error),
    };
    Ok(AuditArgs { profile, fail_on })
}

/// The word `name` was given, matched without regard to case against
/// `choices`; `None` when the option was left out.
fn choice(
    given: &Given,
    name: &str,
    choices: &'static [&'static str],
) -> Result<Option<&'static str>, String> {
    let Some(value) = given.text(name) else {
        return Ok(None);
    };
    choices
        .iter()
        .find(|word| value.eq_ignore_ascii_case(word))
        .copied()
        .map(Some)
        .ok_or_else(|| k::usage_bad_choice(choices.join("|"), name.to_owned(), value))
}

/// A share of the triangles: `0.5` or `50%`.
fn ratio(name: &str, value: &str) -> Result<f32, String> {
    let bad = || k::usage_bad_ratio(name.to_owned(), value.to_owned());
    let text = value.trim();
    let parsed = match text.strip_suffix('%') {
        Some(percent) => percent.trim().parse::<f32>().map(|percent| percent / 100.0),
        None => text.parse::<f32>(),
    }
    .map_err(|_| bad())?;
    if !(LOD_RATIO_MIN..=LOD_RATIO_MAX).contains(&parsed) {
        return Err(bad());
    }
    Ok(parsed)
}

/// `0.5,0.25` or `50%, 25%`.
fn ratio_list(name: &str, value: &str) -> Result<Vec<f32>, String> {
    let ratios = value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| ratio(name, item))
        .collect::<Result<Vec<_>, _>>()?;
    if ratios.is_empty() {
        return Err(k::usage_bad_ratio(name.to_owned(), value.to_owned()));
    }
    if ratios.len() > MAX_LOD_LEVELS {
        return Err(k::usage_too_many_lods(
            MAX_LOD_LEVELS as f64,
            name.to_owned(),
        ));
    }
    Ok(ratios)
}

/// `--help` for `topic`, or the program's own when there is none.
pub(crate) fn help(topic: Option<CommandName>) -> String {
    let mut lines = Vec::new();
    match topic {
        None => {
            lines.push(tr(k::HELP_ABOUT).into_owned());
            lines.push(String::new());
            lines.push(tr(k::HELP_USAGE).into_owned());
            lines.push(format!("  {}", tr(k::HELP_USAGE_MAIN)));
            lines.push(String::new());
            lines.push(tr(k::HELP_COMMANDS).into_owned());
            let rows: Vec<(String, String)> = CommandName::ALL
                .into_iter()
                .map(|command| {
                    (
                        command.word().to_owned(),
                        tr(command.summary()).into_owned(),
                    )
                })
                .collect();
            lines.extend(table(&rows));
            lines.push(String::new());
            lines.push(tr(k::HELP_MORE).into_owned());
            lines.push(String::new());
            lines.push(tr(k::HELP_OPTIONS).into_owned());
            lines.extend(table(&[
                (HELP.to_owned(), tr(k::HELP_OPTION_HELP).into_owned()),
                (VERSION.to_owned(), tr(k::HELP_OPTION_VERSION).into_owned()),
            ]));
        }
        Some(command) => {
            lines.push(tr(command.summary()).into_owned());
            lines.push(String::new());
            lines.push(tr(k::HELP_USAGE).into_owned());
            lines.push(format!("  {}", tr(command.usage())));
            lines.push(String::new());
            lines.push(tr(k::HELP_MODELS).into_owned());
            lines.push(String::new());
            lines.push(tr(k::HELP_OPTIONS).into_owned());
            let rows: Vec<(String, String)> = SPECS
                .iter()
                .filter(|spec| spec.on.contains(&command))
                .map(|spec| (syntax(spec), tr(spec.help).into_owned()))
                .collect();
            lines.extend(table(&rows));
        }
    }
    lines.join("\n")
}

/// An option as `--help` writes it: `--out <folder>`, `--format binary|ascii`.
fn syntax(spec: &Spec) -> String {
    match spec.takes {
        Takes::Nothing => spec.name.to_owned(),
        Takes::Value(value) => format!("{} <{}>", spec.name, tr(value)),
        Takes::Choice(words) => format!("{} {}", spec.name, words.join("|")),
        Takes::ChoiceOrValue(words, value) => {
            format!("{} {}|<{}>", spec.name, words.join("|"), tr(value))
        }
    }
}

/// Two columns, the second lined up.
fn table(rows: &[(String, String)]) -> Vec<String> {
    let width = rows
        .iter()
        .map(|(left, _)| left.chars().count())
        .max()
        .unwrap_or(0);
    rows.iter()
        .map(|(left, right)| format!("  {left:<width$}  {right}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_line(line: &str) -> Result<Parsed, UsageError> {
        parse(line.split_whitespace().map(OsString::from))
    }

    fn run(line: &str) -> Run {
        match parse_line(line) {
            Ok(Parsed::Run(run)) => run,
            other => panic!("{line}: {other:?}"),
        }
    }

    fn usage_error(line: &str) -> String {
        match parse_line(line) {
            Err(error) => error.message,
            other => panic!("{line} parsed: {other:?}"),
        }
    }

    #[test]
    fn an_optimize_line_reads_every_option() {
        let parsed = run(
            "optimize a.fbx props --out out --preset p.json --lods 50%,0.25 --reduce 0.8 \
             --packaging file-per-lod --hierarchy flat --format ASCII --suffix _low \
             --if-exists skip --recurse --report r.json --progress-bar on",
        );
        assert_eq!(
            parsed.common.inputs,
            [PathBuf::from("a.fbx"), PathBuf::from("props")]
        );
        assert!(parsed.common.recurse);
        assert_eq!(parsed.common.report, Some(PathBuf::from("r.json")));
        assert_eq!(parsed.common.progress_bar, BarMode::On);
        let Command::Optimize(args) = parsed.command else {
            panic!("not optimize");
        };
        assert_eq!(args.out, PathBuf::from("out"));
        assert_eq!(args.preset, Some(PathBuf::from("p.json")));
        assert_eq!(args.lods, Some(Lods::Ratios(vec![0.5, 0.25])));
        assert_eq!(args.reduce, Some(0.8));
        assert_eq!(args.packaging, Some(LodPackaging::FilePerLod));
        assert_eq!(args.hierarchy, Some(HierarchyMode::FlatBaked));
        assert_eq!(args.format, Some(FbxFormat::Ascii));
        assert_eq!(args.suffix, "_low");
        assert_eq!(args.if_exists, IfExists::Skip);
    }

    #[test]
    fn values_may_follow_an_equals_sign_and_the_last_one_wins() {
        let parsed = run("audit a.fbx --profile=unreal --fail-on=info --fail-on warning");
        let Command::Audit(args) = parsed.command else {
            panic!("not audit");
        };
        assert_eq!(args.profile, ProfileChoice::Builtin(Engine::Unreal));
        assert_eq!(args.fail_on, FailOn::At(Severity::Warning));
    }

    #[test]
    fn an_empty_value_is_the_option_left_out() {
        let parsed = parse(
            ["audit", "a.fbx", "--profile", "", "--report", ""]
                .into_iter()
                .map(OsString::from),
        );
        let Ok(Parsed::Run(parsed)) = parsed else {
            panic!("{parsed:?}");
        };
        assert_eq!(parsed.common.report, None);
        let Command::Audit(args) = parsed.command else {
            panic!("not audit");
        };
        assert_eq!(args.profile, ProfileChoice::Builtin(Engine::Generic));
        assert_eq!(args.fail_on, FailOn::At(Severity::Error));
    }

    #[test]
    fn a_profile_that_is_not_a_builtin_is_a_file() {
        let Command::Audit(args) = run("audit a.fbx --profile team.json").command else {
            panic!("not audit");
        };
        assert_eq!(
            args.profile,
            ProfileChoice::File(PathBuf::from("team.json"))
        );
    }

    #[test]
    fn what_the_command_line_gets_wrong_is_refused() {
        for line in [
            "",
            "optimise a.fbx",
            "--frobnicate",
            "optimize a.fbx --out",
            "optimize a.fbx --out --recurse",
            "optimize a.fbx --out out",
            "optimize a.fbx --lods 0.5",
            "optimize --out o --lods 0.5",
            "optimize a.fbx --out o --lods 2",
            "optimize a.fbx --out o --lods 0.5,x",
            "optimize a.fbx --out o --lods 0.9,0.8,0.7,0.6,0.5,0.4,0.3,0.2,0.1",
            "optimize a.fbx --out o --lods none",
            "optimize a.fbx --out o --reduce 0",
            "optimize a.fbx --out o --reduce 0.5 --packaging both",
            "optimize a.fbx --out o --reduce 0.5 --recurse=yes",
            "audit a.fbx --out o",
            "audit a.fbx --fail-on errors",
            "info a.fbx --profile unity",
            "--version extra",
        ] {
            assert!(parse_line(line).is_err(), "{line:?} was accepted");
        }
        assert!(usage_error("optimize a.fbx --out o --lods 2").contains("--lods"));
    }

    #[test]
    fn help_wins_and_names_its_command() {
        assert!(matches!(
            parse_line("audit --bogus --help"),
            Ok(Parsed::Help(Some(CommandName::Audit)))
        ));
        assert!(matches!(parse_line("--help"), Ok(Parsed::Help(None))));
        assert!(matches!(parse_line("--version"), Ok(Parsed::Version)));
    }

    #[test]
    fn everything_after_a_double_dash_is_a_model() {
        let parsed = run("info -- --odd.fbx");
        assert_eq!(parsed.common.inputs, [PathBuf::from("--odd.fbx")]);
    }

    #[test]
    fn help_lists_each_commands_own_options() {
        let optimize = help(Some(CommandName::Optimize));
        assert!(optimize.contains("--out <folder>"));
        assert!(optimize.contains("--if-exists fail|skip|replace"));
        assert!(!optimize.contains("--fail-on"));
        let audit = help(Some(CommandName::Audit));
        assert!(audit.contains("--profile unity|unreal|generic|<file>"));
        assert!(!audit.contains("--out"));
        let main = help(None);
        assert!(main.contains("optimize") && main.contains("--version"));
    }
}
