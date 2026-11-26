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

/// A loaded FBX must carry the scene-graph hierarchy and a
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
    // at a real scene-graph node (drives per-node selection / solo).
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
    let path = fixture("Triceratops.fbx");
    if !path.exists() {
        skipping(format!("{} is not present", path.display()));
        return;
    }
    let model = load_model(&path).expect("the triceratops fixture must import");

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

    // The corner -> logical mapping, checked directly: every render corner of
    // one DCC vertex is a copy of that vertex, so all of them carry the same
    // bind-pose position bit for bit. A mapping that drifted by even one
    // vertex would pair corners from different places.
    {
        let mut position_of: Vec<Option<glam::Vec3>> = vec![None; skin.logical_vertex_count()];
        for (corner, &logical) in model.corner_to_logical.iter().enumerate() {
            let position = model.vertices[corner].position;
            let seen = position_of[logical as usize].get_or_insert(position);
            assert_eq!(
                *seen, position,
                "corner {corner} maps to logical vertex {logical} but sits elsewhere"
            );
        }
    }

    // Per-vertex weight sums. These are the file's weights as authored, which
    // Blender does not normalize: this rig has vertices at 1.0 + 0.75 on two
    // bones and single-bone vertices at 0.93 (the viewer, like ufbx, divides
    // by the sum). So the check is on the share that *does* sum to one,
    // measured at 78 % (11235 of 14418 within 0.025), and on every vertex being
    // skinned at all - a mis-mapped lookup reads another vertex's run, or none.
    let mut skinned_vertices = 0usize;
    let mut normalized = 0usize;
    let mut sum_of_sums = 0.0_f64;
    let mut worst = 0.0_f32;
    for logical in 0..skin.logical_vertex_count() {
        let range = skin.influence_range(logical);
        if range.is_empty() {
            continue;
        }
        let total: f32 = skin.weights[range].iter().sum();
        assert!(
            total > 0.0 && total.is_finite(),
            "vertex {logical} sums to {total}"
        );
        skinned_vertices += 1;
        normalized += usize::from((total - 1.0).abs() <= 0.025);
        sum_of_sums += f64::from(total);
        worst = worst.max((total - 1.0).abs());
    }
    assert_eq!(
        skinned_vertices,
        skin.logical_vertex_count(),
        "every vertex of the triceratops is bound to the skeleton"
    );

    let mean_influences = skin.influence_count() as f64 / skinned_vertices as f64;
    let mean_sum = sum_of_sums / skinned_vertices as f64;
    let normalized_share = normalized as f64 / skinned_vertices as f64;
    println!(
        "Triceratops: {} bones, {} nodes, {} logical verts ({} skinned), {} influences, \
         {mean_influences:.2} influences/vertex, mean weight sum {mean_sum:.4} (worst \
         deviation {worst:.4}), {:.1} % sum to one",
        model.stats.bone_count,
        model.nodes.len(),
        skin.logical_vertex_count(),
        skinned_vertices,
        skin.influence_count(),
        normalized_share * 100.0,
    );
    assert_eq!(model.stats.bone_count, 139);
    assert_eq!(skin.influence_count(), 21554);
    assert!(
        normalized_share > 0.75,
        "only {:.1} % of vertices sum to one - the cluster walk is likely reading the \
         wrong weights",
        normalized_share * 100.0
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
        let names = ["neck", "headtop", "Tail.001", "hand.l.1"];
        let samples: Vec<(String, usize)> =
            names.iter().filter_map(|name| count_for(name)).collect();
        println!("per-bone influenced vertices: {samples:?}");
        assert_eq!(
            samples.len(),
            names.len(),
            "every sampled bone exists: {samples:?}"
        );
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
    let path = fixture("Triceratops.fbx");
    if !path.exists() {
        skipping(format!("{} is not present", path.display()));
        return;
    }
    let model = load_model(&path).expect("the triceratops fixture must import");

    assert!(model.skin.is_some(), "the triceratops is skinned");
    println!(
        "clips: {:?}",
        model
            .animations
            .iter()
            .map(|clip| (
                &clip.name,
                clip.time_begin,
                clip.time_end,
                clip.tracks.len()
            ))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        model.animations.len(),
        6,
        "the triceratops carries six clips"
    );
    assert_eq!(model.stats.clip_count, model.animations.len());
    assert!(model.frame_rate > 0.0, "the file declares a frame rate");
    for clip in &model.animations {
        assert!(!clip.name.is_empty(), "every clip is named");
        assert_eq!(
            clip.frame_rate, model.frame_rate,
            "clip '{}' is timed at the rate it was baked at, its file's",
            clip.name
        );
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
/// a real file rather than a synthetic one: a rigid destructible (the node arm
/// of the predicate). The skinned character and the locomotion clip that
/// covered the skin arm were proprietary and left the repository in a9c6db3.
#[test]
fn measured_clip_bounds_match_walking_every_corner() {
    for name in ["SM_Wall_Break_4x3m.fbx"] {
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
        let mut pose = review_model::Pose::new(&model);
        let mut deform = review_model::DeformPose::default();
        for (clip, measured) in model.animations.iter().zip(&measured) {
            let mut expected = review_model::Bounds::EMPTY;
            for frame in 0..clip.frame_count() {
                review_model::anim::evaluate_pose(
                    &model,
                    &ctx,
                    Some(clip),
                    clip.frame_time(frame),
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

/// A file read from memory (what a shell thumbnail handler is given) goes
/// through the same walk as one read from disk, so the two imports are the
/// same model and the same capture — down to each material's texture maps,
/// which is what the thumbnail binds its textures by.
#[test]
fn an_import_from_memory_is_the_import_from_the_file() {
    let path = fixture("SM_Wall_Break_4x3m.fbx");
    if !path.exists() {
        skipping(format!("{} is not present", path.display()));
        return;
    }
    let bytes = std::fs::read(&path).expect("read the fixture");
    let (from_file, file_extras) = review_import::load_model_full(&path).expect("imports");
    let (from_bytes, byte_extras) =
        review_import::load_model_full_from_bytes(&bytes, &path).expect("imports from memory");
    assert_eq!(from_bytes, from_file);
    assert_eq!(byte_extras, file_extras);

    // Named by the name it is given: no path, no directory, the same model.
    let (named, _) =
        review_import::load_model_full_from_bytes(&bytes, std::path::Path::new("wall.fbx"))
            .expect("imports from memory");
    assert_eq!(named.name, "wall");
    assert_eq!(named.vertices, from_file.vertices);

    let garbage = review_import::load_model_full_from_bytes(b"not an fbx", &path);
    assert!(matches!(garbage, Err(ImportError::LoadFailed(_))));
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
    let path = fixture("Triceratops.fbx");
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
    let path = fixture("Triceratops.fbx");
    if !path.exists() {
        skipping(format!("{} is not present", path.display()));
        return;
    }
    let model = load_model(&path).expect("the triceratops fixture must import");
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
/// No remaining fixture declares the slot, so the scene is written here: a
/// Phong at the FBX SDK's 1.0 and a Lambert at Maya's 0.5, one triangle each.
/// The capture is read back too, so the test fails rather than passing
/// vacuously if the factors or shading models ever stop arriving.
#[test]
fn a_classic_material_imports_as_a_dielectric() {
    use review_model::extras::ShaderType;

    fn triangle(geometry: u64, model: u64, material: u64, name: &str) -> String {
        format!(
            r#"	Geometry: {geometry}, "Geometry::{name}", "Mesh" {{
		Vertices: *9 {{
			a: 0,0,0,1,0,0,0,1,0
		}}
		PolygonVertexIndex: *3 {{
			a: 0,1,-3
		}}
		LayerElementMaterial: 0 {{
			Version: 101
			MappingInformationType: "AllSame"
			ReferenceInformationType: "IndexToDirect"
			Materials: *1 {{
				a: 0
			}}
		}}
		Layer: 0 {{
			Version: 100
			LayerElement:  {{
				Type: "LayerElementMaterial"
				TypedIndex: 0
			}}
		}}
	}}
	Model: {model}, "Model::{name}", "Mesh" {{
		Version: 232
	}}
	Material: {material}, "Material::{name}", "" {{
		Version: 102
		ShadingModel: "{name}"
		MultiLayer: 0
		Properties70:  {{
			P: "DiffuseColor", "Color", "", "A",0.5,0.5,0.5
			P: "ReflectionFactor", "Number", "", "A",{factor}
		}}
	}}
"#,
            factor = if name == "phong" { "1" } else { "0.5" },
        )
    }
    let fbx = format!(
        r#"; FBX 7.4.0 project file
FBXHeaderExtension:  {{
	FBXHeaderVersion: 1003
	FBXVersion: 7400
}}
Objects:  {{
{phong}{lambert}}}
Connections:  {{
	C: "OO",1000,2000
	C: "OO",3000,2000
	C: "OO",2000,0
	C: "OO",1001,2001
	C: "OO",3001,2001
	C: "OO",2001,0
}}
"#,
        phong = triangle(1000, 2000, 3000, "phong"),
        lambert = triangle(1001, 2001, 3001, "lambert"),
    );
    let path = temp_fbx("classic-materials", fbx.as_bytes());
    let (model, extras) = match review_import::load_model_full(&path) {
        Ok((model, Some(extras))) => (model, extras),
        Ok((_, None)) => return skipping("no capture in this build"),
        Err(ImportError::UfbxUnavailable) => return skipping("FBX import is unavailable"),
        Err(error) => panic!("the hand-written scene failed to load: {error}"),
    };

    let mut seen = Vec::new();
    for (material, authored) in model.materials.iter().zip(&extras.materials) {
        let factor = authored
            .props
            .iter()
            .find(|prop| prop.name == "ReflectionFactor")
            .map_or(0.0, |prop| prop.value_real[0]);
        seen.push((authored.shader_type, factor));
        assert_eq!(
            material.metallic, 0.0,
            "{} is a classic material (ReflectionFactor {factor}) and must import as a \
             dielectric",
            material.name,
        );
    }
    seen.sort_by(|a, b| a.1.total_cmp(&b.1));
    assert!(
        matches!(
            seen.as_slice(),
            [(ShaderType::FbxLambert, 0.5), (ShaderType::FbxPhong, 1.0)]
        ),
        "the scene's two classic materials must arrive with their factors: {seen:?}",
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
    // Any fixture will do: the sink panics on the Building stage, which every
    // load reports from the marshal.
    let path = fixture("meter_cube.fbx");
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
/// Two files, because ufbx has two cancel paths and they have not always
/// reported alike. Stopping the plain read fails with `UFBX_ERROR_CANCELLED`;
/// stopping inside the bit stream only feeds the inflater zeroes, and in older
/// ufbx surfaced as "Bad DEFLATE data" - an ordinary parse error. Which path a
/// file takes depends on where its first progress report lands: the 2.4 MB
/// ASCII fixture has no deflate stream at all and cancels on the read, and the
/// scene `binary_fbx_with_a_deflated_array` writes is almost entirely one
/// deflate stream, so its first report comes from the inflater. The bridge
/// records its own answer rather than reading `error.type`, and this is what
/// holds it to that - the deflate case is the common one, since most binary FBX
/// is compressed.
#[test]
fn a_superseded_load_is_cancelled_rather_than_parsed() {
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;

    // The token's generation is already behind the shared counter: this is the
    // state a worker is in the moment the user opens another file.
    let generation = Arc::new(AtomicU64::new(7));
    let cancel = review_import::CancelToken::new(Arc::clone(&generation), 6);
    assert!(cancel.is_cancelled());

    let deflated = temp_fbx("deflated", &binary_fbx_with_a_deflated_array(4 << 20));
    for (name, path) in [
        ("SM_Wall_Break_4x3m.fbx", fixture("SM_Wall_Break_4x3m.fbx")),
        ("the deflated scene", deflated.clone()),
    ] {
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
    let small = fixture("meter_cube.fbx");
    if !small.exists() {
        skipping("meter_cube.fbx is not present");
        return;
    }
    let live = review_import::CancelToken::new(Arc::clone(&generation), 7);
    assert!(!live.is_cancelled());
    let staged = review_import::load_model_staged_cancellable(&small, &|_| {}, Some(&live))
        .expect("a current load still imports");
    assert!(staged.model.stats.triangle_count > 0);

    // The deflated scene is a well-formed file, so its Cancelled above is the
    // token's doing and not a parse that would have failed anyway.
    let staged = review_import::load_model_staged_cancellable(&deflated, &|_| {}, Some(&live))
        .expect("the deflated scene imports with a live token");
    assert_eq!(staged.model.stats.triangle_count, 1);
}

/// A binary FBX 7.4 whose `Vertices` array is zlib-compressed and holds at
/// least `bytes` of doubles - one triangle over the first three points, the
/// rest unused - so nearly the whole file is one deflate stream and ufbx's
/// first progress report (every 512 KB) lands inside the inflater.
///
/// The stream is one fixed-Huffman block of literals, which needs no
/// compressor. Not *stored* blocks, which would be simpler still: ufbx copies
/// those with `ufbxi_bit_copy_bytes`, which never reaches the progress check in
/// `ufbxi_bit_refill` - the check this file exists to reach.
fn binary_fbx_with_a_deflated_array(bytes: usize) -> Vec<u8> {
    enum Prop<'a> {
        I32(i32),
        I64(i64),
        Str(&'a [u8]),
        DeflatedF64(&'a [f64]),
        I32Array(&'a [i32]),
    }
    struct Node<'a> {
        name: &'a str,
        props: Vec<Prop<'a>>,
        children: Vec<Node<'a>>,
    }
    /// zlib around one fixed-Huffman deflate block of literals only (RFC 1951
    /// 3.2.6): no compressor needed, and unlike a stored block it is decoded
    /// through the bit stream.
    fn zlib_fixed_huffman(data: &[u8]) -> Vec<u8> {
        let mut out = vec![0x78, 0x01];
        let (mut acc, mut filled) = (0u64, 0u32);
        let mut put = |value: u32, count: u32, out: &mut Vec<u8>| {
            acc |= u64::from(value) << filled;
            filled += count;
            while filled >= 8 {
                out.push(acc as u8);
                acc >>= 8;
                filled -= 8;
            }
        };
        // A Huffman code goes out most significant bit first.
        let code = |code: u32, count: u32| code.reverse_bits() >> (32 - count);
        put(0b011, 3, &mut out); // BFINAL, then BTYPE 01 (fixed Huffman)
        for &byte in data {
            let byte = u32::from(byte);
            if byte < 144 {
                put(code(0x30 + byte, 8), 8, &mut out);
            } else {
                put(code(0x190 + byte - 144, 9), 9, &mut out);
            }
        }
        put(0, 7, &mut out); // end of block
        put(0, 7, &mut out); // flush the last partial byte
        let (mut a, mut b) = (1u32, 0u32);
        for &byte in data {
            a = (a + u32::from(byte)) % 65521;
            b = (b + a) % 65521;
        }
        out.extend_from_slice(&((b << 16) | a).to_be_bytes());
        out
    }
    fn write_prop(prop: &Prop, out: &mut Vec<u8>) {
        match prop {
            Prop::I32(value) => {
                out.push(b'I');
                out.extend_from_slice(&value.to_le_bytes());
            }
            Prop::I64(value) => {
                out.push(b'L');
                out.extend_from_slice(&value.to_le_bytes());
            }
            Prop::Str(value) => {
                out.push(b'S');
                out.extend_from_slice(&(value.len() as u32).to_le_bytes());
                out.extend_from_slice(value);
            }
            Prop::DeflatedF64(values) => {
                let raw: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
                let packed = zlib_fixed_huffman(&raw);
                out.push(b'd');
                out.extend_from_slice(&(values.len() as u32).to_le_bytes());
                out.extend_from_slice(&1u32.to_le_bytes());
                out.extend_from_slice(&(packed.len() as u32).to_le_bytes());
                out.extend_from_slice(&packed);
            }
            Prop::I32Array(values) => {
                out.push(b'i');
                out.extend_from_slice(&(values.len() as u32).to_le_bytes());
                out.extend_from_slice(&0u32.to_le_bytes());
                out.extend_from_slice(&(values.len() as u32 * 4).to_le_bytes());
                for value in *values {
                    out.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
    }
    fn write_node(node: &Node, out: &mut Vec<u8>) {
        let start = out.len();
        out.extend_from_slice(&[0; 12]);
        out.push(node.name.len() as u8);
        out.extend_from_slice(node.name.as_bytes());
        let props_start = out.len();
        for prop in &node.props {
            write_prop(prop, out);
        }
        let props_len = (out.len() - props_start) as u32;
        if !node.children.is_empty() {
            for child in &node.children {
                write_node(child, out);
            }
            out.extend_from_slice(&[0; 13]);
        }
        let end = out.len() as u32;
        out[start..start + 4].copy_from_slice(&end.to_le_bytes());
        out[start + 4..start + 8].copy_from_slice(&(node.props.len() as u32).to_le_bytes());
        out[start + 8..start + 12].copy_from_slice(&props_len.to_le_bytes());
    }
    fn leaf<'a>(name: &'a str, props: Vec<Prop<'a>>) -> Node<'a> {
        Node {
            name,
            props,
            children: Vec::new(),
        }
    }

    let vertices: Vec<f64> = (0..bytes.div_ceil(8 * 3) * 3)
        .map(|index| match index {
            3 | 7 => 1.0,
            0..9 => 0.0,
            _ => (index % 1000) as f64 * 0.001,
        })
        .collect();
    let top = [
        Node {
            name: "FBXHeaderExtension",
            props: Vec::new(),
            children: vec![
                leaf("FBXHeaderVersion", vec![Prop::I32(1003)]),
                leaf("FBXVersion", vec![Prop::I32(7400)]),
            ],
        },
        Node {
            name: "Objects",
            props: Vec::new(),
            children: vec![
                Node {
                    name: "Geometry",
                    props: vec![
                        Prop::I64(1000),
                        Prop::Str(b"Tri\x00\x01Geometry"),
                        Prop::Str(b"Mesh"),
                    ],
                    children: vec![
                        leaf("Vertices", vec![Prop::DeflatedF64(&vertices)]),
                        leaf("PolygonVertexIndex", vec![Prop::I32Array(&[0, 1, -3])]),
                    ],
                },
                Node {
                    name: "Model",
                    props: vec![
                        Prop::I64(2000),
                        Prop::Str(b"Tri\x00\x01Model"),
                        Prop::Str(b"Mesh"),
                    ],
                    children: vec![leaf("Version", vec![Prop::I32(232)])],
                },
            ],
        },
        Node {
            name: "Connections",
            props: Vec::new(),
            children: vec![
                leaf(
                    "C",
                    vec![Prop::Str(b"OO"), Prop::I64(1000), Prop::I64(2000)],
                ),
                leaf("C", vec![Prop::Str(b"OO"), Prop::I64(2000), Prop::I64(0)]),
            ],
        },
    ];
    let mut out = b"Kaydara FBX Binary  \x00\x1a\x00".to_vec();
    out.extend_from_slice(&7400u32.to_le_bytes());
    for node in &top {
        write_node(node, &mut out);
    }
    out.extend_from_slice(&[0; 13]);
    out
}

/// A hand-written triangle with no normals, a fourth control point no face
/// uses, and two named UV sets — the three facts the capture records for the
/// audit and that the geometry itself cannot carry.
#[test]
fn capture_records_authored_normals_uv_sets_and_unused_points() {
    const FBX: &str = r#"; FBX 7.4.0 project file
FBXHeaderExtension:  {
	FBXHeaderVersion: 1003
	FBXVersion: 7400
}
Objects:  {
	Geometry: 1000, "Geometry::Tri", "Mesh" {
		Vertices: *12 {
			a: 0,0,0,1,0,0,0,1,0,5,5,5
		}
		PolygonVertexIndex: *3 {
			a: 0,1,-3
		}
		LayerElementUV: 0 {
			Version: 101
			Name: "map1"
			MappingInformationType: "ByPolygonVertex"
			ReferenceInformationType: "Direct"
			UV: *6 {
				a: 0,0,1,0,0,1
			}
		}
		LayerElementUV: 1 {
			Version: 101
			Name: "lightmap"
			MappingInformationType: "ByPolygonVertex"
			ReferenceInformationType: "Direct"
			UV: *6 {
				a: 0,0,1,0,0,1
			}
		}
	}
	Model: 2000, "Model::Tri", "Mesh" {
		Version: 232
	}
}
Connections:  {
	C: "OO",1000,2000
	C: "OO",2000,0
}
"#;
    let path = temp_fbx("audit-capture", FBX.as_bytes());
    let (model, extras) = match review_import::load_model_full(&path) {
        Ok((model, Some(extras))) => (model, extras),
        Ok((_, None)) => return skipping("no capture in this build"),
        Err(ImportError::UfbxUnavailable) => return skipping("FBX import is unavailable"),
        Err(error) => panic!("the hand-written fixture failed to load: {error}"),
    };
    assert_eq!(model.stats.triangle_count, 1);
    assert_eq!(extras.meshes.len(), 1);
    let mesh = &extras.meshes[0];
    assert!(!mesh.normals_authored, "the file carries no normals");
    let names: Vec<&str> = mesh.uv_sets.iter().map(|set| set.name.as_str()).collect();
    assert_eq!(names, ["map1", "lightmap"]);
    assert!(mesh.uv_sets.iter().all(|set| set.has_values));
    assert_eq!(
        mesh.unused_vertices.len(),
        1,
        "control point 3 is used by no face"
    );
    let (logical, position) = mesh.unused_vertices[0];
    assert_eq!(logical, 3);
    // Import normalizes to meters; a file declaring no unit is read as
    // centimeters, so allow either scale but require the direction.
    let direction = position.normalize();
    assert!((direction - glam::Vec3::splat(1.0).normalize()).length() < 1e-4);
}
