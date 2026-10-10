//! The audit over the repository's real FBX fixtures: every rule runs on real
//! data without panicking, the result does not depend on the thread count, and
//! a re-run that reuses the previous report equals a cold one.
//!
//! Each test skips itself when its fixture is absent or the build carries no
//! FBX importer — unless `REVIEW_REQUIRE_FIXTURES` is set, as `scripts/check`
//! does, which turns a skip into a failure.

use std::path::PathBuf;

use review_audit::{AuditInput, AuditProfile, AuditReport, Engine, RunOptions, Status, run};
use review_model::ModelData;
use review_model::extras::SourceExtras;

const FIXTURES: &[&str] = &[
    "meter_cube.fbx",
    "monkey.fbx",
    "SM_column04.fbx",
    "SM_Ammo_Crate_01a.fbx",
    "SK_Player_01.fbx",
    "xyzrgb_dragon.fbx",
];

fn skipping(reason: impl std::fmt::Display) {
    assert!(
        !std::env::var_os("REVIEW_REQUIRE_FIXTURES").is_some_and(|value| value != "0"),
        "REVIEW_REQUIRE_FIXTURES is set, so this run may not skip: {reason}"
    );
    eprintln!("skipping: {reason}");
}

fn fixture(name: &str) -> Option<(ModelData, SourceExtras)> {
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
    for name in FIXTURES {
        let Some((model, extras)) = fixture(name) else {
            continue;
        };
        for engine in Engine::ALL {
            let report = audit(&model, &extras, &AuditProfile::builtin(engine), 4);
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

#[test]
fn the_report_does_not_depend_on_the_thread_count() {
    for name in ["SK_Player_01.fbx", "SM_Ammo_Crate_01a.fbx"] {
        let Some((model, extras)) = fixture(name) else {
            continue;
        };
        let profile = AuditProfile::builtin(Engine::Unreal);
        assert_eq!(
            content(audit(&model, &extras, &profile, 1)),
            content(audit(&model, &extras, &profile, 8)),
            "{name}"
        );
    }
}

#[test]
fn reusing_the_previous_report_equals_a_cold_run() {
    let Some((model, extras)) = fixture("SK_Player_01.fbx") else {
        return;
    };
    let input = AuditInput {
        model: &model,
        extras: Some(&extras),
    };
    let first = AuditProfile::builtin(Engine::Unity);
    let previous = run(input, &first, None, RunOptions::default()).unwrap();
    let mut edited = first.clone();
    let config = edited
        .rules
        .get_mut(&review_audit::RuleId::Influences)
        .unwrap();
    config
        .params
        .insert("max".into(), review_audit::ParamValue::Number(2.0));
    config.severity = review_audit::Severity::Error;
    edited
        .rules
        .get_mut(&review_audit::RuleId::Ngons)
        .unwrap()
        .severity = review_audit::Severity::Info;
    let reused = run(input, &edited, Some(&previous), RunOptions::default()).unwrap();
    let cold = run(input, &edited, None, RunOptions::default()).unwrap();
    assert_eq!(content(reused), content(cold));
}

/// What the Generic profile finds on each fixture: every failing check and
/// its exact total. A change to a check's behaviour on real data shows here
/// as a diff to read, rather than slipping through a test that only asks
/// whether the rules ran.
#[test]
fn the_findings_on_the_fixtures_are_pinned() {
    const PINNED: &[(&str, &str)] = &[
        ("meter_cube.fbx", "geometry.hard_edges(12)"),
        (
            "monkey.fbx",
            "geometry.duplicate_vertices(4) geometry.hard_edges(961) transform.scale(1) \
             transform.unfrozen(1) uv.lightmap_overlap(8) density.triangle_lod(1)",
        ),
        (
            "SM_column04.fbx",
            "transform.unfrozen(1) uv.overlap(10) uv.flipped(15) density.triangle_lod(1)",
        ),
        (
            "SM_Ammo_Crate_01a.fbx",
            "geometry.duplicate_vertices(6542) geometry.inverted_normals(2) \
             uv.out_of_range(10) uv.overlap(7660) uv.flipped(19) uv.lightmap_overlap(25) \
             density.triangle_lod(3) hierarchy.empty_nodes(1)",
        ),
        (
            "SK_Player_01.fbx",
            "geometry.non_manifold_edges(4) geometry.duplicate_vertices(237) geometry.ngons(3) \
             geometry.inverted_normals(11) transform.unfrozen(1) transform.pivot_offset(1) \
             uv.missing(2) uv.out_of_range(2737) uv.overlap(121075) uv.flipped(18437) \
             uv.lightmap_overlap(10799) skin.unused_bones(2) density.texel(3) \
             density.triangle_lod(2) density.triangle_reduce(1) hierarchy.multiple_roots(2)",
        ),
        (
            "xyzrgb_dragon.fbx",
            "geometry.non_manifold_edges(1) geometry.isolated_vertices(123) \
             geometry.hard_edges(357454) uv.missing(1) density.triangle_reduce(1) \
             naming.invalid_characters(1)",
        ),
    ];
    for (name, expected) in PINNED {
        let Some((model, extras)) = fixture(name) else {
            continue;
        };
        let report = audit(&model, &extras, &AuditProfile::builtin(Engine::Generic), 4);
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
