//! Naming conventions read off node names: the `_LOD<n>` suffix game pipelines
//! use to carry a LOD chain as sibling objects.

use std::collections::HashMap;

use crate::ModelData;

/// Where a name's trailing LOD digits start, when it carries a suffix at all.
fn suffix_digits(name: &str) -> Option<usize> {
    let bytes = name.as_bytes();
    let digits_start = bytes.iter().rposition(|byte| !byte.is_ascii_digit())? + 1;
    if digits_start == bytes.len() {
        return None; // no trailing digits
    }
    let head = &bytes[..digits_start];
    let tagged = (head.len() >= 4 && head[head.len() - 4..].eq_ignore_ascii_case(b"_lod"))
        || head.eq_ignore_ascii_case(b"lod");
    tagged.then_some(digits_start)
}

/// Parse a node name's trailing LOD index: `..._LOD<n>` in any case, or a name
/// that is entirely `LOD<n>` (FBX LOD-group children are often named just
/// that). Anchored at the end so `"Wall_LOD2"` is 2 but `"LOD_Test_Wall"` is
/// nothing.
pub fn lod_suffix(name: &str) -> Option<u32> {
    name[suffix_digits(name)?..].parse().ok()
}

/// The name with its `_LOD<n>` suffix removed — what the levels of one chain
/// share. `None` for a name that carries no suffix.
pub fn lod_base(name: &str) -> Option<&str> {
    lod_suffix(name)?;
    let head = &name[..suffix_digits(name)?];
    Some(if head.len() >= 4 {
        &head[..head.len() - 4]
    } else {
        ""
    })
}

/// `name` as the name of LOD `level`. A name that already carries a suffix has
/// its number replaced, keeping its own spelling (`"Wall_LOD0"` → `"Wall_LOD2"`,
/// `"LOD0"` → `"LOD2"`); any other gets `_LOD<level>` appended. Never a second
/// suffix: an engine reads `"Wall_LOD0_LOD1"` as a chain of its own.
pub fn with_lod(name: &str, level: usize) -> String {
    match suffix_digits(name).filter(|_| lod_suffix(name).is_some()) {
        Some(digits_start) => format!("{}{level}", &name[..digits_start]),
        None => format!("{name}_LOD{level}"),
    }
}

/// A node's LOD identity, and the node that gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LodTag {
    /// The node whose name carries the suffix: the node itself, or its nearest
    /// named ancestor.
    pub node: usize,
    pub level: u32,
}

/// The LOD identity of `node`: the index parsed from the `_LOD<n>` name suffix,
/// on the node itself or the nearest named ancestor (LOD grouping is sometimes
/// on a parent group node). `None` for a node with no LOD identity. The parent
/// walk is bounded so a malformed cycle in the hierarchy terminates.
pub fn lod_tag(model: &ModelData, node: u32) -> Option<LodTag> {
    let mut index = node as usize;
    for _ in 0..=model.nodes.len() {
        let node = model.nodes.get(index)?;
        if let Some(level) = lod_suffix(&node.name) {
            return Some(LodTag { node: index, level });
        }
        index = node.parent?;
    }
    None
}

/// The LOD index of `node`, as [`lod_tag`] finds it.
pub fn node_lod(model: &ModelData, node: u32) -> Option<u32> {
    lod_tag(model, node).map(|tag| tag.level)
}

/// Where a node stands in a LOD chain the file itself authors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LodRole {
    /// Not part of an authored chain.
    Plain,
    /// Part of its chain's most detailed level present — `_LOD0` when the file
    /// has one. `root` is the node whose name carries the suffix.
    Source { root: usize },
    /// Part of a less detailed level of a chain whose source is present.
    Replaced,
}

