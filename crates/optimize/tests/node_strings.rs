//! The export's user-string override ([`NodeStrings`]) — how the viewer's review
//! comments ride along into an exported LOD chain. Checked by reading the written
//! file back through ufbx's own property capture.

#![cfg(all(has_meshopt, has_ufbxw))]

use std::collections::HashMap;

use review_model::extras::{Prop, PropFlags, PropType, Synthetic};
use review_optimize::{
    ExportOptions, LodLevel, LodPackaging, LodParams, NodeStrings, OpKind, OptStack,
    SimplifyAlgorithm, SimplifySettings, WeldParams, export_fbx_with,
};

mod common;

use common::{fixture_full, reload_full, run_with_extras, temp_dir};

const PROPERTY: &str = "ReviewComments";

/// A two-level chain, so the file holds the source nodes and their `_LOD1` copies.
fn chain() -> OptStack {
    let mut stack = OptStack::default();
    stack.push_op(OpKind::Weld(WeldParams::default()));
    stack.push_op(OpKind::SimplifyLod(LodParams {
        simplify: SimplifySettings {
            algorithm: SimplifyAlgorithm::Standard,
            ..SimplifySettings::default()
        },
        levels: vec![LodLevel {
            target_ratio: 0.5,
            target_error: 0.05,
        }],
    }));
    stack
}

/// The override replaces what the capture held on a source node, and nothing
/// else carries it: not the node's `_LOD1` copy, not a node with no value.
#[test]
fn comments_ride_on_the_source_node_and_not_on_its_lod_copies() {
    let Some((model, mut extras)) = fixture_full("SM_Ammo_Crate_01a.fbx") else {
        return;
    };
    // The first mesh node, which gets a stale comment in the capture (what the
    // file held when it was opened) and a new one in the override.
    let (mesh, _) = model
        .nodes
        .iter()
        .enumerate()
        .find(|(index, node)| {
            node.mesh_part.is_some() && extras.nodes[*index].synthetic == Synthetic::None
        })
        .expect("a mesh node");
    extras.nodes[mesh].props.push(Prop {
        name: PROPERTY.to_owned(),
        kind: PropType::Text,
        flags: PropFlags(PropFlags::USER_DEFINED | PropFlags::VALUE_STR),
        value_int: 0,
        value_real: [0.0; 4],
        value_str: "stale".to_owned(),
        value_blob: Vec::new(),
    });

    let result = run_with_extras(&model, &extras, &chain());
    assert!(result.lods.len() > 1, "the stack makes a chain");
    let strings = NodeStrings {
        name: PROPERTY.to_owned(),
        values: HashMap::from([(mesh, "{\"v\":1,\"threads\":[]}".to_owned())]),
    };
    let dir = temp_dir("node_strings");
    let path = dir.join("crate.fbx");
    let options = ExportOptions {
        packaging: LodPackaging::SingleFileSuffixed,
        ..ExportOptions::default()
    };
    export_fbx_with(
        &result.lods,
        &model,
        Some(&extras),
        &path,
        &options,
        Some(&strings),
    )
    .expect("export succeeds");

    let (loaded, loaded_extras) = reload_full(&path);
    let carrying: Vec<(&str, &str)> = loaded
        .nodes
        .iter()
        .zip(&loaded_extras.nodes)
        .flat_map(|(node, extra)| {
            extra
                .props
                .iter()
                .filter(|prop| prop.name == PROPERTY)
                .map(move |prop| (node.name.as_str(), prop.value_str.as_str()))
        })
        .collect();
    let source_name = model.nodes[mesh].name.as_str();
    assert_eq!(
        carrying,
        [(source_name, "{\"v\":1,\"threads\":[]}")],
        "exactly the source node carries the new value"
    );
    let user_defined = loaded_extras
        .nodes
        .iter()
        .flat_map(|extra| &extra.props)
        .find(|prop| prop.name == PROPERTY)
        .is_some_and(|prop| prop.user_defined());
    assert!(user_defined, "written as a user property");
}
