//! The checks themselves, one module per category.
//!
//! Every check is a function from the shared [`Context`] and its rule's
//! [`RuleConfig`] to an [`Outcome`]: the offenders it found and the bound it
//! judged them against. Severity is applied afterwards, from the profile, so a
//! severity edit never re-measures anything.

mod density;
mod geometry;
mod hierarchy;
mod naming;
mod skin;
mod transforms;
mod uv;

use crate::ELEMENT_CAP;
use crate::context::Context;
use crate::finding::{Measured, Offender, RuleResult, Skip, Status, Threshold};
use crate::profile::RuleConfig;
use crate::rule::RuleId;

/// What a check found, before the profile's severity is applied.
pub(crate) struct Outcome {
    pub status: Status,
    pub measured: Option<Measured>,
    pub threshold: Threshold,
    pub offenders: Vec<Offender>,
}

impl Outcome {
    pub fn skip(reason: Skip) -> Self {
        Self {
            status: Status::NotEvaluated(reason),
            measured: None,
            threshold: Threshold::None,
            offenders: Vec::new(),
        }
    }

    /// Fail when any offender was found, pass otherwise.
    pub fn judged(offenders: Vec<Offender>, threshold: Threshold) -> Self {
        Self {
            status: if offenders.is_empty() {
                Status::Pass
            } else {
                Status::Fail
            },
            measured: None,
            threshold,
            offenders,
        }
    }

    pub fn with_measured(mut self, measured: Option<Measured>) -> Self {
        self.measured = measured;
        self
    }

    pub fn into_result(self, rule: RuleId, config: &RuleConfig, key: u64) -> RuleResult {
        let mut offenders = self.offenders;
        let total = offenders.iter().map(|offender| offender.count).sum();
        // Cap the element lists across the whole rule, in node order.
        let mut budget = ELEMENT_CAP;
        let mut truncated = false;
        for offender in &mut offenders {
            let length = offender.elements.len();
            if length > budget {
                truncated |= offender.elements.truncate(budget);
            }
            budget = budget.saturating_sub(length.min(budget));
        }
        RuleResult {
            rule,
            status: self.status,
            severity: config.severity,
            measured: self.measured,
            threshold: self.threshold,
            offenders,
            total,
            truncated,
            measure_key: key,
        }
    }
}

