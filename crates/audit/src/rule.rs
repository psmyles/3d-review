//! The rules: every check the audit knows, and the fixed facts about each —
//! its category, what kind of element it reports, how it is drawn, and which
//! diagnostic view shows it best.
//!
//! They are all in `rules-catalog.toml` at the repository root, the one place a
//! rule is described: `build.rs` checks the catalog and generates [`RuleId`]
//! (in catalog order) and these facts from it, with each rule's parameters
//! ([`RuleId::param_specs`]), defaults and texts. Only the checks themselves are
//! code (`checks/`), and their dispatch matches on `RuleId`, so a rule added to
//! the catalog fails the build until its check exists.
//!
//! The wire name of every rule (`"uv.overlap"`) is a format contract: saved
//! profiles and reports key on it. `wire_names_are_pinned` in the tests is what
//! keeps a rename from silently orphaning every saved file.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The code `build.rs` generates from the catalog.
mod generated {
    #![allow(dead_code)]
    include!(concat!(env!("OUT_DIR"), "/rules.rs"));
}

pub use generated::{CATALOG_TOML, RuleId, catalog};

/// The group a rule is listed under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Geometry,
    Transforms,
    Uv,
    Skin,
    Density,
    Naming,
    Hierarchy,
}

impl Category {
    pub const ALL: [Category; 7] = [
        Category::Geometry,
        Category::Transforms,
        Category::Uv,
        Category::Skin,
        Category::Density,
        Category::Naming,
        Category::Hierarchy,
    ];
}

/// How bad a finding is. Ordered, so the worst of several is their maximum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    pub const ALL: [Severity; 3] = [Severity::Info, Severity::Warning, Severity::Error];

    /// Position in per-severity arrays (`[Info, Warning, Error]`).
    pub fn index(self) -> usize {
        self as usize
    }
}

/// What a rule's offenders are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    Triangle,
    /// Polygons as the file has them (each triangle, in a mesh without
    /// polygons): counted as faces, drawn as their triangles.
    Face,
    Edge,
    Vertex,
    Node,
    Scene,
}

/// How a rule's offenders are drawn in the viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    /// Tinted triangles.
    Fill,
    /// Lines along the offending edges.
    Edge,
    /// A dot per offender — for elements too small to see as a fill.
    Dot,
    /// The whole offending object tinted.
    Object,
    /// Nothing to draw: the finding is about the scene as a whole.
    None,
}

/// A diagnostic view a finding is best looked at through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticView {
    /// The offenders highlighted over neutral clay.
    Issues,
    TexelDensity,
    TriangleDensity,
    Overdraw,
    QuadOverdraw,
}

impl RuleId {
    /// The stable wire name saved profiles and reports key on.
    pub fn as_str(self) -> &'static str {
        self.id()
    }

    /// The rule a wire name names, if any.
    pub fn from_wire(name: &str) -> Option<RuleId> {
        RuleId::from_id(name)
    }

    /// Whether the rule is about a UV layout, so its offenders are worth seeing
    /// in UV space beside the 3D view.
    pub fn is_uv(self) -> bool {
        self.category() == Category::Uv
            && !matches!(self, RuleId::UvMissing | RuleId::LightmapMissing)
    }

    /// The rules in `category`, in catalog order.
    pub fn in_category(category: Category) -> impl Iterator<Item = RuleId> {
        Self::ALL
            .iter()
            .copied()
            .filter(move |rule| rule.category() == category)
    }
}

/// A rule is written as its wire name.
impl Serialize for RuleId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RuleId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<RuleId, D::Error> {
        let name = String::deserialize(deserializer)?;
        RuleId::from_wire(&name)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown rule \"{name}\"")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Saved profiles and reports key on these strings. Changing one orphans
    /// every file that names it, so a rename has to be deliberate: update this
    /// list *and* add a migration in `profile::envelope`.
    #[test]
    fn wire_names_are_pinned() {
        let names: Vec<&str> = RuleId::ALL.iter().map(|rule| rule.as_str()).collect();
        assert_eq!(
            names,
            [
                "geometry.degenerate_triangles",
                "geometry.non_manifold_edges",
                "geometry.isolated_vertices",
                "geometry.duplicate_vertices",
                "geometry.ngons",
                "geometry.missing_normals",
                "geometry.missing_tangents",
                "geometry.inverted_normals",
                "geometry.hard_edges",
                "geometry.triangle_budget",
                "geometry.draw_call_budget",
                "transform.scale",
                "transform.negative_scale",
                "transform.unfrozen",
                "transform.pivot_offset",
                "scene.unit",
                "scene.up_axis",
                "uv.missing",
                "uv.out_of_range",
                "uv.overlap",
                "uv.flipped",
                "uv.lightmap_missing",
                "uv.lightmap_overlap",
                "uv.lightmap_padding",
                "uv.lightmap_out_of_range",
                "skin.influences",
                "skin.unnormalized_weights",
                "skin.bones_per_mesh",
                "skin.unused_bones",
                "skin.bind_pose_mismatch",
                "density.texel",
                "density.triangle_lod",
                "density.triangle_reduce",
                "naming.invalid_characters",
                "naming.pattern",
                "naming.lod_suffix",
                "naming.collision_prefix",
                "naming.asset_prefix",
                "hierarchy.empty_nodes",
                "hierarchy.duplicate_names",
                "hierarchy.multiple_roots",
                "hierarchy.lights_cameras",
                "hierarchy.materials_per_mesh",
            ]
        );
    }

    #[test]
    fn wire_names_round_trip() {
        for rule in RuleId::ALL {
            assert_eq!(RuleId::from_wire(rule.as_str()), Some(rule));
            let json = serde_json::to_string(&rule).unwrap();
            assert_eq!(json, format!("\"{}\"", rule.as_str()));
        }
    }

    #[test]
    fn every_category_has_rules() {
        for category in Category::ALL {
            assert!(RuleId::in_category(category).next().is_some());
        }
    }
}
