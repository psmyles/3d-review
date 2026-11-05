//! The console tool as a caller sees it: the binary run with a command line,
//! judged by its exit status, what it prints and the report it writes.
//!
//! This is the contract Launcher and any script are written against
//! (`docs/cli.md`), so each test pins one clause of it. The model tests skip
//! themselves when the fixture is absent; everything else needs nothing.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use asset_rules::git::Repo;
use asset_rules::results::{
    FileResult, Results, Severity as FindingSeverity, Status as FileStatus,
};
use asset_rules::{Resolution, Rules, StageFrom};
use serde_json::Value;

const BINARY: &str = env!("CARGO_BIN_EXE_3d-review-cli");

fn run(args: &[&str]) -> Output {
    Command::new(BINARY)
        .args(args)
        .output()
        .expect("the binary runs")
}

fn status(output: &Output) -> i32 {
    output.status.code().expect("exited, not killed")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A fresh folder for one test.
fn scratch(name: &str) -> PathBuf {
    let folder = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("cli-{name}"));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("scratch folder");
    folder
}

/// A small real model, or `None` (and the test skips) without the fixtures.
fn cube() -> Option<PathBuf> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/test_models/meter_cube.fbx");
    path.is_file().then_some(path)
}

fn report(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).expect("a report was written");
    serde_json::from_str(&text).expect("the report is JSON")
}

fn text(path: &Path) -> &str {
    path.to_str().expect("a UTF-8 path")
}

#[test]
fn a_command_line_it_cannot_follow_exits_2_and_runs_nothing() {
    let folder = scratch("usage");
    let report_path = folder.join("report.json");
    let results_path = folder.join("results.json");
    // Ones an earlier run left: after any run, the file is this run's or is
    // not there, and a refused line writes none.
    std::fs::write(&report_path, "{}").expect("a stale report");
    std::fs::write(&results_path, "{}").expect("stale results");
    for args in [
        vec![],
        vec!["optimise", "a.fbx"],
        vec!["info", "a.fbx", "--frobnicate"],
        vec!["optimize", "a.fbx", "--lods", "0.5"],
        vec!["optimize", "a.fbx", "--out", "o", "--lods", "2"],
        vec!["audit", "a.fbx", "--fail-on"],
        vec![
            "audit",
            "a.fbx",
            "--report",
            text(&report_path),
            "--fail-on",
            "sometimes",
        ],
        vec![
            "audit",
            "a.fbx",
            "--rules",
            "rules.json",
            "--profile",
            "unity",
            "--results",
            text(&results_path),
        ],
        vec!["audit", "--staged", "a.fbx"],
        vec!["rules", "check"],
        vec!["rules", "explain", "a.fbx"],
    ] {
        let output = run(&args);
        assert_eq!(status(&output), 2, "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?} printed results");
        assert!(!output.stderr.is_empty(), "{args:?} said nothing");
    }
    assert!(
        !report_path.exists(),
        "a refused command line left a report at its --report path"
    );
    assert!(
        !results_path.exists(),
        "a refused command line left results at its --results path"
    );
}

#[test]
fn version_and_help_exit_0() {
    let version = run(&["--version"]);
    assert_eq!(status(&version), 0);
    assert!(stdout(&version).starts_with("3d-review-cli "));
    let main = run(&["--help"]);
    assert_eq!(status(&main), 0);
    assert!(stdout(&main).contains("optimize"));
    for args in [
        &["optimize", "--help"][..],
        &["audit", "-h"],
        &["info", "x", "--help"],
    ] {
        let help = run(args);
        assert_eq!(status(&help), 0, "{args:?}");
        assert!(stdout(&help).contains("--report"), "{args:?}");
    }
}

#[test]
fn a_model_that_is_not_there_fails_the_run_and_is_recorded() {
    let folder = scratch("missing");
    let report_path = folder.join("report.json");
    let missing = folder.join("missing.fbx");
    let output = run(&["info", text(&missing), "--report", text(&report_path)]);
    assert_eq!(status(&output), 1);
    let report = report(&report_path);
    assert_eq!(report["schema"], "3d-review.cli-report");
    assert_eq!(report["ok"], false);
    assert_eq!(report["models"], 1);
    assert_eq!(report["failed"], 1);
    assert_eq!(report["results"][0]["status"], "failed");
}

