//! Naming conventions read off node names: the `_LOD<n>` suffix game pipelines
//! use to carry a LOD chain as sibling objects.

use crate::ModelData;

/// Parse a node name's trailing LOD index: `..._LOD<n>` in any case, or a name
/// that is entirely `LOD<n>` (FBX LOD-group children are often named just
/// that). Anchored at the end so `"Wall_LOD2"` is 2 but `"LOD_Test_Wall"` is
/// nothing.
pub fn lod_suffix(name: &str) -> Option<u32> {
    let bytes = name.as_bytes();
    let digits_start = bytes.iter().rposition(|byte| !byte.is_ascii_digit())? + 1;
    if digits_start == bytes.len() {
        return None; // no trailing digits
    }
    let head = &bytes[..digits_start];
    let tagged = (head.len() >= 4 && head[head.len() - 4..].eq_ignore_ascii_case(b"_lod"))
        || head.eq_ignore_ascii_case(b"lod");
    if !tagged {
        return None;
    }
    name[digits_start..].parse().ok()
}

/// The name with its `_LOD<n>` suffix removed — what the levels of one chain
/// share. `None` for a name that carries no suffix.
pub fn lod_base(name: &str) -> Option<&str> {
    lod_suffix(name)?;
    let bytes = name.as_bytes();
    let digits_start = bytes.iter().rposition(|byte| !byte.is_ascii_digit())? + 1;
    let head = &name[..digits_start];
    Some(if head.len() >= 4 {
        &head[..head.len() - 4]
    } else {
        ""
    })
}

/// The LOD identity of `node`: the index parsed from the `_LOD<n>` name suffix,
/// on the node itself or the nearest named ancestor (LOD grouping is sometimes
/// on a parent group node). `None` for a node with no LOD identity. The parent
/// walk is bounded so a malformed cycle in the hierarchy terminates.
pub fn node_lod(model: &ModelData, node: u32) -> Option<u32> {
    let mut index = node as usize;
    for _ in 0..=model.nodes.len() {
        let node = model.nodes.get(index)?;
        if let Some(lod) = lod_suffix(&node.name) {
            return Some(lod);
        }
        index = node.parent?;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
