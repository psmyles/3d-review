//! Fixture-driven checks of the public import API.
//!
//! These only ever call `load_model` / `load_model_staged` / `measure_clip_bounds`
//! and assert on `review_model` types, so they are an integration test rather
//! than 700 lines inside the crate root. `has_ufbx` gates them exactly as before:
//! a checkout without the vendored sources cannot import anything.

#![cfg(has_ufbx)]

use std::path::PathBuf;

use review_import::{ImportError, load_model};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/test_models")
        .join(name)
}

/// Report why this run is skipping — or refuse to.
///
/// A skip that passes is right for a fresh checkout and wrong for the run that
/// decides whether the tree is good: a green suite would otherwise mean "nothing
/// was found to run" just as readily as "everything passed". `scripts/check` sets
/// `REVIEW_REQUIRE_FIXTURES=1`, which turns every one of these into a failure
/// naming what was missing.
fn skipping(reason: impl std::fmt::Display) {
    assert!(
        !std::env::var_os("REVIEW_REQUIRE_FIXTURES").is_some_and(|value| value != "0"),
        "REVIEW_REQUIRE_FIXTURES is set, so this run may not skip: {reason}"
    );
    eprintln!("skipping: {reason}");
}

/// Write `bytes` to a temp file with an `.fbx` extension and return its path.
///
/// The directory is cargo's own, not the system temp dir: a fixed name under
/// `%TEMP%` is shared with every other checkout, so two concurrent runs
/// delete each other's fixture mid-test.
fn temp_fbx(name: &str, bytes: &[u8]) -> PathBuf {
    let directory = env!("CARGO_TARGET_TMPDIR");
    let path = PathBuf::from(directory).join(format!("review-import-test-{name}.fbx"));
    std::fs::write(&path, bytes).expect("write temp fixture");
    path
}

/// Malformed input must come back as a clean [`ImportError::LoadFailed`] —
/// never a crash — through the whole FFI funnel.
#[test]
fn malformed_fbx_is_a_clean_error() {
    let garbage = temp_fbx(
        "garbage",
        b"this is definitely not an FBX file \xff\xfe\x00",
    );
    let result = load_model(&garbage);
    let _ = std::fs::remove_file(&garbage);
    assert!(matches!(result, Err(ImportError::LoadFailed(_))));
}

/// An empty file is the degenerate malformed case.
#[test]
fn empty_fbx_is_a_clean_error() {
    let empty = temp_fbx("empty", b"");
    let result = load_model(&empty);
    let _ = std::fs::remove_file(&empty);
    assert!(matches!(result, Err(ImportError::LoadFailed(_))));
}

/// A truncated-but-real header: the FBX binary magic followed by nothing.
/// ufbx must reject it without the bridge publishing partial geometry.
#[test]
fn truncated_fbx_is_a_clean_error() {
    let truncated = temp_fbx("truncated", b"Kaydara FBX Binary  \x00\x1a\x00");
    let result = load_model(&truncated);
    let _ = std::fs::remove_file(&truncated);
    assert!(matches!(result, Err(ImportError::LoadFailed(_))));
}