#[test]
fn a_git_lfs_pointer_is_named_as_one() {
    let folder = scratch("lfs");
    let pointer = folder.join("pointer.fbx");
    std::fs::write(
        &pointer,
        "version https://git-lfs.github.com/spec/v1\noid sha256:0\nsize 1\n",
    )
    .expect("pointer");
    let output = run(&["info", text(&pointer)]);
    assert_eq!(status(&output), 1);
    assert!(String::from_utf8_lossy(&output.stderr).contains("git lfs pull"));
}

#[test]
fn a_folder_with_no_models_is_a_problem_not_a_success() {
    let folder = scratch("empty");
    let report_path = folder.join("report.json");
    let output = run(&["audit", text(&folder), "--report", text(&report_path)]);
    assert_eq!(status(&output), 1);
    let report = report(&report_path);
    assert_eq!(report["models"], 0);
    assert!(report["problem"].is_string());
}

#[test]
fn info_reports_the_source_counts() {
    let Some(cube) = cube() else { return };
    let folder = scratch("info");
    let report_path = folder.join("report.json");
    let output = run(&["info", text(&cube), "--report", text(&report_path)]);
    assert_eq!(
        status(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = report(&report_path);
    assert_eq!(report["ok"], true);
    // The DCC's counts: six quads over eight points, never the corner split.
    assert_eq!(report["polygons"], 6);
    assert_eq!(report["triangles"], 12);
    assert_eq!(report["vertices"], 8);
    assert_eq!(report["results"][0]["counts"]["polygons"], 6);
    // The axis is spelled as an audit profile spells one, never as a Rust
    // variant (`PositiveY`, `Unnamed(3)`).
    let up = &report["results"][0]["up_axis"];
    let axes = [
        "positive_x",
        "negative_x",
        "positive_y",
        "negative_y",
        "positive_z",
        "negative_z",
    ];
    assert!(
        up.is_null() || up.as_str().is_some_and(|axis| axes.contains(&axis)),
        "{up}"
    );
}

#[test]
fn optimize_writes_once_and_then_fails_skips_or_replaces_as_told() {
    let Some(cube) = cube() else { return };
    let folder = scratch("optimize");
    let out = folder.join("out");
    let report_path = folder.join("report.json");
    let first = run(&[
        "optimize",
        text(&cube),
        "--out",
        text(&out),
        "--lods",
        "50%",
        "--packaging",
        "file-per-lod",
        "--report",
        text(&report_path),
    ]);
    assert_eq!(
        status(&first),
        0,
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let written = report(&report_path);
    assert_eq!(written["done"], 1);
    assert_eq!(written["written"], 2);
    assert_eq!(
        written["results"][0]["levels"].as_array().map(Vec::len),
        Some(2)
    );
    for level in ["meter_cube_LOD0.fbx", "meter_cube_LOD1.fbx"] {
        assert!(out.join(level).is_file(), "{level}");
    }

    let again = |policy: &str| {
        run(&[
            "optimize",
            text(&cube),
            "--out",
            text(&out),
            "--lods",
            "50%",
            "--packaging",
            "file-per-lod",
            "--if-exists",
            policy,
            "--report",
            text(&report_path),
        ])
    };
    assert_eq!(status(&again("fail")), 1);
    assert_eq!(report(&report_path)["failed"], 1);
    assert_eq!(status(&again("skip")), 0);
    assert_eq!(report(&report_path)["skipped"], 1);
    assert_eq!(status(&again("replace")), 0);
    assert_eq!(report(&report_path)["done"], 1);
}

#[test]
fn optimize_never_writes_over_a_model_it_reads() {
    let Some(cube) = cube() else { return };
    let folder = scratch("in-place");
    let model = folder.join("meter_cube.fbx");
    std::fs::copy(&cube, &model).expect("copied");
    let before = std::fs::read(&model).expect("read");
    let output = run(&[
        "optimize",
        text(&model),
        "--out",
        text(&folder),
        "--lods",
        "0.5",
        "--if-exists",
        "replace",
    ]);
    assert_eq!(status(&output), 1);
    assert_eq!(std::fs::read(&model).expect("read"), before);
}

/// `--replace-models` lets a model's results replace it, whatever
/// `--if-exists` says - but never another model of the run.
#[test]
fn replace_models_lets_a_model_replace_only_itself() {
    let Some(cube) = cube() else { return };
    let folder = scratch("replace-models");
    let model = folder.join("meter_cube.fbx");
    std::fs::copy(&cube, &model).expect("copied");
    let before = std::fs::read(&model).expect("read");
    let report_path = folder.join("report.json");
    let output = run(&[
        "optimize",
        text(&model),
        "--out",
        text(&folder),
        "--lods",
        "0.5",
        "--if-exists",
        "fail",
        "--replace-models",
        "--report",
        text(&report_path),
    ]);
    assert_eq!(status(&output), 0, "{}", stdout(&output));
    assert_eq!(report(&report_path)["done"], 1);
    assert_ne!(std::fs::read(&model).expect("read"), before);

    // `other_LOD1.fbx` is a model of the run, and `other.fbx`'s second level
    // under file-per-lod: it is still refused.
    let other = folder.join("other.fbx");
    let level = folder.join("other_LOD1.fbx");
    std::fs::copy(&cube, &other).expect("copied");
    std::fs::copy(&cube, &level).expect("copied");
    let level_before = std::fs::read(&level).expect("read");
    let output = run(&[
        "optimize",
        text(&other),
        text(&level),
        "--out",
        text(&folder),
        "--lods",
        "0.5",
        "--packaging",
        "file-per-lod",
        "--replace-models",
        "--if-exists",
        "replace",
        "--report",
        text(&report_path),
    ]);
    assert_eq!(status(&output), 1, "{}", stdout(&output));
    assert_eq!(std::fs::read(&level).expect("read"), level_before);
}

/// The output is always `<stem>.fbx` in lower case, and NTFS and APFS ignore
/// case: `Rock.FBX` written out as `Rock.fbx` is the model itself, and so is
/// `rock.fbx` typed for it.
#[test]
fn optimize_never_writes_over_a_model_it_reads_spelled_in_another_case() {
    let Some(cube) = cube() else { return };
    let folder = scratch("in-place-case");
    let model = folder.join("Rock.FBX");
    std::fs::copy(&cube, &model).expect("copied");
    if !folder.join("rock.fbx").is_file() {
        // A case-sensitive file system: `Rock.fbx` is another file, and
        // writing it is right.
        return;
    }
    let before = std::fs::read(&model).expect("read");
    for typed in ["Rock.FBX", "rock.fbx"] {
        let output = run(&[
            "optimize",
            text(&folder.join(typed)),
            "--out",
            text(&folder),
            "--lods",
            "0.5",
            "--if-exists",
            "replace",
        ]);
        assert_eq!(status(&output), 1, "{typed}");
        assert_eq!(std::fs::read(&model).expect("read"), before, "{typed}");
    }
}

/// Two models that would write one file: the second is refused, even when
/// `--out` does not exist until the first model's export creates it - and
/// refused as a collision within the run, not as a file already there that
/// `--if-exists replace` may overwrite.
#[test]
fn two_models_writing_one_file_are_caught_though_the_out_folder_is_new() {
    let Some(cube) = cube() else { return };
    let folder = scratch("written-twice");
    let copy = |relative: &str| {
        let path = folder.join(relative);
        std::fs::create_dir_all(path.parent().expect("a folder")).expect("folder");
        std::fs::copy(&cube, &path).expect("copied");
        path
    };
    let first = copy("a/body.fbx");
    let second = copy("b/body.fbx");
    let case_insensitive = folder.join("A").join("BODY.FBX").is_file();
    let mut cases = vec![(first, second, "build")];
    if case_insensitive {
        // Two names differing only in case are one file too.
        cases.push((copy("c/Body.fbx"), copy("d/body.fbx"), "build-case"));
    }
    for (first, second, out) in cases {
        let out = folder.join(out);
        let report_path = folder.join("report.json");
        let output = run(&[
            "optimize",
            text(&first),
            text(&second),
            "--out",
            text(&out),
            "--reduce",
            "0.5",
            "--if-exists",
            "replace",
            "--report",
            text(&report_path),
        ]);
        assert_eq!(status(&output), 1, "{}", stdout(&output));
        let report = report(&report_path);
        assert_eq!(report["done"], 1);
        assert_eq!(report["failed"], 1);
        assert_eq!(report["written"], 1);
        let problem = report["results"][1]["problem"].as_str().expect("a problem");
        assert!(
            problem.contains(text(&first)),
            "the second model is told which model has the file: {problem}"
        );
    }
}

#[test]
fn audit_fails_the_run_only_at_the_fail_on_severity() {
    let Some(cube) = cube() else { return };
    let folder = scratch("audit");
    let report_path = folder.join("report.json");
    let never = run(&[
        "audit",
        text(&cube),
        "--fail-on",
        "never",
        "--report",
        text(&report_path),
    ]);
    assert_eq!(status(&never), 0);
    let report = report(&report_path);
    assert_eq!(report["flagged"], 0);
    let entry = &report["results"][0];
    assert_eq!(entry["audit"]["schema"], "3d-review.audit-report");

    // `info` is the least severe level, so any finding at all reaches it: the
    // run fails exactly when the model has one.
    let any = run(&["audit", text(&cube), "--fail-on", "info"]);
    let has_findings = entry["worst"].is_string();
    assert_eq!(status(&any), if has_findings { 1 } else { 0 });
}

// Asset Check: `--rules`, `--results`, `--staged`, `--files-from` and the
// `rules` commands (AC13). Each model test skips itself without the fixture,
// as above; they need `git` on PATH, as the hook does.

/// A project repository on `main` with Asset Check's rules in `rules/`: an
/// environment-prop profile narrowed by folder, a prop profile with a stage of
/// its own, and a prop rules file whose texel-density check is an error with a
/// note on it.
struct Project {
    root: PathBuf,
}

const RULES_JSON: &str = r#"{
  "version": 1,
  "mode": "advise",
  "stages": ["prototype", "production"],
  "stage": "prototype",
  "branches": [{ "pattern": "main", "stage": "production" }],
  "types": {
    "texture": { "tool": "fire-cli", "extensions": ["png"] },
    "model": { "tool": "3d-review-cli", "extensions": ["fbx"] }
  },
  "profiles": [
    { "id": "environment-prop", "type": "model", "label": "Environment prop",
      "names": ["SM_*"], "paths": ["Assets/Environment/**"],
      "rules": { "*": "models/environment.json" } },
    { "id": "prop", "type": "model", "label": "Prop", "names": ["SM_*"], "stage": "production",
      "rules": { "production": "models/prop.json", "prototype": "models/prop-proto.json" } }
  ]
}"#;

/// Written by hand, as the Asset Check plan writes one: no `enabled`, and the
/// note under the check it explains.
const PROP_JSON: &str = r#"{ "version": 1, "generator": "hand-written", "profile": {
  "name": "Prop", "base": "unity", "rules": {
    "density.texel": { "severity": "error", "note": "Quest budget for props." }
} } }"#;

