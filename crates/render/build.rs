//! Offline HLSL → DXBC compilation of the generated Direct3D shader sources.
//!
//! The renderer ships its shaders as **compiled DXBC bytecode** (`include_bytes!`
//! at runtime — no runtime shader compilation, matching the §4 "precompute
//! offline" decision). The compiled `.dxbc` blobs are committed next to their
//! source; this build script regenerates them with `fxc` (the Windows SDK HLSL
//! compiler), but only when a blob does not match the source it was built from and
//! `fxc` is actually available. A normal `cargo build` (or CI's `cargo check`, where
//! `fxc` is usually absent) is a no-op and uses the committed bytecode, so the build
//! never *requires* `fxc`. Set `FXC_FORCE=1` to recompile regardless.
//!
//! **Freshness is keyed on content, not timestamps** ([`MANIFEST`]), because D5's
//! two-host discipline is exactly where mtimes stop working. When a shader edited on
//! the Mac arrives here, `git checkout` writes the generated source and the stale
//! `.dxbc` at the *same* moment, so "is the source newer than the blob?" is a coin
//! toss on the one occasion the answer matters. The manifest instead records what
//! each blob was compiled from, so a blob that does not correspond to the committed
//! source is detected however the two files got their timestamps — and
//! `packaging/check-shader-bytecode.ps1` fails the release build on it rather than
//! shipping bytecode that does not match the shaders in the tree.
//!
//! There is one source set: `src/shaders/generated/review_*_hlsl5_*.hlsl`, the
//! Direct3D half of what sokol-shdc generates from the single annotated-GLSL source
//! (`src/shaders/review.glsl`, D4). Every entry point is `main`, and the job list is
//! *discovered* rather than hand-listed — the set of programs is decided in
//! review.glsl, so a program added there must not also have to be added here.
//! Compiling them is what makes a broken shader a build error (D5) rather than a
//! runtime surprise; nothing compiles a shader at run time.
//!
//! macOS compiles the `metal_macos` half of the same generated set to `.metallib`
//! instead; that arrives with the Metal backend (Phase 2 step 2).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

/// One shader to compile: the source file, the `fxc` profile, and the committed
/// output blob (both paths absolute). There is no entry-point field because
/// SPIRV-Cross names every generated entry point `main`.
struct ShaderJob {
    source: PathBuf,
    /// The source's file name, which is how the manifest names it — the manifest
    /// sits beside the sources, so a full path would only tie it to one checkout.
    source_name: String,
    profile: &'static str,
    output: PathBuf,
}

fn main() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let generated_dir = crate_dir.join("src/shaders/generated");

    println!("cargo:rerun-if-env-changed=FXC_FORCE");
    println!("cargo:rerun-if-changed=build.rs");

    let jobs = generated_jobs(&generated_dir);
    warn_if_shader_source_is_newer(&crate_dir, &jobs);

    // Re-run when any source changes, so an edit recompiles where fxc is present.
    // The committed .dxbc outputs are deliberately not watched — we write them.
    let mut sources: Vec<&Path> = jobs.iter().map(|job| job.source.as_path()).collect();
    sources.sort_unstable();
    sources.dedup();
    for source in sources {
        println!("cargo:rerun-if-changed={}", source.display());
    }

    let force = std::env::var_os("FXC_FORCE").is_some();
    let fxc = find_fxc();
    let manifest_path = generated_dir.join(MANIFEST);
    let mut manifest = read_manifest(&manifest_path);
    let mut manifest_dirty = false;

    for job in &jobs {
        let Some(source_fp) = fingerprint(&job.source) else {
            println!(
                "cargo:warning={} could not be read; skipping it.",
                job.source.display()
            );
            continue;
        };
        // Up to date only if the source is the one recorded *and* the blob beside it
        // is the one that compile produced — a blob truncated or replaced after the
        // fact is as stale as one never rebuilt.
        let recorded = manifest.get(&job.source_name);
        let blob_fp = fingerprint(&job.output);
        if !force
            && recorded.map(|row| &row.0) == Some(&source_fp)
            && recorded.map(|row| &row.1) == blob_fp.as_ref()
        {
            continue;
        }
        let Some(fxc) = fxc.as_ref() else {
            // No fxc on this machine, so the committed blob is what ships. Say so
            // when it does not match its source (D5's two-host discipline: a shader
            // edited on the other OS leaves this one's bytecode behind) and warn
            // louder when it is missing entirely. The manifest is deliberately left
            // alone, so the packaging check still fails on this.
            if job.output.exists() {
                println!(
                    "cargo:warning={} was not built from the committed source and fxc is not \
                     available; building with the bytecode as committed.",
                    job.output.display()
                );
            } else {
                println!(
                    "cargo:warning=fxc not found and {} is missing; anything that \
                     include_bytes! it will fail to build. Install the Windows SDK.",
                    job.output.display()
                );
            }
            continue;
        };
        compile(fxc, job);
        let Some(built) = fingerprint(&job.output) else {
            println!(
                "cargo:warning={} could not be read back after compiling.",
                job.output.display()
            );
            continue;
        };
        manifest.insert(job.source_name.clone(), (source_fp, built));
        manifest_dirty = true;
    }

    // Rows whose source is gone (a program removed from review.glsl) go with it.
    // Rows for the *other* host's sources are left exactly as they are: on Windows
    // that is every `*_metal_macos_*` row, which only a Mac can rebuild.
    let before = manifest.len();
    manifest.retain(|source, _| generated_dir.join(source).is_file());
    manifest_dirty |= manifest.len() != before;

    if manifest_dirty {
        write_manifest(&manifest_path, &manifest);
    }
}

