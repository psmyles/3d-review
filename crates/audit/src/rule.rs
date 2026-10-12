//! The rule catalog: every check the audit knows, and the fixed facts about
//! each — its category, what kind of element it reports, how it is drawn, and
//! which diagnostic view shows it best.
//!
//! The wire name of every rule (`"uv.overlap"`) is a format contract: saved
//! profiles and reports key on it. `wire_names_are_pinned` in the tests is what
//! keeps a rename from silently orphaning every saved file.

use serde::{Deserialize, Serialize};

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

macro_rules! rules {
    ($($variant:ident = $wire:literal, $category:ident, $element:ident, $marker:ident;)*) => {
        /// One check in the catalog.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub enum RuleId {
            $(#[serde(rename = $wire)] $variant,)*
        }

        impl RuleId {
            /// Every rule, in catalog (list) order.
            pub const ALL: &'static [RuleId] = &[$(RuleId::$variant,)*];

            /// The stable wire name saved profiles and reports key on.
            pub fn as_str(self) -> &'static str {
                match self { $(RuleId::$variant => $wire,)* }
            }

            /// The rule a wire name names, if any.
            pub fn from_wire(name: &str) -> Option<RuleId> {
                match name { $($wire => Some(RuleId::$variant),)* _ => None }
            }

            pub fn category(self) -> Category {
                match self { $(RuleId::$variant => Category::$category,)* }
            }

            pub fn element(self) -> ElementKind {
                match self { $(RuleId::$variant => ElementKind::$element,)* }
            }

            pub fn marker(self) -> Marker {
                match self { $(RuleId::$variant => Marker::$marker,)* }
            }
        }
    };
}

rules! {
    DegenerateTriangles = "geometry.degenerate_triangles", Geometry, Triangle, Dot;
    NonManifoldEdges = "geometry.non_manifold_edges", Geometry, Edge, Edge;
    IsolatedVertices = "geometry.isolated_vertices", Geometry, Vertex, Dot;
    DuplicateVertices = "geometry.duplicate_vertices", Geometry, Vertex, Dot;
    Ngons = "geometry.ngons", Geometry, Triangle, Fill;
    MissingNormals = "geometry.missing_normals", Geometry, Node, Object;
    MissingTangents = "geometry.missing_tangents", Geometry, Node, Object;
    InvertedNormals = "geometry.inverted_normals", Geometry, Triangle, Fill;
    HardEdges = "geometry.hard_edges", Geometry, Edge, Edge;
    TriangleBudget = "geometry.triangle_budget", Geometry, Node, Object;
    DrawCallBudget = "geometry.draw_call_budget", Geometry, Scene, Object;
    Scale = "transform.scale", Transforms, Node, Object;
    NegativeScale = "transform.negative_scale", Transforms, Node, Object;
    Unfrozen = "transform.unfrozen", Transforms, Node, Object;
    PivotOffset = "transform.pivot_offset", Transforms, Node, Dot;
    Unit = "scene.unit", Transforms, Scene, None;
    UpAxis = "scene.up_axis", Transforms, Scene, None;
    UvMissing = "uv.missing", Uv, Node, Object;
    UvOutOfRange = "uv.out_of_range", Uv, Triangle, Fill;
    UvOverlap = "uv.overlap", Uv, Triangle, Fill;
    UvFlipped = "uv.flipped", Uv, Triangle, Fill;
    LightmapMissing = "uv.lightmap_missing", Uv, Node, Object;
    LightmapOverlap = "uv.lightmap_overlap", Uv, Triangle, Fill;
    LightmapPadding = "uv.lightmap_padding", Uv, Triangle, Fill;
    LightmapOutOfRange = "uv.lightmap_out_of_range", Uv, Triangle, Fill;
    Influences = "skin.influences", Skin, Vertex, Dot;
    UnnormalizedWeights = "skin.unnormalized_weights", Skin, Vertex, Dot;
    BonesPerMesh = "skin.bones_per_mesh", Skin, Node, Object;
    UnusedBones = "skin.unused_bones", Skin, Node, Dot;
    BindPoseMismatch = "skin.bind_pose_mismatch", Skin, Node, Dot;
    TexelDensity = "density.texel", Density, Node, Object;
    TriangleLod = "density.triangle_lod", Density, Node, Object;
    TriangleReduce = "density.triangle_reduce", Density, Node, Object;
    InvalidCharacters = "naming.invalid_characters", Naming, Node, Object;
    NamePattern = "naming.pattern", Naming, Node, Object;
    LodSuffix = "naming.lod_suffix", Naming, Node, Object;
    CollisionPrefix = "naming.collision_prefix", Naming, Node, Object;
    AssetPrefix = "naming.asset_prefix", Naming, Node, Object;
    EmptyNodes = "hierarchy.empty_nodes", Hierarchy, Node, Dot;
    DuplicateNames = "hierarchy.duplicate_names", Hierarchy, Node, Object;
    MultipleRoots = "hierarchy.multiple_roots", Hierarchy, Scene, Object;
    LightsCameras = "hierarchy.lights_cameras", Hierarchy, Node, Dot;
    MaterialsPerMesh = "hierarchy.materials_per_mesh", Hierarchy, Node, Object;
}

impl RuleId {
    /// The view that shows this rule's problem best. Most rules are looked at
    /// as highlighted offenders; the density rules have a heat map of their
    /// own.
    pub fn related_view(self) -> DiagnosticView {
        match self {
            RuleId::TexelDensity => DiagnosticView::TexelDensity,
            RuleId::TriangleLod | RuleId::TriangleReduce | RuleId::TriangleBudget => {
                DiagnosticView::TriangleDensity
            }
            _ => DiagnosticView::Issues,
        }
    }

    /// A second view worth a look for this rule's problem, beside
    /// [`Self::related_view`]: triangles too small for their object are exactly
    /// what quad overdraw measures the cost of.
    pub fn see_also(self) -> Option<DiagnosticView> {
        match self {
            RuleId::TriangleLod | RuleId::TriangleReduce => Some(DiagnosticView::QuadOverdraw),
            RuleId::TriangleBudget => Some(DiagnosticView::Overdraw),
            _ => None,
        }
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
        for &rule in RuleId::ALL {
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