/// No `base`: the generic profile.
const PLAIN_JSON: &str =
    r#"{ "version": 1, "generator": "hand-written", "profile": { "name": "Plain", "rules": {} } }"#;

impl Project {
    fn new(name: &str) -> Project {
        let project = Project {
            root: scratch(name),
        };
        project.git(&["init", "-q", "-b", "main"]);
        project.write("rules/rules.json", RULES_JSON.as_bytes());
        project.write("rules/models/prop.json", PROP_JSON.as_bytes());
        project.write("rules/models/prop-proto.json", PLAIN_JSON.as_bytes());
        project.write("rules/models/environment.json", PLAIN_JSON.as_bytes());
        project.git(&["add", "rules"]);
        project.git(&["commit", "-q", "--no-verify", "-m", "rules"]);
        project
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    fn write(&self, relative: &str, contents: &[u8]) {
        let path = self.path(relative);
        std::fs::create_dir_all(path.parent().expect("a folder")).expect("folder");
        std::fs::write(path, contents).expect("written");
    }

    fn copy(&self, from: &Path, relative: &str) {
        self.write(relative, &std::fs::read(from).expect("read"));
    }

    /// git in the project, with no user's or machine's settings in the way.
    fn git(&self, args: &[&str]) -> String {
        let output = isolated(Command::new("git"))
            .arg("-C")
            .arg(&self.root)
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .expect("git runs");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// The tool, run in the project's top folder as a hook runs it.
    fn cli(&self, args: &[&str]) -> Output {
        isolated(Command::new(BINARY))
            .current_dir(&self.root)
            .args(args)
            .output()
            .expect("the binary runs")
    }

    /// What `asset-rules` itself says applies to `path` here - which is what
    /// `fire-cli`, built on the same crate, says too.
    fn resolve(&self, path: &str) -> Resolution {
        let rules = Rules::load(&self.path("rules/rules.json")).expect("the rules load");
        let ctx = Repo::discover(&self.root)
            .and_then(|repo| repo.context())
            .expect("a repository");
        asset_rules::resolve(&rules, &ctx, Path::new(path))
    }
}

/// A command with the machine's and the user's git settings left out, so a
/// test's repository behaves the same on every machine.
fn isolated(mut command: Command) -> Command {
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "no-global-git-config");
    command
}

/// Read results as any reader of them does: `asset-rules` refuses a file
/// that is not `asset-check.results` version 1, or not shaped like it.
fn results(path: &Path) -> Results {
    Results::read(path).unwrap_or_else(|error| panic!("valid results: {error}"))
}

fn file<'a>(results: &'a Results, path: &str) -> &'a FileResult {
    results
        .files
        .iter()
        .find(|file| file.path == path)
        .unwrap_or_else(|| panic!("{path} is in the results"))
}

