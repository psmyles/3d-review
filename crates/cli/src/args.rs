//! The command line: what `review3d-cli` was asked to do, or why it cannot tell.
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

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use review_audit::{Engine, Severity};
use review_localization::{Key, tr};
use review_optimize::stack::limits::{
    DISTANCE_LIMIT_MAX, DISTANCE_LIMIT_MIN, LOD_ERROR_MAX, LOD_ERROR_MIN, LOD_RATIO_MAX,
    LOD_RATIO_MIN, LOD_TRIANGLES_MAX, LOD_TRIANGLES_MIN, MAX_LOD_LEVELS, VOLUME_CHANGE_MAX,
    VOLUME_CHANGE_MIN,
};
use review_optimize::{FbxFormat, HierarchyMode, LodPackaging, SimplifyAlgorithm, SimplifyFlags};

use crate::keys::cli as k;

/// The program's name, as a command line names it.
pub(crate) const PROGRAM: &str = "review3d-cli";

/// What the command line asked for.
#[derive(Debug)]
pub(crate) enum Parsed {
    Version,
    Help(Option<CommandName>),
    /// Boxed: a run's options are much the largest of the three.
    Run(Box<Run>),
    /// `rules check` or `rules explain`: about a `rules.json`, not about
    /// models, so none of a model command's options apply.
    Rules(RulesArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandName {
    Optimize,
    Audit,
    Info,
    Rules,
}

impl CommandName {
    const ALL: [CommandName; 4] = [
        CommandName::Optimize,
        CommandName::Audit,
        CommandName::Info,
        CommandName::Rules,
    ];

    /// The word that names it on a command line.
    pub(crate) fn word(self) -> &'static str {
        match self {
            CommandName::Optimize => "optimize",
            CommandName::Audit => "audit",
            CommandName::Info => "info",
            CommandName::Rules => "rules",
        }
    }

    /// Its usage lines, one per form it takes.
    fn usage(self) -> &'static [Key] {
        match self {
            CommandName::Optimize => &[k::HELP_USAGE_OPTIMIZE],
            CommandName::Audit => &[k::HELP_USAGE_AUDIT],
            CommandName::Info => &[k::HELP_USAGE_INFO],
            CommandName::Rules => &[
                k::HELP_USAGE_RULES_CHECK,
                k::HELP_USAGE_RULES_EXPLAIN,
                k::HELP_USAGE_RULES_CATALOG,
            ],
        }
    }

    fn summary(self) -> Key {
        match self {
            CommandName::Optimize => k::HELP_COMMAND_OPTIMIZE,
            CommandName::Audit => k::HELP_COMMAND_AUDIT,
            CommandName::Info => k::HELP_COMMAND_INFO,
            CommandName::Rules => k::HELP_COMMAND_RULES,
        }
    }

    /// Whether it works on models, and so takes models, folders and
    /// `--files-from`.
    fn takes_models(self) -> bool {
        self != CommandName::Rules
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
    /// `--files-from`: a file listing more of them, one per line.
    pub(crate) files_from: Option<PathBuf>,
    pub(crate) recurse: bool,
    pub(crate) report: Option<PathBuf>,
    pub(crate) progress_bar: BarMode,
}

/// `rules check` and `rules explain`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RulesArgs {
    /// The problems in a `rules.json`.
    Check { rules: PathBuf },
    /// Which rules apply to each file, and why.
    Explain { rules: PathBuf, files: Vec<PathBuf> },
    /// Every rule this tool checks, as JSON.
    Catalog,
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
    pub(crate) reduce: Option<Target>,
    /// `--target-error`: the error limit of the Reduce and of every LOD level.
    pub(crate) target_error: Option<f32>,
    /// `--simplifier`: the algorithm the Reduce and the chain use.
    pub(crate) simplifier: Option<SimplifyAlgorithm>,
    /// `--simplify-flags`: the whole set of simplifier flags, in place of the
    /// preset's.
    pub(crate) simplify_flags: Option<SimplifyFlags>,
    /// `--max-volume-change`: the limit, or `Some(None)` to switch the preset's off.
    pub(crate) max_volume_change: Option<Option<f32>>,
    /// `--max-distance`: the same, for the distance limit.
    pub(crate) max_distance: Option<Option<f32>>,
    /// `--jobs`: a list of models, each with options of its own.
    pub(crate) jobs: Option<PathBuf>,
    /// The command line's options as given, which a job's line is read on top of.
    pub(crate) base: JobBase,
    pub(crate) packaging: Option<LodPackaging>,
    pub(crate) hierarchy: Option<HierarchyMode>,
    pub(crate) format: Option<FbxFormat>,
    pub(crate) suffix: String,
    pub(crate) if_exists: IfExists,
    /// `--replace-models`: a model's results may be written over the model
    /// itself.
    pub(crate) replace_models: bool,
}

/// `--lods`: the chain to make, or none at all.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Lods {
    None,
    Levels(Vec<Target>),
}

/// One triangle target, as `--lods` and `--reduce` take it: a share of the
/// triangles (`0.5`, `50%`) or a number of them (`1500t`), which is how a game
/// states a budget. Either is measured against the mesh the operation starts from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Target {
    Share(f32),
    Triangles(u32),
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
    pub(crate) judge: Judge,
    pub(crate) fail_on: FailOn,
    /// `--results`: Asset Check's results file.
    pub(crate) results: Option<PathBuf>,
    /// `--staged`: the models staged in the current folder's repository,
    /// instead of models named on the line.
    pub(crate) staged: bool,
}