/// Every node's [`LodRole`], indexed like [`ModelData::nodes`].
///
/// A chain is the suffixed nodes sharing a base name (`Wall_LOD0`, `Wall_LOD1`
/// wherever they sit), or, for the bare `LOD<n>` children an FBX LOD group
/// holds, the suffixed siblings under one parent. A node belongs to the level
/// [`lod_tag`] gives it, so a group named `Wall_LOD1` takes every mesh below it
/// along. Only levels carrying geometry count when the source is chosen: a
/// chain is generated from its most detailed *mesh*, and a chain with no mesh
/// at all is nothing to replace.
pub fn lod_roles(model: &ModelData) -> Vec<LodRole> {
    let tags: Vec<Option<LodTag>> = (0..model.nodes.len())
        .map(|node| lod_tag(model, node as u32))
        .collect();
    let chain_of = |tag: LodTag| -> (Option<usize>, &str) {
        let node = &model.nodes[tag.node];
        match lod_base(&node.name) {
            Some(base) if !base.is_empty() => (None, base),
            _ => (node.parent, ""),
        }
    };

    let mut source_level: HashMap<(Option<usize>, &str), u32> = HashMap::new();
    for (node, tag) in model.nodes.iter().zip(&tags) {
        if let (Some(tag), Some(_)) = (tag, node.mesh_part) {
            source_level
                .entry(chain_of(*tag))
                .and_modify(|level| *level = (*level).min(tag.level))
                .or_insert(tag.level);
        }
    }

    tags.iter()
        .map(|tag| {
            let Some(tag) = *tag else {
                return LodRole::Plain;
            };
            match source_level.get(&chain_of(tag)) {
                Some(&source) if tag.level > source => LodRole::Replaced,
                Some(_) => LodRole::Source { root: tag.node },
                None => LodRole::Plain,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SceneNode;

    #[test]
    fn suffix_is_parsed_case_insensitively_at_the_end() {
        assert_eq!(lod_suffix("Wall_LOD2"), Some(2));
        assert_eq!(lod_suffix("wall_lod10"), Some(10));
        assert_eq!(lod_suffix("LOD3"), Some(3));
        assert_eq!(lod_suffix("LOD_Test_Wall"), None);
        assert_eq!(lod_suffix("Wall_LOD"), None);
        assert_eq!(lod_suffix("Wall2"), None);
    }

    #[test]
    fn base_strips_the_suffix() {
        assert_eq!(lod_base("Wall_LOD2"), Some("Wall"));
        assert_eq!(lod_base("LOD0"), Some(""));
        assert_eq!(lod_base("Wall"), None);
    }

    #[test]
    fn a_level_name_renumbers_a_suffix_rather_than_adding_one() {
        assert_eq!(with_lod("Wall_LOD0", 2), "Wall_LOD2");
        assert_eq!(
            with_lod("wall_lod1", 0),
            "wall_lod0",
            "the spelling is kept"
        );
        assert_eq!(with_lod("LOD0", 3), "LOD3");
        assert_eq!(with_lod("Body", 1), "Body_LOD1");
        assert_eq!(with_lod("Wall2", 1), "Wall2_LOD1");
    }

    fn node(name: &str, parent: Option<usize>, mesh: bool) -> SceneNode {
        SceneNode {
            name: name.to_owned(),
            parent,
            mesh_part: mesh.then_some(0),
            ..SceneNode::default()
        }
    }

    fn model(nodes: Vec<SceneNode>) -> ModelData {
        ModelData {
            nodes,
            ..ModelData::default()
        }
    }

    #[test]
    fn a_sibling_chain_is_generated_from_lod0() {
        let model = model(vec![
            node("", None, false),
            node("Wall_LOD0", Some(0), true),
            node("Wall_LOD1", Some(0), true),
            node("Wall_LOD2", Some(0), true),
            node("Prop", Some(0), true),
        ]);
        assert_eq!(
            lod_roles(&model),
            [
                LodRole::Plain,
                LodRole::Source { root: 1 },
                LodRole::Replaced,
                LodRole::Replaced,
                LodRole::Plain,
            ]
        );
    }

    #[test]
    fn a_chain_without_lod0_is_generated_from_its_most_detailed_level() {
        let model = model(vec![
            node("Wall_LOD1", None, true),
            node("Wall_LOD2", None, true),
        ]);
        assert_eq!(
            lod_roles(&model),
            [LodRole::Source { root: 0 }, LodRole::Replaced]
        );
    }

    #[test]
    fn a_group_names_the_level_of_every_mesh_below_it() {
        let model = model(vec![
            node("Wall_LOD0", None, false),
            node("Body", Some(0), true),
            node("Trim", Some(0), true),
            node("Wall_LOD1", None, false),
            node("Body", Some(3), true),
        ]);
        assert_eq!(
            lod_roles(&model),
            [
                LodRole::Source { root: 0 },
                LodRole::Source { root: 0 },
                LodRole::Source { root: 0 },
                LodRole::Replaced,
                LodRole::Replaced,
            ]
        );
    }

    #[test]
    fn bare_lod_group_children_chain_per_group() {
        // Two FBX LOD groups, each with children named only `LOD<n>`: one chain
        // each, not one chain across the file.
        let model = model(vec![
            node("Rock", None, false),
            node("LOD0", Some(0), true),
            node("LOD1", Some(0), true),
            node("Tree", None, false),
            node("LOD1", Some(3), true),
            node("LOD2", Some(3), true),
        ]);
        assert_eq!(
            lod_roles(&model),
            [
                LodRole::Plain,
                LodRole::Source { root: 1 },
                LodRole::Replaced,
                LodRole::Plain,
                LodRole::Source { root: 4 },
                LodRole::Replaced,
            ]
        );
    }

    #[test]
    fn a_level_without_geometry_does_not_become_the_source() {
        let model = model(vec![
            node("Wall_LOD0", None, false),
            node("Wall_LOD1", None, true),
        ]);
        assert_eq!(
            lod_roles(&model),
            [LodRole::Source { root: 0 }, LodRole::Source { root: 1 }],
            "an empty LOD0 is not a source, and replaces nothing"
        );
    }
}
