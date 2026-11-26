//! A glTF node's extras show in the Inspector, one row per key, each value as
//! the file wrote it.

use egui::{Pos2, RawInput};
use review_model::{GltfNode, GltfSource, ModelData, NodeKind, SceneNode};
use review_render::OrbitCamera;
use review_ui::{Selection, UiState};

/// Every text one pass of the overlay painted.
fn painted(ctx: &egui::Context, state: &mut UiState, model: &ModelData, time: f64) -> Vec<String> {
    let input = RawInput {
        time: Some(time),
        screen_rect: Some(egui::Rect::from_min_size(
            Pos2::ZERO,
            egui::vec2(1280.0, 1600.0),
        )),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        review_ui::draw_overlay(ui, state, OrbitCamera::default(), model, None, None);
    });
    output.textures_delta.clear();
    let mut texts = Vec::new();
    for clipped in &output.shapes {
        collect(&clipped.shape, &mut texts);
    }
    texts
}

fn collect(shape: &egui::Shape, out: &mut Vec<String>) {
    match shape {
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, out)),
        egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
        _ => {}
    }
}

/// The node's extras and its mesh's, each key with its value. A value far wider
/// than the panel is cut to its column: laid out whole, it pushed the mesh's
/// section out of the panel, where it was never painted.
#[test]
fn a_gltf_nodes_extras_are_listed_key_by_key() {
    let long = "x".repeat(200);
    let model = ModelData {
        nodes: vec![SceneNode {
            name: "wheel".into(),
            kind: NodeKind::Empty,
            ..Default::default()
        }],
        gltf: Some(Box::new(GltfSource {
            nodes: vec![GltfNode {
                extras: Some(format!(
                    r#"{{"mass": 2.5, "joint": "hinge", "axis": [0, 1, 0], "note": "{long}"}}"#
                )),
                mesh_extras: Some(r#"{"lod_bias": 1}"#.into()),
                ..Default::default()
            }],
            ..Default::default()
        })),
        ..ModelData::default()
    };
    let ctx = egui::Context::default();
    review_ui::init_style(&ctx);
    let mut state = UiState {
        selection: Selection::Node(0),
        ..UiState::default()
    };
    painted(&ctx, &mut state, &model, 1.0);
    let texts = painted(&ctx, &mut state, &model, 2.0);
    for expected in [
        "Extras",
        "mass",
        "2.5",
        "joint",
        "hinge",
        "axis",
        "[0,1,0]",
        "Mesh extras",
        "lod_bias",
    ] {
        assert!(
            texts.iter().any(|text| text == expected),
            "{expected} is painted: {texts:?}"
        );
    }
}