/// What each model is checked against.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Judge {
    /// `--profile`: one profile for every model.
    Profile(ProfileChoice),
    /// `--rules`: Asset Check's `rules.json`, which picks each model's profile
    /// and stage from its name, its folder and the branch.
    Rules(PathBuf),
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

/// A command line `review3d-cli` cannot follow. Nothing has run.
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
const TARGET_ERROR: &str = "--target-error";
const SIMPLIFIER: &str = "--simplifier";
const SIMPLIFY_FLAGS: &str = "--simplify-flags";
const MAX_VOLUME_CHANGE: &str = "--max-volume-change";
const MAX_DISTANCE: &str = "--max-distance";
const JOBS: &str = "--jobs";
/// What a job's line may set: everything about how one model is optimized and
/// where it goes. The rest (`--report`, `--if-exists`, ...) is the run's.
const JOB_OPTIONS: &[&str] = &[
    OUT,
    PRESET,
    LODS,
    REDUCE,
    TARGET_ERROR,
    SIMPLIFIER,
    SIMPLIFY_FLAGS,
    MAX_VOLUME_CHANGE,
    MAX_DISTANCE,
    PACKAGING,
    HIERARCHY,
    FORMAT,
    SUFFIX,
];
const PACKAGING: &str = "--packaging";
const HIERARCHY: &str = "--hierarchy";
const FORMAT: &str = "--format";
const SUFFIX: &str = "--suffix";
const IF_EXISTS: &str = "--if-exists";
const REPLACE_MODELS: &str = "--replace-models";
const PROFILE: &str = "--profile";
const RULES: &str = "--rules";
const FAIL_ON: &str = "--fail-on";
pub(crate) const RESULTS: &str = "--results";
const STAGED: &str = "--staged";
const FILES_FROM: &str = "--files-from";
const RECURSE: &str = "--recurse";
pub(crate) const REPORT: &str = "--report";
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
const SIMPLIFIER_CHOICES: &[&str] = &["preserve-attributes", "standard", "sloppy"];
/// The simplifier flags `--simplify-flags` takes: each of the Inspector's checkboxes
/// by the name it shows there, in its order.
const SIMPLIFY_FLAG_WORDS: &[&str] = &[
    "lock-border",
    "error-absolute",
    "prune",
    "regularize",
    "regularize-light",
    "collapse-across-seams",
    "preserve-folds",
    "clamp-attribute-error",
];
const NO_FLAGS: &str = "none";

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
/// The commands that work on models.
const MODELS: &[CommandName] = &[CommandName::Optimize, CommandName::Audit, CommandName::Info];
const OPTIMIZE: &[CommandName] = &[CommandName::Optimize];
const AUDIT: &[CommandName] = &[CommandName::Audit];
const AUDIT_AND_RULES: &[CommandName] = &[CommandName::Audit, CommandName::Rules];

