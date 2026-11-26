//! The same input and settings give the same bytes: on every run, at any thread
//! count, and on every OS (the pipeline plan, item 2.4).
//!
//! `optimize` runs over the CAD kit's fine parts
//! (`experiments/cad-kit/inputs`) and the committed fixtures,
//! through stacks that between them take every floating-point path a run has —
//! welding, both simplifiers, normals, the AO bake, the reorders, the remesher
//! and the shrinkwrap — and every file written is hashed against
//! `determinism.sha256`, which was recorded on another OS. A mismatch means this
//! OS's compiler or maths library rounds something differently from the one
//! that recorded it, and names the files it shows in.
//!
//! `REVIEW_RECORD_HASHES=1` rewrites the file instead of checking it. Do that
//! only for a change that is *meant* to change the output, say so in the commit,
//! and check the new hashes on the other OS before relying on them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use review_optimize::{
    BakeAoParams, LodLevel, LodParams, NormalParams, OpKind, OptStack, ReduceParams, RemeshParams,
    ShrinkwrapParams, SimplifyFlags, SimplifySettings, WeldParams, preset,
};
use sha2::{Digest, Sha256};

const BINARY: &str = env!("CARGO_BIN_EXE_review3d-cli");

/// Relative to the repository root. All of them are committed, so none skips.
const KIT: &[&str] = &[
    "experiments/cad-kit/inputs/link.fbx",
    "experiments/cad-kit/inputs/shell.fbx",
    "experiments/cad-kit/inputs/hinge_block.fbx",
    "experiments/cad-kit/inputs/hinge_pin.fbx",
];
const FIXTURES: &[&str] = &[
    "assets/test_models/meter_cube.fbx",
    "assets/test_models/monkey.fbx",
    "assets/test_models/SM_Wall_Break_4x3m.fbx",
];

const EXPECTED: &str = "tests/determinism.sha256";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The stacks, by name, with the models each runs on. Each name is the folder
/// its files are hashed under, so renaming one is a change to the expected file.
fn stacks() -> Vec<(&'static str, OptStack, Vec<&'static str>)> {
    // What a game-ready chain does: weld, cut to a budget evening the triangles
    // out, re-derive the normals, a LOD chain, bake AO into the vertex colour,
    // then the three reorders.
    let mut chain = OptStack::default();
    chain.push_op(OpKind::Weld(WeldParams::default()));
    chain.push_op(OpKind::Reduce(ReduceParams {
        simplify: SimplifySettings {
            flags: SimplifyFlags {
                regularize: true,
                ..SimplifyFlags::default()
            },
            ..SimplifySettings::default()
        },
        target: LodLevel {
            target_ratio: 0.25,
            ..LodLevel::default()
        },
    }));
    chain.push_op(OpKind::RecalculateNormals(NormalParams::default()));
    chain.push_op(OpKind::SimplifyLod(LodParams {
        levels: vec![
            LodLevel {
                target_ratio: 0.5,
                ..LodLevel::default()
            },
            LodLevel {
                target_ratio: 0.25,
                target_error: 1.0,
                ..LodLevel::default()
            },
        ],
        ..LodParams::default()
    }));
    chain.push_op(OpKind::BakeAo(BakeAoParams::default()));
    chain.push_op(OpKind::VertexCache);
    chain.push_op(OpKind::Overdraw { threshold: 1.05 });
    chain.push_op(OpKind::VertexFetch);

    // The two rebuilds, which are this repository's own parallel code: a closed
    // shell, then retopology of it. Not over the wall, whose 94 separate pieces
    // each get a shell of their own: two minutes in a debug build, and nothing
    // the kit's parts do not already cover.
    let mut rebuild = OptStack::default();
    rebuild.push_op(OpKind::Shrinkwrap(ShrinkwrapParams::default()));
    rebuild.push_op(OpKind::Remesh(RemeshParams::default()));

    let everything = KIT.iter().chain(FIXTURES).copied().collect();
    let rebuildable = KIT
        .iter()
        .copied()
        .chain(["assets/test_models/monkey.fbx"])
        .collect();
    vec![
        ("chain", chain, everything),
        ("rebuild", rebuild, rebuildable),
    ]
}

/// `<stack>/<file>` -> SHA-256 hex of every file `optimize` wrote.
fn hash_outputs(scratch: &Path) -> BTreeMap<String, String> {
    let mut hashes = BTreeMap::new();
    for (name, stack, models) in stacks() {
        let preset_path = scratch.join(format!("{name}.json"));
        std::fs::write(
            &preset_path,
            preset::to_json(&stack).expect("the preset serializes"),
        )
        .expect("write the preset");
        let out = scratch.join(name);
        let output = Command::new(BINARY)
            .current_dir(repo())
            .arg("optimize")
            .args(models)
            .args(["--progress-bar", "off", "--packaging", "file-per-lod"])
            .arg("--preset")
            .arg(&preset_path)
            .arg("--out")
            .arg(&out)
            .output()
            .expect("the binary runs");
        assert!(
            output.status.success(),
            "optimize with the {name} stack failed:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        for entry in std::fs::read_dir(&out).expect("the output folder") {
            let path = entry.expect("a folder entry").path();
            let file = path.file_name().unwrap().to_string_lossy().into_owned();
            let digest = Sha256::digest(std::fs::read(&path).expect("read an output"));
            let hex = digest.iter().map(|byte| format!("{byte:02x}")).collect();
            hashes.insert(format!("{name}/{file}"), hex);
        }
    }
    hashes
}

#[test]
fn the_same_input_and_settings_write_the_same_bytes_on_every_os() {
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli-determinism");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).expect("scratch folder");
    let hashes = hash_outputs(&scratch);
    assert!(!hashes.is_empty(), "optimize wrote nothing");

    let expected_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(EXPECTED);
    if std::env::var_os("REVIEW_RECORD_HASHES").is_some_and(|value| value != "0") {
        let mut text = String::from(
            "# SHA-256 of every file tests/determinism.rs makes `optimize` write.\n\
             # Recorded on one OS, checked on every other; see that file before\n\
             # rewriting this one.\n",
        );
        for (file, hex) in &hashes {
            text.push_str(&format!("{hex}  {file}\n"));
        }
        std::fs::write(&expected_path, text).expect("record the hashes");
        return;
    }

    let text = std::fs::read_to_string(&expected_path).unwrap_or_else(|error| {
        panic!(
            "no expected hashes at {} ({error}); record them with REVIEW_RECORD_HASHES=1",
            expected_path.display()
        )
    });
    let expected: BTreeMap<String, String> = text
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .map(|line| {
            let (hex, file) = line.split_once("  ").expect("`<hex>  <file>` lines");
            (file.to_owned(), hex.to_owned())
        })
        .collect();

    let mut problems = Vec::new();
    for (file, hex) in &expected {
        match hashes.get(file) {
            None => problems.push(format!("{file}: expected, not written")),
            Some(actual) if actual != hex => problems.push(format!("{file}: different bytes")),
            Some(_) => {}
        }
    }
    for file in hashes.keys().filter(|file| !expected.contains_key(*file)) {
        problems.push(format!("{file}: written, not expected"));
    }
    assert!(
        problems.is_empty(),
        "{} of {} outputs differ from the recorded ones (kept in {} for comparison):\n  {}",
        problems.len(),
        expected.len(),
        scratch.display(),
        problems.join("\n  ")
    );
}