/// Run `rule`'s check.
pub(crate) fn evaluate(
    rule: RuleId,
    ctx: &Context<'_>,
    config: &RuleConfig,
    profile: &crate::profile::AuditProfile,
) -> Outcome {
    if ctx.model.indices.is_empty() && ctx.model.nodes.is_empty() {
        return Outcome::skip(Skip::EmptyModel);
    }
    let outcome = match rule {
        RuleId::DegenerateTriangles => geometry::degenerate_triangles(ctx),
        RuleId::NonManifoldEdges => geometry::non_manifold_edges(ctx),
        RuleId::IsolatedVertices => geometry::isolated_vertices(ctx),
        RuleId::DuplicateVertices => geometry::duplicate_vertices(ctx, config),
        RuleId::Ngons => geometry::ngons(ctx),
        RuleId::MissingNormals => geometry::missing_normals(ctx),
        RuleId::MissingTangents => geometry::missing_tangents(ctx),
        RuleId::InvertedNormals => geometry::inverted_normals(ctx),
        RuleId::HardEdges => geometry::hard_edges(ctx, config),
        RuleId::TriangleBudget => geometry::triangle_budget(ctx, config),
        RuleId::DrawCallBudget => geometry::draw_call_budget(ctx, config),
        RuleId::Scale => transforms::scale(ctx),
        RuleId::NegativeScale => transforms::negative_scale(ctx),
        RuleId::Unfrozen => transforms::unfrozen(ctx),
        RuleId::PivotOffset => transforms::pivot_offset(ctx, config),
        RuleId::Unit => transforms::unit(ctx, config),
        RuleId::UpAxis => transforms::up_axis(ctx, config),
        RuleId::UvMissing => uv::missing(ctx),
        RuleId::UvOutOfRange => uv::out_of_range(ctx, 0),
        RuleId::UvOverlap => uv::overlap_per_material(ctx, config),
        RuleId::UvFlipped => uv::flipped(ctx),
        RuleId::LightmapMissing => uv::lightmap_missing(ctx, config),
        RuleId::LightmapOverlap => uv::lightmap_overlap(ctx, config),
        RuleId::LightmapPadding => uv::lightmap_padding(ctx, config),
        RuleId::LightmapOutOfRange => uv::lightmap_out_of_range(ctx, config),
        RuleId::Influences => skin::influences(ctx, config),
        RuleId::UnnormalizedWeights => skin::unnormalized_weights(ctx, config),
        RuleId::BonesPerMesh => skin::bones_per_mesh(ctx, config),
        RuleId::UnusedBones => skin::unused_bones(ctx),
        RuleId::BindPoseMismatch => skin::bind_pose_mismatch(ctx),
        RuleId::TexelDensity => density::texel(ctx, config),
        RuleId::TriangleLod => density::triangle_lod(ctx, config, false),
        RuleId::TriangleReduce => {
            density::triangle_lod(ctx, &profile.rule(RuleId::TriangleLod), true)
        }
        RuleId::InvalidCharacters => naming::invalid_characters(ctx),
        RuleId::NamePattern => naming::pattern(ctx, config),
        RuleId::LodSuffix => naming::lod_suffix(ctx),
        RuleId::CollisionPrefix => naming::collision_prefix(ctx, config),
        RuleId::AssetPrefix => naming::asset_prefix(ctx, config),
        RuleId::EmptyNodes => hierarchy::empty_nodes(ctx, config),
        RuleId::DuplicateNames => hierarchy::duplicate_names(ctx),
        RuleId::MultipleRoots => hierarchy::multiple_roots(ctx),
        RuleId::LightsCameras => hierarchy::lights_cameras(ctx),
        RuleId::MaterialsPerMesh => hierarchy::materials_per_mesh(ctx, config),
    };
    if ctx.cancelled() {
        Outcome::skip(Skip::Cancelled)
    } else {
        outcome
    }
}

/// A count parameter as an integer.
pub(crate) fn count_param(config: &RuleConfig, key: &str, fallback: u64) -> u64 {
    config
        .number(key)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map_or(fallback, |value| value.round() as u64)
}

/// The headline figure of a set of per-object offenders: the largest.
pub(crate) fn largest(
    offenders: &[Offender],
    value: impl Fn(&Measured) -> Option<f64>,
) -> Option<Measured> {
    offenders
        .iter()
        .filter_map(|offender| offender.measured.as_ref())
        .max_by(|a, b| {
            value(a)
                .unwrap_or(f64::NEG_INFINITY)
                .total_cmp(&value(b).unwrap_or(f64::NEG_INFINITY))
        })
        .cloned()
}

/// Group `(node, element)` pairs, already in node order, into one offender per
/// node.
pub(crate) fn group_by_node<T>(
    items: impl IntoIterator<Item = (u32, T)>,
    make: impl Fn(Vec<T>) -> crate::finding::ElementSet,
) -> Vec<Offender> {
    let mut offenders = Vec::new();
    let mut current: Option<(u32, Vec<T>)> = None;
    for (node, item) in items {
        match &mut current {
            Some((open, list)) if *open == node => list.push(item),
            _ => {
                if let Some((open, list)) = current.take() {
                    offenders.push(Offender::elements(
                        (open != u32::MAX).then_some(open as usize),
                        make(list),
                    ));
                }
                current = Some((node, vec![item]));
            }
        }
    }
    if let Some((open, list)) = current {
        offenders.push(Offender::elements(
            (open != u32::MAX).then_some(open as usize),
            make(list),
        ));
    }
    offenders
}