/// Phase 0 plumbing: a loaded FBX must carry the scene-graph hierarchy and a
/// per-triangle material slot parallel to the triangle list.
#[test]
fn import_carries_nodes_and_per_triangle_material() {
    let model = load_model(fixture("meter_cube.fbx")).expect("meter_cube.fbx should import");

    assert!(
        !model.nodes.is_empty(),
        "imported scene-graph hierarchy must be non-empty"
    );
    assert_eq!(
        model.triangles.material.len(),
        model.stats.triangle_count,
        "tri_material must hold exactly one entry per triangle"
    );
    assert_eq!(
        model.triangles.material.len(),
        model.triangles.to_face.len(),
        "tri_material must run parallel to tri_to_face"
    );

    // Every recorded slot is either a valid material index or the
    // no-material sentinel.
    for &slot in &model.triangles.material {
        assert!(
            slot == u32::MAX || (slot as usize) < model.materials.len(),
            "tri_material slot {slot} out of range"
        );
    }

    // Per-triangle node index runs parallel to the triangle list and points
    // at a real scene-graph node (Phase 2: drives per-node selection / solo).
    assert_eq!(
        model.triangles.node.len(),
        model.stats.triangle_count,
        "tri_node must hold exactly one entry per triangle"
    );
    for &node in &model.triangles.node {
        assert!(
            (node as usize) < model.nodes.len(),
            "tri_node index {node} out of range"
        );
    }

    // Round-trip marshaling: the loaded model is internally consistent.
    let triangle_count = model.stats.triangle_count;
    assert!(
        model
            .triangles
            .validate(
                triangle_count,
                model.faces.len(),
                model.materials.len(),
                model.nodes.len(),
            )
            .is_ok(),
        "per-triangle arrays must stay in lockstep"
    );
    assert_eq!(
        model.indices.len(),
        triangle_count * 3,
        "index count must be three per triangle"
    );
    assert_eq!(
        model.triangles.to_face.len(),
        triangle_count,
        "tri_to_face must hold one entry per triangle"
    );
    for &face in &model.triangles.to_face {
        assert!(
            (face as usize) < model.faces.len(),
            "tri_to_face index {face} out of range"
        );
    }
    assert!(
        model.bounds.is_some(),
        "a non-empty imported mesh must compute bounds"
    );
    // Multi-set UV tables, when present, carry one full per-vertex channel each.
    if !model.uv_channels.is_empty() {
        assert_eq!(
            model.uv_channels.len(),
            model.stats.uv_set_count,
            "uv_channels must hold one entry per UV set"
        );
        for channel in &model.uv_channels {
            assert_eq!(
                channel.len(),
                model.vertices.len(),
                "each UV channel must cover every vertex"
            );
        }
    }
}

