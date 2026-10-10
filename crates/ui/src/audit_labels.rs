//! Catalog text for the audit's typed values: rule names and explanations,
//! categories, severities, skip reasons, parameters and measured figures.
//!
//! Every map is an exhaustive `match` with no fall-through arm, the same rule
//! `labels.rs` keeps: a rule added to `review-audit` is a compile error here
//! rather than English on a translated screen, and a catalog message nothing
//! names is `dead_code`.

use review_audit::{
    AuditProfile, AuditReport, Axis, Category, Engine, Measured, RuleId, Severity, Skip, Status,
    Threshold,
};
use review_localization::{Key, tr};

use crate::keys::ui_audit as k;

pub(crate) fn category(category: Category) -> Key {
    match category {
        Category::Geometry => k::CATEGORY_GEOMETRY,
        Category::Transforms => k::CATEGORY_TRANSFORMS,
        Category::Uv => k::CATEGORY_UV,
        Category::Skin => k::CATEGORY_SKIN,
        Category::Density => k::CATEGORY_DENSITY,
        Category::Naming => k::CATEGORY_NAMING,
        Category::Hierarchy => k::CATEGORY_HIERARCHY,
    }
}

pub(crate) fn severity(severity: Severity) -> Key {
    match severity {
        Severity::Error => k::SEVERITY_ERROR,
        Severity::Warning => k::SEVERITY_WARNING,
        Severity::Info => k::SEVERITY_INFO,
    }
}

pub(crate) fn skip(skip: Skip) -> Key {
    match skip {
        Skip::Disabled => k::SKIP_DISABLED,
        Skip::SourcePropertiesUnavailable => k::SKIP_NO_PROPERTIES,
        Skip::NoSkin => k::SKIP_NO_SKIN,
        Skip::NoUvSet => k::SKIP_NO_UV_SET,
        Skip::InvalidPattern => k::SKIP_INVALID_PATTERN,
        Skip::EmptyModel => k::SKIP_EMPTY,
        Skip::Cancelled => k::SKIP_CANCELLED,
    }
}

pub(crate) fn engine(engine: Engine) -> Key {
    match engine {
        Engine::Unity => k::ENGINE_UNITY,
        Engine::Unreal => k::ENGINE_UNREAL,
        Engine::Generic => k::ENGINE_GENERIC,
    }
}

pub(crate) fn axis(axis: Axis) -> Key {
    match axis {
        Axis::PositiveX => k::AXIS_X,
        Axis::NegativeX => k::AXIS_NEGATIVE_X,
        Axis::PositiveY => k::AXIS_Y,
        Axis::NegativeY => k::AXIS_NEGATIVE_Y,
        Axis::PositiveZ => k::AXIS_Z,
        Axis::NegativeZ => k::AXIS_NEGATIVE_Z,
    }
}

