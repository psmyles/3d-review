//! Round trips of the source-property capture: animation and the layers over it.
//!
//! The authored curves of every animated property (not the baked clips the
//! viewer plays, which are derived), the display layers and selection sets, and
//! the synthetic stack ufbx_write seeds every scene with.

#![cfg(all(has_meshopt, has_ufbxw))]

use review_model::extras::ElementRef;
use review_model::{ModelData, SourceExtras};
use review_optimize::FbxFormat;

mod common;

use common::extras::round_trip;
use common::{fixture_full as fixture, load_full as load, temp_dir};

/// The layer's animated properties keyed by target name and property, for a
/// comparison that does not depend on element order.
fn curves_by_target<'a>(
    model: &'a ModelData,
    extras: &'a SourceExtras,
) -> std::collections::BTreeMap<(String, String), &'a review_model::extras::AnimPropCurves> {
    let mut out = std::collections::BTreeMap::new();
    for layer in &extras.anim_layers {
        for anim in &layer.anim {
            let target = match &anim.target {
                ElementRef::Node(node) => format!("node:{}", model.nodes[*node as usize].name),
                ElementRef::NodeAttribute(node) => {
                    format!("attr:{}", model.nodes[*node as usize].name)
                }
                ElementRef::Material(material) => {
                    format!("material:{}", model.materials[*material as usize].name)
                }
                ElementRef::Texture(texture) => {
                    format!("texture:{}", extras.textures[*texture as usize].name)
                }
                ElementRef::Video(video) => {
                    format!("video:{}", extras.videos[*video as usize].name)
                }
                ElementRef::BlendChannel(channel) => format!(
                    "channel:{}",
                    model
                        .morph
                        .as_ref()
                        .map_or("", |morph| morph.channels[*channel as usize].name.as_str())
                ),
                ElementRef::DisplayLayer(layer) => {
                    format!("layer:{}", extras.display_layers[*layer as usize].name)
                }
                ElementRef::AnimLayer(layer) => {
                    format!("animlayer:{}", extras.anim_layers[*layer as usize].name)
                }
                ElementRef::Unmapped { name, .. } => format!("unmapped:{name}"),
            };
            out.insert((target, anim.prop_name.clone()), anim);
        }
    }
    out
}

fn assert_curves_equal(
    source: &review_model::extras::AnimPropCurves,
    back: &review_model::extras::AnimPropCurves,
    context: &str,
) {
    for component in 0..3 {
        // A component the source never keyed comes back as an empty curve (the
        // writer creates one per component); both mean "the default".
        fn present(
            curve: &Option<review_model::extras::Curve>,
        ) -> Option<&review_model::extras::Curve> {
            curve.as_ref().filter(|c| !c.keys.is_empty())
        }
        match (
            present(&source.curves[component]),
            present(&back.curves[component]),
        ) {
            (None, None) => {}
            (Some(a), Some(b)) => {
                assert_eq!(
                    a.keys.len(),
                    b.keys.len(),
                    "{context}[{component}] key count"
                );
                assert_eq!(a.pre, b.pre, "{context}[{component}] pre-extrapolation");
                assert_eq!(a.post, b.post, "{context}[{component}] post-extrapolation");
                for (index, (ka, kb)) in a.keys.iter().zip(&b.keys).enumerate() {
                    assert!(
                        (ka.time - kb.time).abs() < 1e-6,
                        "{context}[{component}] key {index} time"
                    );
                    assert!(
                        (ka.value - kb.value).abs() < 1e-5,
                        "{context}[{component}] key {index} value {} vs {}",
                        ka.value,
                        kb.value
                    );
                    assert_eq!(
                        ka.interpolation, kb.interpolation,
                        "{context}[{component}] key {index} interpolation"
                    );
                    if ka.interpolation == review_model::extras::Interpolation::Cubic {
                        // A first key's left handle has no previous key to hang on
                        // (the reader derives it from the previous key's attribute),
                        // so it cannot come back — and nothing evaluates it.
                        type Side<'a> = (&'a str, (f32, f32), (f32, f32));
                        let sides: &[Side] = if index == 0 {
                            &[("right", ka.right, kb.right)]
                        } else {
                            &[("left", ka.left, kb.left), ("right", ka.right, kb.right)]
                        };
                        for &(name, sa, sb) in sides {
                            assert!(
                                (sa.0 - sb.0).abs() < 1e-4
                                    && (sa.1 - sb.1).abs() < 1e-3 * (1.0 + sa.1.abs()),
                                "{context}[{component}] key {index} {name} tangent {sa:?} vs {sb:?}"
                            );
                        }
                    }
                }
            }
            _ => panic!("{context}[{component}]: one side lacks the curve"),
        }
    }
}