/// The record of what each committed blob was compiled from: one line per generated
/// source, `<source> <sha256> <bytes> <blob sha256> <bytes>`, sorted, written by
/// whichever host compiled it. Both ends are recorded, so
/// `packaging/check-shader-bytecode.ps1` can verify the blob it is about to ship and
/// not merely the source it came from. Both hosts' rows live in the one file — a
/// Windows build never touches a `*_metal_macos_*` row and vice versa — so a shader
/// rebuilt on one OS and not the other is visible in the diff as much as in the check.
const MANIFEST: &str = "bytecode.manifest";

/// One file's identity: the SHA-256 of its bytes, plus its length. The digest is
/// not a security measure — the question is only "did this file change" — but it has
/// to be one every consumer can recompute without a toolchain, and both packaging
/// scripts are shells (`Get-FileHash` here, `shasum -a 256` on the Mac). That is what
/// picks SHA-256 over the ten-line hash this would otherwise want.
type Fingerprint = (String, u64);

/// One row: the generated source, then the blob compiled from it.
type Row = (Fingerprint, Fingerprint);

fn fingerprint(path: &Path) -> Option<Fingerprint> {
    use sha2::{Digest, Sha256};

    let bytes = std::fs::read(path).ok()?;
    let digest = Sha256::digest(&bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    Some((hex, bytes.len() as u64))
}

fn read_manifest(path: &Path) -> BTreeMap<String, Row> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    text.lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let source = fields.next()?.to_owned();
            let source_hash = fields.next()?.to_owned();
            let source_len = fields.next()?.parse().ok()?;
            let blob_hash = fields.next()?.to_owned();
            let blob_len = fields.next()?.parse().ok()?;
            Some((source, ((source_hash, source_len), (blob_hash, blob_len))))
        })
        .collect()
}

fn write_manifest(path: &Path, manifest: &BTreeMap<String, Row>) {
    let mut text = String::from(concat!(
        "# Written by crates/render/build.rs: what each committed shader blob was\n",
        "# compiled from. Commit it with the blobs; do not edit by hand.\n",
        "# <generated source> <sha256> <bytes> <blob sha256> <bytes>\n",
    ));
    for (source, ((source_hash, source_len), (blob_hash, blob_len))) in manifest {
        text.push_str(&format!(
            "{source} {source_hash} {source_len} {blob_hash} {blob_len}\n"
        ));
    }
    if let Err(err) = std::fs::write(path, text) {
        println!("cargo:warning=could not write {}: {err}", path.display());
    }
}

