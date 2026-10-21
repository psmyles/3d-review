//! Import checks over scenes written here as ASCII FBX, so each one pins exactly
//! the property it is about and none of them needs a fixture on disk or skips.
//!
//! Every scene is one or two quads in the XY plane (normal +Z, UVs along X/Y)
//! under nodes whose transform is the thing under test: a centimetre unit, a
//! non-uniform scale, a mirror.

#![cfg(has_ufbx)]

use std::path::PathBuf;

use glam::Vec3;
use review_import::{ImportError, load_model, load_model_full, load_model_staged};
use review_model::ModelData;

/// One second in FBX time units.
const KTIME_SECOND: i64 = 46_186_158_000;

/// Write `text` to a temp `.fbx` under cargo's own temp directory and return
/// its path.
fn temp_fbx(name: &str, text: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("review-import-synthetic-{name}.fbx"));
    std::fs::write(&path, text).expect("write temp fixture");
    path
}

/// A whole ASCII FBX file: `unit_scale_factor` is FBX's centimetres per unit
/// (1 = a centimetre file, 100 = metres).
fn scene(unit_scale_factor: f64, objects: &str, connections: &str) -> String {
    format!(
        r#"; FBX 7.4.0 project file
FBXHeaderExtension:  {{
	FBXHeaderVersion: 1003
	FBXVersion: 7400
}}
GlobalSettings:  {{
	Version: 1000
	Properties70:  {{
		P: "UpAxis", "int", "Integer", "",1
		P: "UpAxisSign", "int", "Integer", "",1
		P: "FrontAxis", "int", "Integer", "",2
		P: "FrontAxisSign", "int", "Integer", "",1
		P: "CoordAxis", "int", "Integer", "",0
		P: "CoordAxisSign", "int", "Integer", "",1
		P: "UnitScaleFactor", "double", "Number", "",{unit_scale_factor}
		P: "OriginalUnitScaleFactor", "double", "Number", "",{unit_scale_factor}
		P: "TimeMode", "enum", "", "",6
	}}
}}
Objects:  {{
{objects}
}}
Connections:  {{
{connections}
}}
"#
    )
}

/// The tangent and binormal layers of a quad, one value per polygon vertex.
fn tangent_layers(tangent: [f64; 3], binormal: Option<[f64; 3]>) -> String {
    let repeat = |v: [f64; 3]| {
        std::iter::repeat_n(format!("{},{},{}", v[0], v[1], v[2]), 4)
            .collect::<Vec<_>>()
            .join(",")
    };
    let mut text = format!(
        r#"		LayerElementTangent: 0 {{
			Version: 102
			Name: "map1"
			MappingInformationType: "ByPolygonVertex"
			ReferenceInformationType: "Direct"
			Tangents: *12 {{
				a: {}
			}}
		}}
"#,
        repeat(tangent)
    );
    if let Some(binormal) = binormal {
        text.push_str(&format!(
            r#"		LayerElementBinormal: 0 {{
			Version: 102
			Name: "map1"
			MappingInformationType: "ByPolygonVertex"
			ReferenceInformationType: "Direct"
			Binormals: *12 {{
				a: {}
			}}
		}}
"#,
            repeat(binormal)
        ));
    }
    text
}

/// A unit quad geometry with normals, one UV set, `extra_layers` and a `Layer`
/// block binding `layer_types` (beyond normal and UV) to layer 0.
fn quad_geometry(id: u64, extra_layers: &str, layer_types: &[&str]) -> String {
    let mut bindings = String::new();
    for kind in ["LayerElementNormal", "LayerElementUV"]
        .iter()
        .chain(layer_types)
    {
        bindings.push_str(&format!(
            "\t\t\tLayerElement:  {{\n\t\t\t\tType: \"{kind}\"\n\t\t\t\tTypedIndex: 0\n\t\t\t}}\n"
        ));
    }
    format!(
        r#"	Geometry: {id}, "Geometry::Quad{id}", "Mesh" {{
		Vertices: *12 {{
			a: 0,0,0,1,0,0,1,1,0,0,1,0
		}}
		PolygonVertexIndex: *4 {{
			a: 0,1,2,-4
		}}
		LayerElementNormal: 0 {{
			Version: 101
			Name: ""
			MappingInformationType: "ByPolygonVertex"
			ReferenceInformationType: "Direct"
			Normals: *12 {{
				a: 0,0,1,0,0,1,0,0,1,0,0,1
			}}
		}}
		LayerElementUV: 0 {{
			Version: 101
			Name: "map1"
			MappingInformationType: "ByPolygonVertex"
			ReferenceInformationType: "Direct"
			UV: *8 {{
				a: 0,0,1,0,1,1,0,1
			}}
		}}
{extra_layers}		Layer: 0 {{
			Version: 100
{bindings}		}}
	}}
