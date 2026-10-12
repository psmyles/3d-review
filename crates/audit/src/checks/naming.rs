//! Naming conventions: characters engines choke on, per-kind patterns, LOD
//! suffix chains, collision-mesh prefixes, and asset prefixes.

use std::collections::BTreeMap;

use review_model::NodeKind;
use review_model::naming::{lod_base, lod_suffix as parse_lod_suffix};

use super::Outcome;
use crate::context::Context;
use crate::finding::{Measured, Offender, Skip, Threshold};
use crate::glob::GlobSet;
use crate::profile::RuleConfig;

/// The characters every engine and file system accepts in an object name.
fn is_safe(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
}

pub(super) fn invalid_characters(ctx: &Context<'_>) -> Outcome {
    let offenders = ctx
        .authored_nodes()
        .filter(|&index| {
            let name = &ctx.model.nodes[index].name;
            !name.is_empty() && !name.chars().all(is_safe)
        })
        .map(|index| {
            Offender::node(
                index,
                Some(Measured::Text(ctx.model.nodes[index].name.clone())),
            )
        })
        .collect();
    Outcome::judged(
        offenders,
        Threshold::Patterns(vec!["[A-Za-z0-9_.-]".to_owned()]),
    )
}

/// The pattern key a node is judged by, or `None` for a kind no pattern
/// covers.
fn kind_key(ctx: &Context<'_>, index: usize) -> Option<&'static str> {
    Some(match ctx.kind(index) {
        NodeKind::Mesh if ctx.is_skinned(index) => "skinned",
        NodeKind::Mesh => "mesh",
        NodeKind::Bone => "bone",
        NodeKind::Empty => "empty",
        NodeKind::Light => "light",
        NodeKind::Camera => "camera",
        NodeKind::Other => return None,
    })
}

pub(super) fn pattern(ctx: &Context<'_>, config: &RuleConfig) -> Outcome {
    let mut sets = BTreeMap::new();
    for key in ["mesh", "skinned", "bone", "empty", "light", "camera"] {
        match GlobSet::new(config.patterns(key)) {
            Ok(set) => {
                sets.insert(key, set);
            }
            Err(_) => return Outcome::skip(Skip::InvalidPattern),
        }
    }
    let offenders = ctx
        .authored_nodes()
        .filter(|&index| {
            kind_key(ctx, index)
                .and_then(|key| sets.get(key))
                .is_some_and(|set| !set.is_empty() && !set.matches(&ctx.model.nodes[index].name))
        })
        .map(|index| {
            Offender::node(
                index,
                Some(Measured::Text(ctx.model.nodes[index].name.clone())),
            )
        })
        .collect();
    Outcome::judged(offenders, Threshold::None)
}

/// A LOD chain: its base name, scoped to a parent when the base is empty.
type ChainKey = (String, Option<usize>);

/// `_LOD<n>` chains with a gap, no LOD0, two objects claiming one level, or the
/// bare base name sitting beside its own suffixed levels.
pub(super) fn lod_suffix(ctx: &Context<'_>) -> Outcome {
    let model = ctx.model;
    // Chains are keyed by base name; a bare "LOD0" (an FBX LOD-group child) has
    // no base of its own, so its chain is its parent's children.
    let mut chains: BTreeMap<ChainKey, Vec<(usize, u32)>> = BTreeMap::new();
    for index in ctx.authored_nodes() {
        let name = &model.nodes[index].name;
        if let (Some(level), Some(base)) = (parse_lod_suffix(name), lod_base(name)) {
            let scope = base
                .is_empty()
                .then_some(model.nodes[index].parent)
                .flatten();
            chains
                .entry((base.to_owned(), scope))
                .or_default()
                .push((index, level));
        }
    }
    let mut offenders = Vec::new();
    for ((base, _), members) in &chains {
        let mut levels: Vec<u32> = members.iter().map(|&(_, level)| level).collect();
        levels.sort_unstable();
        let duplicate = levels.windows(2).any(|pair| pair[0] == pair[1]);
        levels.dedup();
        let contiguous = levels
            .iter()
            .enumerate()
            .all(|(i, &level)| level == i as u32);
        let bare = (!base.is_empty())
            .then(|| {
                ctx.authored_nodes()
                    .find(|&index| model.nodes[index].name == *base)
            })
            .flatten();
        if duplicate || !contiguous || bare.is_some() {
            for &(index, _) in members {
                offenders.push(Offender::node(index, Some(Measured::Text(base.clone()))));
            }
            if let Some(index) = bare {
                offenders.push(Offender::node(index, Some(Measured::Text(base.clone()))));
            }
        }
    }
    offenders.sort_by_key(|offender| offender.node);
    Outcome::judged(offenders, Threshold::None)
}

/// The literal text before a pattern's first wildcard — the prefix a collision
/// pattern like `UCX_*` stands for.
fn literal_head(pattern: &str) -> &str {
    let end = pattern.find(['*', '?', '[']).unwrap_or(pattern.len());
    &pattern[..end]
}

/// `name` with a trailing `_<digits>` index removed (`Crate_01` → `Crate`).
fn strip_index(name: &str) -> &str {
    match name.rfind('_') {
        Some(at) if at + 1 < name.len() && name[at + 1..].bytes().all(|b| b.is_ascii_digit()) => {
            &name[..at]
        }
        _ => name,
    }
}

/// Collision meshes (`UCX_<Mesh>` and friends) that name no render mesh: an
/// engine attaches collision to the mesh its name points at, and silently drops
/// one that points nowhere.
pub(super) fn collision_prefix(ctx: &Context<'_>, config: &RuleConfig) -> Outcome {
    let patterns = config.patterns("prefixes");
    let Ok(set) = GlobSet::new(patterns) else {
        return Outcome::skip(Skip::InvalidPattern);
    };
    let model = ctx.model;
    let render_meshes: std::collections::HashSet<&str> = ctx
        .authored_nodes()
        .filter(|&index| {
            ctx.kind(index) == NodeKind::Mesh && !set.matches(&model.nodes[index].name)
        })
        .map(|index| model.nodes[index].name.as_str())
        .collect();
    let offenders = ctx
        .authored_nodes()
        .filter_map(|index| {
            let name = &model.nodes[index].name;
            let head = patterns
                .iter()
                .find(|pattern| {
                    crate::glob::Glob::new(pattern).is_ok_and(|glob| glob.matches(name))
                })
                .map(|pattern| literal_head(pattern))?;
            let target = name.get(head.len()..).unwrap_or("");
            let named =
                render_meshes.contains(target) || render_meshes.contains(strip_index(target));
            (!named).then(|| {
                Offender::node(index, Some(Measured::Text(strip_index(target).to_owned())))
            })
        })
        .collect();
    Outcome::judged(offenders, Threshold::Patterns(patterns.to_vec()))
}

/// Top-level assets whose name lacks the prefix for what they are: a static
/// mesh, or a skinned one.
pub(super) fn asset_prefix(ctx: &Context<'_>, config: &RuleConfig) -> Outcome {
    let (Ok(static_set), Ok(skinned_set)) = (
        GlobSet::new(config.patterns("static")),
        GlobSet::new(config.patterns("skinned")),
    ) else {
        return Outcome::skip(Skip::InvalidPattern);
    };
    let model = ctx.model;
    // What lives under each authored root.
    let mut has_mesh = vec![false; model.nodes.len()];
    let mut has_skin = vec![false; model.nodes.len()];
    for index in 0..model.nodes.len() {
        if ctx.kind(index) != NodeKind::Mesh {
            continue;
        }
        let skinned = ctx.is_skinned(index);
        let mut cursor = Some(index);
        let mut steps = 0;
        while let Some(at) = cursor {
            has_mesh[at] = true;
            has_skin[at] |= skinned;
            cursor = model.nodes[at].parent;
            steps += 1;
            if steps > model.nodes.len() {
                break;
            }
        }
    }
    let offenders = review_model::hierarchy::authored_roots(model, ctx.extras)
        .into_iter()
        .filter(|&index| has_mesh[index])
        .filter(|&index| {
            let set = if has_skin[index] {
                &skinned_set
            } else {
                &static_set
            };
            !set.is_empty() && !set.matches(&model.nodes[index].name)
        })
        .map(|index| Offender::node(index, Some(Measured::Text(model.nodes[index].name.clone()))))
        .collect();
    Outcome::judged(offenders, Threshold::None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heads_and_indices() {
        assert_eq!(literal_head("UCX_*"), "UCX_");
        assert_eq!(literal_head("UC[XP]_*"), "UC");
        assert_eq!(strip_index("Crate_01"), "Crate");
        assert_eq!(strip_index("Crate"), "Crate");
        assert_eq!(strip_index("Crate_"), "Crate_");
    }
}
