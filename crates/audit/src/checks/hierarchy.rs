//! Hierarchy and scene content: empty groups, duplicate names, several
//! top-level objects, stray lights and cameras, and material slots per mesh.

use std::collections::{BTreeMap, BTreeSet};

use review_model::NodeKind;

use super::{Outcome, count_param, largest};
use crate::context::Context;
use crate::finding::{ElementSet, Measured, Offender, Skip, Threshold};
use crate::glob::GlobSet;
use crate::profile::RuleConfig;

fn point_offender(ctx: &Context<'_>, index: usize, measured: Option<Measured>) -> Offender {
    Offender {
        node: Some(index as u32),
        count: 1,
        measured,
        detail: None,
        elements: ElementSet::Points(vec![(ctx.node_position(index).to_array(), index as u32)]),
    }
}

/// Empty nodes with nothing that renders or deforms below them.
pub(super) fn empty_nodes(ctx: &Context<'_>, config: &RuleConfig) -> Outcome {
    let Ok(ignore) = GlobSet::new(config.patterns("ignore")) else {
        return Outcome::skip(Skip::InvalidPattern);
    };
    let model = ctx.model;
    let mut useful = vec![false; model.nodes.len()];
    for index in 0..model.nodes.len() {
        if !matches!(
            ctx.kind(index),
            NodeKind::Mesh | NodeKind::Bone | NodeKind::Light | NodeKind::Camera
        ) {
            continue;
        }
        let mut cursor = Some(index);
        let mut steps = 0;
        while let Some(at) = cursor {
            if useful[at] && at != index {
                break;
            }
            useful[at] = true;
            cursor = model.nodes[at].parent;
            steps += 1;
            if steps > model.nodes.len() {
                break;
            }
        }
    }
    let offenders = ctx
        .authored_nodes()
        .filter(|&index| {
            ctx.kind(index) == NodeKind::Empty
                && !useful[index]
                && !ignore.matches(&model.nodes[index].name)
        })
        .map(|index| point_offender(ctx, index, None))
        .collect();
    Outcome::judged(offenders, Threshold::None)
}

pub(super) fn duplicate_names(ctx: &Context<'_>) -> Outcome {
    let model = ctx.model;
    let mut by_name: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for index in ctx.authored_nodes() {
        let name = model.nodes[index].name.as_str();
        if !name.is_empty() {
            by_name.entry(name).or_default().push(index);
        }
    }
    let mut offenders: Vec<Offender> = by_name
        .into_iter()
        .filter(|(_, nodes)| nodes.len() > 1)
        .flat_map(|(name, nodes)| {
            nodes
                .into_iter()
                .map(move |index| Offender::node(index, Some(Measured::Text(name.to_owned()))))
        })
        .collect();
    offenders.sort_by_key(|offender| offender.node);
    Outcome::judged(offenders, Threshold::None)
}

pub(super) fn multiple_roots(ctx: &Context<'_>) -> Outcome {
    let roots = review_model::hierarchy::authored_roots(ctx.model, ctx.extras);
    let count = roots.len() as u64;
    let offenders = if roots.len() > 1 {
        roots
            .into_iter()
            .map(|index| Offender::node(index, None))
            .collect()
    } else {
        Vec::new()
    };
    Outcome::judged(offenders, Threshold::Max(Measured::Count(1)))
        .with_measured(Some(Measured::Count(count)))
}

pub(super) fn lights_cameras(ctx: &Context<'_>) -> Outcome {
    let offenders = ctx
        .authored_nodes()
        .filter(|&index| matches!(ctx.kind(index), NodeKind::Light | NodeKind::Camera))
        .map(|index| point_offender(ctx, index, None))
        .collect();
    Outcome::judged(offenders, Threshold::None)
}