/// A check's short name, as listed.
pub(crate) fn rule_name(rule: RuleId) -> Key {
    match rule {
        RuleId::DegenerateTriangles => k::RULE_DEGENERATE_TRIANGLES,
        RuleId::NonManifoldEdges => k::RULE_NON_MANIFOLD_EDGES,
        RuleId::IsolatedVertices => k::RULE_ISOLATED_VERTICES,
        RuleId::DuplicateVertices => k::RULE_DUPLICATE_VERTICES,
        RuleId::Ngons => k::RULE_NGONS,
        RuleId::MissingNormals => k::RULE_MISSING_NORMALS,
        RuleId::MissingTangents => k::RULE_MISSING_TANGENTS,
        RuleId::InvertedNormals => k::RULE_INVERTED_NORMALS,
        RuleId::HardEdges => k::RULE_HARD_EDGES,
        RuleId::TriangleBudget => k::RULE_TRIANGLE_BUDGET,
        RuleId::DrawCallBudget => k::RULE_DRAW_CALL_BUDGET,
        RuleId::Scale => k::RULE_SCALE,
        RuleId::NegativeScale => k::RULE_NEGATIVE_SCALE,
        RuleId::Unfrozen => k::RULE_UNFROZEN,
        RuleId::PivotOffset => k::RULE_PIVOT_OFFSET,
        RuleId::Unit => k::RULE_UNIT,
        RuleId::UpAxis => k::RULE_UP_AXIS,
        RuleId::UvMissing => k::RULE_UV_MISSING,
        RuleId::UvOutOfRange => k::RULE_UV_OUT_OF_RANGE,
        RuleId::UvOverlap => k::RULE_UV_OVERLAP,
        RuleId::UvFlipped => k::RULE_UV_FLIPPED,
        RuleId::LightmapMissing => k::RULE_LIGHTMAP_MISSING,
        RuleId::LightmapOverlap => k::RULE_LIGHTMAP_OVERLAP,
        RuleId::LightmapPadding => k::RULE_LIGHTMAP_PADDING,
        RuleId::LightmapOutOfRange => k::RULE_LIGHTMAP_OUT_OF_RANGE,
        RuleId::Influences => k::RULE_INFLUENCES,
        RuleId::UnnormalizedWeights => k::RULE_UNNORMALIZED_WEIGHTS,
        RuleId::BonesPerMesh => k::RULE_BONES_PER_MESH,
        RuleId::UnusedBones => k::RULE_UNUSED_BONES,
        RuleId::BindPoseMismatch => k::RULE_BIND_POSE_MISMATCH,
        RuleId::TexelDensity => k::RULE_TEXEL_DENSITY,
        RuleId::TriangleLod => k::RULE_TRIANGLE_LOD,
        RuleId::TriangleReduce => k::RULE_TRIANGLE_REDUCE,
        RuleId::InvalidCharacters => k::RULE_INVALID_CHARACTERS,
        RuleId::NamePattern => k::RULE_NAME_PATTERN,
        RuleId::LodSuffix => k::RULE_LOD_SUFFIX,
        RuleId::CollisionPrefix => k::RULE_COLLISION_PREFIX,
        RuleId::AssetPrefix => k::RULE_ASSET_PREFIX,
        RuleId::EmptyNodes => k::RULE_EMPTY_NODES,
        RuleId::DuplicateNames => k::RULE_DUPLICATE_NAMES,
        RuleId::MultipleRoots => k::RULE_MULTIPLE_ROOTS,
        RuleId::LightsCameras => k::RULE_LIGHTS_CAMERAS,
        RuleId::MaterialsPerMesh => k::RULE_MATERIALS_PER_MESH,
    }
}

/// What a check looks for, in one sentence.
pub(crate) fn rule_what(rule: RuleId) -> Key {
    match rule {
        RuleId::DegenerateTriangles => k::RULE_DEGENERATE_TRIANGLES_WHAT,
        RuleId::NonManifoldEdges => k::RULE_NON_MANIFOLD_EDGES_WHAT,
        RuleId::IsolatedVertices => k::RULE_ISOLATED_VERTICES_WHAT,
        RuleId::DuplicateVertices => k::RULE_DUPLICATE_VERTICES_WHAT,
        RuleId::Ngons => k::RULE_NGONS_WHAT,
        RuleId::MissingNormals => k::RULE_MISSING_NORMALS_WHAT,
        RuleId::MissingTangents => k::RULE_MISSING_TANGENTS_WHAT,
        RuleId::InvertedNormals => k::RULE_INVERTED_NORMALS_WHAT,
        RuleId::HardEdges => k::RULE_HARD_EDGES_WHAT,
        RuleId::TriangleBudget => k::RULE_TRIANGLE_BUDGET_WHAT,
        RuleId::DrawCallBudget => k::RULE_DRAW_CALL_BUDGET_WHAT,
        RuleId::Scale => k::RULE_SCALE_WHAT,
        RuleId::NegativeScale => k::RULE_NEGATIVE_SCALE_WHAT,
        RuleId::Unfrozen => k::RULE_UNFROZEN_WHAT,
        RuleId::PivotOffset => k::RULE_PIVOT_OFFSET_WHAT,
        RuleId::Unit => k::RULE_UNIT_WHAT,
        RuleId::UpAxis => k::RULE_UP_AXIS_WHAT,
        RuleId::UvMissing => k::RULE_UV_MISSING_WHAT,
        RuleId::UvOutOfRange => k::RULE_UV_OUT_OF_RANGE_WHAT,
        RuleId::UvOverlap => k::RULE_UV_OVERLAP_WHAT,
        RuleId::UvFlipped => k::RULE_UV_FLIPPED_WHAT,
        RuleId::LightmapMissing => k::RULE_LIGHTMAP_MISSING_WHAT,
        RuleId::LightmapOverlap => k::RULE_LIGHTMAP_OVERLAP_WHAT,
        RuleId::LightmapPadding => k::RULE_LIGHTMAP_PADDING_WHAT,
        RuleId::LightmapOutOfRange => k::RULE_LIGHTMAP_OUT_OF_RANGE_WHAT,
        RuleId::Influences => k::RULE_INFLUENCES_WHAT,
        RuleId::UnnormalizedWeights => k::RULE_UNNORMALIZED_WEIGHTS_WHAT,
        RuleId::BonesPerMesh => k::RULE_BONES_PER_MESH_WHAT,
        RuleId::UnusedBones => k::RULE_UNUSED_BONES_WHAT,
        RuleId::BindPoseMismatch => k::RULE_BIND_POSE_MISMATCH_WHAT,
        RuleId::TexelDensity => k::RULE_TEXEL_DENSITY_WHAT,
        RuleId::TriangleLod => k::RULE_TRIANGLE_LOD_WHAT,
        RuleId::TriangleReduce => k::RULE_TRIANGLE_REDUCE_WHAT,
        RuleId::InvalidCharacters => k::RULE_INVALID_CHARACTERS_WHAT,
        RuleId::NamePattern => k::RULE_NAME_PATTERN_WHAT,
        RuleId::LodSuffix => k::RULE_LOD_SUFFIX_WHAT,
        RuleId::CollisionPrefix => k::RULE_COLLISION_PREFIX_WHAT,
        RuleId::AssetPrefix => k::RULE_ASSET_PREFIX_WHAT,
        RuleId::EmptyNodes => k::RULE_EMPTY_NODES_WHAT,
        RuleId::DuplicateNames => k::RULE_DUPLICATE_NAMES_WHAT,
        RuleId::MultipleRoots => k::RULE_MULTIPLE_ROOTS_WHAT,
        RuleId::LightsCameras => k::RULE_LIGHTS_CAMERAS_WHAT,
        RuleId::MaterialsPerMesh => k::RULE_MATERIALS_PER_MESH_WHAT,
    }
}

