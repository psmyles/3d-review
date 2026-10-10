//! The JSON report: one document shared by the viewer's "Save report" and the
//! headless batch run.
//!
//! It names everything the way the artist's DCC does — objects by name, faces
//! by their polygon index within the mesh, vertices by their control-point
//! index — never by the viewer's internal triangle or corner numbering, so a
//! report can be followed in Maya or Blender. It carries ids and values only;
//! prose belongs to whoever renders the report.

use serde::Serialize;
use serde_json::{Value, json};

use crate::AuditError;
use crate::finding::{AuditReport, ElementSet, Offender, RuleResult};
use crate::profile::AuditProfile;
use review_model::ModelData;
use review_model::extras::SourceExtras;

/// The report's schema id and version.
pub const REPORT_SCHEMA: &str = "3d-review.audit-report";
pub const REPORT_VERSION: u32 = 1;

/// How many elements of one offender the report lists. Counts stay exact.
pub const REPORT_ELEMENT_CAP: usize = 1000;

/// What the report says about where it came from.
#[derive(Debug, Clone, Default)]
pub struct ReportSource {
    /// The audited file's path as the user knows it.
    pub file: String,
    /// The program that wrote the report.
    pub generator: String,
    /// When, as seconds since the Unix epoch.
    pub created_unix: u64,
}

/// Serialize `report` (of `model`, judged against `profile`) as a
/// pretty-printed JSON document.
pub fn to_json(
    report: &AuditReport,
    model: &ModelData,
    extras: Option<&SourceExtras>,
    profile: &AuditProfile,
    source: &ReportSource,
) -> Result<String, AuditError> {
    let document = json!({
        "schema": REPORT_SCHEMA,
        "version": REPORT_VERSION,
        "generator": source.generator,
        "created_unix": source.created_unix,
        "source": {
            "file": source.file,
            "unit_meters": model.stats.source_unit_meters,
            "up_axis": extras.map(|extras| format!("{:?}", extras.scene.axes[1])),
            "counts": {
                "polygons": model.stats.polygon_count,
                "triangles": model.stats.triangle_count,
                "vertices": model.stats.vertex_count,
                "nodes": model.nodes.len(),
                "materials": model.materials.len(),
                "uv_sets": model.stats.uv_set_count,
                "bones": model.stats.bone_count,
            },
        },
        "profile": {
            "digest": format!("{:016x}", report.profile_digest),
            "resolved": to_value(profile)?,
        },
        "summary": {
            "failed": {
                "error": report.summary.failed[2],
                "warning": report.summary.failed[1],
                "info": report.summary.failed[0],
            },
            "passed": report.summary.passed,
            "not_evaluated": report.summary.not_evaluated,
            "worst": report.summary.worst,
        },
        "results": report
            .results
            .iter()
            .map(|result| result_value(result, model, extras))
            .collect::<Result<Vec<_>, _>>()?,
    });
    serde_json::to_string_pretty(&document).map_err(|error| AuditError::Report(error.to_string()))
}

fn to_value(value: &impl Serialize) -> Result<Value, AuditError> {
    serde_json::to_value(value).map_err(|error| AuditError::Report(error.to_string()))
}

fn result_value(
    result: &RuleResult,
    model: &ModelData,
    extras: Option<&SourceExtras>,
) -> Result<Value, AuditError> {
    Ok(json!({
        "rule": result.rule.as_str(),
        "category": result.rule.category(),
        "status": to_value(&result.status)?,
        "severity": result.severity,
        "measured": to_value(&result.measured)?,
        "threshold": to_value(&result.threshold)?,
        "total": result.total,
        "offenders": result
            .offenders
            .iter()
            .map(|offender| offender_value(offender, model, extras))
            .collect::<Result<Vec<_>, _>>()?,
    }))
}

fn offender_value(
    offender: &Offender,
    model: &ModelData,
    extras: Option<&SourceExtras>,
) -> Result<Value, AuditError> {
    let node = offender.node.map(|node| node as usize);
    let mesh = node.and_then(|node| extras.and_then(|extras| extras.mesh_of_node(node as u32)));
    // Per-mesh numbering, as the DCC counts: the part's own polygon and
    // control-point indices.
    let face_first = mesh.map_or(0, |mesh| mesh.face_first);
    let logical_first = mesh.map_or(0, |mesh| mesh.logical_first);
    let control_point = |corner: u32| {
        model
            .corner_to_logical
            .get(corner as usize)
            .map(|&logical| logical.saturating_sub(logical_first))
            .unwrap_or(corner)
    };
    let elements = match &offender.elements {
        ElementSet::None => Value::Null,
        ElementSet::Triangles(set) => {
            let mut polygons: Vec<u32> = set
                .iter()
                .map(|triangle| {
                    model
                        .triangles
                        .to_face
                        .get(triangle as usize)
                        .map_or(triangle, |&face| face.saturating_sub(face_first))
                })
                .collect();
            polygons.dedup();
            polygons.truncate(REPORT_ELEMENT_CAP);
            json!({ "polygons": polygons })
        }
        ElementSet::Edges(edges) => json!({
            "edges": edges
                .iter()
                .take(REPORT_ELEMENT_CAP)
                .map(|[a, b]| [control_point(*a), control_point(*b)])
                .collect::<Vec<_>>(),
        }),
        ElementSet::Vertices(corners) => json!({
            "control_points": corners
                .iter()
                .take(REPORT_ELEMENT_CAP)
                .map(|&corner| control_point(corner))
                .collect::<Vec<_>>(),
        }),
        ElementSet::Points(points) => json!({
            "points": points
                .iter()
                .take(REPORT_ELEMENT_CAP)
                .map(|(position, id)| json!({ "position": position, "id": id }))
                .collect::<Vec<_>>(),
        }),
    };
    Ok(json!({
        "node": node,
        "name": node.and_then(|node| model.nodes.get(node)).map(|node| node.name.clone()),
        "count": offender.count,
        "measured": to_value(&offender.measured)?,
        "detail": to_value(&offender.detail)?,
        "elements": elements,
    }))
}
