//! The audit over the repository's real FBX fixtures: every rule runs on real
//! data without panicking, the result does not depend on the thread count, and
//! a re-run that reuses the previous report equals a cold one.
//!
//! Each fixture is imported once per run and shared by every test, since the
//! 614k-triangle Benk scan takes seconds to import in a debug build.
//!
//! Each test skips itself when its fixture is absent or the build carries no
//! FBX importer — unless `REVIEW_REQUIRE_FIXTURES` is set, as `scripts/check`
//! does, which turns a skip into a failure.

use std::path::PathBuf;
use std::sync::OnceLock;

use review_audit::{AuditInput, AuditProfile, AuditReport, Engine, RunOptions, Status, run};
use review_model::ModelData;
use review_model::extras::SourceExtras;

const FIXTURES: &[&str] = &[
    "meter_cube.fbx",
    "monkey.fbx",
    "SM_Wall_Break_4x3m.fbx",
    "Triceratops.fbx",
    "Johannes_Benk_C.fbx",
];

/// The 614k-triangle scan. An audit of it takes ~8 s in a debug build, so it runs
/// once, in [`the_findings_on_the_fixtures_are_pinned`] - which runs every Generic
/// rule on it - and not once per engine or thread count: it is one mesh, so the
/// per-object work has nothing to split across threads.
const SCAN: &str = "Johannes_Benk_C.fbx";

fn skipping(reason: impl std::fmt::Display) {
    assert!(
        !std::env::var_os("REVIEW_REQUIRE_FIXTURES").is_some_and(|value| value != "0"),
        "REVIEW_REQUIRE_FIXTURES is set, so this run may not skip: {reason}"
    );
    eprintln!("skipping: {reason}");
}

/// A fixture with its capture, imported on first use and shared after it.
fn fixture(name: &str) -> Option<&'static (ModelData, SourceExtras)> {
    static LOADED: [OnceLock<Option<(ModelData, SourceExtras)>>; FIXTURES.len()] =
        [const { OnceLock::new() }; FIXTURES.len()];
    let slot = FIXTURES
        .iter()
        .position(|fixture| *fixture == name)
        .unwrap_or_else(|| panic!("{name} is not in FIXTURES"));
    LOADED[slot].get_or_init(|| load(name)).as_ref()
}

fn load(name: &str) -> Option<(ModelData, SourceExtras)> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/test_models")
        .join(name);
    if !path.exists() {
        skipping(format!("{name} is not present"));
        return None;
    }
    match review_import::load_model_full(&path) {
        Ok((model, Some(extras))) => Some((model, extras)),
        Ok((_, None)) => {
            skipping(format!("{name}: no capture in this build"));
            None
        }
        Err(review_import::ImportError::UfbxUnavailable) => {
            skipping("FBX import is unavailable");
            None
        }
        Err(error) => panic!("{name} is present but failed to load: {error}"),
    }
}

fn audit(
    model: &ModelData,
    extras: &SourceExtras,
    profile: &AuditProfile,
    threads: usize,
) -> AuditReport {
    run(
        AuditInput {
            model,
            extras: Some(extras),
        },
        profile,
        None,
        RunOptions {
            cancel: None,
            threads,
        },
    )
    .expect("an uncancelled run completes")
}

/// Strip the timing so two reports compare on content.
fn content(mut report: AuditReport) -> AuditReport {
    report.elapsed_ms = 0.0;
    report
}

#[test]
fn every_rule_runs_on_every_fixture() {
    for name in FIXTURES.iter().filter(|name| **name != SCAN) {
        let Some((model, extras)) = fixture(name) else {
            continue;
        };
        for engine in Engine::ALL {
            let report = audit(model, extras, &AuditProfile::builtin(engine), 4);
            assert_eq!(report.results.len(), review_audit::RuleId::ALL.len());
            let failing: Vec<String> = report
                .results
                .iter()
                .filter(|result| result.status == Status::Fail)
                .map(|result| format!("{}({})", result.rule.as_str(), result.total))
                .collect();
            eprintln!(
                "{name} [{}] {:.0} ms, {} B: {}",
                engine.as_str(),
                report.elapsed_ms,
                report.heap_bytes(),
                failing.join(" ")
            );
        }
    }
}

