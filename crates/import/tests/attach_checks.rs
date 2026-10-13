//! An animation file in other units and axes than its rig plays the way an
//! engine importing both into one coordinate system would show it, and says so.
//!
//! Both files are written here as ASCII FBX, so the suite needs no fixture and
//! never skips. The rig is the one in `animation_only.rs`: a three-bone chain,
//! half a metre a bone, standing up +Y in a centimetre file. The animation file
//! is the same skeleton exported from a Z-up, metre scene: its root turned a
//! quarter about X to stand the chain up +Z, every offset in metres. It lifts
//! the root 10 cm and bends the middle bone a quarter turn about its own Z.

#![cfg(has_ufbx)]

use std::path::PathBuf;

use glam::Vec3;
use review_import::load_model;
use review_model::anim::{AnimContext, Pose, evaluate_pose};
use review_model::{AttachedAnimation, Axis, ModelData};

/// One second in FBX time units.
const KTIME_SECOND: &str = "46186158000";

fn temp_fbx(name: &str, text: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("review-import-attach-checks-{name}.fbx"));
    std::fs::write(&path, text).expect("write temp fixture");
    path
}

fn load(name: &str, text: &str) -> ModelData {
    let path = temp_fbx(name, text);
    let model = load_model(&path);
    let _ = std::fs::remove_file(&path);
    model.expect("import")
}

/// The FBX `GlobalSettings` for a scene `up` (1 = Y, 2 = Z) in a unit of
/// `unit_cm` centimetres.
fn global_settings(up: u8, unit_cm: f64) -> String {
    // A right-handed system either way: Y-up faces +Z, Z-up faces -Y.
    let (front, front_sign) = if up == 1 { (2, 1) } else { (1, -1) };
    format!(
        r#"GlobalSettings:  {{
	Version: 1000
	Properties70:  {{
		P: "UpAxis", "int", "Integer", "",{up}
		P: "UpAxisSign", "int", "Integer", "",1
		P: "FrontAxis", "int", "Integer", "",{front}
		P: "FrontAxisSign", "int", "Integer", "",{front_sign}
		P: "CoordAxis", "int", "Integer", "",0
		P: "CoordAxisSign", "int", "Integer", "",1
		P: "UnitScaleFactor", "double", "Number", "",{unit_cm}
		P: "OriginalUnitScaleFactor", "double", "Number", "",{unit_cm}
		P: "TimeMode", "enum", "", "",11
	}}
}}
"#
    )
}

/// The chain: `root_rotation` is the root's authored `Lcl Rotation`, `offset`
/// each child's distance from its parent, in the file's unit.
fn chain(root_rotation: &str, offset: f64) -> String {
    format!(
        r#"	NodeAttribute: 101, "NodeAttribute::", "LimbNode" {{
		TypeFlags: "Skeleton"
	}}
	NodeAttribute: 102, "NodeAttribute::", "LimbNode" {{
		TypeFlags: "Skeleton"
	}}
	NodeAttribute: 103, "NodeAttribute::", "LimbNode" {{
		TypeFlags: "Skeleton"
	}}
	Model: 201, "Model::Root", "LimbNode" {{
		Version: 232
		Properties70:  {{
			P: "Lcl Rotation", "Lcl Rotation", "", "A",{root_rotation}
		}}
	}}
	Model: 202, "Model::Mid", "LimbNode" {{
		Version: 232
		Properties70:  {{
			P: "Lcl Translation", "Lcl Translation", "", "A",0,{offset},0
		}}
	}}
	Model: 203, "Model::Tip", "LimbNode" {{
		Version: 232
		Properties70:  {{
			P: "Lcl Translation", "Lcl Translation", "", "A",0,{offset},0
		}}
	}}
"#
    )
}

const CHAIN_CONNECTIONS: &str = r#"	C: "OO",201,0
	C: "OO",202,201
	C: "OO",203,202
	C: "OO",101,201
	C: "OO",102,202
	C: "OO",103,203