/// The skeletal fixture must come through with a classified node table and a
/// consistent skin CSR. This is the end-to-end guard on the corner -> logical
/// mapping: if it drifted, the per-vertex influence sums below would be wrong
/// (or `SkinData::validate` at the funnel would already have failed the load).
#[test]
fn import_carries_skeleton_and_skin() {
    let path = fixture("SK_Player_01.fbx");
    if !path.exists() {
        skipping(format!("{} is not present", path.display()));
        return;
    }
    let model = load_model(&path).expect("the skeletal fixture must import");

    // ── Bones ───────────────────────────────────────────────────────────
    let bones: Vec<usize> = model
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.kind == review_model::NodeKind::Bone)
        .map(|(index, _)| index)
        .collect();
    assert!(
        !bones.is_empty(),
        "a skeletal mesh must import at least one bone node"
    );
    assert_eq!(
        model.stats.bone_count,
        bones.len(),
        "the Bones stat must be the measured bone-node count"
    );
    for &bone in &bones {
        assert!(
            model.nodes[bone].bone.is_some(),
            "a bone node must carry its display params"
        );
    }
    {
        // The overlay sizes its leaf/root joint markers from these, so it
        // matters whether the file actually declared them.
        let params: Vec<(f32, f32)> = bones
            .iter()
            .filter_map(|&bone| model.nodes[bone].bone)
            .map(|bone| (bone.radius, bone.relative_length))
            .collect();
        let with_radius = params.iter().filter(|(radius, _)| *radius > 0.0).count();
        let with_length = params.iter().filter(|(_, length)| *length > 0.0).count();
        println!(
            "bone display params: {with_radius}/{} carry a radius,                  {with_length}/{} a relative length; first = {:?}",
            params.len(),
            params.len(),
            params.first()
        );
    }
    assert!(
        model
            .nodes
            .iter()
            .any(|node| node.kind == review_model::NodeKind::Mesh),
        "the skeletal fixture also carries geometry"
    );

    // ── Skin ────────────────────────────────────────────────────────────
    let skin = model
        .skin
        .as_ref()
        .expect("a skinned mesh must import skin data");
    // The funnel already ran `validate`; re-assert the shape here so a failure
    // reports as this test rather than as a generic load error.
    assert_eq!(
        skin.validate(model.stats.vertex_count, model.nodes.len()),
        Ok(())
    );
    assert_eq!(model.corner_to_logical.len(), model.vertices.len());
    assert_eq!(skin.offsets.len(), model.stats.vertex_count + 1);
    assert!(
        !skin.clusters.is_empty(),
        "a skinned mesh binds at least one cluster"
    );
    assert!(
        !skin.deformers.is_empty(),
        "every skinned mesh node reports its deformer"
    );
    for deformer in &skin.deformers {
        assert!((deformer.mesh_node as usize) < model.nodes.len());
        assert!(deformer.max_weights_per_vertex > 0);
    }

    // Every influence must name a node the Outliner can actually show.
    for &bone in &skin.bones {
        assert!(
            (bone as usize) < model.nodes.len(),
            "skin influence references node {bone} of {}",
            model.nodes.len()
        );
    }

    // Per-vertex weight sums: a sane rig normalizes to ~1.0. This is the
    // assertion that would catch a broken corner -> logical mapping, since a
    // mis-mapped vertex reads another vertex's (or no) influences.
    let mut skinned_vertices = 0usize;
    let mut sum_of_sums = 0.0_f64;
    let mut worst = 0.0_f32;
    for logical in 0..skin.logical_vertex_count() {
        let range = skin.influence_range(logical);
        if range.is_empty() {
            continue;
        }
        let total: f32 = skin.weights[range].iter().sum();
        skinned_vertices += 1;
        sum_of_sums += f64::from(total);
        worst = worst.max((total - 1.0).abs());
    }
    assert!(skinned_vertices > 0, "no vertex carried any influence");

    let mean_influences = skin.influence_count() as f64 / skinned_vertices as f64;
    let mean_sum = sum_of_sums / skinned_vertices as f64;
    println!(
        "SK_Player_01: {} bones, {} nodes, {} logical verts ({} skinned),              {} influences, {mean_influences:.2} influences/vertex,              mean weight sum {mean_sum:.4} (worst deviation {worst:.4})",
        model.stats.bone_count,
        model.nodes.len(),
        skin.logical_vertex_count(),
        skinned_vertices,
        skin.influence_count(),
    );

    assert!(
        (0.99..=1.01).contains(&mean_sum),
        "mean per-vertex weight sum {mean_sum} is not ~1.0 — the corner -> logical              mapping or the cluster walk is likely wrong"
    );
    assert!(
        mean_influences > 1.0,
        "a real rig blends more than one bone per vertex on average, got {mean_influences}"
    );

    // Per-bone influence counts must *vary* — a finger should move far fewer
    // vertices than a spine. A flat count across bones would mean the lookup
    // is ignoring which bone was asked about.
    {
        let count_for = |name: &str| -> Option<(String, usize)> {
            let node = model
                .nodes
                .iter()
                .position(|n| n.name.contains(name) && n.kind == review_model::NodeKind::Bone)?;
            let key = [node as u32];
            let count = (0..skin.logical_vertex_count())
                .filter(|&logical| {
                    skin.bones[skin.influence_range(logical)]
                        .iter()
                        .any(|bone| key.binary_search(bone).is_ok())
                })
                .count();
            Some((model.nodes[node].name.clone(), count))
        };
        let samples: Vec<(String, usize)> = ["Spine1", "Head", "Pinky3_L", "Hand_L"]
            .iter()
            .filter_map(|name| count_for(name))
            .collect();
        println!("per-bone influenced vertices: {samples:?}");
        let counts: Vec<usize> = samples.iter().map(|(_, count)| *count).collect();
        assert!(
            counts.iter().any(|&count| count > 0),
            "no sampled bone influenced anything"
        );
        assert!(
            counts.iter().min() != counts.iter().max(),
            "every sampled bone influenced the same number of vertices - the                  per-bone lookup is not actually discriminating: {samples:?}"
        );
    }

    // The corner map must land inside the logical range for every render vertex.
    for &logical in &model.corner_to_logical {
        assert!(
            (logical as usize) < skin.logical_vertex_count(),
            "corner maps to logical vertex {logical} of {}",
            skin.logical_vertex_count()
        );
    }
}