pub(super) fn materials_per_mesh(ctx: &Context<'_>, config: &RuleConfig) -> Outcome {
    let max = count_param(config, "max", 4);
    let materials = &ctx.model.triangles.material;
    let offenders: Vec<Offender> = ctx
        .node_triangles()
        .iter()
        .enumerate()
        .filter_map(|(node, triangles)| {
            let slots: BTreeSet<u32> = triangles
                .iter()
                .filter_map(|&t| materials.get(t as usize).copied())
                .collect();
            (slots.len() as u64 > max)
                .then(|| Offender::node(node, Some(Measured::Count(slots.len() as u64))))
        })
        .collect();
    let measured = largest(&offenders, |m| match m {
        Measured::Count(c) => Some(*c as f64),
        _ => None,
    });
    Outcome::judged(offenders, Threshold::Max(Measured::Count(max))).with_measured(measured)
}

/// The kind of value an extra holds, as a `key:type` entry names it.
fn json_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// The same for an FBX user property, by its declared type.
fn prop_kind(kind: review_model::extras::PropType) -> &'static str {
    use review_model::extras::PropType;
    match kind {
        PropType::Boolean => "bool",
        PropType::Integer | PropType::Number | PropType::Distance => "number",
        PropType::Text | PropType::DateTime | PropType::Reference => "string",
        PropType::Vector
        | PropType::Color
        | PropType::ColorWithAlpha
        | PropType::Translation
        | PropType::Rotation
        | PropType::Scaling => "array",
        PropType::Compound => "object",
        _ => "unknown",
    }
}

/// Objects matching `objects` that lack a required extra, or carry it as the
/// wrong type. An extra is a glTF node's `extras` entry, or an FBX node's user
/// property (a custom property, as Blender and Maya export them); a model with
/// neither has every key missing.
pub(super) fn required_extras(ctx: &Context<'_>, config: &RuleConfig) -> Outcome {
    let Ok(objects) = GlobSet::new(config.patterns("objects")) else {
        return Outcome::skip(Skip::InvalidPattern);
    };
    // `key` or `key:type`.
    let required: Vec<(&str, Option<&str>)> = config
        .patterns("keys")
        .iter()
        .map(|entry| match entry.split_once(':') {
            Some((key, kind)) => (key.trim(), Some(kind.trim())),
            None => (entry.trim(), None),
        })
        .filter(|(key, _)| !key.is_empty())
        .collect();
    const KINDS: [&str; 7] = [
        "string", "number", "bool", "boolean", "array", "object", "null",
    ];
    if required
        .iter()
        .any(|(_, kind)| kind.is_some_and(|kind| !KINDS.contains(&kind)))
    {
        return Outcome::skip(Skip::InvalidPattern);
    }
    if required.is_empty() {
        return Outcome::judged(Vec::new(), Threshold::None);
    }
    let model = ctx.model;
    let offenders: Vec<Offender> = ctx
        .authored_nodes()
        .filter(|&index| objects.matches(&model.nodes[index].name))
        .filter_map(|index| {
            let gltf: Option<serde_json::Value> = model
                .gltf
                .as_deref()
                .and_then(|gltf| gltf.nodes.get(index))
                .and_then(|node| node.extras.as_deref())
                .and_then(|text| serde_json::from_str(text).ok());
            let props = ctx.extras.and_then(|extras| extras.nodes.get(index));
            // The kind of value `key` holds on this node, when it has one.
            let kind_of = |key: &str| -> Option<&'static str> {
                gltf.as_ref()
                    .and_then(|extras| extras.get(key))
                    .map(json_kind)
                    .or_else(|| {
                        props?
                            .props
                            .iter()
                            .find(|prop| prop.user_defined() && prop.name == key)
                            .map(|prop| prop_kind(prop.kind))
                    })
            };
            let wrong: Vec<String> = required
                .iter()
                .filter(|(key, kind)| match kind_of(key) {
                    None => true,
                    Some(found) => kind.is_some_and(|kind| {
                        let kind = if kind == "boolean" { "bool" } else { kind };
                        found != kind
                    }),
                })
                .map(|(key, kind)| match kind {
                    Some(kind) => format!("{key}:{kind}"),
                    None => (*key).to_owned(),
                })
                .collect();
            (!wrong.is_empty()).then(|| Offender {
                detail: Some(Measured::Text(wrong.join(", "))),
                ..Offender::node(index, None)
            })
        })
        .collect();
    Outcome::judged(
        offenders,
        Threshold::Patterns(config.patterns("keys").to_vec()),
    )
}
