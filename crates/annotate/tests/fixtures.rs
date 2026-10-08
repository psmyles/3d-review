//! The patcher against every real file in `assets/test_models`: binary FBX
//! 7300, 7400, 7500 and 7700 from the Autodesk SDK and Blender, and an ASCII 7.7
//! file. Each test skips itself when the fixtures are absent.
//!
//! The bar is the one a lossless edit has to meet: rewriting with no edit
//! reproduces the file byte for byte; an edit is read back identically by this
//! crate *and* by ufbx (the viewer's importer), with the scene otherwise
//! unchanged; and removing the edit restores the original bytes.

use std::path::{Path, PathBuf};

use review_annotate::fbx::{self, Edit, Format};
use review_annotate::mapping::{ImportedNode, map_nodes};
use review_model::extras::Synthetic;

const PROPERTY: &str = "ReviewComments";

/// Files ufbx is asked to re-import. The biggest fixtures are left to the
/// byte-level tests, which are fast at any size; a debug-build ufbx import of a
/// 40 MB file would dominate the suite without testing anything new.
const UFBX_SIZE_LIMIT: u64 = 12 * 1024 * 1024;

fn fixtures() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/test_models");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("fbx"))
        })
        .collect();
    files.sort();
    files
}

fn small(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.len() <= UFBX_SIZE_LIMIT)
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// A comment payload with what text throws at a container: quotes, a
/// backslash, non-ASCII and an emoji. (No `&` — ufbx and this crate agree on its
/// ASCII encoding only up to the `\u0026` rewrite, tested separately.)
const PAYLOAD: &str = r#"{"v":1,"threads":[{"text":"Seam \"visible\" — here ✓ 🎯 C:\\tmp"}]}"#;

/// Write `path` to a temp file the importer can open (it reads from disk).
fn temp_copy(bytes: &[u8], name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("annotate-fixtures");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("temp write");
    path
}

#[test]
fn rewriting_without_an_edit_is_byte_identical() {
    for path in fixtures() {
        let bytes = read(&path);
        let rewritten = fbx::patch(&bytes, PROPERTY, &[])
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert!(
            rewritten == bytes,
            "{}: rewrite differs from the input",
            path.display()
        );
    }
}

#[test]
fn every_fixture_scans() {
    for path in fixtures() {
        let bytes = read(&path);
        let scan = fbx::scan(&bytes, PROPERTY)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert!(
            !scan.models.is_empty(),
            "{}: no Model objects",
            path.display()
        );
        assert!(
            scan.strings.is_empty(),
            "{}: already carries the property",
            path.display()
        );
    }
}

/// Every real node the importer produced maps to exactly one `Model` object of
/// the same name, and every `Model` is mapped — the pairing the viewer relies on
/// to attach a comment to a scene node. (ufbx lists nodes by depth rather than
/// in file order, so this cannot be a positional zip; see `mapping`.)
#[test]
fn every_imported_node_maps_to_its_model() {
    for path in fixtures().into_iter().filter(|path| small(path)) {
        let bytes = read(&path);
        let scan = fbx::scan(&bytes, PROPERTY).expect("scans");
        let (model, extras) = review_import::load_model_full(&path).expect("imports");
        let extras = extras.expect("extras captured");
        let nodes: Vec<ImportedNode<'_>> = model
            .nodes
            .iter()
            .zip(&extras.nodes)
            .map(|(node, extra)| ImportedNode {
                name: &node.name,
                parent: node.parent,
                real: extra.synthetic == Synthetic::None,
            })
            .collect();
        let mapped = map_nodes(&scan.models, &nodes);
        let mut seen = vec![false; scan.models.len()];
        for (index, (node, target)) in nodes.iter().zip(&mapped).enumerate() {
            match (node.real, target) {
                (true, Some(target)) => {
                    assert_eq!(scan.models[*target].name, node.name, "{}", path.display());
                    assert!(
                        !seen[*target],
                        "{}: two nodes map to one Model",
                        path.display()
                    );
                    seen[*target] = true;
                }
                (true, None) => panic!(
                    "{}: node {index} ({}) has no Model",
                    path.display(),
                    node.name
                ),
                (false, Some(_)) => panic!("{}: synthetic node {index} mapped", path.display()),
                (false, None) => {}
            }
        }
        assert!(
            seen.iter().all(|&seen| seen),
            "{}: a Model maps to no node",
            path.display()
        );
    }
}