/// The negative control: a plain mesh must import with no bones and no skin
/// payload at all, so the skeletal UI stays hidden for ordinary models.
#[test]
fn unskinned_import_carries_no_skeleton() {
    let path = fixture("meter_cube.fbx");
    if !path.exists() {
        skipping(format!("{} is not present", path.display()));
        return;
    }
    let model = load_model(&path).expect("the cube fixture must import");
    assert_eq!(model.stats.bone_count, 0);
    assert!(model.skin.is_none(), "an unskinned mesh must carry no skin");
    assert!(
        model
            .nodes
            .iter()
            .all(|node| node.kind != review_model::NodeKind::Bone),
        "an unskinned mesh must classify no node as a bone"
    );
    assert!(model.animations.is_empty(), "the cube carries no clips");
    assert_eq!(model.stats.clip_count, 0);
}

/// The rest local transforms the bridge captures must compose back to the
/// world transforms it also captures — the check that a pose recomposed
/// from them (and therefore every animated frame) lands where the file says.
fn assert_rest_locals_recompose(model: &review_model::ModelData) {
    let ctx = review_model::AnimContext::new(model);
    assert!(
        review_model::anim::rest_locals_recompose(model, &ctx, 1e-3),
        "composing the rest local transforms must reproduce node_to_world"
    );
}

/// The skinned, multi-clip fixture: every stack imports as a clip with a
/// playable range, every track names a real node, and the rest pose's
/// skinning is consistent with the file's world transforms.
#[test]
fn import_carries_animation_clips() {
    let path = fixture("AN_ZombiedogLocomotion.fbx");
    if !path.exists() {
        skipping(format!("{} is not present", path.display()));
        return;
    }
    let model = load_model(&path).expect("the locomotion fixture must import");

    assert!(model.skin.is_some(), "the dog is skinned");
    assert!(
        model.animations.len() > 1,
        "the fixture carries several clips, got {}",
        model.animations.len()
    );
    assert_eq!(model.stats.clip_count, model.animations.len());
    assert!(model.frame_rate > 0.0, "the file declares a frame rate");
    for clip in &model.animations {
        assert!(!clip.name.is_empty(), "every clip is named");
        assert!(
            clip.time_end > clip.time_begin,
            "clip '{}' has an empty range {}..{}",
            clip.name,
            clip.time_begin,
            clip.time_end
        );
        assert!(
            !clip.tracks.is_empty(),
            "clip '{}' animates nothing",
            clip.name
        );
        for track in &clip.tracks {
            assert!((track.node as usize) < model.nodes.len());
        }
    }
    assert!(model.bounds.is_some());
    assert_rest_locals_recompose(&model);
    assert_eq!(model.validate_deform(), Ok(()));
}

/// The unskinned, rigidly animated fixture: node tracks and no skin, and at
/// least one node ends the clip somewhere other than its rest transform.
#[test]
fn rigid_clip_moves_nodes() {
    let path = fixture("SM_Wall_Break_4x3m.fbx");
    if !path.exists() {
        skipping(format!("{} is not present", path.display()));
        return;
    }
    let model = load_model(&path).expect("the wall-break fixture must import");

    assert!(model.skin.is_none(), "the wall pieces are not skinned");
    assert!(!model.animations.is_empty(), "the fixture carries a clip");
    assert_rest_locals_recompose(&model);

    let ctx = review_model::AnimContext::new(&model);
    let mut pose = review_model::Pose::new(&model);
    let clip = &model.animations[0];
    review_model::anim::evaluate_pose(&model, &ctx, Some(clip), clip.time_end, &mut pose);
    let moved = model
        .nodes
        .iter()
        .zip(&pose.world)
        .any(|(node, world)| !world.abs_diff_eq(node.transform, 1e-4));
    assert!(moved, "the clip's last frame must move at least one node");
}