"#
    )
}

/// A mesh node with `properties` lines in its `Properties70`.
fn mesh_model(id: u64, name: &str, properties: &str) -> String {
    format!(
        r#"	Model: {id}, "Model::{name}", "Mesh" {{
		Version: 232
		Properties70:  {{
{properties}		}}
	}}
"#
    )
}

fn scaling(scale: [f64; 3]) -> String {
    format!(
        "\t\t\tP: \"Lcl Scaling\", \"Lcl Scaling\", \"\", \"A\",{},{},{}\n",
        scale[0], scale[1], scale[2]
    )
}

fn load(name: &str, text: &str) -> ModelData {
    let path = temp_fbx(name, text);
    let model = load_model(&path);
    let _ = std::fs::remove_file(&path);
    model.unwrap_or_else(|error| panic!("{name} must import: {error}"))
}

/// The corners of the mesh on node `name`.
fn corners_of<'a>(model: &'a ModelData, name: &str) -> Vec<&'a review_model::Vertex> {
    let node = model
        .nodes
        .iter()
        .position(|node| node.name == name)
        .unwrap_or_else(|| panic!("no node {name}"));
    let mut seen = std::collections::BTreeSet::new();
    for (triangle, &owner) in model.triangles.node.iter().enumerate() {
        if owner as usize == node {
            seen.extend(
                model.indices[triangle * 3..triangle * 3 + 3]
                    .iter()
                    .copied(),
            );
        }
    }
    seen.into_iter()
        .map(|index| &model.vertices[index as usize])
        .collect()
}

/// A tangent is a direction along the surface, so it rides the node's linear
/// map like a position does — not the normal matrix. Under a non-uniform scale
/// the two disagree: the authored `(1, 1, 0) / sqrt 2` stretched by `(1, 2, 1)`
/// points at `(1, 2, 0) / sqrt 5`, which is still the UV direction it was
/// authored along, while the normal matrix bent it the other way, to
/// `(2, 1, 0) / sqrt 5`.
#[test]
fn a_tangent_rides_the_positions_map_under_a_non_uniform_scale() {
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let objects = quad_geometry(
        1000,
        &tangent_layers([s, s, 0.0], Some([-s, s, 0.0])),
        &["LayerElementTangent", "LayerElementBinormal"],
    ) + &mesh_model(2000, "Stretched", &scaling([1.0, 2.0, 1.0]));
    let model = load(
        "stretched-tangent",
        &scene(
            100.0,
            &objects,
            "\tC: \"OO\",1000,2000\n\tC: \"OO\",2000,0\n",
        ),
    );

    let expected = Vec3::new(1.0, 2.0, 0.0).normalize();
    for vertex in corners_of(&model, "Stretched") {
        assert!(
            vertex.tangent.truncate().abs_diff_eq(expected, 1e-5),
            "tangent {:?}, expected {expected:?}",
            vertex.tangent
        );
        assert!(vertex.normal.abs_diff_eq(Vec3::Z, 1e-5));
        assert_eq!(vertex.tangent.w, 1.0, "the authored basis is right-handed");
    }
}

