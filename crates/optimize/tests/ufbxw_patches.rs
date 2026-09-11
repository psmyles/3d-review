//! Read-back checks for the review patches carried on the vendored ufbx_write
//! (`third_party/ufbx-write/review.patch`).
//!
//! `src/ufbxw_probe.c` writes one scene using every patched entry point. Each
//! test here reads it back two ways: through the vendored ufbx reader (the file
//! must load, and the parts the importer already surfaces must be right) and,
//! for the ASCII form, by inspecting the text the writer produced — the
//! properties, layers and object types the importer does not yet expose. The
//! full round trip of those through `SourceExtras` lives in
//! `extras_round_trip.rs`.

#![cfg(has_ufbxw_probe)]

use std::path::{Path, PathBuf};

use review_model::{ModelData, NodeKind};
use review_optimize::probe::write_patch_probe;

fn temp_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the temp directory");
    dir
}

/// Write the probe scene and read it back, or `None` when this checkout cannot
/// import FBX at all.
fn probe(dir: &Path, ascii: bool) -> Option<(PathBuf, ModelData)> {
    let path = dir.join(if ascii {
        "probe_ascii.fbx"
    } else {
        "probe_binary.fbx"
    });
    write_patch_probe(&path, ascii).unwrap_or_else(|error| panic!("probe write failed: {error}"));
    match review_import::load_model(&path) {
        Ok(model) => Some((path, model)),
        Err(review_import::ImportError::UfbxUnavailable) => {
            eprintln!("skipping: FBX import is unavailable in this build");
            None
        }
        Err(error) => panic!("could not read back {}: {error}", path.display()),
    }
}

fn ascii_text(dir: &Path) -> Option<String> {
    let (path, _) = probe(dir, true)?;
    Some(std::fs::read_to_string(&path).expect("read the ASCII probe"))
}

/// The first line containing `needle`, or a panic naming it.
fn line_with<'a>(text: &'a str, needle: &str) -> &'a str {
    text.lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no line contains {needle:?}"))
}

/// The first line containing every needle. A property template in the
/// Definitions block precedes the element that overrides it, so a value check
/// has to name the value as well as the property.
fn line_with_all<'a>(text: &'a str, needles: &[&str]) -> &'a str {
    text.lines()
        .find(|line| needles.iter().all(|needle| line.contains(needle)))
        .unwrap_or_else(|| panic!("no line contains all of {needles:?}"))
}

fn node<'a>(model: &'a ModelData, name: &str) -> &'a review_model::SceneNode {
    model
        .nodes
        .iter()
        .find(|node| node.name == name)
        .unwrap_or_else(|| panic!("no node named {name:?}"))
}

#[test]
fn the_probe_reads_back_in_both_formats() {
    let dir = temp_dir("ufbxw_patches_formats");
    for ascii in [false, true] {
        let Some((_, model)) = probe(&dir, ascii) else {
            return;
        };
        // P2: the Null attribute is what makes a node an Empty to the reader.
        assert_eq!(node(&model, "Locator").kind, NodeKind::Empty);
        assert_eq!(node(&model, "Cam").kind, NodeKind::Camera);
        assert_eq!(node(&model, "Quad").kind, NodeKind::Mesh);
        // P4: the LOD group and its levels are ordinary nodes to the reader.
        for name in ["LodRoot", "LodRoot_LOD0", "LodRoot_LOD1", "LodRoot_LOD2"] {
            node(&model, name);
        }
        // P3: two quads, still two polygons after the layers ride along.
        assert_eq!(model.stats.polygon_count, 2, "ascii={ascii}");
        assert_eq!(model.stats.triangle_count, 4, "ascii={ascii}");
        assert_eq!(model.stats.vertex_count, 6, "ascii={ascii}");
    }
}

#[test]
fn the_written_version_is_7700() {
    let dir = temp_dir("ufbxw_patches_version");
    let Some(text) = ascii_text(&dir) else {
        return;
    };
    assert!(line_with(&text, "FBXVersion:").contains("7700"));
}

