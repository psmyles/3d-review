//! One small hand-built model per rule: each test builds exactly the defect a
//! rule exists for (and, where it matters, the look-alike it must not flag) and
//! checks what the audit reports.

use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use review_audit::{
    AuditInput, AuditProfile, AuditReport, ElementSet, Engine, ParamValue, RuleId, RunOptions,
    Status, run,
};
use review_model::{
    LocalTransform, ModelData, NodeKind, SceneNode, SkinCluster, SkinData, TopologyFace, Vertex,
};

/// One polygon: its owning node and its corners as (position, uv).
struct Face {
    node: u32,
    corners: Vec<(Vec3, Vec2)>,
}

fn face(node: u32, corners: &[([f32; 3], [f32; 2])]) -> Face {
    Face {
        node,
        corners: corners
            .iter()
            .map(|&(p, uv)| (Vec3::from_array(p), Vec2::from_array(uv)))
            .collect(),
    }
}

/// A corner-split model the way import builds one: one render vertex per face
/// corner, fan-triangulated polygons, logical vertices shared by position
/// within a node, and flat normals from each face's winding.
fn build(nodes: Vec<SceneNode>, faces: Vec<Face>) -> ModelData {
    let mut model = ModelData {
        nodes,
        ..Default::default()
    };
    let mut logical_of: std::collections::HashMap<(u32, [u32; 3]), u32> = Default::default();
    for face in &faces {
        let positions: Vec<Vec3> = face.corners.iter().map(|c| c.0).collect();
        let normal = (positions[1] - positions[0])
            .cross(positions[2] - positions[0])
            .normalize_or_zero();
        let first = model.vertices.len() as u32;
        for &(position, uv) in &face.corners {
            model.vertices.push(Vertex {
                position,
                normal,
                uv,
                tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
                vertex_color: Vec4::ONE,
            });
            let key = (face.node, position.to_array().map(f32::to_bits));
            let next = logical_of.len() as u32;
            model
                .corner_to_logical
                .push(*logical_of.entry(key).or_insert(next));
        }
        let face_index = model.faces.len() as u32;
        model.faces.push(TopologyFace {
            first_index: first,
            index_count: face.corners.len() as u32,
        });
        for i in 1..face.corners.len() as u32 - 1 {
            model.indices.extend([first, first + i, first + i + 1]);
            model.triangles.to_face.push(face_index);
            model.triangles.material.push(0);
            model.triangles.node.push(face.node);
        }
    }
    model.stats.vertex_count = logical_of.len();
    model.stats.triangle_count = model.indices.len() / 3;
    model.stats.polygon_count = model.faces.len();
    model.stats.uv_set_count = 1;
    model.stats.source_unit_meters = 1.0;
    model.recompute_bounds();
    model
}

fn mesh_node(name: &str) -> SceneNode {
    SceneNode {
        name: name.to_owned(),
        kind: NodeKind::Mesh,
        mesh_part: Some(0),
        ..Default::default()
    }
}

fn quad(node: u32, origin: [f32; 3]) -> Face {
    let [x, y, z] = origin;
    face(
        node,
        &[
            ([x, y, z], [0.0, 0.0]),
            ([x + 1.0, y, z], [1.0, 0.0]),
            ([x + 1.0, y + 1.0, z], [1.0, 1.0]),
            ([x, y + 1.0, z], [0.0, 1.0]),
        ],
    )
}

/// Audit `model` with only `rule` switched on (everything else off), so each
/// test reads one result.
fn audit_one(
    model: &ModelData,
    rule: RuleId,
    tweak: impl FnOnce(&mut AuditProfile),
) -> AuditReport {
    let mut profile = AuditProfile::builtin(Engine::Generic);
    for config in profile.rules.values_mut() {
        config.enabled = false;
    }
    profile.rules.get_mut(&rule).unwrap().enabled = true;
    tweak(&mut profile);
    run(
        AuditInput {
            model,
            extras: None,
        },
        &profile,
        None,
        RunOptions::default(),
    )
    .unwrap()
}

fn total(report: &AuditReport, rule: RuleId) -> u64 {
    let result = report.result(rule).unwrap();
    assert!(
        matches!(result.status, Status::Pass | Status::Fail),
        "{} did not run: {:?}",
        rule.as_str(),
        result.status
    );
    result.total
}