/// The dog's clips come back as the same stacks, layers and authored curves,
/// and bake to the same tracks.
#[test]
fn animation_curves_and_baked_clips_survive() {
    let Some((model, extras)) = fixture("AN_ZombiedogLocomotion.fbx") else {
        return;
    };
    assert!(!model.animations.is_empty());
    let dir = temp_dir("extras_animation");
    let (_, loaded, loaded_extras) = round_trip(&model, &extras, &dir, FbxFormat::Binary);

    assert_eq!(
        loaded.animations.len(),
        model.animations.len(),
        "clip count"
    );
    assert_eq!(
        loaded_extras.animations.len(),
        extras.animations.len(),
        "stack count"
    );
    for (source, back) in extras.animations.iter().zip(&loaded_extras.animations) {
        assert_eq!(source.name, back.name);
        assert!(
            (source.time_begin - back.time_begin).abs() < 1e-6,
            "{}",
            source.name
        );
        assert!(
            (source.time_end - back.time_end).abs() < 1e-6,
            "{}",
            source.name
        );
        assert_eq!(
            source.layers.len(),
            back.layers.len(),
            "{} layers",
            source.name
        );
    }

    let source_curves = curves_by_target(&model, &extras);
    let back_curves = curves_by_target(&loaded, &loaded_extras);
    let mut compared = 0;
    for (key, anim) in &source_curves {
        let back = back_curves
            .get(key)
            .unwrap_or_else(|| panic!("animated property {key:?} was not written"));
        assert_curves_equal(anim, back, &format!("{key:?}"));
        compared += 1;
    }
    assert!(compared > 0, "the fixture animates something");

    // The reader bakes both files the same way, so the baked tracks agree.
    for (clip, back) in model.animations.iter().zip(&loaded.animations) {
        assert_eq!(clip.name, back.name);
        assert_eq!(clip.tracks.len(), back.tracks.len(), "{} tracks", clip.name);
        for track in &clip.tracks {
            let node = &model.nodes[track.node as usize].name;
            let back_track = back
                .tracks
                .iter()
                .find(|candidate| &loaded.nodes[candidate.node as usize].name == node)
                .unwrap_or_else(|| panic!("{}: no track for {node}", clip.name));
            assert_eq!(
                track.translation.len(),
                back_track.translation.len(),
                "{node} translation keys"
            );
            for (a, b) in track.translation.iter().zip(&back_track.translation) {
                assert!(
                    a.value.distance(b.value) < 1e-4,
                    "{node} translation {a:?} vs {b:?}"
                );
            }
            for (a, b) in track.rotation.iter().zip(&back_track.rotation) {
                assert!(
                    a.value.dot(b.value).abs() > 1.0 - 1e-4,
                    "{node} rotation {a:?} vs {b:?}"
                );
            }
            for (a, b) in track.scale.iter().zip(&back_track.scale) {
                assert!(
                    a.value.distance(b.value) < 1e-4,
                    "{node} scale {a:?} vs {b:?}"
                );
            }
        }
    }
}

/// Display layers keep their members and colors.
#[test]
fn display_layers_survive() {
    let Some((model, extras)) = fixture("AN_ZombiedogLocomotion.fbx") else {
        return;
    };
    assert!(
        !extras.display_layers.is_empty(),
        "the fixture carries display layers"
    );
    let dir = temp_dir("extras_display_layers");
    let (_, loaded, loaded_extras) = round_trip(&model, &extras, &dir, FbxFormat::Binary);
    assert_eq!(
        loaded_extras.display_layers.len(),
        extras.display_layers.len()
    );
    for layer in &extras.display_layers {
        let back = loaded_extras
            .display_layers
            .iter()
            .find(|candidate| candidate.name == layer.name)
            .unwrap_or_else(|| panic!("display layer {:?} was not written", layer.name));
        assert_eq!(back.visible, layer.visible, "{}", layer.name);
        assert_eq!(back.frozen, layer.frozen, "{}", layer.name);
        assert!(
            back.ui_color.distance(layer.ui_color) < 1e-6,
            "{}",
            layer.name
        );
        let mut members: Vec<&str> = layer
            .nodes
            .iter()
            .map(|&n| model.nodes[n as usize].name.as_str())
            .collect();
        let mut back_members: Vec<&str> = back
            .nodes
            .iter()
            .map(|&n| loaded.nodes[n as usize].name.as_str())
            .collect();
        members.sort_unstable();
        back_members.sort_unstable();
        assert_eq!(members, back_members, "{}", layer.name);
    }
}