/// [`measure_clip_bounds`] — the deferred measurement `app` runs once the model
/// is on screen — must give exactly what walking the whole mesh in every frame
/// gives. `clip_bounds` narrows that walk to the corners the clip can actually
/// move, the difference between a 5-second load and a 100-second one on a large
/// scene, so this pins the narrowed answer to the definition it replaced, over
/// real files rather than a synthetic one: a skinned character (the skin arm of
/// the predicate), a rigid destructible (the node arm), and a locomotion clip.
#[test]
fn measured_clip_bounds_match_walking_every_corner() {
    for name in [
        "SK_Player_01.fbx",
        "AN_ZombiedogLocomotion.fbx",
        "SM_Wall_Break_4x3m.fbx",
    ] {
        let path = fixture(name);
        if !path.exists() {
            skipping(format!("{} is not present", path.display()));
            continue;
        }
        let model = load_model(&path).unwrap_or_else(|error| panic!("{name}: {error}"));
        if model.animations.is_empty() {
            continue;
        }

        let measured = review_import::measure_clip_bounds(&model);
        assert_eq!(
            measured.len(),
            model.animations.len(),
            "{name}: one envelope per clip"
        );

        let ctx = review_model::AnimContext::new(&model);
        let fps = model.frame_rate_or_default();
        let mut pose = review_model::Pose::new(&model);
        let mut deform = review_model::DeformPose::default();
        for (clip, measured) in model.animations.iter().zip(&measured) {
            let mut expected = review_model::Bounds::EMPTY;
            for frame in 0..clip.frame_count(fps) {
                review_model::anim::evaluate_pose(
                    &model,
                    &ctx,
                    Some(clip),
                    clip.frame_time(frame, fps),
                    &mut pose,
                );
                review_model::anim::build_palette(&model, &ctx, &pose, &mut deform);
                if let Some(frame_bounds) = review_model::anim::posed_bounds(&model, &ctx, &deform)
                {
                    expected.include_point(frame_bounds.min);
                    expected.include_point(frame_bounds.max);
                }
            }
            let measured = measured.expect("an animated clip has an envelope");
            assert_eq!(measured.min, expected.min, "{name} / {} min", clip.name);
            assert_eq!(measured.max, expected.max, "{name} / {} max", clip.name);
        }
    }
}

/// The staged import publishes a *drawable* model and leaves the two costly
/// measurements to the caller: whatever the viewport needs on the first frame
/// must already be there, and the deferred pair must not be.
#[test]
fn a_staged_import_is_drawable_but_unmeasured() {
    let path = fixture("SM_Wall_Break_4x3m.fbx");
    if !path.exists() {
        skipping(format!("{} is not present", path.display()));
        return;
    }
    let staged =
        review_import::load_model_with_progress(&path, &|_| {}).expect("the fixture must import");

    // Everything the first frame draws with.
    assert!(!staged.vertices.is_empty(), "geometry");
    assert!(!staged.indices.is_empty(), "indices");
    assert!(!staged.nodes.is_empty(), "scene graph");
    assert!(staged.stats.draw_count > 0, "draw groups");
    assert!(staged.bounds.is_some(), "the camera frames on the bounds");
    assert!(!staged.has_degenerate_tangents(), "tangents");

    // …and neither of the two measurements that cost the most.
    assert_eq!(
        staged.stats.gpu_vertex_count, 0,
        "GPU Verts is measured after the model is up"
    );

    // The complete entry point makes them, so a test or batch caller still
    // gets a fully measured model.
    let complete = load_model(&path).expect("the fixture must import");
    assert_eq!(
        complete.stats.gpu_vertex_count,
        complete.count_gpu_vertices()
    );
    assert!(complete.stats.gpu_vertex_count > 0);
}

