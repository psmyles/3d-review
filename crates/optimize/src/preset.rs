//! Saving and loading an operation stack as JSON.
//!
//! The file is a versioned envelope rather than a bare [`OptStack`] so a stack
//! written by a future build can be *rejected with an explanation* instead of
//! silently deserializing into something half-understood.
//!
//! File IO lives with the caller (`app` owns the file dialogs); this module
//! only converts between a stack and a string.

use serde::{Deserialize, Serialize};

use crate::OptError;
use crate::stack::OptStack;

/// Envelope version this build writes and is willing to read.
pub const PRESET_VERSION: u32 = 1;

/// Conventional file extension for a saved stack.
pub const PRESET_EXTENSION: &str = "json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Preset {
    version: u32,
    /// Free-text provenance, written for a human opening the file in an editor
    /// and ignored on load.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    generator: String,
    stack: OptStack,
}

/// Serialize `stack` as a pretty-printed preset document.
pub fn to_json(stack: &OptStack) -> Result<String, OptError> {
    let preset = Preset {
        version: PRESET_VERSION,
        generator: format!("3D Review optimize v{}", env!("CARGO_PKG_VERSION")),
        stack: stack.clone(),
    };
    serde_json::to_string_pretty(&preset).map_err(|error| OptError::Preset(error.to_string()))
}