"#;

/// A curve from `from` at 0 s to `to` at 1 s, linear.
fn curve(id: u32, from: f64, to: f64) -> String {
    format!(
        r#"	AnimationCurve: {id}, "AnimCurve::", "" {{
		Default: {from}
		KeyVer: 4008
		KeyTime: *2 {{
			a: 0,{KTIME_SECOND}
		}}
		KeyValueFloat: *2 {{
			a: {from},{to}
		}}
		KeyAttrFlags: *1 {{
			a: 4
		}}
		KeyAttrDataFloat: *4 {{
			a: 0,0,0,0
		}}
		KeyAttrRefCount: *1 {{
			a: 2
		}}
	}}
"#
    )
}

/// A curve node with all three components at `[x, y, z]`.
fn curve_node(id: u32, name: &str, [x, y, z]: [f64; 3]) -> String {
    format!(
        r#"	AnimationCurveNode: {id}, "AnimCurveNode::{name}", "" {{
		Properties70:  {{
			P: "d|X", "Number", "", "A",{x}
			P: "d|Y", "Number", "", "A",{y}
			P: "d|Z", "Number", "", "A",{z}
		}}
	}}
"#
    )
}

fn scene(settings: &str, objects: &str, connections: &str) -> String {
    format!(
        "; FBX 7.4.0 project file\nFBXHeaderExtension:  {{\n\tFBXHeaderVersion: 1003\n\t\
         FBXVersion: 7400\n}}\n{settings}Objects:  {{\n{objects}}}\nConnections:  {{\n\
         {connections}}}\n"
    )
}

/// The rig: centimetres, Y-up, standing straight up, no clip.
fn rig_fbx() -> String {
    scene(
        &global_settings(1, 1.0),
        &chain("0,0,0", 50.0),
        CHAIN_CONNECTIONS,
    )
}

/// The animation file: metres, Z-up, the root turned to stand the chain up +Z,
/// with the root lifted `lift` metres and the middle bone held at `mid` metres
/// from the root while it bends a quarter turn.
fn animation_fbx(lift: f64, mid: f64) -> String {
    let mut objects = chain("90,0,0", 0.5);
    objects.push_str(&format!(
        r#"	AnimationStack: 301, "AnimStack::Bend", "" {{
		Properties70:  {{
			P: "LocalStart", "KTime", "Time", "",0
			P: "LocalStop", "KTime", "Time", "",{KTIME_SECOND}
			P: "ReferenceStart", "KTime", "Time", "",0
			P: "ReferenceStop", "KTime", "Time", "",{KTIME_SECOND}
		}}
	}}
	AnimationLayer: 302, "AnimLayer::BaseLayer", "" {{
	}}
"#
    ));
    objects.push_str(&curve_node(310, "T", [0.0, 0.0, lift]));
    objects.push_str(&curve(311, lift, lift));
    objects.push_str(&curve_node(320, "R", [90.0, 0.0, 0.0]));
    objects.push_str(&curve(321, 90.0, 90.0));
    objects.push_str(&curve_node(330, "T", [0.0, mid, 0.0]));
    objects.push_str(&curve(331, mid, mid));
    objects.push_str(&curve_node(340, "R", [0.0, 0.0, 0.0]));
    objects.push_str(&curve(341, 0.0, 90.0));
    let mut connections = String::from(CHAIN_CONNECTIONS);
    connections.push_str(
        r#"	C: "OO",302,301
	C: "OO",310,302
	C: "OO",320,302
	C: "OO",330,302
	C: "OO",340,302
	C: "OP",310,201, "Lcl Translation"
	C: "OP",311,310, "d|Z"
	C: "OP",320,201, "Lcl Rotation"
	C: "OP",321,320, "d|X"
	C: "OP",330,202, "Lcl Translation"
	C: "OP",331,330, "d|Y"
	C: "OP",340,202, "Lcl Rotation"
	C: "OP",341,340, "d|Z"
"#,
    );
    scene(&global_settings(2, 100.0), &objects, &connections)
}