#[test]
fn degenerate_triangles_are_found_and_healthy_ones_are_not() {
    let model = build(
        vec![mesh_node("M")],
        vec![
            quad(0, [0.0, 0.0, 0.0]),
            // Three collinear points.
            face(
                0,
                &[
                    ([0.0, 3.0, 0.0], [0.0, 0.0]),
                    ([1.0, 3.0, 0.0], [0.5, 0.0]),
                    ([2.0, 3.0, 0.0], [1.0, 0.0]),
                ],
            ),
        ],
    );
    assert_eq!(
        total(
            &audit_one(&model, RuleId::DegenerateTriangles, |_| {}),
            RuleId::DegenerateTriangles
        ),
        1
    );
}

#[test]
fn a_fin_is_a_non_manifold_edge() {
    let base = [
        quad(0, [0.0, 0.0, 0.0]),
        // Shares the quad's edge (1,0,0)-(1,1,0).
        face(
            0,
            &[
                ([1.0, 0.0, 0.0], [0.0, 0.0]),
                ([2.0, 0.0, 0.0], [1.0, 0.0]),
                ([2.0, 1.0, 0.0], [1.0, 1.0]),
                ([1.0, 1.0, 0.0], [0.0, 1.0]),
            ],
        ),
    ];
    let manifold = build(vec![mesh_node("M")], base.into_iter().collect());
    assert_eq!(
        total(
            &audit_one(&manifold, RuleId::NonManifoldEdges, |_| {}),
            RuleId::NonManifoldEdges
        ),
        0
    );

    let mut faces: Vec<Face> = vec![
        quad(0, [0.0, 0.0, 0.0]),
        face(
            0,
            &[
                ([1.0, 0.0, 0.0], [0.0, 0.0]),
                ([2.0, 0.0, 0.0], [1.0, 0.0]),
                ([2.0, 1.0, 0.0], [1.0, 1.0]),
                ([1.0, 1.0, 0.0], [0.0, 1.0]),
            ],
        ),
    ];
    // A third face standing on the same edge.
    faces.push(face(
        0,
        &[
            ([1.0, 1.0, 0.0], [0.0, 0.0]),
            ([1.0, 0.0, 0.0], [1.0, 0.0]),
            ([1.0, 0.0, 1.0], [1.0, 1.0]),
        ],
    ));
    let fin = build(vec![mesh_node("M")], faces);
    let report = audit_one(&fin, RuleId::NonManifoldEdges, |_| {});
    assert_eq!(total(&report, RuleId::NonManifoldEdges), 1);
    let offender = &report.result(RuleId::NonManifoldEdges).unwrap().offenders[0];
    assert!(matches!(&offender.elements, ElementSet::Edges(edges) if edges.len() == 1));
}

#[test]
fn coincident_unwelded_points_are_duplicates() {
    // Two quads side by side whose shared edge is two separate control points
    // each — what an unwelded mesh looks like.
    let mut model = build(
        vec![mesh_node("M")],
        vec![quad(0, [0.0, 0.0, 0.0]), quad(0, [1.0, 0.0, 0.0])],
    );
    // Split the logical ids of the second quad entirely.
    let offset = 100;
    for logical in &mut model.corner_to_logical[4..] {
        *logical += offset;
    }
    model.stats.vertex_count += offset as usize;
    let report = audit_one(&model, RuleId::DuplicateVertices, |_| {});
    assert_eq!(
        total(&report, RuleId::DuplicateVertices),
        4,
        "both ends of the seam, on both sides"
    );
}

#[test]
fn ngons_are_counted_by_polygon() {
    let pentagon = face(
        0,
        &[
            ([0.0, 0.0, 0.0], [0.0, 0.0]),
            ([1.0, 0.0, 0.0], [1.0, 0.0]),
            ([1.5, 0.8, 0.0], [1.0, 0.5]),
            ([0.5, 1.5, 0.0], [0.5, 1.0]),
            ([-0.5, 0.8, 0.0], [0.0, 0.5]),
        ],
    );
    let model = build(
        vec![mesh_node("M")],
        vec![pentagon, quad(0, [3.0, 0.0, 0.0])],
    );
    let report = audit_one(&model, RuleId::Ngons, |_| {});
    let result = report.result(RuleId::Ngons).unwrap();
    assert_eq!(result.total, 1, "one polygon");
    assert!(matches!(&result.offenders[0].elements, ElementSet::Triangles(set) if set.len() == 3));
}