/// Gate of Asset Check step 2: a model gets the profile and stage
/// `asset-rules` resolves for it - so the same as `fire-cli` gives it - from
/// its name, its folder and the branch, in precedence order.
#[test]
fn rules_give_each_model_the_profile_and_stage_asset_rules_resolves() {
    let Some(cube) = cube() else { return };
    let project = Project::new("rules-resolve");
    project.copy(&cube, "Assets/Environment/SM_Rock.fbx");
    project.copy(&cube, "Assets/Props/SM_Crate.fbx");
    project.copy(&cube, "Assets/Props/Crate.fbx");
    let results_path = project.path("results.json");

    let check = |branch_stage: &str| {
        let output = project.cli(&[
            "audit",
            "Assets",
            "--recurse",
            "--rules",
            "rules/rules.json",
            "--results",
            text(&results_path),
        ]);
        // A warning or worse fails a run with --rules: Crate matches no profile.
        assert_eq!(status(&output), 1, "{}", stdout(&output));
        let results = results(&results_path);
        assert_eq!(results.files.len(), 3);
        for file in &results.files {
            match project.resolve(&file.path) {
                Resolution::Resolved(resolved) => {
                    assert_eq!(file.profile.as_deref(), Some(&*resolved.profile_id));
                    assert_eq!(file.stage.as_deref(), Some(&*resolved.stage));
                    assert_eq!(file.stage_from, Some(resolved.stage_from));
                    assert_eq!(file.kind.as_deref(), Some("model"));
                }
                Resolution::Unmatched { .. } => {
                    assert_eq!(file.profile, None);
                    assert_eq!(file.findings[0].rule, "file.no_profile");
                }
                Resolution::Ignored | Resolution::Skipped { .. } => {
                    panic!("{} is left out", file.path)
                }
            }
        }
        let rock = file(&results, "Assets/Environment/SM_Rock.fbx");
        assert_eq!(rock.profile.as_deref(), Some("environment-prop"));
        assert_eq!(rock.stage.as_deref(), Some(branch_stage));
        results
    };

    // On main the branch decides: production for both.
    let on_main = check("production");
    let prop = file(&on_main, "Assets/Props/SM_Crate.fbx");
    assert_eq!(prop.profile.as_deref(), Some("prop"));
    assert_eq!(prop.stage_from, Some(StageFrom::Branch));
    assert_eq!(on_main.context.branch.as_deref(), Some("main"));

    // Elsewhere the prop keeps its own stage and the rest take the project's.
    project.git(&["checkout", "-q", "-b", "feature/rocks"]);
    let elsewhere = check("prototype");
    let prop = file(&elsewhere, "Assets/Props/SM_Crate.fbx");
    assert_eq!(prop.stage.as_deref(), Some("production"));
    assert_eq!(prop.stage_from, Some(StageFrom::Profile));
    let rock = file(&elsewhere, "Assets/Environment/SM_Rock.fbx");
    assert_eq!(rock.stage_from, Some(StageFrom::Project));
}