/// The probe's animation (a translation with weighted user tangents, a linear
/// user property, a material color, a stepped visibility), its display layer
/// and its selection set with vertex and face members.
#[cfg(has_ufbxw_probe)]
#[test]
fn synthetic_animation_selection_and_layer_survive() {
    let dir = temp_dir("extras_probe_anim");
    let probe_path = dir.join("probe.fbx");
    review_optimize::probe::write_patch_probe(&probe_path, false).expect("probe written");
    let Some((model, extras)) = load(&probe_path) else {
        return;
    };
    let (_, loaded, loaded_extras) = round_trip(&model, &extras, &dir, FbxFormat::Ascii);

    let source_curves = curves_by_target(&model, &extras);
    let back_curves = curves_by_target(&loaded, &loaded_extras);
    for key in [
        ("node:Locator", "Lcl Translation"),
        ("node:Props", "MyNumber"),
        ("material:Mat", "DiffuseColor"),
        ("node:Quad", "Visibility"),
    ] {
        let key = (key.0.to_owned(), key.1.to_owned());
        let source = source_curves
            .get(&key)
            .unwrap_or_else(|| panic!("probe lacks {key:?}"));
        let back = back_curves
            .get(&key)
            .unwrap_or_else(|| panic!("{key:?} was not written"));
        assert_curves_equal(source, back, &format!("{key:?}"));
    }
    let translation = &back_curves[&("node:Locator".to_owned(), "Lcl Translation".to_owned())];
    let curve = translation.curves[0].as_ref().unwrap();
    assert_eq!(
        curve.post.mode,
        review_model::extras::ExtrapolationMode::Repeat
    );
    assert_eq!(curve.post.repeat_count, 3);
    let visibility = &back_curves[&("node:Quad".to_owned(), "Visibility".to_owned())];
    let steps: Vec<_> = visibility.curves[0]
        .as_ref()
        .unwrap()
        .keys
        .iter()
        .map(|k| k.interpolation)
        .collect();
    assert_eq!(
        steps,
        [
            review_model::extras::Interpolation::ConstantPrev,
            review_model::extras::Interpolation::ConstantNext,
            review_model::extras::Interpolation::ConstantPrev
        ]
    );
    assert_eq!(loaded.animations.len(), 1);
    assert_eq!(loaded.animations[0].name, "Take");

    let layer = loaded_extras
        .display_layers
        .iter()
        .find(|layer| layer.name == "Layer1")
        .expect("display layer written");
    let mut members: Vec<&str> = layer
        .nodes
        .iter()
        .map(|&n| loaded.nodes[n as usize].name.as_str())
        .collect();
    members.sort_unstable();
    assert_eq!(members, ["Locator", "Quad"]);
    assert!(layer.ui_color.distance(glam::Vec3::new(0.25, 0.5, 0.75)) < 1e-6);

    let set = loaded_extras
        .selection_sets
        .iter()
        .find(|set| set.name == "Sel")
        .expect("selection set written");
    assert_eq!(set.nodes.len(), 1);
    let entry = &set.nodes[0];
    assert_eq!(loaded.nodes[entry.node.unwrap() as usize].name, "Quad");
    assert!(entry.include_node);
    // The selected source vertices 0 and 2 are the positions (0,0,0) and (1,1,0);
    // the export split vertex 2 (two corners, two colors), so both copies are in.
    let part = loaded_extras.mesh_of_node(entry.node.unwrap()).unwrap();
    let mut selected: Vec<[i32; 3]> = entry
        .vertices
        .iter()
        .map(|&logical| {
            let corner = loaded
                .corner_to_logical
                .iter()
                .position(|&c| c == logical)
                .unwrap();
            let p = (loaded.vertices[corner].position * 1000.0).round();
            [p.x as i32, p.y as i32, p.z as i32]
        })
        .collect();
    selected.sort_unstable();
    selected.dedup();
    let unit = (1.0f32 * loaded.stats.source_unit_meters * 1000.0).round() as i32;
    assert_eq!(selected, [[0, 0, 0], [unit, unit, 0]]);
    let _ = part;
    assert_eq!(entry.faces.len(), 1);
    assert_eq!(
        entry.faces[0] as usize,
        part.face_first as usize + 1,
        "the second quad"
    );
}
