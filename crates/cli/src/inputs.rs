//! The models a run works on, from the files and folders it was given.
//!
//! A file is taken as given, whatever its extension (the import says so if it
//! is not an FBX). A folder stands for the `.fbx` files in it, in name order -
//! and, with `--recurse`, those in its folders too, skipping any folder whose
//! name starts with `.` and never following a symlinked folder, which is what
//! keeps a link cycle from walking forever. A model reached twice is worked on
//! once.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::args::Common;
use crate::console;
use crate::keys::cli as k;
use crate::report::{Entry, Report, Status};

/// One model to work on.
#[derive(Debug, Clone)]
pub(crate) struct Model {
    pub(crate) path: PathBuf,
    /// Where it sits under the argument that named it: its file name for a file
    /// given directly, `props/chair.fbx` for one found in a folder. What an
    /// output keeps of the input's place.
    pub(crate) relative: PathBuf,
}

impl Model {
    /// The model as a line or a report names it: the path it was reached by.
    pub(crate) fn label(&self) -> String {
        self.path.display().to_string()
    }
}

/// An argument that could not be used, and why - a failed entry in the run.
#[derive(Debug, Clone)]
pub(crate) struct Unusable {
    pub(crate) path: PathBuf,
    pub(crate) problem: String,
}

#[derive(Debug, Default)]
pub(crate) struct Inputs {
    pub(crate) models: Vec<Model>,
    pub(crate) unusable: Vec<Unusable>,
}

/// The models a run works on, every argument it cannot use already said and
/// recorded as a failed entry. Empty when there is nothing to do, in which case
/// a run that found nothing at all has said so and recorded it as the run's
/// problem.
pub(crate) fn gather(common: &Common, report: &mut Report) -> Vec<Model> {
    let inputs = collect(&common.inputs, common.recurse);
    report.expect(inputs.models.len() + inputs.unusable.len());
    for unusable in &inputs.unusable {
        console::warn(&unusable.problem);
        let entry = Entry::new(&unusable.path.display().to_string());
        report.add(Status::Failed, entry, Some(unusable.problem.clone()));
    }
    if inputs.models.is_empty() && inputs.unusable.is_empty() {
        let problem = review_localization::tr(k::NO_MODELS).into_owned();
        console::warn(&problem);
        report.problem(problem);
    }
    inputs.models
}

pub(crate) fn collect(arguments: &[PathBuf], recurse: bool) -> Inputs {
    let mut inputs = Inputs::default();
    for argument in arguments {
        match std::fs::metadata(argument) {
            Ok(meta) if meta.is_dir() => walk(argument, Path::new(""), recurse, &mut inputs),
            Ok(_) => inputs.models.push(Model {
                path: argument.clone(),
                relative: argument
                    .file_name()
                    .map_or_else(|| argument.clone(), PathBuf::from),
            }),
            Err(_) => inputs.unusable.push(Unusable {
                path: argument.clone(),
                problem: k::not_there(argument.display().to_string()),
            }),
        }
    }
    let mut seen = HashSet::new();
    inputs
        .models
        .retain(|model| seen.insert(canonical(&model.path)));
    inputs
}

/// The `.fbx` files in `folder` (`relative` below the argument), depth first,
/// in name order.
fn walk(folder: &Path, relative: &Path, recurse: bool, inputs: &mut Inputs) {
    let mut entries: Vec<_> = match std::fs::read_dir(folder) {
        Ok(entries) => entries.filter_map(Result::ok).collect(),
        Err(error) => {
            inputs.unusable.push(Unusable {
                path: folder.to_path_buf(),
                problem: k::folder_unreadable(folder.display().to_string(), error.to_string()),
            });
            return;
        }
    };
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        // `file_type` does not follow symlinks: a symlinked folder is never
        // entered, while a symlinked file is still a file below.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name();
        if kind.is_dir() {
            if recurse && !name.to_string_lossy().starts_with('.') {
                walk(&path, &relative.join(&name), recurse, inputs);
            }
        } else if is_fbx(&path) && path.is_file() {
            inputs.models.push(Model {
                relative: relative.join(&name),
                path,
            });
        }
    }
}

fn is_fbx(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("fbx"))
}

/// `path` with every link and `..` resolved, or as given when it cannot be -
/// for telling two spellings of one file apart.
pub(crate) fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// [`canonical`] for a file that may not exist yet: its folder resolved, its
/// name kept.
pub(crate) fn canonical_destination(path: &Path) -> PathBuf {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) if !parent.as_os_str().is_empty() => {
            canonical(parent).join(name)
        }
        (_, Some(name)) => canonical(Path::new(".")).join(name),
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_gives_its_models_in_name_order_and_recursing_keeps_their_place() {
        let root = std::env::temp_dir().join(format!("review-cli-inputs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for file in [
            "b.fbx",
            "a.FBX",
            "notes.txt",
            "props/chair.fbx",
            ".hidden/skip.fbx",
        ] {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().expect("has a folder")).expect("folder");
            std::fs::write(&path, b"").expect("file");
        }

        let flat = collect(std::slice::from_ref(&root), false);
        let names: Vec<_> = flat.models.iter().map(|m| m.relative.clone()).collect();
        assert_eq!(names, [PathBuf::from("a.FBX"), PathBuf::from("b.fbx")]);

        let deep = collect(&[root.clone(), root.join("b.fbx")], true);
        let names: Vec<_> = deep.models.iter().map(|m| m.relative.clone()).collect();
        assert_eq!(
            names,
            [
                PathBuf::from("a.FBX"),
                PathBuf::from("b.fbx"),
                Path::new("props").join("chair.fbx"),
            ],
            "the second spelling of b.fbx is dropped, the dot-folder skipped"
        );

        let missing = collect(&[root.join("nope.fbx")], false);
        assert!(missing.models.is_empty());
        assert_eq!(missing.unusable.len(), 1);

        let _ = std::fs::remove_dir_all(&root);
    }
}