/// `rules.json` version 2: a model outside `check`, or matching `skip`, isn't
/// checked or reported; one named on the line gets a line saying why.
#[test]
fn models_the_rules_skip_are_left_out() {
    let Some(cube) = cube() else { return };
    let project = Project::new("rules-skip");
    let rules = RULES_JSON
        .replacen("\"version\": 1,", "\"version\": 2,", 1)
        .replacen(
            "\"types\":",
            r#""check": ["Assets/**"],
  "skip": [{ "label": "Third-party", "paths": ["Assets/Plugins/**"] }],
  "types":"#,
            1,
        );
    project.write("rules/rules.json", rules.as_bytes());
    project.copy(&cube, "Assets/Props/SM_Crate.fbx");
    project.copy(&cube, "Assets/Plugins/SM_Lib.fbx");
    project.copy(&cube, "Docs/SM_Diagram.fbx");
    let results_path = project.path("results.json");
    let output = project.cli(&[
        "audit",
        "Assets/Props/SM_Crate.fbx",
        "Assets/Plugins/SM_Lib.fbx",
        "Docs/SM_Diagram.fbx",
        "--rules",
        "rules/rules.json",
        "--results",
        text(&results_path),
    ]);
    let said = stdout(&output);
    let results = results(&results_path);
    let paths: Vec<&str> = results.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["Assets/Props/SM_Crate.fbx"], "{said}");
    assert!(said.contains("the rules skip it (Third-party)"), "{said}");
    assert!(
        said.contains("isn't in the paths the rules check"),
        "{said}"
    );
}