/// Every model gets the property; this crate and ufbx both read it back, and
/// the scene is otherwise what it was.
#[test]
fn a_patched_file_reads_back_through_both_readers() {
    for path in fixtures().into_iter().filter(|path| small(path)) {
        let bytes = read(&path);
        let scan = fbx::scan(&bytes, PROPERTY).expect("scans");
        for hidden in [false, true] {
            let edits: Vec<Edit> = scan
                .models
                .iter()
                .map(|model| Edit {
                    model: model.id,
                    value: Some(PAYLOAD.to_owned()),
                    hidden,
                })
                .collect();
            let patched = fbx::patch(&bytes, PROPERTY, &edits).expect("patches");

            let rescan = fbx::scan(&patched, PROPERTY).expect("rescans");
            assert_eq!(
                rescan.models,
                scan.models,
                "{}: objects changed",
                path.display()
            );
            assert_eq!(
                rescan.strings.len(),
                scan.models.len(),
                "{}",
                path.display()
            );
            for stored in &rescan.strings {
                assert_eq!(stored.value, PAYLOAD, "{}", path.display());
                assert_eq!(stored.hidden, hidden, "{}", path.display());
            }

            let name = format!(
                "{}-{}",
                if hidden { "hidden" } else { "visible" },
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("fixture.fbx")
            );
            let copy = temp_copy(&patched, &name);
            let (original, _) = review_import::load_model_full(&path).expect("imports original");
            let (model, extras) = review_import::load_model_full(&copy).unwrap_or_else(|error| {
                panic!("{}: ufbx rejected the patch: {error}", path.display())
            });
            assert_eq!(
                model.vertices.len(),
                original.vertices.len(),
                "{}",
                path.display()
            );
            assert_eq!(model.indices, original.indices, "{}", path.display());
            assert_eq!(model.stats, original.stats, "{}", path.display());
            assert_eq!(
                model.animations.len(),
                original.animations.len(),
                "{}",
                path.display()
            );
            let extras = extras.expect("extras captured");
            let carried = extras
                .nodes
                .iter()
                .filter(|node| node.synthetic == Synthetic::None)
                .filter(|node| {
                    node.props.iter().any(|prop| {
                        prop.name == PROPERTY
                            && prop.user_defined()
                            && prop.flags.hidden() == hidden
                            && prop.value_str == PAYLOAD
                    })
                })
                .count();
            assert_eq!(
                carried,
                scan.models.len(),
                "{}: ufbx read the property back on too few nodes",
                path.display()
            );
        }
    }
}

/// Setting then removing the property restores the original file exactly —
/// for every fixture whose models already have a `Properties70` block, which
/// is all of them.
#[test]
fn removing_the_property_restores_the_original_bytes() {
    for path in fixtures() {
        let bytes = read(&path);
        let scan = fbx::scan(&bytes, PROPERTY).expect("scans");
        let Some(first) = scan.models.first() else {
            continue;
        };
        let set = fbx::patch(
            &bytes,
            PROPERTY,
            &[Edit {
                model: first.id,
                value: Some(PAYLOAD.to_owned()),
                hidden: false,
            }],
        )
        .expect("sets");
        assert_ne!(set, bytes);
        let removed = fbx::patch(
            &set,
            PROPERTY,
            &[Edit {
                model: first.id,
                value: None,
                hidden: false,
            }],
        )
        .expect("removes");
        assert!(
            removed == bytes,
            "{}: removal did not restore the original",
            path.display()
        );
    }
}

/// A binary footer keeps its writer's alignment convention at the new length:
/// the version field lands at the same offset modulo 16 as in the original.
#[test]
fn a_binary_footer_keeps_its_alignment() {
    for path in fixtures() {
        let bytes = read(&path);
        if !matches!(fbx::format(&bytes), Ok(Format::Binary { .. })) {
            continue;
        }
        let scan = fbx::scan(&bytes, PROPERTY).expect("scans");
        // Payloads of a few lengths, so the new length hits every alignment.
        for extra in 0..16 {
            let value = format!("{{\"pad\":\"{}\"}}", "x".repeat(extra));
            let patched = fbx::patch(
                &bytes,
                PROPERTY,
                &[Edit {
                    model: scan.models[0].id,
                    value: Some(value),
                    hidden: false,
                }],
            )
            .expect("patches");
            // Version, 120 zeros and a 16-byte magic end every footer.
            let phase = |file: &[u8]| (file.len() - 140) % 16;
            assert_eq!(phase(&patched), phase(&bytes), "{}", path.display());
            assert_eq!(
                &patched[patched.len() - 140..],
                &bytes[bytes.len() - 140..],
                "{}",
                path.display()
            );
        }
    }
}

/// `&` survives an ASCII file as JSON's `\u0026`, which both readers return —
/// equal as JSON to what was written.
#[test]
fn an_ampersand_round_trips_through_ascii_as_a_json_escape() {
    for path in fixtures() {
        let bytes = read(&path);
        if fbx::format(&bytes) != Ok(Format::Ascii) {
            continue;
        }
        let scan = fbx::scan(&bytes, PROPERTY).expect("scans");
        let patched = fbx::patch(
            &bytes,
            PROPERTY,
            &[Edit {
                model: scan.models[0].id,
                value: Some(r#"{"text":"A & B"}"#.to_owned()),
                hidden: false,
            }],
        )
        .expect("patches");
        let rescan = fbx::scan(&patched, PROPERTY).expect("rescans");
        assert_eq!(rescan.strings[0].value, "{\"text\":\"A \\u0026 B\"}");
        let copy = temp_copy(&patched, "ampersand.fbx");
        let (_, extras) = review_import::load_model_full(&copy).expect("ufbx reads it");
        let found = extras
            .expect("extras")
            .nodes
            .iter()
            .flat_map(|node| &node.props)
            .any(|prop| prop.name == PROPERTY && prop.value_str == "{\"text\":\"A \\u0026 B\"}");
        assert!(found, "ufbx decoded the property differently");
    }
}
