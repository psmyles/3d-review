//! The console tool as a caller sees it: the binary run with a command line,
//! judged by its exit status, what it prints and the report it writes.
//!
//! This is the contract Launcher and any script are written against
//! (`docs/cli.md`), so each test pins one clause of it. The model tests skip
//! themselves when the fixture is absent; everything else needs nothing.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

const BINARY: &str = env!("CARGO_BIN_EXE_review-cli");

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
    // One an earlier run left: after any run, the file is this run's or is
    // not there, and a refused line writes none.
    std::fs::write(&report_path, "{}").expect("a stale report");
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
}

#[test]
fn version_and_help_exit_0() {
    let version = run(&["--version"]);
    assert_eq!(status(&version), 0);
    assert!(stdout(&version).starts_with("review-cli "));
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