/// Each failing check is a finding an artist can act on: what, with the
/// figures; why; how; and the technical artist's note.
#[test]
fn results_say_what_why_and_how_with_the_note() {
    let Some(cube) = cube() else { return };
    let project = Project::new("rules-text");
    project.copy(&cube, "Assets/Props/SM_Crate.fbx");
    let results_path = project.path("results.json");
    let output = project.cli(&[
        "audit",
        "Assets/Props/SM_Crate.fbx",
        "--rules",
        "rules/rules.json",
        "--results",
        text(&results_path),
    ]);
    assert_eq!(status(&output), 1, "the texel check is an error");
    let results = results(&results_path);
    let prop = file(&results, "Assets/Props/SM_Crate.fbx");
    assert_eq!(prop.status, FileStatus::Checked);
    let texel = prop
        .findings
        .iter()
        .find(|finding| finding.rule == "density.texel")
        .expect("the cube's texel density is off the Unity target");
    assert_eq!(texel.severity, FindingSeverity::Error);
    assert_eq!(texel.note.as_deref(), Some("Quest budget for props."));
    assert_eq!(texel.title, "Texel density");
    for (field, value) in [
        ("what", &texel.what),
        ("why", &texel.why),
        ("how", &texel.how),
    ] {
        assert!(!value.trim().is_empty(), "{field} is empty");
    }
    // The figure and the limit, in words and as values.
    let measured = texel.measured.as_ref().expect("measured");
    let limit = texel.limit.as_ref().expect("a limit");
    assert_eq!(measured["total"], 1);
    assert_eq!(measured["unit"], "px_per_meter");
    let figure = |value: &Value| value.as_f64().expect("a number").to_string();
    for value in [&measured["value"], &limit["min"], &limit["max"]] {
        assert!(texel.what.contains(&figure(value)), "{}", texel.what);
    }
    assert_eq!(texel.objects, ["meter_cube"]);
    // Findings are worst first.
    assert_eq!(prop.findings[0].rule, "density.texel");

    // `fix` is reserved: always written, always null.
    let raw: Value =
        serde_json::from_str(&std::fs::read_to_string(&results_path).expect("read")).expect("JSON");
    let raw_finding = &raw["files"][0]["findings"][0];
    assert!(raw_finding.get("fix").is_some_and(Value::is_null));
    assert_eq!(raw["schema"], "asset-check.results");
}