/// Every option, in the order `--help` lists them.
const SPECS: &[Spec] = &[
    Spec {
        name: OUT,
        takes: Takes::Value(k::HELP_VALUE_FOLDER),
        help: k::HELP_OPTION_OUT,
        on: OPTIMIZE,
    },
    Spec {
        name: JOBS,
        takes: Takes::Value(k::HELP_VALUE_FILE),
        help: k::HELP_OPTION_JOBS,
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
        takes: Takes::Value(k::HELP_VALUE_TARGETS),
        help: k::HELP_OPTION_LODS,
        on: OPTIMIZE,
    },
    Spec {
        name: REDUCE,
        takes: Takes::Value(k::HELP_VALUE_TARGET),
        help: k::HELP_OPTION_REDUCE,
        on: OPTIMIZE,
    },
    Spec {
        name: TARGET_ERROR,
        takes: Takes::Value(k::HELP_VALUE_LIMIT),
        help: k::HELP_OPTION_TARGET_ERROR,
        on: OPTIMIZE,
    },
    Spec {
        name: SIMPLIFIER,
        takes: Takes::Choice(SIMPLIFIER_CHOICES),
        help: k::HELP_OPTION_SIMPLIFIER,
        on: OPTIMIZE,
    },
    Spec {
        name: SIMPLIFY_FLAGS,
        takes: Takes::Value(k::HELP_VALUE_FLAGS),
        help: k::HELP_OPTION_SIMPLIFY_FLAGS,
        on: OPTIMIZE,
    },
    Spec {
        name: MAX_VOLUME_CHANGE,
        takes: Takes::Value(k::HELP_VALUE_SHARE),
        help: k::HELP_OPTION_MAX_VOLUME_CHANGE,
        on: OPTIMIZE,
    },
    Spec {
        name: MAX_DISTANCE,
        takes: Takes::Value(k::HELP_VALUE_SHARE),
        help: k::HELP_OPTION_MAX_DISTANCE,
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
        name: REPLACE_MODELS,
        takes: Takes::Nothing,
        help: k::HELP_OPTION_REPLACE_MODELS,
        on: OPTIMIZE,
    },
    Spec {
        name: PROFILE,
        takes: Takes::ChoiceOrValue(PROFILE_CHOICES, k::HELP_VALUE_FILE),
        help: k::HELP_OPTION_PROFILE,
        on: AUDIT,
    },
    Spec {
        name: RULES,
        takes: Takes::Value(k::HELP_VALUE_RULES),
        help: k::HELP_OPTION_RULES,
        on: AUDIT_AND_RULES,
    },
    Spec {
        name: FAIL_ON,
        takes: Takes::Choice(FAIL_ON_CHOICES),
        help: k::HELP_OPTION_FAIL_ON,
        on: AUDIT,
    },
    Spec {
        name: RESULTS,
        takes: Takes::Value(k::HELP_VALUE_FILE),
        help: k::HELP_OPTION_RESULTS,
        on: AUDIT,
    },
    Spec {
        name: STAGED,
        takes: Takes::Nothing,
        help: k::HELP_OPTION_STAGED,
        on: AUDIT,
    },
    Spec {
        name: FILES_FROM,
        takes: Takes::Value(k::HELP_VALUE_FILE),
        help: k::HELP_OPTION_FILES_FROM,
        on: MODELS,
    },
    Spec {
        name: RECURSE,
        takes: Takes::Nothing,
        help: k::HELP_OPTION_RECURSE,
        on: MODELS,
    },
    Spec {
        name: REPORT,
        takes: Takes::Value(k::HELP_VALUE_FILE),
        help: k::HELP_OPTION_REPORT,
        on: MODELS,
    },
    Spec {
        name: PROGRESS_BAR,
        takes: Takes::Choice(BAR_CHOICES),
        help: k::HELP_OPTION_PROGRESS_BAR,
        on: MODELS,
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
#[derive(Debug, Clone, Default)]
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

    /// The value as trimmed text, for an option whose value is a word or a
    /// number rather than a path. (Not `text`: that is a text sink's name to
    /// `no_inline_strings`, which would read the option names as prose.)
    fn trimmed(&self, name: &str) -> Option<String> {
        self.value(name)
            .map(|value| value.to_string_lossy().trim().to_owned())
    }

    fn path(&self, name: &str) -> Option<PathBuf> {
        self.value(name).map(PathBuf::from)
    }
}

/// The options and models of one command line, or of one job's line: each value
/// as given, the last one of an option winning.
fn read_options(command: CommandName, args: Vec<OsString>) -> Result<Given, String> {
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
        let (name, inline) = match split_inline(&arg) {
            Some((name, value)) => (name, Some(value)),
            None => (text.clone(), None),
        };
        let Some(spec) = SPECS
            .iter()
            .find(|spec| spec.name == name && spec.on.contains(&command))
        else {
            return Err(k::usage_unknown_option(name));
        };
        match spec.takes {
            Takes::Nothing => {
                if inline.is_some() {
                    return Err(k::usage_takes_no_value(name));
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
                        _ => return Err(k::usage_needs_value(name)),
                    },
                };
                if !value.to_string_lossy().trim().is_empty() {
                    given.values.push((spec.name, Some(value)));
                }
            }
        }
    }

    Ok(given)
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
    let mut given = read_options(command, args).map_err(error)?;

    if !command.takes_models() {
        return rules_args(&given).map(Parsed::Rules).map_err(error);
    }
    // `--staged` names the models itself; `--files-from` names them in a file,
    // where an empty list is a run with nothing to do rather than a mistake.
    let staged = given.flag(STAGED);
    let files_from = given.path(FILES_FROM);
    if staged && (!given.inputs.is_empty() || files_from.is_some()) {
        return Err(error(tr(k::USAGE_STAGED_AND_MODELS).into_owned()));
    }
    // A job list names its models too, each on its line.
    let jobs = matches!(command, CommandName::Optimize) && given.path(JOBS).is_some();
    if given.inputs.is_empty() && !staged && files_from.is_none() && !jobs {
        return Err(error(tr(k::USAGE_NO_MODELS).into_owned()));
    }

    let common = Common {
        inputs: std::mem::take(&mut given.inputs),
        files_from,
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
        CommandName::Rules => unreachable!("handled above: it takes no models"),
    };
    Ok(Parsed::Run(Box::new(Run { common, command })))
}

/// `rules check <rules.json>`, `rules explain <files>... --rules <rules.json>`
/// or `rules catalog`.
/// The first word after `rules` says which, so it arrives as the first input.
fn rules_args(given: &Given) -> Result<RulesArgs, String> {
    let Some((first, rest)) = given.inputs.split_first() else {
        return Err(tr(k::USAGE_RULES_NO_COMMAND).into_owned());
    };
    let rules = given.path(RULES);
    match first.to_string_lossy().as_ref() {
        "check" => match (rules.as_ref(), rest) {
            (Some(rules), []) | (None, [rules]) => Ok(RulesArgs::Check {
                rules: rules.clone(),
            }),
            _ => Err(tr(k::USAGE_RULES_CHECK_ONE).into_owned()),
        },
        "explain" => {
            let Some(rules) = rules else {
                return Err(tr(k::USAGE_RULES_EXPLAIN_NEEDS_RULES).into_owned());
            };
            if rest.is_empty() {
                return Err(tr(k::USAGE_RULES_EXPLAIN_NO_FILES).into_owned());
            }
            Ok(RulesArgs::Explain {
                rules,
                files: rest.to_vec(),
            })
        }
        "catalog" => match (rules, rest) {
            (None, []) => Ok(RulesArgs::Catalog),
            _ => Err(tr(k::USAGE_RULES_CATALOG_ALONE).into_owned()),
        },
        other => Err(k::usage_rules_unknown_command(other.to_owned())),
    }
}