#[test]
fn user_property_flags_are_written() {
    let dir = temp_dir("ufbxw_patches_flags");
    let Some(text) = ascii_text(&dir) else {
        return;
    };
    // P1: the `U` flag, joined with `+` as the SDK writes it.
    let int_line = line_with(&text, "\"MyInt\"");
    assert!(
        int_line.contains("\"int\", \"Integer\", \"U\""),
        "{int_line}"
    );
    assert!(int_line.trim_end().ends_with(",7"), "{int_line}");

    let number_line = line_with(&text, "\"MyNumber\"");
    assert!(
        number_line.contains("\"double\", \"Number\", \"A+U\""),
        "{number_line}"
    );
    assert!(number_line.contains("2.5"), "{number_line}");

    let note_line = line_with(&text, "\"MyNote\"");
    assert!(
        note_line.contains("\"KString\", \"\", \"U\""),
        "{note_line}"
    );
    assert!(note_line.contains("\"hello\""), "{note_line}");

    let hidden_line = line_with(&text, "\"MyHidden\"");
    assert!(
        hidden_line.contains("\"double\", \"Number\", \"H\""),
        "{hidden_line}"
    );

    let vector_line = line_with(&text, "\"MyVector\"");
    assert!(
        vector_line.contains("\"Vector\", \"\", \"U\""),
        "{vector_line}"
    );

    // P1: blob values are written rather than asserted on.
    let blob_line = line_with(&text, "\"MyBlob\"");
    assert!(blob_line.contains("\"Blob\", \"\", \"U\""), "{blob_line}");
    assert!(blob_line.contains("blob!"), "{blob_line}");

    // A template property keeps its value when re-flagged.
    let translation_line = line_with(
        &text,
        "\"Lcl Translation\", \"Lcl Translation\", \"\", \"A+U\"",
    );
    assert!(
        translation_line.contains("4") && translation_line.contains("6"),
        "{translation_line}"
    );
}

#[test]
fn the_null_attribute_is_written() {
    let dir = temp_dir("ufbxw_patches_null");
    let Some(text) = ascii_text(&dir) else {
        return;
    };
    // P2: object header sub-type, TypeFlags and the Size property.
    assert!(text.contains("\"NodeAttribute::"), "{text}");
    line_with(&text, "TypeFlags: \"Null\"");
    // The template's default (100) precedes the element's own value.
    line_with_all(&text, &["\"Size\", \"double\", \"Number\"", "42.5"]);
    // The Definitions carry the template so readers can fill defaults.
    line_with(&text, "\"FbxNull\"");
    // The camera gains its TypeFlags too.
    line_with(&text, "TypeFlags: \"Camera\"");
}

#[test]
fn the_topology_layers_are_written() {
    let dir = temp_dir("ufbxw_patches_layers");
    let Some(text) = ascii_text(&dir) else {
        return;
    };
    // P3: every layer element with its mapping and value array.
    for (element, values, count) in [
        ("LayerElementEdgeCrease", "EdgeCrease", 7),
        ("LayerElementVertexCrease", "VertexCrease", 6),
        ("LayerElementHole", "Hole", 2),
        ("LayerElementVisibility", "Visibility", 7),
        ("LayerElementPolygonGroup", "PolygonGroup", 2),
        ("LayerElementSmoothing", "Smoothing", 7),
    ] {
        line_with(&text, &format!("{element}: 0"));
        line_with(&text, &format!("{values}: *{count}"));
        line_with(&text, &format!("Type: \"{element}\""));
    }
    assert!(line_with(&text, "Edges: *").contains("*7"));
    for mapping in ["\"ByEdge\"", "\"ByVertice\"", "\"ByPolygon\""] {
        line_with(&text, &format!("MappingInformationType: {mapping}"));
    }
}