/// Why a check matters, worded for the profile's target engine where the
/// engines differ.
pub(crate) fn rule_why(rule: RuleId, target: Engine) -> String {
    let e = target.as_str();
    let plain = |key: Key| tr(key).into_owned();
    match rule {
        RuleId::DegenerateTriangles => plain(k::RULE_DEGENERATE_TRIANGLES_WHY),
        RuleId::NonManifoldEdges => plain(k::RULE_NON_MANIFOLD_EDGES_WHY),
        RuleId::IsolatedVertices => plain(k::RULE_ISOLATED_VERTICES_WHY),
        RuleId::DuplicateVertices => plain(k::RULE_DUPLICATE_VERTICES_WHY),
        RuleId::Ngons => k::rule_ngons_why(e),
        RuleId::MissingNormals => k::rule_missing_normals_why(e),
        RuleId::MissingTangents => plain(k::RULE_MISSING_TANGENTS_WHY),
        RuleId::InvertedNormals => plain(k::RULE_INVERTED_NORMALS_WHY),
        RuleId::HardEdges => plain(k::RULE_HARD_EDGES_WHY),
        RuleId::TriangleBudget => plain(k::RULE_TRIANGLE_BUDGET_WHY),
        RuleId::DrawCallBudget => plain(k::RULE_DRAW_CALL_BUDGET_WHY),
        RuleId::Scale => k::rule_scale_why(e),
        RuleId::NegativeScale => plain(k::RULE_NEGATIVE_SCALE_WHY),
        RuleId::Unfrozen => plain(k::RULE_UNFROZEN_WHY),
        RuleId::PivotOffset => plain(k::RULE_PIVOT_OFFSET_WHY),
        RuleId::Unit => k::rule_unit_why(e),
        RuleId::UpAxis => k::rule_up_axis_why(e),
        RuleId::UvMissing => plain(k::RULE_UV_MISSING_WHY),
        RuleId::UvOutOfRange => plain(k::RULE_UV_OUT_OF_RANGE_WHY),
        RuleId::UvOverlap => plain(k::RULE_UV_OVERLAP_WHY),
        RuleId::UvFlipped => plain(k::RULE_UV_FLIPPED_WHY),
        RuleId::LightmapMissing => k::rule_lightmap_missing_why(e),
        RuleId::LightmapOverlap => plain(k::RULE_LIGHTMAP_OVERLAP_WHY),
        RuleId::LightmapPadding => plain(k::RULE_LIGHTMAP_PADDING_WHY),
        RuleId::LightmapOutOfRange => plain(k::RULE_LIGHTMAP_OUT_OF_RANGE_WHY),
        RuleId::Influences => k::rule_influences_why(e),
        RuleId::UnnormalizedWeights => plain(k::RULE_UNNORMALIZED_WEIGHTS_WHY),
        RuleId::BonesPerMesh => k::rule_bones_per_mesh_why(e),
        RuleId::UnusedBones => plain(k::RULE_UNUSED_BONES_WHY),
        RuleId::BindPoseMismatch => plain(k::RULE_BIND_POSE_MISMATCH_WHY),
        RuleId::TexelDensity => plain(k::RULE_TEXEL_DENSITY_WHY),
        RuleId::TriangleLod => plain(k::RULE_TRIANGLE_LOD_WHY),
        RuleId::TriangleReduce => plain(k::RULE_TRIANGLE_REDUCE_WHY),
        RuleId::InvalidCharacters => plain(k::RULE_INVALID_CHARACTERS_WHY),
        RuleId::NamePattern => plain(k::RULE_NAME_PATTERN_WHY),
        RuleId::LodSuffix => k::rule_lod_suffix_why(e),
        RuleId::CollisionPrefix => k::rule_collision_prefix_why(e),
        RuleId::AssetPrefix => k::rule_asset_prefix_why(e),
        RuleId::EmptyNodes => plain(k::RULE_EMPTY_NODES_WHY),
        RuleId::DuplicateNames => plain(k::RULE_DUPLICATE_NAMES_WHY),
        RuleId::MultipleRoots => plain(k::RULE_MULTIPLE_ROOTS_WHY),
        RuleId::LightsCameras => plain(k::RULE_LIGHTS_CAMERAS_WHY),
        RuleId::MaterialsPerMesh => plain(k::RULE_MATERIALS_PER_MESH_WHY),
    }
}