/// `--name=value` split at its first `=`, or `None` when there is none.
///
/// Split on the argument as the OS handed it over, not on a lossy reading of
/// it: a value is often a path, and one that is not valid Unicode must arrive
/// exactly as it does after a space (`--out <path>`), not with its odd bytes
/// replaced. The name is the option's, plain ASCII when it is one at all.
fn split_inline(arg: &OsStr) -> Option<(String, OsString)> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        let wide: Vec<u16> = arg.encode_wide().collect();
        let at = wide.iter().position(|&unit| unit == u16::from(b'='))?;
        Some((
            String::from_utf16_lossy(&wide[..at]),
            OsString::from_wide(&wide[at + 1..]),
        ))
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let bytes = arg.as_bytes();
        let at = bytes.iter().position(|&byte| byte == b'=')?;
        Some((
            String::from_utf8_lossy(&bytes[..at]).into_owned(),
            OsStr::from_bytes(&bytes[at + 1..]).to_os_string(),
        ))
    }
    #[cfg(not(any(windows, unix)))]
    {
        let text = arg.to_string_lossy();
        let (name, value) = text.split_once('=')?;
        Some((name.to_owned(), OsString::from(value)))
    }
}

/// The file a command line names with `option` (`--report`, `--results`), read
/// leniently - for one [`parse`] refused, so a file an earlier run left at that
/// path can still be taken away. The last one before any `--` wins, as it does
/// when the line is accepted, and an empty value names none.
pub(crate) fn output_path(args: &[OsString], option: &str) -> Option<PathBuf> {
    let mut found = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--" {
            break;
        }
        let value = match split_inline(arg) {
            Some((name, value)) if name == option => Some(value),
            Some(_) => continue,
            None if arg == option => args
                .next()
                .filter(|value| !value.to_string_lossy().starts_with("--"))
                .cloned(),
            None => continue,
        };
        found = value
            .filter(|value| !value.to_string_lossy().trim().is_empty())
            .map(PathBuf::from);
    }
    found
}

fn optimize_args(given: &Given) -> Result<OptimizeArgs, String> {
    let Some(out) = given.path(OUT) else {
        return Err(tr(k::USAGE_NEEDS_OUT).into_owned());
    };
    let lods = match given.trimmed(LODS) {
        None => None,
        Some(value) if value.eq_ignore_ascii_case(NO_LODS) => Some(Lods::None),
        Some(value) => Some(Lods::Levels(target_list(LODS, &value)?)),
    };
    let reduce = given
        .trimmed(REDUCE)
        .map(|value| target(REDUCE, &value))
        .transpose()?;
    let target_error = given
        .trimmed(TARGET_ERROR)
        .map(|value| error_limit(TARGET_ERROR, &value))
        .transpose()?;
    let simplifier = choice(given, SIMPLIFIER, SIMPLIFIER_CHOICES)?.map(|word| match word {
        "standard" => SimplifyAlgorithm::Standard,
        "sloppy" => SimplifyAlgorithm::Sloppy,
        _ => SimplifyAlgorithm::WithAttributes,
    });
    let simplify_flags = given
        .trimmed(SIMPLIFY_FLAGS)
        .map(|value| flag_list(SIMPLIFY_FLAGS, &value))
        .transpose()?;
    let max_volume_change = given
        .trimmed(MAX_VOLUME_CHANGE)
        .map(|value| {
            quality_limit(
                MAX_VOLUME_CHANGE,
                &value,
                VOLUME_CHANGE_MIN,
                VOLUME_CHANGE_MAX,
            )
        })
        .transpose()?;
    let max_distance = given
        .trimmed(MAX_DISTANCE)
        .map(|value| quality_limit(MAX_DISTANCE, &value, DISTANCE_LIMIT_MIN, DISTANCE_LIMIT_MAX))
        .transpose()?;
    let preset = given.path(PRESET);
    let makes_lods = matches!(lods, Some(Lods::Levels(_)));
    let jobs = given.path(JOBS);
    // With a job list, each job's line says what to do with its model.
    if preset.is_none() && !makes_lods && reduce.is_none() && jobs.is_none() {
        return Err(tr(k::USAGE_NOTHING_TO_DO).into_owned());
    }
    Ok(OptimizeArgs {
        out,
        preset,
        lods,
        reduce,
        target_error,
        simplifier,
        simplify_flags,
        max_volume_change,
        max_distance,
        jobs,
        base: JobBase(Given {
            values: given.values.clone(),
            inputs: Vec::new(),
        }),
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
        suffix: given.trimmed(SUFFIX).unwrap_or_default(),
        if_exists: match choice(given, IF_EXISTS, IF_EXISTS_CHOICES)? {
            Some("skip") => IfExists::Skip,
            Some("replace") => IfExists::Replace,
            _ => IfExists::Fail,
        },
        replace_models: given.flag(REPLACE_MODELS),
    })
}