/// `--results` with a `--profile` instead of rules, and a model that cannot be
/// read: it is in the results as unreadable, with why.
#[test]
fn a_model_that_cannot_be_read_is_unreadable_in_the_results() {
    let Some(cube) = cube() else { return };
    let folder = scratch("results-unreadable");
    let broken = folder.join("broken.fbx");
    std::fs::write(&broken, b"not an FBX at all").expect("written");
    let results_path = folder.join("results.json");
    let output = run(&[
        "audit",
        text(&cube),
        text(&broken),
        "--profile",
        "unity",
        "--fail-on",
        "never",
        "--results",
        text(&results_path),
    ]);
    assert_eq!(status(&output), 1, "a model that cannot be checked fails");
    let results = results(&results_path);
    assert_eq!(results.files.len(), 2);
    let [checked, unreadable] = [&results.files[0], &results.files[1]];
    assert_eq!(checked.status, FileStatus::Checked);
    assert_eq!(checked.profile.as_deref(), Some("unity"));
    assert!(!checked.findings.is_empty());
    assert_eq!(unreadable.status, FileStatus::Unreadable);
    assert!(unreadable.problem.as_deref().is_some_and(|p| !p.is_empty()));
    assert!(unreadable.findings.is_empty());
}

/// The hook's run: what is staged is what is checked - not the working file,
/// which has changed since - and a second run of the same commit comes from
/// the cache, with the same results.
#[test]
fn staged_models_are_checked_as_staged_and_then_from_the_cache() {
    let Some(cube) = cube() else { return };
    let project = Project::new("staged");
    project.copy(&cube, "Assets/Props/SM_Crate.fbx");
    project.copy(&cube, "Assets/Props/SM_Unstaged.fbx");
    project.write("Assets/Props/T_Crate.png", b"a texture, fire-cli's");
    project.git(&[
        "add",
        "Assets/Props/SM_Crate.fbx",
        "Assets/Props/T_Crate.png",
    ]);
    project.write(
        "Assets/Props/SM_Crate.fbx",
        b"edited since, and no longer a model",
    );

    let results_path = project.path("results.json");
    let report_path = project.path("report.json");
    let staged = || {
        project.cli(&[
            "audit",
            "--staged",
            "--rules",
            "rules/rules.json",
            "--results",
            text(&results_path),
            "--report",
            text(&report_path),
        ])
    };
    let first = staged();
    assert_eq!(status(&first), 1, "{}", stdout(&first));
    let fresh = results(&results_path);
    assert_eq!(
        fresh.files.len(),
        1,
        "only the staged model, not the texture"
    );
    let prop = file(&fresh, "Assets/Props/SM_Crate.fbx");
    assert_eq!(prop.status, FileStatus::Checked, "{:?}", prop.problem);
    assert!(report(&report_path)["results"][0].get("cached").is_none());

    let second = staged();
    assert_eq!(status(&second), 1);
    assert_eq!(results(&results_path).files, fresh.files);
    assert_eq!(report(&report_path)["results"][0]["cached"], true);
    assert!(
        project.path(".git/asset-check/cache").is_dir(),
        "the cache is in the git folder"
    );
}

/// A commit with no models in it is nothing to check, not a failure: the
/// hook runs on every commit.
#[test]
fn nothing_staged_checks_nothing_and_passes() {
    let project = Project::new("staged-none");
    project.write("Assets/notes.txt", b"not a model");
    project.git(&["add", "Assets"]);
    let results_path = project.path("results.json");
    let output = project.cli(&[
        "audit",
        "--staged",
        "--rules",
        "rules/rules.json",
        "--results",
        text(&results_path),
    ]);
    assert_eq!(status(&output), 0, "{}", stdout(&output));
    assert!(results(&results_path).files.is_empty());
}

/// A rules.json asking for a newer tool says so, as a finding of the run's.
#[test]
fn a_tool_older_than_the_rules_ask_says_what_to_update() {
    let Some(cube) = cube() else { return };
    let project = Project::new("too-old");
    let newer = RULES_JSON.replace(
        r#""mode": "advise","#,
        r#""mode": "advise", "requires": { "3d-review-cli": "99.0.0" },"#,
    );
    project.write("rules/rules.json", newer.as_bytes());
    project.copy(&cube, "Assets/Environment/SM_Rock.fbx");
    let results_path = project.path("results.json");
    let output = project.cli(&[
        "audit",
        "Assets/Environment/SM_Rock.fbx",
        "--rules",
        "rules/rules.json",
        "--results",
        text(&results_path),
    ]);
    assert_eq!(status(&output), 1, "the old tool is a warning");
    let results = results(&results_path);
    assert_eq!(results.findings.len(), 1);
    assert_eq!(results.findings[0].rule, "tool.too_old");
    assert!(results.findings[0].how.contains("99.0.0"));
}