/// How to fix a finding in the modelling program, in general terms.
pub(crate) fn rule_fix(rule: RuleId) -> Key {
    match rule {
        RuleId::DegenerateTriangles => k::RULE_DEGENERATE_TRIANGLES_FIX,
        RuleId::NonManifoldEdges => k::RULE_NON_MANIFOLD_EDGES_FIX,
        RuleId::IsolatedVertices => k::RULE_ISOLATED_VERTICES_FIX,
        RuleId::DuplicateVertices => k::RULE_DUPLICATE_VERTICES_FIX,
        RuleId::Ngons => k::RULE_NGONS_FIX,
        RuleId::MissingNormals => k::RULE_MISSING_NORMALS_FIX,
        RuleId::MissingTangents => k::RULE_MISSING_TANGENTS_FIX,
        RuleId::InvertedNormals => k::RULE_INVERTED_NORMALS_FIX,
        RuleId::HardEdges => k::RULE_HARD_EDGES_FIX,
        RuleId::TriangleBudget => k::RULE_TRIANGLE_BUDGET_FIX,
        RuleId::DrawCallBudget => k::RULE_DRAW_CALL_BUDGET_FIX,
        RuleId::Scale => k::RULE_SCALE_FIX,
        RuleId::NegativeScale => k::RULE_NEGATIVE_SCALE_FIX,
        RuleId::Unfrozen => k::RULE_UNFROZEN_FIX,
        RuleId::PivotOffset => k::RULE_PIVOT_OFFSET_FIX,
        RuleId::Unit => k::RULE_UNIT_FIX,
        RuleId::UpAxis => k::RULE_UP_AXIS_FIX,
        RuleId::UvMissing => k::RULE_UV_MISSING_FIX,
        RuleId::UvOutOfRange => k::RULE_UV_OUT_OF_RANGE_FIX,
        RuleId::UvOverlap => k::RULE_UV_OVERLAP_FIX,
        RuleId::UvFlipped => k::RULE_UV_FLIPPED_FIX,
        RuleId::LightmapMissing => k::RULE_LIGHTMAP_MISSING_FIX,
        RuleId::LightmapOverlap => k::RULE_LIGHTMAP_OVERLAP_FIX,
        RuleId::LightmapPadding => k::RULE_LIGHTMAP_PADDING_FIX,
        RuleId::LightmapOutOfRange => k::RULE_LIGHTMAP_OUT_OF_RANGE_FIX,
        RuleId::Influences => k::RULE_INFLUENCES_FIX,
        RuleId::UnnormalizedWeights => k::RULE_UNNORMALIZED_WEIGHTS_FIX,
        RuleId::BonesPerMesh => k::RULE_BONES_PER_MESH_FIX,
        RuleId::UnusedBones => k::RULE_UNUSED_BONES_FIX,
        RuleId::BindPoseMismatch => k::RULE_BIND_POSE_MISMATCH_FIX,
        RuleId::TexelDensity => k::RULE_TEXEL_DENSITY_FIX,
        RuleId::TriangleLod => k::RULE_TRIANGLE_LOD_FIX,
        RuleId::TriangleReduce => k::RULE_TRIANGLE_REDUCE_FIX,
        RuleId::InvalidCharacters => k::RULE_INVALID_CHARACTERS_FIX,
        RuleId::NamePattern => k::RULE_NAME_PATTERN_FIX,
        RuleId::LodSuffix => k::RULE_LOD_SUFFIX_FIX,
        RuleId::CollisionPrefix => k::RULE_COLLISION_PREFIX_FIX,
        RuleId::AssetPrefix => k::RULE_ASSET_PREFIX_FIX,
        RuleId::EmptyNodes => k::RULE_EMPTY_NODES_FIX,
        RuleId::DuplicateNames => k::RULE_DUPLICATE_NAMES_FIX,
        RuleId::MultipleRoots => k::RULE_MULTIPLE_ROOTS_FIX,
        RuleId::LightsCameras => k::RULE_LIGHTS_CAMERAS_FIX,
        RuleId::MaterialsPerMesh => k::RULE_MATERIALS_PER_MESH_FIX,
    }
}