/// Parse a preset document.
///
/// Operation ids are re-keyed from a fresh sequence on the way out: the loaded
/// stack replaces one whose ids the UI may still be holding, and a collision
/// would silently point a selection or an override at the wrong operation.
pub fn from_json(json: &str) -> Result<OptStack, OptError> {
    let preset: Preset =
        serde_json::from_str(json).map_err(|error| OptError::Preset(error.to_string()))?;

    if preset.version > PRESET_VERSION {
        return Err(OptError::PresetVersion {
            found: preset.version,
            supported: PRESET_VERSION,
        });
    }

    let mut stack = preset.stack;
    stack.reassign_ids();
    stack.prune_overrides();
    Ok(stack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stack::{
        AoQuality, AoTarget, BakeAoParams, LodParams, OpKind, ReduceParams, SimplifyAlgorithm,
        WeldParams,
    };

    #[test]
    fn a_stack_round_trips() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::Weld(WeldParams {
            attribute_tolerance: 0.25,
            compare_normals: false,
            compare_uvs: true,
            compare_colors: true,
        }));
        stack.push_op(OpKind::SimplifyLod(LodParams::default()));
        stack.push_op(OpKind::Overdraw { threshold: 1.2 });
        stack.node_override_mut(2).exclude = true;
        stack.export.format = crate::stack::FbxFormat::Ascii;

        let json = to_json(&stack).expect("serializes");
        let loaded = from_json(&json).expect("parses");

        assert_eq!(loaded.ops.len(), stack.ops.len());
        assert_eq!(loaded.ops[0].kind, stack.ops[0].kind);
        assert_eq!(loaded.export, stack.export);
        assert_eq!(loaded.overrides, stack.overrides);
    }

    #[test]
    fn a_reduce_operation_round_trips() {
        let mut stack = OptStack::default();
        let mut reduce = ReduceParams::default();
        reduce.simplify.algorithm = SimplifyAlgorithm::Sloppy;
        reduce.target.target_ratio = 0.3;
        stack.push_op(OpKind::Reduce(reduce));

        let loaded = from_json(&to_json(&stack).expect("serializes")).expect("parses");
        assert_eq!(loaded.ops[0].kind, OpKind::Reduce(reduce));
    }

    #[test]
    fn a_bake_ao_operation_round_trips() {
        let mut stack = OptStack::default();
        let params = BakeAoParams {
            quality: AoQuality::Ultra,
            max_distance: 2.5,
            intensity: 1.5,
            target: AoTarget::MultiplyRgb,
            srgb: true,
        };
        stack.push_op(OpKind::BakeAo(params));

        let json = to_json(&stack).expect("serializes");
        assert!(
            json.contains("\"BakeAo\""),
            "the wire name is pinned — renaming the variant breaks saved presets: {json}"
        );

        let loaded = from_json(&json).expect("parses");
        assert_eq!(loaded.ops[0].kind, OpKind::BakeAo(params));
    }

    /// The simplifier settings live in their own struct now but are flattened on
    /// the wire, so a preset written before that split still loads — and a new one
    /// still names them at the operation's top level.
    #[test]
    fn simplifier_settings_stay_flat_on_disk() {
        let json = r#"{
            "version": 1,
            "stack": {
                "ops": [{
                    "id": 7,
                    "enabled": true,
                    "kind": {
                        "SimplifyLod": {
                            "algorithm": "Sloppy",
                            "levels": [{ "target_ratio": 0.25, "target_error": 0.5 }]
                        }
                    }
                }]
            }
        }"#;

        let loaded = from_json(json).expect("an older preset still parses");
        let OpKind::SimplifyLod(params) = &loaded.ops[0].kind else {
            panic!(
                "the operation survives as a LOD op: {:?}",
                loaded.ops[0].kind
            );
        };
        assert_eq!(params.simplify.algorithm, SimplifyAlgorithm::Sloppy);
        assert_eq!(params.levels.len(), 1);
        assert_eq!(params.levels[0].target_ratio, 0.25);

        let written = to_json(&loaded).expect("serializes");
        assert!(
            written.contains("\"algorithm\""),
            "the settings are written at the operation's top level: {written}"
        );
    }

    #[test]
    fn loading_re_keys_operation_ids_and_their_overrides() {
        let mut stack = OptStack::default();
        // Push and remove so the surviving operation has a non-1 id, making a
        // failure to re-key visible.
        stack.push_op(OpKind::FilterTriangles);
        stack.push_op(OpKind::FilterTriangles);
        let id = stack.push_op(OpKind::VertexCache);
        stack.remove_op(1);
        stack
            .node_override_mut(0)
            .ops
            .push(crate::stack::OpInstance {
                id,
                enabled: true,
                kind: OpKind::VertexCache,
            });

        let loaded = from_json(&to_json(&stack).expect("serializes")).expect("parses");

        let new_id = loaded
            .ops
            .iter()
            .find(|op| op.kind == OpKind::VertexCache)
            .expect("the operation survives")
            .id;
        assert_eq!(
            loaded.overrides[0].ops[0].id, new_id,
            "an override follows its operation's new id"
        );
    }

    #[test]
    fn a_newer_preset_version_is_refused_with_both_numbers() {
        let json = r#"{"version": 99, "stack": {}}"#;
        match from_json(json) {
            Err(OptError::PresetVersion { found, supported }) => {
                assert_eq!(found, 99);
                assert_eq!(supported, PRESET_VERSION);
            }
            other => panic!("expected a version error, got {other:?}"),
        }
    }

    #[test]
    fn malformed_json_reports_a_preset_error() {
        assert!(matches!(from_json("not json"), Err(OptError::Preset(_))));
    }

    #[test]
    fn an_overrides_object_name_survives_the_round_trip() {
        let mut stack = OptStack::default();
        stack.node_override_mut(2).exclude = true;
        stack.stamp_node_names(&[
            review_model::SceneNode::default(),
            review_model::SceneNode::default(),
            review_model::SceneNode {
                name: "hero_prop".to_owned(),
                ..review_model::SceneNode::default()
            },
        ]);

        let loaded = from_json(&to_json(&stack).expect("serializes")).expect("parses");
        assert_eq!(
            loaded.overrides[0].name, "hero_prop",
            "the name is what rebinds the override to another model"
        );
    }

    #[test]
    fn an_override_written_before_object_names_still_loads() {
        // Exactly what a preset from an older build deserializes to: a `node`
        // index and no `name`. `#[serde(default)]` is what keeps it readable.
        let json = r#"{"version": 1, "stack": {"overrides": [{"node": 3, "exclude": true}]}}"#;
        let stack = from_json(json).expect("parses");
        assert_eq!(stack.overrides.len(), 1);
        assert_eq!(stack.overrides[0].node, 3);
        assert!(stack.overrides[0].name.is_empty());
    }

    #[test]
    fn a_minimal_document_loads_as_an_empty_stack() {
        // Every stack field carries `#[serde(default)]`, so a hand-written
        // preset need only name the version.
        let stack = from_json(r#"{"version": 1, "stack": {}}"#).expect("parses");
        assert!(stack.ops.is_empty());
        assert_eq!(stack.export, crate::stack::ExportOptions::default());
    }
}