/// Every fixture's source-property capture must describe the model it came
/// with: `load_model_full` runs the funnel guard, so an index that drifted
/// from the geometry fails here rather than in an export.
#[test]
fn every_fixture_captures_valid_extras() {
    let dir = fixture("");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        skipping(format!("{} is not present", dir.display()));
        return;
    };
    let mut checked = 0;
    // `RVO_EXTRAS_FIXTURE=<name>` narrows the walk to one file, for
    // isolating a fixture that misbehaves.
    let only = std::env::var("RVO_EXTRAS_FIXTURE").ok();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("fbx"))
        {
            continue;
        }
        if let Some(only) = &only
            && path.file_name().is_none_or(|name| name != only.as_str())
        {
            continue;
        }
        let (model, extras) = review_import::load_model_full(&path)
            .unwrap_or_else(|error| panic!("{} must import: {error}", path.display()));
        let extras = extras.unwrap_or_else(|| panic!("{} captured no extras", path.display()));
        // Beyond the guard: the parts cover the model, each node's props were
        // read, and the clip map points at the clips the model has.
        assert_eq!(extras.nodes.len(), model.nodes.len());
        assert_eq!(extras.materials.len(), model.materials.len());
        assert!(
            extras.nodes.iter().any(|node| !node.props.is_empty()),
            "{} has nodes without any authored property",
            path.display()
        );
        assert_eq!(
            extras
                .animations
                .iter()
                .filter(|stack| stack.clip.is_some())
                .count(),
            model.animations.len(),
            "{} stacks vs clips",
            path.display()
        );
        assert!(
            extras.scene.version > 0,
            "{} has no version",
            path.display()
        );
        eprintln!(
            "{}: v{} {} nodes, {} materials, {} textures, {} meshes, {} poses, {} layers, {} sets, {} stacks",
            path.file_name().unwrap().to_string_lossy(),
            extras.scene.version,
            extras.nodes.len(),
            extras.materials.len(),
            extras.textures.len(),
            extras.meshes.len(),
            extras.poses.len(),
            extras.display_layers.len(),
            extras.selection_sets.len(),
            extras.animations.len(),
        );
        checked += 1;
    }
    eprintln!("checked {checked} fixtures");
}

/// The skinned fixture carries the authored cluster matrices: `Transform`
/// (mesh node → bone) and `TransformLink` (bind → world), which must be
/// consistent with the palette matrix the viewer derives.
#[test]
fn skin_clusters_carry_authored_matrices() {
    let path = fixture("SK_Player_01.fbx");
    if !path.exists() {
        skipping(format!("{} is not present", path.display()));
        return;
    }
    let model = load_model(&path).expect("the fixture must import");
    let skin = model.skin.as_ref().expect("skinned");
    for cluster in &skin.clusters {
        assert!(cluster.bind_to_world.is_finite());
        assert!(cluster.mesh_node_to_bone.is_finite());
        // `TransformLink` is the bone's world at bind; its inverse composed
        // with the mesh node's world is what `mesh_node_to_bone` encodes.
        let mesh_world = model.nodes[cluster.mesh_node as usize].transform;
        let expected = cluster.bind_to_world.inverse() * mesh_world;
        let delta = (expected - cluster.mesh_node_to_bone).abs();
        let max = delta.to_cols_array().into_iter().fold(0f32, f32::max);
        assert!(max < 1e-3, "cluster {} drifts by {max}", cluster.name);
    }
}