/// The wall is 94 separate objects, so the per-object checks genuinely split
/// across threads there; the triceratops adds the skin checks, and the rest are
/// a single object each.
#[test]
fn the_report_does_not_depend_on_the_thread_count() {
    for name in FIXTURES.iter().filter(|name| **name != SCAN) {
        let Some((model, extras)) = fixture(name) else {
            continue;
        };
        let profile = AuditProfile::builtin(Engine::Unreal);
        assert_eq!(
            content(audit(model, extras, &profile, 1)),
            content(audit(model, extras, &profile, 8)),
            "{name}"
        );
    }
}

/// Run on a static mesh and on the skinned triceratops, so the influence-limit
/// edit below is one rule that cannot run in the first case and one that
/// re-measures a real skin in the second.
#[test]
fn reusing_the_previous_report_equals_a_cold_run() {
    for name in ["monkey.fbx", "Triceratops.fbx"] {
        let Some((model, extras)) = fixture(name) else {
            continue;
        };
        reuse_equals_cold(name, model, extras);
    }
}

fn reuse_equals_cold(name: &str, model: &ModelData, extras: &SourceExtras) {
    let input = AuditInput {
        model,
        extras: Some(extras),
    };
    let first = AuditProfile::builtin(Engine::Unity);
    let previous = run(input, &first, None, RunOptions::default()).unwrap();
    let mut edited = first.clone();
    // A threshold edit, which re-measures its rule...
    let config = edited
        .rules
        .get_mut(&review_audit::RuleId::HardEdges)
        .unwrap();
    config
        .params
        .insert("max_ratio".into(), review_audit::ParamValue::Number(0.05));
    config.severity = review_audit::Severity::Error;
    // ...one on the skin's influence limit (the monkey has no skin to run it
    // on; the triceratops has up to several influences per vertex)...
    edited
        .rules
        .get_mut(&review_audit::RuleId::Influences)
        .unwrap()
        .params
        .insert("max".into(), review_audit::ParamValue::Number(2.0));
    // ...and a severity edit, which only relabels the kept result.
    edited
        .rules
        .get_mut(&review_audit::RuleId::UvOverlap)
        .unwrap()
        .severity = review_audit::Severity::Warning;
    let reused = run(input, &edited, Some(&previous), RunOptions::default()).unwrap();
    let cold = run(input, &edited, None, RunOptions::default()).unwrap();
    assert_ne!(
        content(previous),
        content(cold.clone()),
        "{name}: the edits change the report, so equality below means something"
    );
    assert_eq!(content(reused), content(cold), "{name}");
}

/// What the Generic profile finds on each fixture: every failing check and
/// its exact total. A change to a check's behaviour on real data shows here
/// as a diff to read, rather than slipping through a test that only asks
/// whether the rules ran.
#[test]
fn the_findings_on_the_fixtures_are_pinned() {
    const PINNED: &[(&str, &str)] = &[
        ("meter_cube.fbx", "geometry.hard_edges(12) density.texel(1)"),
        (
            "monkey.fbx",
            "geometry.duplicate_vertices(4) geometry.hard_edges(961) transform.scale(1) \
             transform.unfrozen(1) uv.lightmap_overlap(8) density.texel(1) \
             density.triangle_lod(1)",
        ),
        (
            "SM_Wall_Break_4x3m.fbx",
            "geometry.missing_normals(94) geometry.hard_edges(1128) \
             geometry.draw_call_budget(1) transform.pivot_offset(84) uv.overlap(1128) \
             density.texel(94) hierarchy.multiple_roots(94)",
        ),
        (
            "Triceratops.fbx",
            "geometry.inverted_normals(5) transform.scale(2) transform.unfrozen(1) \
             uv.overlap(6) skin.influences(13) skin.unnormalized_weights(4987) \
             skin.bones_per_mesh(1) skin.unused_bones(35) skin.bind_pose_mismatch(109) \
             density.texel(1) density.triangle_lod(1) hierarchy.multiple_roots(2)",
        ),
        (
            SCAN,
            "geometry.inverted_normals(151) geometry.triangle_budget(1) \
             transform.unfrozen(1) uv.overlap(117) uv.flipped(583487) density.texel(1) \
             density.triangle_reduce(1)",
        ),
    ];
    for (name, expected) in PINNED {
        let Some((model, extras)) = fixture(name) else {
            continue;
        };
        let report = audit(model, extras, &AuditProfile::builtin(Engine::Generic), 4);
        let found: Vec<String> = report
            .results
            .iter()
            .filter(|result| result.status == Status::Fail)
            .map(|result| format!("{}({})", result.rule.as_str(), result.total))
            .collect();
        let expected: Vec<&str> = expected.split_whitespace().collect();
        assert_eq!(found, expected, "{name}");
    }
}