fn audit_args(given: &Given) -> Result<AuditArgs, String> {
    let rules = given.path(RULES);
    let profile = match given.trimmed(PROFILE) {
        None => ProfileChoice::Builtin(Engine::Generic),
        // Each model's profile comes from the rules, so a second source of one
        // is a contradiction, not a default to override.
        Some(_) if rules.is_some() => {
            return Err(tr(k::USAGE_RULES_AND_PROFILE).into_owned());
        }
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
        Some("error") => FailOn::At(Severity::Error),
        Some("warning") => FailOn::At(Severity::Warning),
        Some("info") => FailOn::At(Severity::Info),
        Some("never") => FailOn::Never,
        // Asset Check's convention (AC10): a run fails on a warning, which is
        // also where its results open the window for the artist.
        _ if rules.is_some() => FailOn::At(Severity::Warning),
        _ => FailOn::At(Severity::Error),
    };
    Ok(AuditArgs {
        judge: match rules {
            Some(rules) => Judge::Rules(rules),
            None => Judge::Profile(profile),
        },
        fail_on,
        results: given.path(RESULTS),
        staged: given.flag(STAGED),
    })
}

/// The word `name` was given, matched without regard to case against
/// `choices`; `None` when the option was left out.
fn choice(
    given: &Given,
    name: &str,
    choices: &'static [&'static str],
) -> Result<Option<&'static str>, String> {
    let Some(value) = given.trimmed(name) else {
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
fn target(name: &str, value: &str) -> Result<Target, String> {
    let text = value.trim();
    // A count: digits, then a `t` (`1500t`, `1500 t`). A whole number only; a
    // fraction of a triangle is a typo, not a target.
    if let Some(count) = text.strip_suffix(['t', 'T']) {
        let bad = || {
            k::usage_bad_count(
                f64::from(LOD_TRIANGLES_MAX),
                name.to_owned(),
                value.to_owned(),
            )
        };
        let parsed = count.trim().parse::<u32>().map_err(|_| bad())?;
        if !(LOD_TRIANGLES_MIN..=LOD_TRIANGLES_MAX).contains(&parsed) {
            return Err(bad());
        }
        return Ok(Target::Triangles(parsed));
    }
    let bad = || k::usage_bad_ratio(name.to_owned(), value.to_owned());
    let parsed = match text.strip_suffix('%') {
        Some(percent) => percent.trim().parse::<f32>().map(|percent| percent / 100.0),
        None => text.parse::<f32>(),
    }
    .map_err(|_| bad())?;
    if !(LOD_RATIO_MIN..=LOD_RATIO_MAX).contains(&parsed) {
        return Err(bad());
    }
    Ok(Target::Share(parsed))
}

/// The command line's options, kept for reading each job's line on top of.
#[derive(Debug, Clone, Default)]
pub(crate) struct JobBase(Given);

/// One line of a `--jobs` list: a model, then the options for it alone, read on
/// top of the command line's — exactly as if they had been added to its end — so
/// a job is checked by the same rules a command line is. Double quotes keep a
/// path with spaces together.
pub(crate) fn job_line(line: &str, base: &OptimizeArgs) -> Result<(PathBuf, OptimizeArgs), String> {
    let mut words = split_words(line).into_iter();
    let model = match words.next() {
        Some(word) if !word.starts_with('-') => PathBuf::from(word),
        _ => return Err(tr(k::JOBS_NEEDS_MODEL).into_owned()),
    };
    let own = read_options(CommandName::Optimize, words.map(OsString::from).collect())?;
    if !own.inputs.is_empty() {
        return Err(tr(k::JOBS_ONE_MODEL).into_owned());
    }
    if let Some((name, _)) = own
        .values
        .iter()
        .find(|(name, _)| !JOB_OPTIONS.contains(name))
    {
        return Err(k::jobs_not_per_model((*name).to_owned()));
    }
    let mut merged = base.base.0.clone();
    merged.values.extend(own.values);
    merged.values.retain(|(name, _)| *name != JOBS);
    merged.inputs = vec![model.clone()];
    let mut args = optimize_args(&merged)?;
    args.jobs = None;
    Ok((model, args))
}

/// A line's words, split at spaces and tabs; a run between double quotes is one
/// word, without its quotes.
fn split_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let (mut quoted, mut started) = (false, false);
    for character in line.chars() {
        match character {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            ' ' | '\t' if !quoted => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            other => {
                word.push(other);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}

/// An error limit, as the Inspector's slider takes it: `0` to `1`.
fn error_limit(name: &str, value: &str) -> Result<f32, String> {
    value
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|limit| (LOD_ERROR_MIN..=LOD_ERROR_MAX).contains(limit))
        .ok_or_else(|| k::usage_bad_error(name.to_owned(), value.to_owned()))
}

/// A quality limit: a share (`0.05`) or a percentage (`5%`) within `min..=max`, or
/// `none` to switch a preset's limit off.
fn quality_limit(name: &str, value: &str, min: f32, max: f32) -> Result<Option<f32>, String> {
    let text = value.trim();
    if text.eq_ignore_ascii_case(NO_FLAGS) {
        return Ok(None);
    }
    // The bounds as the shortest text of their `f32`: widened to `f64` for the
    // message, 0.1 would print as `0.10000000149011612`.
    let bad = || {
        k::usage_bad_limit(
            (max * 100.0).to_string(),
            (min * 100.0).to_string(),
            name.to_owned(),
            value.to_owned(),
        )
    };
    let parsed = match text.strip_suffix('%') {
        Some(percent) => percent.trim().parse::<f32>().map(|percent| percent / 100.0),
        None => text.parse::<f32>(),
    }
    .map_err(|_| bad())?;
    if !(min..=max).contains(&parsed) {
        return Err(bad());
    }
    Ok(Some(parsed))
}

/// `regularize,lock-border`, or `none`: the whole set, so a flag the preset has on
/// and the list leaves out is off.
fn flag_list(name: &str, value: &str) -> Result<SimplifyFlags, String> {
    let mut flags = SimplifyFlags::default();
    if value.trim().eq_ignore_ascii_case(NO_FLAGS) {
        return Ok(flags);
    }
    let words = || [SIMPLIFY_FLAG_WORDS, &[NO_FLAGS]].concat().join("|");
    for item in value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let word = SIMPLIFY_FLAG_WORDS
            .iter()
            .find(|word| item.eq_ignore_ascii_case(word))
            .ok_or_else(|| k::usage_bad_choice(words(), name.to_owned(), item.to_owned()))?;
        match *word {
            "lock-border" => flags.lock_border = true,
            "error-absolute" => flags.error_absolute = true,
            "prune" => flags.prune = true,
            "regularize" => flags.regularize = true,
            "regularize-light" => flags.regularize_light = true,
            "collapse-across-seams" => flags.permissive = true,
            "preserve-folds" => flags.preserve_folds = true,
            _ => flags.clamp_attribute_error = true,
        }
    }
    Ok(flags)
}

/// `0.5,0.25`, `50%, 25%`, `750t,375t`, or a mix of the two.
fn target_list(name: &str, value: &str) -> Result<Vec<Target>, String> {
    let targets = value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| target(name, item))
        .collect::<Result<Vec<_>, _>>()?;
    if targets.is_empty() {
        return Err(k::usage_bad_ratio(name.to_owned(), value.to_owned()));
    }
    if targets.len() > MAX_LOD_LEVELS {
        return Err(k::usage_too_many_lods(
            MAX_LOD_LEVELS as f64,
            name.to_owned(),
        ));
    }
    Ok(targets)
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
            for usage in command.usage() {
                lines.push(format!("  {}", tr(*usage)));
            }
            lines.push(String::new());
            if command.takes_models() {
                lines.push(tr(k::HELP_MODELS).into_owned());
                lines.push(String::new());
            }
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
            Ok(Parsed::Run(run)) => *run,
            other => panic!("{line}: {other:?}"),
        }
    }

    fn usage_error(line: &str) -> String {
        match parse_line(line) {
            Err(error) => error.message,
            other => panic!("{line} parsed: {other:?}"),
        }
    }

    /// A count is digits and a `t`, as a budget is written (`1500t`, `1500T`), and
    /// it mixes with shares in one chain; a fraction of a triangle, a zero or a bare
    /// `t` is refused with the count's own message.
    #[test]
    fn triangle_counts_are_read_with_a_t() {
        let parsed = run("optimize a.fbx --out o --lods 750t,25%,375T --reduce 1500t");
        let Command::Optimize(args) = parsed.command else {
            panic!("not optimize");
        };
        assert_eq!(
            args.lods,
            Some(Lods::Levels(vec![
                Target::Triangles(750),
                Target::Share(0.25),
                Target::Triangles(375)
            ]))
        );
        assert_eq!(args.reduce, Some(Target::Triangles(1500)));
        for bad in ["1.5t", "0t", "t", "-3t"] {
            let error = target(REDUCE, bad).expect_err(bad);
            assert!(
                error.starts_with("--reduce takes a whole number of triangles between 1 and 100")
                    && error.contains(&format!("not \"{bad}\"")),
                "{bad}: the count's message, its values in their places: {error}"
            );
        }
    }

    /// The simplifier's settings without a preset: the error limit, the algorithm and
    /// the whole set of flags, by the Inspector's names, in any case; `none` is the
    /// empty set, and anything else is refused before a model is read.
    #[test]
    fn the_simplifiers_settings_are_read() {
        let parsed = run(
            "optimize a.fbx --out o --reduce 400t --target-error 1 --simplifier standard \
             --simplify-flags regularize,LOCK-BORDER,collapse-across-seams",
        );
        let Command::Optimize(args) = parsed.command else {
            panic!("not optimize");
        };
        assert_eq!(args.target_error, Some(1.0));
        assert_eq!(args.simplifier, Some(SimplifyAlgorithm::Standard));
        assert_eq!(
            args.simplify_flags,
            Some(SimplifyFlags {
                regularize: true,
                lock_border: true,
                permissive: true,
                ..SimplifyFlags::default()
            })
        );
        let Command::Optimize(args) =
            run("optimize a.fbx --out o --reduce 0.5 --simplify-flags none").command
        else {
            panic!("not optimize");
        };
        assert_eq!(args.simplify_flags, Some(SimplifyFlags::default()));
        for line in [
            "optimize a.fbx --out o --reduce 0.5 --target-error 2",
            "optimize a.fbx --out o --reduce 0.5 --target-error x",
            "optimize a.fbx --out o --reduce 0.5 --simplifier fast",
            "optimize a.fbx --out o --reduce 0.5 --simplify-flags regularize,smooth",
        ] {
            let error = usage_error(line);
            assert!(!error.is_empty(), "{line} is refused");
        }
        assert!(
            usage_error("optimize a.fbx --out o --reduce 0.5 --simplify-flags smooth")
                .contains("lock-border|error-absolute"),
            "a wrong flag is answered with the flags there are"
        );
    }

    /// A job's line is read on top of the command line: what it gives replaces the
    /// command line's for that model, what it leaves out comes from the command
    /// line, a quoted path keeps its spaces, and what belongs to the whole run is
    /// refused.
    #[test]
    fn a_jobs_line_is_read_on_top_of_the_command_line() {
        let Command::Optimize(base) =
            run("optimize --jobs jobs.txt --out out --reduce 0.5 --target-error 1 --suffix _low")
                .command
        else {
            panic!("not optimize");
        };
        assert!(base.jobs.is_some());
        let (model, job) = job_line(
            r#""parts/my link.fbx" --reduce 1500t --out "build/my parts""#,
            &base,
        )
        .expect("a job");
        assert_eq!(model, PathBuf::from("parts/my link.fbx"));
        assert_eq!(job.reduce, Some(Target::Triangles(1500)), "the line's own");
        assert_eq!(job.out, PathBuf::from("build/my parts"));
        assert_eq!(job.target_error, Some(1.0), "the command line's");
        assert_eq!(job.suffix, "_low");
        assert!(job.jobs.is_none(), "a job does not read the list again");

        for (line, says) in [
            ("--reduce 0.5", "starts with its model"),
            ("a.fbx b.fbx --reduce 0.5", "one model"),
            ("a.fbx --report r.json", "--report is for the whole run"),
            ("a.fbx --reduce 2", "takes a share"),
        ] {
            let error = job_line(line, &base).expect_err(line);
            assert!(error.contains(says), "{line}: {error}");
        }
    }

    /// A quality limit is a percentage or a share, and `none` switches a preset's
    /// off; outside its range it is refused with the range.
    #[test]
    fn quality_limits_are_read() {
        let Command::Optimize(args) =
            run("optimize a.fbx --out o --reduce 0.5 --max-volume-change 5% --max-distance none")
                .command
        else {
            panic!("not optimize");
        };
        assert_eq!(args.max_volume_change, Some(Some(0.05)));
        assert_eq!(args.max_distance, Some(None));
        let error = usage_error("optimize a.fbx --out o --reduce 0.5 --max-volume-change 300%");
        assert!(error.contains("between 0.1% and 100%"), "{error}");
        assert!(!usage_error("optimize a.fbx --out o --reduce 0.5 --max-distance 0").is_empty());
    }

    #[test]
    fn an_optimize_line_reads_every_option() {
        let parsed = run(
            "optimize a.fbx props --out out --preset p.json --lods 50%,0.25 --reduce 0.8 \
             --packaging file-per-lod --hierarchy flat --format ASCII --suffix _low \
             --if-exists skip --replace-models --recurse --report r.json --progress-bar on",
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
        assert_eq!(
            args.lods,
            Some(Lods::Levels(vec![Target::Share(0.5), Target::Share(0.25)]))
        );
        assert_eq!(args.reduce, Some(Target::Share(0.8)));
        assert_eq!(args.packaging, Some(LodPackaging::FilePerLod));
        assert_eq!(args.hierarchy, Some(HierarchyMode::FlatBaked));
        assert_eq!(args.format, Some(FbxFormat::Ascii));
        assert_eq!(args.suffix, "_low");
        assert_eq!(args.if_exists, IfExists::Skip);
        assert!(args.replace_models);
    }

    #[test]
    fn values_may_follow_an_equals_sign_and_the_last_one_wins() {
        let parsed = run("audit a.fbx --profile=unreal --fail-on=info --fail-on warning");
        let Command::Audit(args) = parsed.command else {
            panic!("not audit");
        };
        assert_eq!(
            args.judge,
            Judge::Profile(ProfileChoice::Builtin(Engine::Unreal))
        );
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
        assert_eq!(
            args.judge,
            Judge::Profile(ProfileChoice::Builtin(Engine::Generic))
        );
        assert_eq!(args.fail_on, FailOn::At(Severity::Error));
    }

    #[test]
    fn a_profile_that_is_not_a_builtin_is_a_file() {
        let Command::Audit(args) = run("audit a.fbx --profile team.json").command else {
            panic!("not audit");
        };
        assert_eq!(
            args.judge,
            Judge::Profile(ProfileChoice::File(PathBuf::from("team.json")))
        );
    }

    #[test]
    fn rules_pick_the_profile_and_a_warning_fails_the_run() {
        let parsed = run("audit --staged --rules rules/rules.json --results r.json");
        assert!(parsed.common.inputs.is_empty());
        let Command::Audit(args) = parsed.command else {
            panic!("not audit");
        };
        assert_eq!(args.judge, Judge::Rules(PathBuf::from("rules/rules.json")));
        assert_eq!(args.fail_on, FailOn::At(Severity::Warning));
        assert_eq!(args.results, Some(PathBuf::from("r.json")));
        assert!(args.staged);
        // An explicit --fail-on still wins.
        let Command::Audit(args) = run("audit a.fbx --rules r.json --fail-on error").command else {
            panic!("not audit");
        };
        assert_eq!(args.fail_on, FailOn::At(Severity::Error));
    }

    #[test]
    fn a_list_of_files_stands_in_for_the_models() {
        let parsed = run("info --files-from list.txt");
        assert!(parsed.common.inputs.is_empty());
        assert_eq!(parsed.common.files_from, Some(PathBuf::from("list.txt")));
        let parsed = run("optimize a.fbx --files-from list.txt --out o --lods 0.5");
        assert_eq!(parsed.common.inputs, [PathBuf::from("a.fbx")]);
    }

    #[test]
    fn the_rules_commands_read_their_own_lines() {
        let rules = |line: &str| match parse_line(line) {
            Ok(Parsed::Rules(args)) => args,
            other => panic!("{line}: {other:?}"),
        };
        let check = RulesArgs::Check {
            rules: PathBuf::from("rules.json"),
        };
        assert_eq!(rules("rules check rules.json"), check);
        assert_eq!(rules("rules check --rules rules.json"), check);
        assert_eq!(
            rules("rules explain SM_Rock.fbx a/b.png --rules rules.json"),
            RulesArgs::Explain {
                rules: PathBuf::from("rules.json"),
                files: vec![PathBuf::from("SM_Rock.fbx"), PathBuf::from("a/b.png")],
            }
        );
        assert_eq!(rules("rules catalog"), RulesArgs::Catalog);
        assert!(parse_line("rules catalog rules.json").is_err());
        assert!(parse_line("rules catalog --rules rules.json").is_err());
        assert!(matches!(
            parse_line("rules explain --help"),
            Ok(Parsed::Help(Some(CommandName::Rules)))
        ));
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
            "optimize a.fbx --out o --reduce 0t",
            "optimize a.fbx --out o --reduce 1.5t",
            "optimize a.fbx --out o --reduce t",
            "optimize a.fbx --out o --lods 750t,-3t",
            "optimize a.fbx --out o --reduce 0",
            "optimize a.fbx --out o --reduce 0.5 --packaging both",
            "optimize a.fbx --out o --reduce 0.5 --recurse=yes",
            "audit a.fbx --out o",
            "audit a.fbx --fail-on errors",
            "info a.fbx --profile unity",
            "--version extra",
            "audit a.fbx --rules r.json --profile unity",
            "audit --staged a.fbx",
            "audit --staged --files-from list.txt",
            "audit",
            "info --staged",
            "info a.fbx --rules r.json",
            "optimize a.fbx --out o --lods 0.5 --results r.json",
            "rules",
            "rules lint r.json",
            "rules check",
            "rules check a.json b.json",
            "rules check a.json --rules b.json",
            "rules explain a.fbx",
            "rules explain --rules r.json",
            "rules check r.json --report x.json",
            "rules check r.json --recurse",
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

    /// A value after `=` keeps the bytes it was given, as one after a space
    /// does: a path that is not valid Unicode used to arrive with U+FFFD in it.
    #[test]
    fn a_value_after_an_equals_sign_keeps_bytes_that_are_not_unicode() {
        #[cfg(windows)]
        let (inline, value) = {
            use std::os::windows::ffi::OsStringExt;
            let value: Vec<u16> = vec![u16::from(b'o'), 0xD800, u16::from(b'x')];
            let mut inline: Vec<u16> = "--out=".encode_utf16().collect();
            inline.extend(&value);
            (OsString::from_wide(&inline), OsString::from_wide(&value))
        };
        #[cfg(unix)]
        let (inline, value) = {
            use std::os::unix::ffi::OsStringExt;
            (
                OsString::from_vec(b"--out=o\xffx".to_vec()),
                OsString::from_vec(b"o\xffx".to_vec()),
            )
        };
        #[cfg(any(windows, unix))]
        {
            assert!(value.to_str().is_none(), "the value is not Unicode");
            let line = ["optimize", "a.fbx", "--lods", "0.5"]
                .into_iter()
                .map(OsString::from)
                .chain([inline]);
            let Ok(Parsed::Run(run)) = parse(line) else {
                panic!("refused");
            };
            let Command::Optimize(args) = run.command else {
                panic!("not optimize");
            };
            assert_eq!(args.out, PathBuf::from(value));
        }
    }

    #[test]
    fn a_refused_line_still_names_its_report() {
        let path = |line: &str| {
            output_path(
                &line
                    .split_whitespace()
                    .map(OsString::from)
                    .collect::<Vec<_>>(),
                REPORT,
            )
        };
        assert_eq!(
            path("audit a.fbx --report r.json --fail-on sometimes"),
            Some(PathBuf::from("r.json"))
        );
        assert_eq!(
            path("audit --report=first.json --frobnicate --report=last.json"),
            Some(PathBuf::from("last.json"))
        );
        assert_eq!(path("audit a.fbx --report --recurse"), None);
        assert_eq!(path("info a.fbx -- --report r.json"), None);
        assert_eq!(path("info a.fbx --reporting r.json"), None);
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
        assert!(main.contains("rules"));
        let rules = help(Some(CommandName::Rules));
        assert!(rules.contains("rules check") && rules.contains("rules explain"));
        assert!(rules.contains("--rules <rules.json>"));
        assert!(
            !rules.contains("--report"),
            "a rules command writes no report"
        );
    }
}
