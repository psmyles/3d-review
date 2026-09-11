//! Helpers for the `extras_*` suites: the export-and-read-back round trip and
//! the property comparisons every group of them makes.
//!
//! The capture is checked in four suites (nodes/materials, topology, deform,
//! animation) because one file of it ran past a thousand lines; the scaffolding
//! they share lives here rather than in whichever of them happened to be first.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use review_model::extras::{Prop, PropType};
use review_model::{ModelData, SourceExtras};
use review_optimize::{
    ExportOptions, FbxFormat, HierarchyMode, OpKind, OptStack, ProcessInput, ProcessedResult,
    export_fbx, process,
};

use super::VERTEX_SIZE;
use super::reload_full as reload;

pub fn passthrough(model: &ModelData, extras: &SourceExtras) -> ProcessedResult {
    let mut stack = OptStack::default();
    stack.push_op(OpKind::FilterTriangles);
    process(ProcessInput {
        model,
        stack: &stack,
        render_vertex_size: VERTEX_SIZE,
        hidden_nodes: &[],
        extras: Some(extras),
    })
    .expect("processing succeeds")
}

/// Export the passthrough of a fixture with its capture and read it back.
pub fn round_trip(
    model: &ModelData,
    extras: &SourceExtras,
    dir: &Path,
    format: FbxFormat,
) -> (PathBuf, ModelData, SourceExtras) {
    let result = passthrough(model, extras);
    let path = dir.join(match format {
        FbxFormat::Binary => "round_trip.fbx",
        FbxFormat::Ascii => "round_trip_ascii.fbx",
    });
    let report = export_fbx(
        &result.lods,
        model,
        Some(extras),
        &path,
        &ExportOptions {
            format,
            hierarchy: HierarchyMode::Rebuild,
            ..ExportOptions::default()
        },
    )
    .expect("export succeeds");
    assert!(
        !report
            .notes
            .iter()
            .any(|note| note.contains("had not finished loading")),
        "the capture was used: {:?}",
        report.notes
    );
    let (loaded, loaded_extras) = reload(&path);
    (path, loaded, loaded_extras)
}

pub fn node_by_name<'a>(
    model: &'a ModelData,
    name: &str,
) -> Option<(usize, &'a review_model::SceneNode)> {
    model
        .nodes
        .iter()
        .enumerate()
        .find(|(_, node)| node.name == name)
}

pub fn prop<'a>(props: &'a [Prop], name: &str) -> &'a Prop {
    props
        .iter()
        .find(|prop| prop.name == name)
        .unwrap_or_else(|| {
            panic!(
                "no property {name:?} in {:?}",
                props.iter().map(|p| &p.name).collect::<Vec<_>>()
            )
        })
}

pub fn assert_prop_eq(source: &Prop, loaded: &Prop, context: &str) {
    assert_eq!(source.name, loaded.name, "{context}");
    let reals = source.flags.real_count();
    for component in 0..reals {
        assert!(
            (source.value_real[component] - loaded.value_real[component]).abs() < 1e-6,
            "{context} {}[{component}]: {} vs {}",
            source.name,
            source.value_real[component],
            loaded.value_real[component]
        );
    }
    if source.flags.has_int() && !source.flags.has_string() {
        assert_eq!(
            source.value_int, loaded.value_int,
            "{context} {}",
            source.name
        );
    }
    if source.flags.has_string() && source.kind == PropType::Text {
        assert_eq!(
            source.value_str, loaded.value_str,
            "{context} {}",
            source.name
        );
    }
    assert_eq!(
        source.flags.user_defined(),
        loaded.flags.user_defined(),
        "{context} {} user flag",
        source.name
    );
}

/// Every authored property of `source` is present in `loaded` with the same
/// value. Compound headers have no value and are skipped.
pub fn assert_props_survive(source: &[Prop], loaded: &[Prop], context: &str) {
    for authored in source {
        if authored.kind == PropType::Compound || !authored.flags.has_value() {
            continue;
        }
        let Some(back) = loaded.iter().find(|prop| prop.name == authored.name) else {
            panic!(
                "{context}: property {:?} was not written back",
                authored.name
            );
        };
        assert_prop_eq(authored, back, context);
    }
}

/// Export one processed result with the capture and read it back.
pub fn export_result(
    result: &ProcessedResult,
    model: &ModelData,
    extras: &SourceExtras,
    path: &Path,
) -> (ModelData, SourceExtras) {
    export_fbx(
        &result.lods,
        model,
        Some(extras),
        path,
        &ExportOptions::default(),
    )
    .expect("export");
    reload(path)
}

pub fn assert_matrix_close(a: glam::Mat4, b: glam::Mat4, tolerance: f32, context: &str) {
    let delta = (a - b).abs();
    let max = delta.to_cols_array().into_iter().fold(0f32, f32::max);
    assert!(
        max < tolerance,
        "{context}: drifts by {max}\n{a:?}\nvs\n{b:?}"
    );
}
