//! An FBX holding a skeleton and its clips but no mesh — an animation saved apart
//! from the character it drives — imports as a skeleton to be played, rather
//! than being refused for having nothing to triangulate.
//!
//! The scene is written here as ASCII FBX, so the suite needs no fixture on
//! disk and never skips: three bones in a vertical chain, half a metre apart in
//! a centimetre file, and one clip turning the middle bone a quarter turn about
//! Z over one second at 24 fps.

#![cfg(has_ufbx)]

use std::path::PathBuf;

use glam::Vec3;
use review_import::{ImportError, load_model, measure_clip_bounds};
use review_model::NodeKind;

/// One second in FBX time units.
const KTIME_SECOND: &str = "46186158000";

/// Write `text` to a temp `.fbx` and return its path. Under cargo's own temp
/// directory, so concurrent checkouts cannot delete each other's file.
fn temp_fbx(name: &str, text: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("review-import-animation-only-{name}.fbx"));
    std::fs::write(&path, text).expect("write temp fixture");
    path
}

/// The skeleton-and-clip scene described in the module docs.
fn skeleton_only_fbx() -> String {
    let mut text = String::from(
        r#"; FBX 7.4.0 project file
FBXHeaderExtension:  {
	FBXHeaderVersion: 1003
	FBXVersion: 7400
}
GlobalSettings:  {
	Version: 1000
	Properties70:  {
		P: "UpAxis", "int", "Integer", "",1
		P: "UpAxisSign", "int", "Integer", "",1
		P: "FrontAxis", "int", "Integer", "",2
		P: "FrontAxisSign", "int", "Integer", "",1
		P: "CoordAxis", "int", "Integer", "",0
		P: "CoordAxisSign", "int", "Integer", "",1
		P: "UnitScaleFactor", "double", "Number", "",1
		P: "OriginalUnitScaleFactor", "double", "Number", "",1
		P: "TimeMode", "enum", "", "",11
	}
}
Objects:  {
	NodeAttribute: 101, "NodeAttribute::", "LimbNode" {
		TypeFlags: "Skeleton"
	}
	NodeAttribute: 102, "NodeAttribute::", "LimbNode" {
		TypeFlags: "Skeleton"
	}
	NodeAttribute: 103, "NodeAttribute::", "LimbNode" {
		TypeFlags: "Skeleton"
	}
	Model: 201, "Model::Root", "LimbNode" {
		Version: 232
	}
	Model: 202, "Model::Mid", "LimbNode" {
		Version: 232
		Properties70:  {
			P: "Lcl Translation", "Lcl Translation", "", "A",0,50,0
		}
	}
	Model: 203, "Model::Tip", "LimbNode" {
		Version: 232
		Properties70:  {
			P: "Lcl Translation", "Lcl Translation", "", "A",0,50,0
		}
	}
	AnimationStack: 301, "AnimStack::Bend", "" {
		Properties70:  {
			P: "LocalStart", "KTime", "Time", "",0
			P: "LocalStop", "KTime", "Time", "",KTIME_SECOND
			P: "ReferenceStart", "KTime", "Time", "",0
			P: "ReferenceStop", "KTime", "Time", "",KTIME_SECOND
		}
	}
	AnimationLayer: 302, "AnimLayer::BaseLayer", "" {
	}
	AnimationCurveNode: 303, "AnimCurveNode::R", "" {
		Properties70:  {
			P: "d|X", "Number", "", "A",0
			P: "d|Y", "Number", "", "A",0
			P: "d|Z", "Number", "", "A",0
		}
	}
	AnimationCurve: 304, "AnimCurve::", "" {
		Default: 0
		KeyVer: 4008
		KeyTime: *2 {
			a: 0,KTIME_SECOND
		}
		KeyValueFloat: *2 {
			a: 0,90
		}
		KeyAttrFlags: *1 {
			a: 4
		}
		KeyAttrDataFloat: *4 {
			a: 0,0,0,0
		}
		KeyAttrRefCount: *1 {
			a: 2
		}
	}
}
Connections:  {
	C: "OO",201,0
	C: "OO",202,201
	C: "OO",203,202
	C: "OO",101,201
	C: "OO",102,202
	C: "OO",103,203
	C: "OO",302,301
	C: "OO",303,302
	C: "OP",303,202, "Lcl Rotation"
	C: "OP",304,303, "d|Z"
}
"#,
    );
    text = text.replace("KTIME_SECOND", KTIME_SECOND);
    text
}

#[test]
fn a_skeleton_with_clips_and_no_mesh_imports_as_a_skeleton() {
    let path = temp_fbx("skeleton", &skeleton_only_fbx());
    let model = load_model(&path);
    let _ = std::fs::remove_file(&path);
    let model = model.expect("an animation file with no mesh must import");

    assert!(!model.has_geometry(), "there is nothing to triangulate");
    assert!(!model.is_empty(), "but it is not the empty start state");
    let bones: Vec<&str> = model
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Bone)
        .map(|node| node.name.as_str())
        .collect();
    assert_eq!(bones, ["Root", "Mid", "Tip"]);
    assert_eq!(model.stats.bone_count, 3);

    // Framed on the joints: a centimetre file normalised to metres, so the
    // chain stands one metre tall.
    let bounds = model.bounds.expect("a skeleton is framed on its joints");
    assert!(bounds.min.abs_diff_eq(Vec3::ZERO, 1e-5), "{bounds:?}");
    assert!(bounds.max.abs_diff_eq(Vec3::Y, 1e-5), "{bounds:?}");

    assert_eq!(model.animations.len(), 1);
    let clip = &model.animations[0];
    assert_eq!(clip.name, "Bend");
    assert_eq!(clip.frame_rate, 24.0, "the clip keeps its file's rate");
    assert_eq!(clip.frame_count(), 25, "one second at 24 fps, both ends");
}

#[test]
fn a_skeleton_clips_envelope_follows_its_joints() {
    let path = temp_fbx("envelope", &skeleton_only_fbx());
    let model = load_model(&path);
    let _ = std::fs::remove_file(&path);
    let model = model.expect("import");

    // The middle bone turns +90 degrees about Z, swinging the tip from straight
    // above it to half a metre along -X.
    let envelopes = measure_clip_bounds(&model);
    let envelope = envelopes[0].expect("the joints move");
    assert!((envelope.min.x + 0.5).abs() < 1e-4, "{envelope:?}");
    assert!((envelope.max.y - 1.0).abs() < 1e-4, "{envelope:?}");
    assert!(envelope.min.y.abs() < 1e-4, "{envelope:?}");
}

#[test]
fn a_file_with_neither_mesh_nor_skeleton_is_refused() {
    // The skeleton scene with every bone turned into a plain null.
    let nulls = skeleton_only_fbx()
        .replace("TypeFlags: \"Skeleton\"", "TypeFlags: \"Null\"")
        .replace("\"LimbNode\"", "\"Null\"");
    assert!(!nulls.contains("LimbNode") && !nulls.contains("Skeleton"));
    let path = temp_fbx("nulls", &nulls);
    let result = load_model(&path);
    let _ = std::fs::remove_file(&path);
    match result {
        Err(ImportError::LoadFailed(message)) => {
            assert!(message.contains("no mesh or skeleton"), "{message}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}