/// A rule parameter's label and explanation, by its profile key.
pub(crate) fn param(key: &str) -> (Key, Key) {
    match key {
        "tolerance" => (k::PARAM_TOLERANCE, k::PARAM_TOLERANCE_DESCRIPTION),
        "max_ratio" => (k::PARAM_MAX_RATIO, k::PARAM_MAX_RATIO_DESCRIPTION),
        "max_per_object" => (k::PARAM_MAX_PER_OBJECT, k::PARAM_MAX_PER_OBJECT_DESCRIPTION),
        "max_total" => (k::PARAM_MAX_TOTAL, k::PARAM_MAX_TOTAL_DESCRIPTION),
        "max_distance" => (k::PARAM_MAX_DISTANCE, k::PARAM_MAX_DISTANCE_DESCRIPTION),
        "target" => (k::PARAM_TARGET, k::PARAM_TARGET_DESCRIPTION),
        "resolution" => (k::PARAM_RESOLUTION, k::PARAM_RESOLUTION_DESCRIPTION),
        "channel" => (k::PARAM_CHANNEL, k::PARAM_CHANNEL_DESCRIPTION),
        "min_padding" => (k::PARAM_MIN_PADDING, k::PARAM_MIN_PADDING_DESCRIPTION),
        "texture_size" => (k::PARAM_TEXTURE_SIZE, k::PARAM_TEXTURE_SIZE_DESCRIPTION),
        "min_pixel_area" => (k::PARAM_MIN_PIXEL_AREA, k::PARAM_MIN_PIXEL_AREA_DESCRIPTION),
        "screen_height" => (k::PARAM_SCREEN_HEIGHT, k::PARAM_SCREEN_HEIGHT_DESCRIPTION),
        "min_screen_size" => (
            k::PARAM_MIN_SCREEN_SIZE,
            k::PARAM_MIN_SCREEN_SIZE_DESCRIPTION,
        ),
        "mesh" => (k::PARAM_MESH, k::PARAM_MESH_DESCRIPTION),
        "skinned" => (k::PARAM_SKINNED, k::PARAM_SKINNED_DESCRIPTION),
        "bone" => (k::PARAM_BONE, k::PARAM_BONE_DESCRIPTION),
        "empty" => (k::PARAM_EMPTY, k::PARAM_EMPTY_DESCRIPTION),
        "light" => (k::PARAM_LIGHT, k::PARAM_LIGHT_DESCRIPTION),
        "camera" => (k::PARAM_CAMERA, k::PARAM_CAMERA_DESCRIPTION),
        "prefixes" => (k::PARAM_PREFIXES, k::PARAM_PREFIXES_DESCRIPTION),
        "static" => (k::PARAM_STATIC, k::PARAM_STATIC_DESCRIPTION),
        "ignore" => (k::PARAM_IGNORE, k::PARAM_IGNORE_DESCRIPTION),
        // "max", and anything a newer profile names that this build does not.
        _ => (k::PARAM_MAX, k::PARAM_MAX_DESCRIPTION),
    }
}