/// A mirroring node turns a right-handed basis left-handed in world space, so
/// a tangent layer without a bitangent — right-handed in the geometry's own
/// space by convention — has to arrive with `w = -1`, exactly as the same basis
/// spelled out with its bitangent does.
#[test]
fn a_mirrored_node_flips_the_handedness_of_an_authored_tangent() {
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let objects = quad_geometry(
        1000,
        &tangent_layers([s, s, 0.0], None),
        &["LayerElementTangent"],
    ) + &quad_geometry(
        1001,
        &tangent_layers([s, s, 0.0], Some([-s, s, 0.0])),
        &["LayerElementTangent", "LayerElementBinormal"],
    ) + &mesh_model(2000, "MirroredBare", &scaling([-1.0, 1.0, 1.0]))
        + &mesh_model(2001, "MirroredFull", &scaling([-1.0, 1.0, 1.0]));
    let connections =
        "\tC: \"OO\",1000,2000\n\tC: \"OO\",2000,0\n\tC: \"OO\",1001,2001\n\tC: \"OO\",2001,0\n";
    let model = load("mirrored-tangent", &scene(100.0, &objects, connections));

    for name in ["MirroredBare", "MirroredFull"] {
        for vertex in corners_of(&model, name) {
            assert!(
                vertex
                    .tangent
                    .truncate()
                    .abs_diff_eq(Vec3::new(-1.0, 1.0, 0.0).normalize(), 1e-5),
                "{name}: tangent {:?}",
                vertex.tangent
            );
            assert_eq!(
                vertex.tangent.w, -1.0,
                "{name}: a mirrored right-handed basis is left-handed in world space"
            );
        }
    }
}

/// One mesh with a tangent layer and one without, in one file: the first keeps
/// its authored tangents and the second gains real ones. Asking whether *every*
/// tangent was missing used to leave the second mesh with none at all.
#[test]
fn a_mesh_without_tangents_gains_them_beside_one_that_has_them() {
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let objects = quad_geometry(
        1000,
        &tangent_layers([s, s, 0.0], Some([-s, s, 0.0])),
        &["LayerElementTangent", "LayerElementBinormal"],
    ) + &quad_geometry(1001, "", &[])
        + &mesh_model(2000, "Authored", "")
        + &mesh_model(2001, "Bare", "");
    let connections =
        "\tC: \"OO\",1000,2000\n\tC: \"OO\",2000,0\n\tC: \"OO\",1001,2001\n\tC: \"OO\",2001,0\n";
    let model = load("two-part-tangents", &scene(100.0, &objects, connections));

    assert!(!model.has_degenerate_tangents(), "every vertex has a basis");
    for vertex in corners_of(&model, "Authored") {
        assert!(
            vertex
                .tangent
                .truncate()
                .abs_diff_eq(Vec3::new(1.0, 1.0, 0.0).normalize(), 1e-5),
            "the authored tangent is kept as written: {:?}",
            vertex.tangent
        );
    }
    for vertex in corners_of(&model, "Bare") {
        assert!(
            vertex.tangent.truncate().abs_diff_eq(Vec3::X, 1e-5),
            "a synthesized tangent follows +U: {:?}",
            vertex.tangent
        );
    }
}

/// A quad carrying a blend shape that lifts corner 2 by 10 units and bends its
/// normal by `normal_delta`, in a file of `unit_scale_factor`.
fn blend_shape_scene(unit_scale_factor: f64, normal_delta: [f64; 3]) -> String {
    let [nx, ny, nz] = normal_delta;
    let objects = quad_geometry(1000, "", &[])
        + &mesh_model(2000, "Face", "")
        + &format!(
            r#"	Geometry: 3000, "Geometry::Smile", "Shape" {{
		Version: 100
		Indexes: *1 {{
			a: 2
		}}
		Vertices: *3 {{
			a: 0,0,10
		}}
		Normals: *3 {{
			a: {nx},{ny},{nz}
		}}
	}}
	Deformer: 4000, "Deformer::Blend", "BlendShape" {{
		Version: 100
	}}
	Deformer: 4001, "SubDeformer::Smile", "BlendShapeChannel" {{
		Version: 100
		DeformPercent: 0
		FullWeights: *1 {{
			a: 100
		}}
	}}
