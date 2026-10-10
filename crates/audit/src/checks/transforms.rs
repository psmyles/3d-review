//! Transforms and scene setup: scale, mirroring, unfrozen rotations and
//! pivots, where the asset sits, and the file's unit and up axis.

use glam::{Mat4, Vec3};
use review_model::NodeKind;
use review_model::extras::CoordinateAxis;

use super::Outcome;
use crate::context::Context;
use crate::finding::{Axis, ElementSet, Measured, Offender, Skip, Threshold};
use crate::profile::RuleConfig;

/// How far a scale component may sit from 1 before it counts.
const SCALE_EPSILON: f32 = 1.0e-4;
/// The smallest local rotation, in degrees, that counts as unfrozen.
const ROTATION_EPSILON_DEGREES: f32 = 0.01;
/// The pivot and offset properties a DCC leaves behind when an object's
/// transform was moved without moving its geometry.
const PIVOT_PROPS: [&str; 4] = [
    "RotationPivot",
    "ScalingPivot",
    "RotationOffset",
    "ScalingOffset",
];

pub(super) fn scale(ctx: &Context<'_>) -> Outcome {
    let offenders = ctx
        .authored_nodes()
        .filter_map(|index| {
            let scale = ctx.model.nodes[index].rest_local.scale;
            ((scale - Vec3::ONE).abs().max_element() > SCALE_EPSILON)
                .then(|| Offender::node(index, Some(Measured::Scale(scale.to_array()))))
        })
        .collect();
    Outcome::judged(offenders, Threshold::Equals(Measured::Scale([1.0; 3])))
}

/// Where mirroring *starts*: a node whose world transform (with its geometric
/// transform) flips handedness while its parent's does not. Every descendant
/// inherits the flip, so reporting them all would bury the one to fix.
pub(super) fn negative_scale(ctx: &Context<'_>) -> Outcome {
    let model = ctx.model;
    let offenders = ctx
        .authored_nodes()
        .filter_map(|index| {
            let own = ctx.geometry_to_world(index).determinant() < 0.0;
            let parent = model.nodes[index]
                .parent
                .is_some_and(|parent| model.nodes[parent].transform.determinant() < 0.0);
            (own && !parent).then(|| {
                Offender::node(
                    index,
                    Some(Measured::Scale(
                        model.nodes[index].rest_local.scale.to_array(),
                    )),
                )
            })
        })
        .collect();
    Outcome::judged(offenders, Threshold::None)
}

/// Mesh objects carrying a rotation, a geometric transform, or pivot offsets —
/// a transform that was never frozen into the geometry.
pub(super) fn unfrozen(ctx: &Context<'_>) -> Outcome {
    let model = ctx.model;
    let offenders = ctx
        .authored_nodes()
        .filter(|&index| ctx.kind(index) == NodeKind::Mesh)
        .filter_map(|index| {
            let (_, angle) = model.nodes[index].rest_local.rotation.to_axis_angle();
            let degrees = angle.to_degrees().abs();
            let degrees = degrees.min(360.0 - degrees);
            let node_extras = ctx.extras.and_then(|extras| extras.nodes.get(index));
            let geometric = node_extras
                .is_some_and(|extras| !extras.geometry_to_node.abs_diff_eq(Mat4::IDENTITY, 1.0e-5));
            let pivots = node_extras.is_some_and(|extras| {
                extras.props.iter().any(|prop| {
                    PIVOT_PROPS.contains(&prop.name.as_str())
                        && prop.value_real[..3]
                            .iter()
                            .any(|value| value.abs() > 1.0e-6)
                })
            });
            (degrees > ROTATION_EPSILON_DEGREES || geometric || pivots)
                .then(|| Offender::node(index, Some(Measured::Degrees(f64::from(degrees)))))
        })
        .collect();
    Outcome::judged(offenders, Threshold::None)
}

/// Top-level objects whose pivot sits away from the world origin: the asset
/// lands off-centre wherever an engine places it.
pub(super) fn pivot_offset(ctx: &Context<'_>, config: &RuleConfig) -> Outcome {
    let max = config.number("max_distance").unwrap_or(0.01);
    let offenders = review_model::hierarchy::authored_roots(ctx.model, ctx.extras)
        .into_iter()
        .filter_map(|index| {
            let position = ctx.node_position(index);
            let distance = f64::from(position.length());
            (distance > max).then(|| Offender {
                node: Some(index as u32),
                count: 1,
                measured: Some(Measured::Meters(distance)),
                detail: None,
                elements: ElementSet::Points(vec![(position.to_array(), index as u32)]),
            })
        })
        .collect::<Vec<_>>();
    let measured = super::largest(&offenders, |m| match m {
        Measured::Meters(d) => Some(*d),
        _ => None,
    });
    Outcome::judged(offenders, Threshold::Max(Measured::Meters(max))).with_measured(measured)
}

pub(super) fn unit(ctx: &Context<'_>, config: &RuleConfig) -> Outcome {
    let target = config.number("target").unwrap_or(1.0);
    let source = f64::from(ctx.model.stats.source_unit_meters);
    if source <= 0.0 || !source.is_finite() {
        return Outcome::skip(Skip::SourcePropertiesUnavailable);
    }
    let matches = (source - target).abs() <= target * 1.0e-4;
    let offenders = if matches {
        Vec::new()
    } else {
        vec![scene_offender(Measured::UnitMeters(source))]
    };
    Outcome::judged(offenders, Threshold::Equals(Measured::UnitMeters(target)))
        .with_measured(Some(Measured::UnitMeters(source)))
}

pub(super) fn up_axis(ctx: &Context<'_>, config: &RuleConfig) -> Outcome {
    let Some(extras) = ctx.extras else {
        return Outcome::skip(Skip::SourcePropertiesUnavailable);
    };
    let Some(source) = axis(extras.scene.axes[1]) else {
        return Outcome::skip(Skip::SourcePropertiesUnavailable);
    };
    let target = config.axis("target").unwrap_or(Axis::PositiveY);
    let offenders = if source == target {
        Vec::new()
    } else {
        vec![scene_offender(Measured::Axis(source))]
    };
    Outcome::judged(offenders, Threshold::Equals(Measured::Axis(target)))
        .with_measured(Some(Measured::Axis(source)))
}

fn scene_offender(measured: Measured) -> Offender {
    Offender {
        node: None,
        count: 1,
        measured: Some(measured),
        detail: None,
        elements: ElementSet::None,
    }
}

fn axis(axis: CoordinateAxis) -> Option<Axis> {
    Some(match axis {
        CoordinateAxis::PositiveX => Axis::PositiveX,
        CoordinateAxis::NegativeX => Axis::NegativeX,
        CoordinateAxis::PositiveY => Axis::PositiveY,
        CoordinateAxis::NegativeY => Axis::NegativeY,
        CoordinateAxis::PositiveZ => Axis::PositiveZ,
        CoordinateAxis::NegativeZ => Axis::NegativeZ,
        CoordinateAxis::Unknown | CoordinateAxis::Unnamed(_) => return None,
    })
}