/// The skinned fixture's clusters must be oriented correctly: at the rest
/// pose a vertex owned by a single bone lands exactly on its baked position
/// transformed by that cluster's skinning matrix, and the recomposed rest
/// pose reproduces every world transform.
#[test]
fn rest_pose_skinning_matches_bind() {
    let path = fixture("SK_Player_01.fbx");
    if !path.exists() {
        skipping(format!("{} is not present", path.display()));
        return;
    }
    let model = load_model(&path).expect("the player fixture must import");
    assert_rest_locals_recompose(&model);

    let skin = model.skin.as_ref().expect("skinned");
    let ctx = review_model::AnimContext::new(&model);
    let mut pose = review_model::Pose::new(&model);
    let mut deform = review_model::DeformPose::default();
    review_model::anim::rest_pose(&model, &ctx, &mut pose);
    review_model::anim::build_palette(&model, &ctx, &pose, &mut deform);
    assert_eq!(
        deform.palette.len(),
        review_model::anim::palette_len(&model)
    );

    // The palette's node entries are identity at rest by construction; the
    // cluster entries are `bone_world * world_to_bone_bind`. For each corner
    // the CPU reference must agree with applying that blend by hand.
    let mut checked = 0;
    for corner in (0..model.vertices.len()).step_by(97) {
        let logical = model.corner_to_logical[corner] as usize;
        let range = skin.influence_range(logical);
        if range.is_empty() {
            continue;
        }
        let base = model.vertices[corner].position;
        let mut expected = glam::Vec3::ZERO;
        let mut total = 0.0;
        for influence in range {
            let entry = model.nodes.len() + skin.influence_cluster[influence] as usize;
            expected += deform.palette[entry].transform_point3(base) * skin.weights[influence];
            total += skin.weights[influence];
        }
        if (total - 1.0_f32).abs() > 1e-6 {
            expected /= total;
        }
        let (actual, _) = review_model::anim::deform_corner(&model, &ctx, &deform, corner);
        assert!(
            actual.abs_diff_eq(expected, 1e-4),
            "corner {corner}: {actual} vs {expected}"
        );
        checked += 1;
    }
    assert!(checked > 10, "sampled too few corners: {checked}");

    // The rest bounds are finite and of the same order as the bind-pose
    // buffer (the default pose may differ from the bind pose, but not by a
    // scene's worth).
    let mut bind = review_model::Bounds::EMPTY;
    for vertex in &model.vertices {
        bind.include_point(vertex.position);
    }
    let rest = model.bounds.expect("rest bounds");
    assert!(rest.size().max_element() > 0.0);
    assert!(
        rest.size().max_element() < bind.size().max_element() * 4.0,
        "rest {rest:?} vs bind {bind:?}"
    );
}

/// A classic Lambert/Phong declares no metalness at all, and its
/// `ReflectionFactor` is not one: that slot is Phong reflectivity, which DCCs
/// write with a non-zero default nobody authored (Maya 0.5, the FBX SDK 1.0).
/// Reading it back as metalness made nearly every real game asset — stone,
/// wood, bark, leaves, skin — import half or fully metal, which the IBL path
/// draws as a mirror of the environment instead of a lit surface. So every
/// classic material must arrive dielectric.
///
/// The sweep is only worth anything if some fixture really does declare a
/// non-zero factor, so that is asserted too: without it the test would keep
/// passing against a fixture set that simply never exercises the slot.
#[test]
fn a_classic_material_imports_as_a_dielectric() {
    use review_model::extras::ShaderType;

    let dir = fixture("");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        skipping(format!("{} is not present", dir.display()));
        return;
    };
    let mut saw_a_reflection_factor = false;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("fbx"))
        {
            continue;
        }
        let (model, extras) = review_import::load_model_full(&path)
            .unwrap_or_else(|error| panic!("{} must import: {error}", path.display()));
        let Some(extras) = extras else { continue };
        for (material, authored) in model.materials.iter().zip(&extras.materials) {
            if !matches!(
                authored.shader_type,
                ShaderType::Unknown | ShaderType::FbxLambert | ShaderType::FbxPhong
            ) {
                continue;
            }
            let factor = authored
                .props
                .iter()
                .find(|prop| prop.name == "ReflectionFactor")
                .map_or(0.0, |prop| prop.value_real[0]);
            saw_a_reflection_factor |= factor > 0.0;
            assert_eq!(
                material.metallic,
                0.0,
                "{} / {} is a classic material (ReflectionFactor {factor}) and must import \
                 as a dielectric",
                path.file_name().unwrap().to_string_lossy(),
                material.name,
            );
        }
    }
    assert!(
        saw_a_reflection_factor,
        "no fixture declares a non-zero ReflectionFactor, so this test proves nothing",
    );
}