/// Warn when `review.glsl` is newer than everything generated from it — the one
/// staleness question mtimes *can* answer, because it is asked of one working copy
/// where the edit just happened. (A fresh checkout writes both at the same instant,
/// and this compares strictly, so it stays quiet there.) Nothing downstream can
/// catch this: the blobs would match their committed sources perfectly, and those
/// sources would simply be a shader revision behind.
fn warn_if_shader_source_is_newer(crate_dir: &Path, jobs: &[ShaderJob]) {
    let glsl = crate_dir.join("src/shaders/review.glsl");
    println!("cargo:rerun-if-changed={}", glsl.display());
    let Ok(glsl_mtime) = mtime(&glsl) else {
        return;
    };
    let newest = jobs.iter().filter_map(|job| mtime(&job.source).ok()).max();
    if newest.is_some_and(|generated| glsl_mtime > generated) {
        println!(
            "cargo:warning=review.glsl is newer than src/shaders/generated/; run \
             scripts/gen-shaders.ps1 (or .sh) and commit what it writes."
        );
    }
}

/// Discover the generated Direct3D sources: every
/// `review_<program>_hlsl5_<vertex|fragment>.hlsl` under `src/shaders/generated/`,
/// each compiled to a `.dxbc` sibling. The entry point is always `main` and the
/// stage comes from the filename — so there is nothing to hand-list, which is the
/// point: `review.glsl` alone decides which programs exist.
fn generated_jobs(generated_dir: &Path) -> Vec<ShaderJob> {
    let Ok(entries) = std::fs::read_dir(generated_dir) else {
        // The directory is absent only before `scripts/gen-shaders` has ever run.
        return Vec::new();
    };
    let mut jobs: Vec<ShaderJob> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "hlsl"))
        .filter_map(|path| {
            let stem = path.file_stem()?.to_str()?;
            let profile = match stem.rsplit_once('_')? {
                (_, "vertex") => "vs_5_0",
                (_, "fragment") => "ps_5_0",
                _ => return None,
            };
            Some(ShaderJob {
                output: path.with_extension("dxbc"),
                source_name: path.file_name()?.to_str()?.to_owned(),
                source: path,
                profile,
            })
        })
        .collect();
    // `read_dir` order is filesystem-defined; sort so build output is reproducible.
    jobs.sort_unstable_by(|a, b| a.source.cmp(&b.source));
    jobs
}

fn mtime(path: &Path) -> std::io::Result<SystemTime> {
    std::fs::metadata(path)?.modified()
}

/// Invoke `fxc` to compile one generated source to a committed `.dxbc` blob. A
/// compile failure is fatal (panics the build) — when fxc *is* present, a broken
/// shader must not slip through as a silently-stale blob.
fn compile(fxc: &Path, job: &ShaderJob) {
    // /O3 highest optimization; /WX warnings-as-errors so a sloppy translation
    // fails the build rather than shipping; default (column-major) matrix packing.
    let status = Command::new(fxc)
        .arg("/nologo")
        .arg("/T")
        .arg(job.profile)
        .arg("/E")
        .arg("main")
        .arg("/O3")
        .arg("/WX")
        .arg("/Fo")
        .arg(&job.output)
        .arg(&job.source)
        .status()
        .unwrap_or_else(|err| panic!("failed to launch fxc ({}): {err}", fxc.display()));

    if !status.success() {
        panic!(
            "fxc failed to compile {} ({}) -> {}",
            job.source.display(),
            job.profile,
            job.output.display()
        );
    }
}

/// Locate `fxc.exe`: first on `PATH`, then by scanning the Windows SDK's
/// per-version `bin\<ver>\x64` directories and picking the newest. Returns `None`
/// when no SDK is installed (a normal CI box, or any Mac), in which case the
/// committed blobs are used as-is.
fn find_fxc() -> Option<PathBuf> {
    // On PATH (e.g. a Developer Command Prompt that added the SDK bin).
    if Command::new("fxc").arg("/?").output().is_ok() {
        return Some(PathBuf::from("fxc"));
    }

    // Scan the standard SDK install roots for the newest fxc\x64.
    let roots = [
        r"C:\Program Files (x86)\Windows Kits\10\bin",
        r"C:\Program Files\Windows Kits\10\bin",
    ];
    let mut candidates: Vec<PathBuf> = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let candidate = entry.path().join("x64").join("fxc.exe");
            if candidate.is_file() {
                candidates.push(candidate);
            }
        }
    }
    // The version directories sort lexically in ascending order, so the last is the
    // newest (10.0.22621.0 < 10.0.26100.0).
    candidates.sort();
    candidates.pop()
}