/// A number with `digits` decimals, trailing zeros trimmed.
fn number(value: f64, digits: usize) -> String {
    let text = format!("{value:.digits$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

/// A measured figure, with its unit.
pub(crate) fn measured(value: &Measured) -> String {
    match value {
        Measured::Count(count) => k::value_count(*count as f64),
        Measured::Ratio(ratio) => k::value_percent(number(ratio * 100.0, 1)),
        Measured::Meters(meters) => k::value_meters(number(*meters, 4)),
        Measured::Degrees(degrees) => k::value_degrees(number(*degrees, 1)),
        Measured::PerSquareMeter(value) => k::value_per_square_meter(number(*value, 0)),
        Measured::PxPerMeter(value) => k::value_px_per_meter(number(*value, 0)),
        Measured::SquarePixels(value) => k::value_square_pixels(number(*value, 1)),
        Measured::ScreenSize(value) => k::value_screen_size(number(*value, 2)),
        Measured::UnitMeters(value) => match crate::units::match_known_unit(*value as f32) {
            Some(unit) => tr(unit.symbol).into_owned(),
            None => k::value_unit(number(*value, 4)),
        },
        Measured::Axis(value) => tr(axis(*value)).into_owned(),
        Measured::Scale([x, y, z]) => k::value_scale(
            number(f64::from(*x), 3),
            number(f64::from(*y), 3),
            number(f64::from(*z), 3),
        ),
        Measured::Text(text) => text.clone(),
    }
}

/// The bound a figure was judged against, or `None` for a check that has none
/// to state.
pub(crate) fn threshold(value: &Threshold) -> Option<String> {
    match value {
        Threshold::None => None,
        Threshold::Max(limit) => Some(k::value_at_most(measured(limit))),
        Threshold::Min(limit) => Some(k::value_at_least(measured(limit))),
        Threshold::Range { min, max } => Some(k::value_range(measured(max), measured(min))),
        Threshold::Equals(target) => Some(measured(target)),
        Threshold::Patterns(patterns) => Some(patterns.join(", ")),
    }
}

/// The profile's display name: its own, or its engine's when it has none, with
/// a marker when it differs from that engine's built-in.
pub(crate) fn profile_name(profile: &AuditProfile) -> String {
    let name = if profile.name.trim().is_empty() {
        tr(engine(profile.base)).into_owned()
    } else {
        profile.name.clone()
    };
    if profile.is_builtin() {
        name
    } else {
        k::profile_modified(name)
    }
}

/// The plain-text summary "Copy summary" puts on the clipboard: a heading, the
/// counts, then one line per failing check, worst first.
pub fn audit_summary_text(report: &AuditReport, profile: &AuditProfile, file: &str) -> String {
    let mut lines = vec![
        k::text_heading(file.to_owned(), profile_name(profile)),
        k::text_counts(
            f64::from(report.summary.failed[Severity::Error.index()]),
            f64::from(report.summary.failed[Severity::Info.index()]),
            f64::from(report.summary.passed),
            f64::from(report.summary.failed[Severity::Warning.index()]),
        ),
    ];
    let mut failing: Vec<_> = report
        .results
        .iter()
        .filter(|result| result.status == Status::Fail)
        .collect();
    failing.sort_by_key(|result| std::cmp::Reverse(result.severity));
    if failing.is_empty() {
        lines.push(tr(k::TEXT_NONE).into_owned());
    }
    for result in failing {
        lines.push(k::text_finding(
            result.total as f64,
            tr(rule_name(result.rule)).into_owned(),
            tr(severity(result.severity)).into_owned(),
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_trim_trailing_zeros() {
        assert_eq!(number(1.50, 2), "1.5");
        assert_eq!(number(2.0, 2), "2");
        assert_eq!(number(1024.0, 0), "1024");
    }

    #[test]
    fn every_rule_has_its_text() {
        for &rule in RuleId::ALL {
            for target in Engine::ALL {
                assert!(!rule_why(rule, target).is_empty(), "{}", rule.as_str());
            }
            assert!(!tr(rule_name(rule)).is_empty());
            assert!(!tr(rule_what(rule)).is_empty());
            assert!(!tr(rule_fix(rule)).is_empty());
        }
    }
}