/// A panicking progress sink must not strand the bridge's C-side scene.
///
/// The scene used to be freed by a statement after the marshal, which every
/// `Result` path reaches but an unwind does not — and the sink is caller code
/// called from inside the marshal. `SceneHandle`'s `Drop` is what covers it now.
/// Only a test build can reach this: the release profile is `panic = "abort"`.
///
/// The leak is not directly observable from here, so what this pins is the pair
/// of properties that make the fix work: the panic propagates as an ordinary
/// unwind (rather than being swallowed or aborting), and the importer is still
/// usable afterwards.
#[test]
fn a_panicking_progress_sink_unwinds_without_taking_the_importer_with_it() {
    let path = fixture("SM_Speaker_01a.fbx");
    if !path.exists() {
        skipping("the fixture is not present");
        return;
    }

    let panicked = std::panic::catch_unwind(|| {
        // The stages after Reading are reported from the marshal, with the C
        // scene live and every borrowed slice still in flight.
        let sink = |_progress: review_import::ImportProgress| {
            panic!("a progress sink of the caller's that goes wrong");
        };
        review_import::load_model_staged(&path, &sink)
    });
    assert!(
        panicked.is_err(),
        "the sink's panic must unwind, not be swallowed"
    );

    // The scene the unwind passed through was freed on the way out, so a normal
    // load still works.
    let model = load_model(&path).expect("the importer still works");
    assert!(model.stats.triangle_count > 0);
}

/// An already-superseded load must stop rather than parse to completion.
///
/// Dropping a stale result on arrival keeps the wrong model off screen, but the
/// parse behind it ran either way — seconds of work on a large file for something
/// nobody will see. The check rides ufbx's own progress callback, so a token that
/// is already stale stops the load at its first report.
///
/// Two fixtures, because ufbx has two cancel paths and they do not report alike.
/// Stopping the plain read fails with `UFBX_ERROR_CANCELLED`, but stopping inside
/// the bit stream only feeds the inflater zeroes, so it surfaces as "Bad DEFLATE
/// data" — an ordinary parse error. Which path a file takes depends on where its
/// first progress report lands: the 326 KB fixture cancels on the read, the 26 MB
/// one inside the deflate stream. The bridge records its own answer rather than
/// reading `error.type`, and this is what holds it to that — the deflate case is
/// the common one, since most binary FBX is compressed.
#[test]
fn a_superseded_load_is_cancelled_rather_than_parsed() {
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;

    // The token's generation is already behind the shared counter: this is the
    // state a worker is in the moment the user opens another file.
    let generation = Arc::new(AtomicU64::new(7));
    let cancel = review_import::CancelToken::new(Arc::clone(&generation), 6);
    assert!(cancel.is_cancelled());

    for name in ["SM_Speaker_01a.fbx", "stylized_tree_branch_01.fbx"] {
        let path = fixture(name);
        if !path.exists() {
            skipping(format!("{name} is not present"));
            continue;
        }
        let result = review_import::load_model_staged_cancellable(&path, &|_| {}, Some(&cancel));
        assert!(
            matches!(result, Err(ImportError::Cancelled)),
            "{name}: a superseded load must report Cancelled, not a model or a parse error              (got {:?})",
            result
                .as_ref()
                .map(|staged| staged.model.stats.triangle_count)
                .map_err(|error| error.to_string())
        );
    }

    // A live token loads normally — the cancel path must not be reachable by
    // accident. A small fixture, since what is being checked is the token, not
    // the parse.
    let small = fixture("SM_Speaker_01a.fbx");
    if !small.exists() {
        skipping("SM_Speaker_01a.fbx is not present");
        return;
    }
    let live = review_import::CancelToken::new(Arc::clone(&generation), 7);
    assert!(!live.is_cancelled());
    let staged = review_import::load_model_staged_cancellable(&small, &|_| {}, Some(&live))
        .expect("a current load still imports");
    assert!(staged.model.stats.triangle_count > 0);
}