#[test]
fn the_lod_group_is_written() {
    let dir = temp_dir("ufbxw_patches_lod");
    let Some(text) = ascii_text(&dir) else {
        return;
    };
    // P4: the LodGroup sub-type and its per-level properties. FBX allows one
    // property template per object type and the Null took `NodeAttribute`'s,
    // so the group's values are all written on the element itself — which is
    // what the reader derives its levels from.
    line_with(&text, "\"NodeAttribute::\", \"LodGroup\"");
    let threshold0 = line_with(&text, "\"Thresholds|Level0\", \"Distance\"");
    assert!(
        threshold0.contains("120") && threshold0.contains("\"cm\""),
        "{threshold0}"
    );
    let threshold1 = line_with(&text, "\"Thresholds|Level1\", \"Distance\"");
    assert!(threshold1.contains("480"), "{threshold1}");
    assert!(
        !text.contains("\"Thresholds|Level2\""),
        "the first level has no threshold"
    );
    for (level, display) in [(0, 0), (1, 1), (2, 2)] {
        let line = line_with(&text, &format!("\"DisplayLevels|Level{level}\", \"enum\""));
        assert!(line.trim_end().ends_with(&format!(",{display}")), "{line}");
    }
    line_with(&text, "\"MinMaxDistance\", \"bool\"");
    let min_line = line_with(&text, "\"MinDistance\", \"double\"");
    assert!(min_line.contains(",5"), "{min_line}");
    let max_line = line_with(&text, "\"MaxDistance\", \"double\"");
    assert!(max_line.contains("600"), "{max_line}");
    line_with(&text, "\"WorldSpace\", \"bool\"");
}

#[test]
fn the_layered_texture_is_written() {
    let dir = temp_dir("ufbxw_patches_layered");
    let Some(text) = ascii_text(&dir) else {
        return;
    };
    // P4: its own object type, the template, the per-layer arrays and the
    // connections: two file textures into the layered one, which feeds the
    // material property.
    line_with(&text, "ObjectType: \"LayeredTexture\"");
    line_with(&text, "\"FbxLayeredTexture\"");
    line_with(&text, "\"LayeredTexture::Layered\"");
    line_with(&text, "BlendModes: *2");
    line_with(&text, "Alphas: *2");
    line_with(&text, "\"Texture::Base\"");
    line_with(&text, "\"Texture::Detail\"");
    line_with(&text, "FileName: \"C:/tex/base.png\"");
    // The layered texture itself has no file name (the Takes section names
    // its `.tak` file with the same key, so count the texture paths).
    assert_eq!(text.matches("FileName: \"C:/tex/").count(), 2, "{text}");
    line_with(&text, "\"DiffuseColor\"");
}

#[test]
fn curve_nodes_keep_only_the_curves_asked_for() {
    let dir = temp_dir("ufbxw_patches_masked");
    let Some((path, _)) = probe(&dir, false) else {
        return;
    };
    // P5: `Lcl Scaling` on Props has a curve on Y alone, `Lcl Rotation` none
    // at all; both curve nodes come back with their defaults and nothing
    // else, and neither made ufbx add a scale helper.
    let (model, extras) = match review_import::load_model_full(&path) {
        Ok((model, Some(extras))) => (model, extras),
        Ok((_, None)) => panic!("no extras captured"),
        Err(error) => panic!("{error}"),
    };
    assert!(
        model
            .nodes
            .iter()
            .all(|node| !node.name.is_empty() || node.parent.is_none()),
        "a scale helper was created: {:?}",
        model
            .nodes
            .iter()
            .map(|node| &node.name)
            .collect::<Vec<_>>()
    );
    let props = model
        .nodes
        .iter()
        .position(|node| node.name == "Props")
        .expect("Props node");
    let layer = extras.anim_layers.first().expect("one layer");
    let find = |prop: &str| {
        layer
            .anim
            .iter()
            .find(|anim| {
                anim.prop_name == prop
                    && anim.target == review_model::extras::ElementRef::Node(props as u32)
            })
            .unwrap_or_else(|| panic!("no curve node for {prop}"))
    };
    let scaling = find("Lcl Scaling");
    assert!(scaling.curves[0].is_none(), "X has no curve");
    assert_eq!(
        scaling.curves[1].as_ref().map(|curve| curve.keys.len()),
        Some(2),
        "Y has the two keys"
    );
    assert!(scaling.curves[2].is_none(), "Z has no curve");
    assert_eq!(scaling.default, glam::Vec3::ONE);
    let rotation = find("Lcl Rotation");
    assert!(rotation.curves.iter().all(Option::is_none));
    assert_eq!(rotation.default, glam::Vec3::new(0.0, 90.0, 0.0));
}
