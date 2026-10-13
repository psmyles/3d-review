//! What the viewer says about an animation file that does not line up with the
//! model it was attached to: one line per kind of mismatch the file's
//! [`BindingReport`](review_model::BindingReport) found, for the warning card
//! `app` raises the moment the file is added.
//!
//! Only the mismatches are said. What a file leaves unanimated, or the control
//! rig it carries beside the skeleton, is expected and stays in the report.

use review_model::{AttachedAnimation, Axis, CoordinateAxes, ModelData};

use crate::dimensions::format_dimension;
use crate::keys;
use crate::units::{match_known_unit, meters_value};

/// The warning lines for `animation`, attached to `model` — empty when nothing
/// needs saying.
pub fn binding_warnings(model: &ModelData, animation: &AttachedAnimation) -> Vec<String> {
    let report = &animation.report;
    let name = |node: u32| {
        animation
            .nodes
            .get(node as usize)
            .map_or_else(String::new, |node| node.name.clone())
    };
    let mut lines = Vec::new();
    if let Some(units) = report.units {
        lines.push(keys::ui_binding::units(
            unit_name(units.file_meters),
            unit_name(units.model_meters),
        ));
    }
    if let Some(axes) = report.axes {
        let (file, model) = (axis_system(axes.file), axis_system(axes.model));
        lines.push(if axes.converted {
            keys::ui_binding::axes(file, model)
        } else {
            keys::ui_binding::axes_unconverted(file, model)
        });
    }
    if let Some(largest) = report.bone_offsets.first() {
        lines.push(keys::ui_binding::bone_offsets(
            name(largest.node),
            report.bone_offsets.len() as f64,
            format_dimension(largest.difference, model.stats.source_unit_meters),
        ));
    }
    if let Some(first) = report.vertex_counts.first() {
        lines.push(keys::ui_binding::vertex_counts(
            report.vertex_counts.len() as f64,
            first.file as f64,
            name(first.node),
            first.model as f64,
        ));
    }
    if let Some(&first) = report.unmatched_bones.first() {
        lines.push(keys::ui_binding::unmatched_bones(
            name(first),
            report.unmatched_bones.len() as f64,
        ));
    }
    lines
}

/// A unit by its symbol, or in meters when it is none the viewer knows.
fn unit_name(meters: f32) -> String {
    match match_known_unit(meters) {
        Some(unit) => review_localization::tr(unit.symbol).into_owned(),
        None => meters_value(format!("{meters:.4}")),
    }
}

/// "+Z up, facing -Y".
fn axis_system(axes: CoordinateAxes) -> String {
    keys::ui_binding::axis_system(axis_name(axes.front), axis_name(axes.up))
}

fn axis_name(axis: Axis) -> String {
    let key = match axis {
        Axis::PositiveX => keys::ui_binding::AXIS_POSITIVE_X,
        Axis::NegativeX => keys::ui_binding::AXIS_NEGATIVE_X,
        Axis::PositiveY => keys::ui_binding::AXIS_POSITIVE_Y,
        Axis::NegativeY => keys::ui_binding::AXIS_NEGATIVE_Y,
        Axis::PositiveZ => keys::ui_binding::AXIS_POSITIVE_Z,
        Axis::NegativeZ => keys::ui_binding::AXIS_NEGATIVE_Z,
    };
    review_localization::tr(key).into_owned()
}

#[cfg(test)]
mod tests {
    use review_model::BindingReport;
    use review_model::attach::{AxisChange, BoneOffset, UnitChange};

    use super::*;

    const Y_UP: CoordinateAxes = CoordinateAxes {
        right: Axis::PositiveX,
        up: Axis::PositiveY,
        front: Axis::PositiveZ,
    };
    const Z_UP: CoordinateAxes = CoordinateAxes {
        right: Axis::PositiveX,
        up: Axis::PositiveZ,
        front: Axis::NegativeY,
    };

    fn attached(report: BindingReport) -> AttachedAnimation {
        AttachedAnimation {
            nodes: vec![review_model::anim::node_with_local(
                "Spine",
                None,
                review_model::LocalTransform::IDENTITY,
            )],
            report,
            ..AttachedAnimation::default()
        }
    }

    #[test]
    fn a_clean_file_says_nothing() {
        let lines = binding_warnings(&ModelData::default(), &attached(BindingReport::default()));
        assert!(lines.is_empty(), "{lines:?}");
    }

    #[test]
    fn each_mismatch_is_one_line_naming_what_differs() {
        let mut model = ModelData::default();
        model.stats.source_unit_meters = 0.01;
        let report = BindingReport {
            units: Some(UnitChange {
                file_meters: 1.0,
                model_meters: 0.01,
            }),
            axes: Some(AxisChange {
                file: Z_UP,
                model: Y_UP,
                converted: true,
            }),
            bone_offsets: vec![BoneOffset {
                node: 0,
                model_node: 2,
                file_length: 0.6,
                model_length: 0.5,
                difference: 0.25,
            }],
            ..BindingReport::default()
        };
        let lines = binding_warnings(&model, &attached(report));
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(
            lines[0].contains("in m and") && lines[0].contains("in cm."),
            "{}",
            lines[0]
        );
        assert!(lines[1].contains("+Z up, facing -Y"), "{}", lines[1]);
        assert!(
            lines[2].starts_with("Bone 'Spine' is 25.0 cm away"),
            "one bone, in the model's unit: {}",
            lines[2]
        );
        assert!(
            lines.iter().all(|line| !line.contains('\n')),
            "the catalog's wrapping is unwrapped: {lines:?}"
        );
    }
}