#[test]
fn a_flipped_face_is_inverted_but_a_mirrored_object_is_not() {
    let mut model = build(vec![mesh_node("M")], vec![quad(0, [0.0, 0.0, 0.0])]);
    assert_eq!(
        total(
            &audit_one(&model, RuleId::InvertedNormals, |_| {}),
            RuleId::InvertedNormals
        ),
        0
    );

    for vertex in &mut model.vertices {
        vertex.normal = -vertex.normal;
    }
    assert_eq!(
        total(
            &audit_one(&model, RuleId::InvertedNormals, |_| {}),
            RuleId::InvertedNormals
        ),
        1
    );

    // The same flip, but the object is mirrored: its winding is flipped by the
    // transform while import keeps the normals pointing out, so this is a
    // healthy mirrored mesh.
    model.nodes[0].transform = Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0));
    assert_eq!(
        total(
            &audit_one(&model, RuleId::InvertedNormals, |_| {}),
            RuleId::InvertedNormals
        ),
        0
    );
}

#[test]
fn uv_overlap_and_flipped_islands() {
    // Two separate quads on the same UVs: they overlap.
    let stacked = build(
        vec![mesh_node("M")],
        vec![quad(0, [0.0, 0.0, 0.0]), quad(0, [5.0, 0.0, 0.0])],
    );
    let report = audit_one(&stacked, RuleId::UvOverlap, |profile| {
        profile
            .rules
            .get_mut(&RuleId::UvOverlap)
            .unwrap()
            .params
            .insert("resolution".into(), ParamValue::Number(32.0));
    });
    assert_eq!(
        total(&report, RuleId::UvOverlap),
        4,
        "every triangle of both quads"
    );
    assert_eq!(
        total(
            &audit_one(&stacked, RuleId::UvFlipped, |_| {}),
            RuleId::UvFlipped
        ),
        0
    );

    // Mirror one quad's UVs: one island is flipped.
    let mut mirrored = stacked.clone();
    for vertex in &mut mirrored.vertices[4..] {
        vertex.uv.x = 1.0 - vertex.uv.x;
    }
    assert_eq!(
        total(
            &audit_one(&mirrored, RuleId::UvFlipped, |_| {}),
            RuleId::UvFlipped
        ),
        2
    );
}

#[test]
fn uvs_outside_the_tile() {
    let mut model = build(vec![mesh_node("M")], vec![quad(0, [0.0, 0.0, 0.0])]);
    model.vertices[2].uv = Vec2::new(1.5, 1.0);
    // Corner 2 is shared by both triangles of the fan.
    assert_eq!(
        total(
            &audit_one(&model, RuleId::UvOutOfRange, |_| {}),
            RuleId::UvOutOfRange
        ),
        2
    );
}

fn skinned(weights: &[(u32, f32)]) -> ModelData {
    let mut nodes = vec![mesh_node("Body")];
    for bone in 0..9 {
        nodes.push(SceneNode {
            name: format!("Bone{bone}"),
            kind: NodeKind::Bone,
            parent: (bone > 0).then_some(bone),
            ..Default::default()
        });
    }
    let mut model = build(nodes, vec![quad(0, [0.0, 0.0, 0.0])]);
    let logical = model.stats.vertex_count;
    let clusters: Vec<SkinCluster> = (1..10)
        .map(|bone| SkinCluster {
            bone,
            mesh_node: 0,
            world_to_bone_bind: Mat4::IDENTITY,
            mesh_node_to_bone: Mat4::IDENTITY,
            bind_to_world: Mat4::IDENTITY,
            name: String::new(),
        })
        .collect();
    let mut skin = SkinData {
        clusters,
        ..Default::default()
    };
    skin.offsets.push(0);
    for vertex in 0..logical {
        // Vertex 0 gets every listed weight; the rest ride bone 1 fully.
        let row: Vec<(u32, f32)> = if vertex == 0 {
            weights.to_vec()
        } else {
            vec![(1, 1.0)]
        };
        for (bone, weight) in row {
            skin.bones.push(bone);
            skin.weights.push(weight);
            skin.influence_cluster.push(bone - 1);
        }
        skin.offsets.push(skin.bones.len() as u32);
    }
    model.skin = Some(skin);
    model
}

#[test]
fn too_many_and_unnormalized_influences() {
    let nine: Vec<(u32, f32)> = (1..10).map(|bone| (bone, 1.0 / 9.0)).collect();
    let model = skinned(&nine);
    let report = audit_one(&model, RuleId::Influences, |profile| {
        profile
            .rules
            .get_mut(&RuleId::Influences)
            .unwrap()
            .params
            .insert("max".into(), ParamValue::Number(8.0));
    });
    assert_eq!(total(&report, RuleId::Influences), 1);
    assert_eq!(
        total(
            &audit_one(&model, RuleId::UnnormalizedWeights, |_| {}),
            RuleId::UnnormalizedWeights
        ),
        0
    );

    let heavy = skinned(&[(1, 0.7), (2, 0.7)]);
    assert_eq!(
        total(
            &audit_one(&heavy, RuleId::UnnormalizedWeights, |_| {}),
            RuleId::UnnormalizedWeights
        ),
        1
    );
}