"#
        );
    let connections = "\tC: \"OO\",1000,2000\n\tC: \"OO\",2000,0\n\tC: \"OO\",4000,1000\n\
                       \tC: \"OO\",4001,4000\n\tC: \"OO\",3000,4001\n";
    scene(unit_scale_factor, &objects, connections)
}

/// A blend shape's normal delta keeps the strength the file gave it whatever
/// unit the file is in. Import parks a centimetre file's 0.01 in the node
/// transforms, and the delta used to ride the unnormalized normal matrix, which
/// scales by the square of that: a Maya-default facial rig moved its vertices
/// and kept 1/10000 of every normal change, so its shading never moved.
#[test]
fn a_blend_shape_normal_delta_is_independent_of_the_unit() {
    for (unit_scale_factor, label) in [(1.0, "centimetres"), (100.0, "metres")] {
        let model = load(
            &format!("blend-shape-{label}"),
            &blend_shape_scene(unit_scale_factor, [0.0, 0.5, 0.0]),
        );
        let morph = model.morph.as_ref().expect("the shape imports");
        assert_eq!(morph.position.len(), 1, "{label}");
        // The position offset is a length, so it does follow the unit...
        let metres = 10.0 * unit_scale_factor as f32 / 100.0;
        assert!(
            morph.position[0].abs_diff_eq(Vec3::new(0.0, 0.0, metres), 1e-5),
            "{label}: position offset {:?}",
            morph.position[0]
        );
        // ...and the normal delta, a difference of unit vectors, does not.
        assert!(
            morph.normal[0].abs_diff_eq(Vec3::new(0.0, 0.5, 0.0), 1e-5),
            "{label}: normal delta {:?}",
            morph.normal[0]
        );
    }
}

/// An animation key the file wrote as a number too large for a float (it reads
/// as infinity), and a blend shape offset likewise: each costs its own piece and
/// the import says so, rather than refusing the file.
#[test]
fn a_value_that_is_not_a_number_costs_its_piece_not_the_model() {
    let objects = quad_geometry(1000, "", &[])
        + &mesh_model(2000, "Prop", "")
        + &format!(
            r#"	Geometry: 3000, "Geometry::Broken", "Shape" {{
		Version: 100
		Indexes: *1 {{
			a: 2
		}}
		Vertices: *3 {{
			a: 0,0,1e999
		}}
	}}
	Deformer: 4000, "Deformer::Blend", "BlendShape" {{
		Version: 100
	}}
	Deformer: 4001, "SubDeformer::Broken", "BlendShapeChannel" {{
		Version: 100
		DeformPercent: 0
		FullWeights: *1 {{
			a: 100
		}}
	}}
	AnimationStack: 5000, "AnimStack::Slide", "" {{
		Properties70:  {{
			P: "LocalStart", "KTime", "Time", "",0
			P: "LocalStop", "KTime", "Time", "",{second}
		}}
	}}
	AnimationLayer: 5001, "AnimLayer::BaseLayer", "" {{
	}}
	AnimationCurveNode: 5002, "AnimCurveNode::T", "" {{
		Properties70:  {{
			P: "d|X", "Number", "", "A",0
			P: "d|Y", "Number", "", "A",0
			P: "d|Z", "Number", "", "A",0
		}}
	}}
	AnimationCurve: 5003, "AnimCurve::", "" {{
		Default: 0
		KeyVer: 4008
		KeyTime: *3 {{
			a: 0,{half},{second}
		}}
		KeyValueFloat: *3 {{
			a: 0,1e999,1
		}}
		KeyAttrFlags: *1 {{
			a: 4
		}}
		KeyAttrDataFloat: *4 {{
			a: 0,0,0,0
		}}
		KeyAttrRefCount: *1 {{
			a: 3
		}}
	}}
