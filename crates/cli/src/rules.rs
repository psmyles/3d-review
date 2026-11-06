//! `review3d-cli rules check`, `rules explain` and `rules catalog`: Asset
//! Check's commands for the technical artist who writes `rules.json` and the
//! rules files (AC9).
//!
//! `check` and `explain` are `asset-rules`' own, so `fire-cli`'s `rules`
//! commands say the same thing about the same file: `check` lists the problems
//! in a configuration, and in the model rules files it names read against this
//! tool's catalog (`fire-cli` checks the texture rules files); `explain` says
//! which type, profile, stage and rules file apply to a file and why. `catalog`
//! prints this tool's rules catalog, which Rule Editor reads. What they print is that crate's text, in English, as a library's
//! diagnostics are (invariant 12): it quotes `rules.json`'s own fields, which
//! are English too.
//!
//! `check` exits 1 on any problem, warnings included - the convention every
//! Asset Check exit status keeps (AC10) - so a CI step that runs it stops on
//! one.

use std::path::{Path, PathBuf};

use asset_rules::git::Repo;
use asset_rules::{Context, Level, Rules};

use crate::args::RulesArgs;
use crate::console;
use crate::keys::cli as k;
use crate::results::tool;

pub(crate) fn run(args: &RulesArgs) -> bool {
    match args {
        RulesArgs::Check { rules } => check(rules),
        RulesArgs::Explain { rules, files } => explain(rules, files),
        RulesArgs::Catalog => catalog(),
    }
}

/// Every rule this tool checks, as JSON, with this tool's version.
fn catalog() -> bool {
    let json = review_audit::rule::catalog().to_json(&tool().version);
    console::say(json.trim_end());
    true
}

/// Load `path`, saying why when it cannot be - and, for a `rules.json` too new
/// for this tool, what to update.
pub(crate) fn load(path: &Path) -> Result<Rules, String> {
    Rules::load(path).map_err(|error| {
        let mut problem = k::rules_unreadable(path.display().to_string(), error.to_string());
        if let Some(finding) = error.too_old(&tool()) {
            problem = format!("{problem}\n{}\n{}", finding.what, finding.how);
        }
        problem
    })
}

fn check(path: &Path) -> bool {
    let rules = match load(path) {
        Ok(rules) => rules,
        Err(problem) => {
            console::warn(&problem);
            return false;
        }
    };
    let mut problems = rules.check();
    problems.extend(rules.check_rules_files(review_audit::rule::catalog()));
    for problem in &problems {
        console::say(&problem.to_string());
    }
    let too_old = rules.too_old(&tool());
    if let Some(finding) = &too_old {
        console::say(&finding.what);
        console::say(&finding.how);
    }
    let errors = problems
        .iter()
        .filter(|problem| problem.level() == Level::Error)
        .count();
    let warnings = problems.len() - errors + usize::from(too_old.is_some());
    let file = path.display().to_string();
    if errors + warnings == 0 {
        console::say(&k::rules_check_clean(file));
        true
    } else {
        console::say(&k::rules_check_problems(
            errors as f64,
            file,
            warnings as f64,
        ));
        false
    }
}

fn explain(path: &Path, files: &[PathBuf]) -> bool {
    let rules = match load(path) {
        Ok(rules) => rules,
        Err(problem) => {
            console::warn(&problem);
            return false;
        }
    };
    let ctx = context();
    for (index, file) in files.iter().enumerate() {
        if index > 0 {
            console::say("");
        }
        let absolute = std::path::absolute(file).unwrap_or_else(|_| file.clone());
        console::say(asset_rules::explain(&rules, &ctx, &absolute).trim_end());
    }
    true
}

/// The repository the current folder is in, for the paths profiles match and
/// the branch the stage can come from. Outside one there is no branch, and only
/// profiles without `paths` can match.
pub(crate) fn context() -> Context {
    Repo::discover(Path::new("."))
        .and_then(|repo| repo.context())
        .unwrap_or_else(|_| Context {
            repo_root: std::path::absolute(".").unwrap_or_default(),
            branch: None,
        })
}