/// `--files-from`: one path per line, in UTF-8 or the UTF-16 Windows
/// PowerShell writes; only models are taken from it, and a list naming none
/// is nothing to do rather than a mistake.
#[test]
fn a_list_names_the_models_one_per_line() {
    let Some(cube) = cube() else { return };
    let folder = scratch("files-from");
    let list = folder.join("list.txt");
    let lines = format!(
        "{}\r\n\r\n{}\r\n",
        text(&cube),
        text(&folder.join("notes.txt"))
    );
    let mut utf16 = vec![0xFF, 0xFE];
    utf16.extend(lines.encode_utf16().flat_map(u16::to_le_bytes));
    std::fs::write(&list, utf16).expect("list");
    let report_path = folder.join("report.json");
    let output = run(&[
        "info",
        "--files-from",
        text(&list),
        "--report",
        text(&report_path),
    ]);
    assert_eq!(
        status(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = report(&report_path);
    assert_eq!(report["models"], 1);
    assert_eq!(report["done"], 1);

    let empty = folder.join("empty.txt");
    std::fs::write(&empty, "\n").expect("list");
    let output = run(&[
        "audit",
        "--files-from",
        text(&empty),
        "--report",
        text(&report_path),
    ]);
    assert_eq!(status(&output), 0);
    assert!(crate::report(&report_path)["problem"].is_null());

    let output = run(&["info", "--files-from", text(&folder.join("missing.txt"))]);
    assert_eq!(status(&output), 1, "a list that is not there is a problem");
}

/// `rules check` lists a configuration's problems and fails on any;
/// `rules explain` says which rules apply to a file, and why.
#[test]
fn the_rules_commands_check_and_explain() {
    let project = Project::new("rules-commands");
    let clean = project.cli(&["rules", "check", "rules/rules.json"]);
    assert_eq!(status(&clean), 0, "{}", stdout(&clean));

    let broken = RULES_JSON.replace("models/environment.json", "models/missing.json");
    project.write("rules/broken.json", broken.as_bytes());
    let output = project.cli(&["rules", "check", "rules/broken.json"]);
    assert_eq!(status(&output), 1);
    assert!(
        stdout(&output).contains("missing.json"),
        "{}",
        stdout(&output)
    );

    let output = project.cli(&[
        "rules",
        "explain",
        "Assets/Environment/SM_Rock.fbx",
        "--rules",
        "rules/rules.json",
    ]);
    assert_eq!(status(&output), 0);
    let explained = stdout(&output);
    assert!(explained.contains("environment-prop"), "{explained}");
    assert!(explained.contains("production"), "{explained}");

    let output = project.cli(&["rules", "check", "rules/nowhere.json"]);
    assert_eq!(status(&output), 1);

    // The model rules files are read against 3d-review-cli's catalog: a
    // misspelt rule is an error for rules check, and a warning for audit,
    // which passes it over.
    project.write(
        "rules/models/environment.json",
        br#"{ "version": 1, "profile": { "name": "Environment", "rules": {
              "geometry.ngon": { "severity": "error" } } } }"#,
    );
    let output = project.cli(&["rules", "check", "rules/rules.json"]);
    assert_eq!(status(&output), 1);
    assert!(
        stdout(&output).contains(
            r#"error: models/environment.json rules["geometry.ngon"]: 3d-review-cli has no rule "geometry.ngon""#
        ),
        "{}",
        stdout(&output)
    );
    if let Some(cube) = cube() {
        project.copy(&cube, "Assets/Environment/SM_Rock.fbx");
        let output = project.cli(&[
            "audit",
            "Assets/Environment/SM_Rock.fbx",
            "--rules",
            "rules/rules.json",
            "--fail-on",
            "never",
        ]);
        let said = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(said.contains("passes over what it can't use"), "{said}");
        assert!(said.contains("geometry.ngon"), "{said}");
    }

    // Every rule it checks, for Rule Editor.
    let output = project.cli(&["rules", "catalog"]);
    assert_eq!(status(&output), 0);
    let catalog = asset_rules::catalog::Catalog::from_json(&stdout(&output)).expect("its JSON");
    assert_eq!(catalog.tool, "3d-review-cli");
    assert_eq!(catalog.rules.len(), 43);
    assert_eq!(catalog.bases, ["unity", "unreal", "generic"]);
    assert!(catalog.check(catalog.version.as_deref()).is_empty());
}