"#,
            second = KTIME_SECOND,
            half = KTIME_SECOND / 2,
        );
    let connections = "\tC: \"OO\",1000,2000\n\tC: \"OO\",2000,0\n\tC: \"OO\",4000,1000\n\
                       \tC: \"OO\",4001,4000\n\tC: \"OO\",3000,4001\n\tC: \"OO\",5001,5000\n\
                       \tC: \"OO\",5002,5001\n\tC: \"OP\",5002,2000, \"Lcl Translation\"\n\
                       \tC: \"OP\",5003,5002, \"d|X\"\n";
    let path = temp_fbx("not-a-number", &scene(100.0, &objects, connections));
    let staged = load_model_staged(&path, &|_| {});
    let full = load_model_full(&path);
    let _ = std::fs::remove_file(&path);

    let staged = staged.unwrap_or_else(|error| panic!("the model must still load: {error}"));
    let warnings = staged.warnings();
    assert!(
        warnings.iter().any(|line| line.contains("Slide")),
        "the clip's dropped key is reported: {warnings:?}"
    );
    assert!(
        warnings.iter().any(|line| line.contains("Broken")),
        "the dropped shape is reported: {warnings:?}"
    );
    let model = &staged.model;
    assert_eq!(model.validate_deform(), Ok(()));
    assert_eq!(model.animations.len(), 1, "the clip is kept");
    let track = &model.animations[0].tracks[0];
    assert!(
        track.translation.iter().all(|key| key.value.is_finite()),
        "no key that is not a number survives"
    );
    assert!(
        !track.translation.is_empty(),
        "the finite keys of the channel are kept"
    );

    // The capture survives too: its authored curve held the same key.
    let (_, extras) = full.unwrap_or_else(|error| panic!("the full load works: {error}"));
    assert!(extras.is_some(), "the source-property capture is kept");
}

/// ufbx hands a texture its video's embedded content *by pointer*, and the
/// capture used to copy it once for the texture and once for the video — twice
/// more in Rust — so every embedded image was held four times at the peak.
/// Stored once, it lives on the video, which is where FBX keeps it.
#[test]
fn embedded_content_is_stored_once_on_its_video() {
    let objects = quad_geometry(1000, "", &[])
        + &mesh_model(2000, "Crate", "")
        + r#"	Video: 5000, "Video::Image", "Clip" {
		Type: "Clip"
		Filename: "image.png"
		RelativeFilename: "image.png"
		Content: "aGVsbG8sIHRleHR1cmU="
	}
	Texture: 6000, "Texture::Image", "" {
		Type: "TextureVideoClip"
		FileName: "image.png"
		RelativeFilename: "image.png"
	}
	Texture: 6001, "Texture::Again", "" {
		Type: "TextureVideoClip"
		FileName: "image.png"
		RelativeFilename: "image.png"
	}
	Material: 7000, "Material::Wood", "" {
		Version: 102
		ShadingModel: "phong"
	}
"#;
    let connections = "\tC: \"OO\",1000,2000\n\tC: \"OO\",2000,0\n\tC: \"OO\",7000,2000\n\
                       \tC: \"OO\",5000,6000\n\tC: \"OO\",5000,6001\n\
                       \tC: \"OP\",6000,7000, \"DiffuseColor\"\n\
                       \tC: \"OP\",6001,7000, \"NormalMap\"\n";
    let path = temp_fbx("embedded", &scene(100.0, &objects, connections));
    let loaded = load_model_full(&path);
    let _ = std::fs::remove_file(&path);
    let (_, extras) = match loaded {
        Ok((model, Some(extras))) => (model, extras),
        Ok((_, None)) => panic!("the capture is present"),
        Err(ImportError::UfbxUnavailable) => return,
        Err(error) => panic!("the scene must import: {error}"),
    };
    assert_eq!(extras.videos.len(), 1);
    assert_eq!(extras.videos[0].content, b"hello, texture");
    assert_eq!(extras.textures.len(), 2);
    for texture in &extras.textures {
        assert_eq!(texture.video, Some(0), "{} rides the video", texture.name);
        assert!(
            texture.content.is_empty(),
            "{}: its content is the video's, not a copy of it",
            texture.name
        );
    }
}