/// Where each joint of `model` is at `time` into `animation`'s first clip.
fn joints(model: &ModelData, animation: &AttachedAnimation, time: f64) -> Vec<Vec3> {
    let context = AnimContext::new(model);
    let mut pose = Pose::new(model);
    evaluate_pose(model, &context, Some(&animation.clips[0]), time, &mut pose);
    ["Root", "Mid", "Tip"]
        .into_iter()
        .map(|name| {
            let index = model
                .nodes
                .iter()
                .position(|node| node.name == name)
                .expect("the rig's joint");
            pose.world[index].w_axis.truncate()
        })
        .collect()
}

#[test]
fn the_files_declare_their_own_units_and_axes() {
    let rig = load("rig", &rig_fbx());
    let animation = load("anim", &animation_fbx(0.1, 0.5));
    assert!((rig.stats.source_unit_meters - 0.01).abs() < 1e-6);
    assert!((animation.stats.source_unit_meters - 1.0).abs() < 1e-6);
    let rig_axes = rig.stats.source_axes.expect("declared");
    let animation_axes = animation.stats.source_axes.expect("declared");
    assert_eq!(
        (rig_axes.up, rig_axes.front),
        (Axis::PositiveY, Axis::PositiveZ)
    );
    assert_eq!(
        (animation_axes.up, animation_axes.front),
        (Axis::PositiveZ, Axis::NegativeY)
    );
}

/// The root lifts 10 cm *up* — the file's +Z is the rig's +Y — and the tip
/// swings from straight above the middle joint to half a metre along -X: the
/// motion the file authored, in the rig's metres and axes.
#[test]
fn a_metre_z_up_clip_plays_on_a_centimetre_y_up_rig_as_authored() {
    let rig = load("rig-play", &rig_fbx());
    let animation = AttachedAnimation::new(
        &rig,
        load("anim-play", &animation_fbx(0.1, 0.5)),
        "anim".to_owned(),
    );

    let start = joints(&rig, &animation, 0.0);
    let expected = [
        Vec3::new(0.0, 0.1, 0.0),
        Vec3::new(0.0, 0.6, 0.0),
        Vec3::new(0.0, 1.1, 0.0),
    ];
    for (joint, want) in start.iter().zip(expected) {
        assert!(joint.abs_diff_eq(want, 1e-4), "start: {start:?}");
    }
    let end = joints(&rig, &animation, 1.0);
    assert!(
        end[2].abs_diff_eq(Vec3::new(-0.5, 0.6, 0.0), 1e-4),
        "end: {end:?}"
    );

    let report = &animation.report;
    let units = report.units.expect("metres against centimetres");
    assert!((units.file_meters - 1.0).abs() < 1e-6);
    assert!((units.model_meters - 0.01).abs() < 1e-6);
    let axes = report.axes.expect("Z-up against Y-up");
    assert!(axes.converted);
    assert_eq!(report.bone_offsets, [], "half a metre is fifty centimetres");
    assert!(report.has_warnings(), "a conversion is still said");
}

#[test]
fn a_longer_bone_in_the_animation_file_is_reported() {
    let rig = load("rig-long", &rig_fbx());
    let animation = AttachedAnimation::new(
        &rig,
        load("anim-long", &animation_fbx(0.0, 0.55)),
        "anim".to_owned(),
    );
    let [offset] = animation.report.bone_offsets[..] else {
        panic!("{:?}", animation.report.bone_offsets);
    };
    assert_eq!(animation.nodes[offset.node as usize].name, "Mid");
    assert!((offset.difference - 0.05).abs() < 1e-4, "{offset:?}");
    assert!((offset.model_length - 0.5).abs() < 1e-4, "{offset:?}");
}