#[test]
fn unused_bones_are_the_unweighted_leaves() {
    // Bones 1..=9 in a chain; only bone 1 and bone 2 carry weight, so 3..=9
    // are unused (each has no weighted bone below it).
    let model = skinned(&[(1, 0.5), (2, 0.5)]);
    assert_eq!(
        total(
            &audit_one(&model, RuleId::UnusedBones, |_| {}),
            RuleId::UnusedBones
        ),
        7
    );
}

fn named(names: &[(&str, NodeKind, Option<usize>)]) -> ModelData {
    ModelData {
        nodes: names
            .iter()
            .map(|&(name, kind, parent)| SceneNode {
                name: name.to_owned(),
                kind,
                parent,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn lod_chains_with_gaps_or_a_bare_base() {
    let ok = named(&[
        ("Wall_LOD0", NodeKind::Mesh, None),
        ("Wall_LOD1", NodeKind::Mesh, None),
    ]);
    assert_eq!(
        total(
            &audit_one(&ok, RuleId::LodSuffix, |_| {}),
            RuleId::LodSuffix
        ),
        0
    );
    let gap = named(&[
        ("Wall_LOD0", NodeKind::Mesh, None),
        ("Wall_LOD2", NodeKind::Mesh, None),
    ]);
    assert_eq!(
        total(
            &audit_one(&gap, RuleId::LodSuffix, |_| {}),
            RuleId::LodSuffix
        ),
        2
    );
    let bare = named(&[
        ("Wall", NodeKind::Mesh, None),
        ("Wall_LOD0", NodeKind::Mesh, None),
    ]);
    assert_eq!(
        total(
            &audit_one(&bare, RuleId::LodSuffix, |_| {}),
            RuleId::LodSuffix
        ),
        2
    );
}

#[test]
fn collision_meshes_must_name_a_render_mesh() {
    let model = named(&[
        ("Crate", NodeKind::Mesh, None),
        ("UCX_Crate_01", NodeKind::Mesh, None),
        ("UCX_Barrel_00", NodeKind::Mesh, None),
    ]);
    let report = audit_one(&model, RuleId::CollisionPrefix, |_| {});
    let result = report.result(RuleId::CollisionPrefix).unwrap();
    assert_eq!(result.total, 1);
    assert_eq!(result.offenders[0].node, Some(2));
}

#[test]
fn hierarchy_rules() {
    let model = named(&[
        ("Root", NodeKind::Empty, None),
        ("Body", NodeKind::Mesh, Some(0)),
        ("Group", NodeKind::Empty, Some(0)),
        ("SOCKET_Hand", NodeKind::Empty, Some(0)),
        ("Lamp", NodeKind::Light, Some(0)),
        ("Body", NodeKind::Mesh, None),
    ]);
    assert_eq!(
        total(
            &audit_one(&model, RuleId::EmptyNodes, |_| {}),
            RuleId::EmptyNodes
        ),
        1,
        "only Group: Root has children that render, sockets are ignored"
    );
    assert_eq!(
        total(
            &audit_one(&model, RuleId::DuplicateNames, |_| {}),
            RuleId::DuplicateNames
        ),
        2
    );
    assert_eq!(
        total(
            &audit_one(&model, RuleId::MultipleRoots, |_| {}),
            RuleId::MultipleRoots
        ),
        2
    );
    assert_eq!(
        total(
            &audit_one(&model, RuleId::LightsCameras, |_| {}),
            RuleId::LightsCameras
        ),
        1
    );
}

#[test]
fn naming_patterns_are_globs_per_kind() {
    let model = named(&[
        ("SM_Crate", NodeKind::Mesh, None),
        ("crate_lid", NodeKind::Mesh, None),
        ("bad name!", NodeKind::Empty, None),
    ]);
    let report = audit_one(&model, RuleId::NamePattern, |profile| {
        profile
            .rules
            .get_mut(&RuleId::NamePattern)
            .unwrap()
            .params
            .insert("mesh".into(), ParamValue::Patterns(vec!["SM_*".into()]));
    });
    let result = report.result(RuleId::NamePattern).unwrap();
    assert_eq!(result.offenders.len(), 1);
    assert_eq!(result.offenders[0].node, Some(1));
    assert_eq!(
        total(
            &audit_one(&model, RuleId::InvalidCharacters, |_| {}),
            RuleId::InvalidCharacters
        ),
        1
    );

    let broken = audit_one(&model, RuleId::NamePattern, |profile| {
        profile
            .rules
            .get_mut(&RuleId::NamePattern)
            .unwrap()
            .params
            .insert("mesh".into(), ParamValue::Patterns(vec!["SM_[".into()]));
    });
    assert_eq!(
        broken.result(RuleId::NamePattern).unwrap().status,
        Status::NotEvaluated(review_audit::Skip::InvalidPattern)
    );
}

#[test]
fn scale_and_mirroring() {
    let mut model = named(&[("A", NodeKind::Mesh, None), ("B", NodeKind::Mesh, Some(0))]);
    model.nodes[0].rest_local = LocalTransform {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::new(-1.0, 1.0, 1.0),
    };
    model.nodes[0].transform = model.nodes[0].rest_local.to_mat4();
    model.nodes[1].transform = model.nodes[0].transform;
    assert_eq!(
        total(&audit_one(&model, RuleId::Scale, |_| {}), RuleId::Scale),
        1
    );
    // B inherits A's mirror; only A, where it starts, is reported.
    let report = audit_one(&model, RuleId::NegativeScale, |_| {});
    let result = report.result(RuleId::NegativeScale).unwrap();
    assert_eq!(result.offenders.len(), 1);
    assert_eq!(result.offenders[0].node, Some(0));
}

#[test]
fn rules_that_need_source_properties_say_so() {
    let model = build(vec![mesh_node("M")], vec![quad(0, [0.0, 0.0, 0.0])]);
    let report = audit_one(&model, RuleId::MissingNormals, |_| {});
    assert_eq!(
        report.result(RuleId::MissingNormals).unwrap().status,
        Status::NotEvaluated(review_audit::Skip::SourcePropertiesUnavailable)
    );
}

#[test]
fn disabled_rules_are_not_evaluated_and_the_summary_counts_failures() {
    let model = named(&[("A", NodeKind::Light, None), ("A", NodeKind::Camera, None)]);
    let report = audit_one(&model, RuleId::LightsCameras, |profile| {
        profile
            .rules
            .get_mut(&RuleId::LightsCameras)
            .unwrap()
            .severity = review_audit::Severity::Warning;
    });
    assert_eq!(
        report.result(RuleId::Ngons).unwrap().status,
        Status::NotEvaluated(review_audit::Skip::Disabled)
    );
    assert_eq!(report.summary.attention_count(), 1);
    assert_eq!(
        report.summary.node_worst,
        vec![Some(review_audit::Severity::Warning); 2]
    );
}

#[test]
fn the_json_report_names_polygons_the_way_the_dcc_does() {
    let pentagon = face(
        0,
        &[
            ([0.0, 0.0, 0.0], [0.0, 0.0]),
            ([1.0, 0.0, 0.0], [1.0, 0.0]),
            ([1.5, 0.8, 0.0], [1.0, 0.5]),
            ([0.5, 1.5, 0.0], [0.5, 1.0]),
            ([-0.5, 0.8, 0.0], [0.0, 0.5]),
        ],
    );
    let model = build(
        vec![mesh_node("Crate")],
        vec![quad(0, [3.0, 0.0, 0.0]), pentagon],
    );
    let profile = AuditProfile::builtin(Engine::Generic);
    let report = run(
        AuditInput {
            model: &model,
            extras: None,
        },
        &profile,
        None,
        RunOptions::default(),
    )
    .unwrap();
    let json = review_audit::report::to_json(
        &report,
        &model,
        None,
        &profile,
        &review_audit::report::ReportSource {
            file: "crate.fbx".into(),
            generator: "test".into(),
            created_unix: 0,
        },
    )
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["schema"], "3d-review.audit-report");
    assert_eq!(value["version"], 1);
    assert_eq!(value["source"]["file"], "crate.fbx");
    let ngons = value["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|result| result["rule"] == "geometry.ngons")
        .unwrap();
    assert_eq!(ngons["status"]["status"], "fail");
    // The pentagon is polygon 1 of its mesh, reported once, not per triangle.
    assert_eq!(ngons["offenders"][0]["name"], "Crate");
    assert_eq!(
        ngons["offenders"][0]["elements"]["polygons"],
        serde_json::json!([1])
    );
}
