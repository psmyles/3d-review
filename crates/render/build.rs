//! Offline compilation of the generated shader sources to this host's bytecode.
//!
//! The renderer ships its shaders as **compiled bytecode** (`include_bytes!` at
//! runtime — no runtime shader compilation, matching the §4 "precompute offline"
//! decision): DXBC via `fxc` on Windows, a `.metallib` via `xcrun metal` on macOS.
//! The blobs are committed next to their source; this build script regenerates them,
//! but only when a blob does not match the source it was built from and the host's
//! shader compiler is actually available. A normal `cargo build` (or CI's
//! `cargo check`, where neither compiler is installed) is a no-op and uses the
//! committed bytecode, so the build never *requires* a shader toolchain. Set
//! `FXC_FORCE=1` to recompile regardless.
//!
//! Each host owns exactly one half of the generated set and never touches the other's
//! blobs or manifest rows — that is D5's two-host discipline, and it is why the
//! source list, the compiler and the blob extension below are all `cfg`-chosen from
//! one shared skeleton rather than duplicated per OS.
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
//! The source set is this host's half of what sokol-shdc generates from the single
//! annotated-GLSL source (`src/shaders/review.glsl`, D4):
//! `src/shaders/generated/review_*_hlsl5_*.hlsl` on Windows and
//! `review_*_metal_macos_*.metal` on macOS. The job list is *discovered* rather than
//! hand-listed — the set of programs is decided in review.glsl, so a program added
//! there must not also have to be added here. Compiling them is what makes a broken
//! shader a build error (D5) rather than a runtime surprise; nothing compiles a
//! shader at run time.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One shader to compile: the source file, its stage profile, and the committed
/// output blob (both paths absolute). There is no entry-point field because the
/// generated entry point is fixed per backend (`main` for HLSL, `main0` for MSL) and
/// the reflection in `review.rs` already names it.
struct ShaderJob {
    source: PathBuf,
    /// The source's file name, which is how the manifest names it — the manifest
    /// sits beside the sources, so a full path would only tie it to one checkout.
    source_name: String,
    /// What [`stage_profile`] made of the filename's stage suffix: the `fxc` target
    /// profile on Windows, and empty on macOS, where `metal` reads the stage from the
    /// source's own `[[vertex]]` / `[[fragment]]` attribute — so only the Windows
    /// `compile` reads it.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    profile: &'static str,
    output: PathBuf,
}

fn main() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let generated_dir = crate_dir.join("src/shaders/generated");

    println!("cargo:rerun-if-env-changed=FXC_FORCE");
    println!("cargo:rerun-if-changed=build.rs");

    let jobs = generated_jobs(&generated_dir);
    warn_if_shader_source_changed(&crate_dir, &generated_dir);

    // Re-run when any source changes, so an edit recompiles where fxc is present.
    // The committed .dxbc outputs are deliberately not watched — we write them.
    let mut sources: Vec<&Path> = jobs.iter().map(|job| job.source.as_path()).collect();
    sources.sort_unstable();
    sources.dedup();
    for source in sources {
        println!("cargo:rerun-if-changed={}", source.display());
    }

    let force = std::env::var_os("FXC_FORCE").is_some();
    let tool = find_tool();
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
        let Some(tool) = tool.as_ref() else {
            // No shader compiler on this machine, so the committed blob is what
            // ships. Say so when it does not match its source (D5's two-host
            // discipline: a shader edited on the other OS leaves this one's bytecode
            // behind) and warn louder when it is missing entirely. The manifest is
            // deliberately left alone, so the packaging check still fails on this.
            if job.output.exists() {
                println!(
                    "cargo:warning={} was not built from the committed source and {} is not \
                     available; building with the bytecode as committed.",
                    job.output.display(),
                    TOOL_NAME
                );
            } else {
                println!(
                    "cargo:warning={} not found and {} is missing; anything that \
                     include_bytes! it will fail to build. {}",
                    TOOL_NAME,
                    job.output.display(),
                    TOOL_INSTALL_HINT
                );
            }
            continue;
        };
        compile(tool, job);
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
    // that is every `*_metal_macos_*` row and on macOS every `*_hlsl5_*` one, each
    // rebuildable only on the host that owns it.
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

/// Where `scripts/gen-shaders.{sh,ps1}` records the digest of the `review.glsl` it
/// last ran on, beside the sources it produced from it.
const SOURCE_DIGEST: &str = "review.glsl.sha256";

/// Warn when `review.glsl` is not the shader the generated sources were made from —
/// i.e. it was edited and `scripts/gen-shaders` never run.
///
/// Nothing else catches this: the blobs would match their committed sources perfectly
/// and the whole set would simply be a shader revision behind. It is the one staleness
/// question [`MANIFEST`] cannot answer, because the manifest's chain starts at the
/// generated source and this is the link above it.
///
/// This used to compare mtimes, which was wrong for the same reason the manifest keys
/// on content: a fresh checkout writes `review.glsl` and the generated directory in
/// whatever order git happens to walk them, microseconds apart, and a strict `>` then
/// fires on every build of a clean tree. (Measured: 6 ms apart on this repo's first
/// macOS checkout.) A tolerance cannot fix it either — a *real* edit is followed by a
/// build seconds later, so no window separates the two cases. The digest does, exactly.
fn warn_if_shader_source_changed(crate_dir: &Path, generated_dir: &Path) {
    let glsl = crate_dir.join("src/shaders/review.glsl");
    println!("cargo:rerun-if-changed={}", glsl.display());
    let digest_path = generated_dir.join(SOURCE_DIGEST);
    println!("cargo:rerun-if-changed={}", digest_path.display());

    let Some((current, _)) = fingerprint(&glsl) else {
        return;
    };
    // No record at all: an old checkout from before this file existed. Silent — the
    // next `gen-shaders` writes one, and a warning nobody can act on is noise.
    let Ok(recorded) = std::fs::read_to_string(&digest_path) else {
        return;
    };
    if recorded.trim() != current {
        println!(
            "cargo:warning=review.glsl does not match what src/shaders/generated/ was \
             built from; run scripts/gen-shaders.sh (or .ps1) and commit what it writes."
        );
    }
}

/// Discover this host's generated sources: every
/// `review_<program>_<backend>_<vertex|fragment>.<ext>` under `src/shaders/generated/`
/// whose extension this host owns, each compiled to a [`BLOB_EXT`] sibling. The stage
/// comes from the filename — so there is nothing to hand-list, which is the point:
/// `review.glsl` alone decides which programs exist.
///
/// The other host's sources are skipped entirely, by extension: a `.metal` here is
/// not a job with no compiler, it is not this build's business at all.
fn generated_jobs(generated_dir: &Path) -> Vec<ShaderJob> {
    let Ok(entries) = std::fs::read_dir(generated_dir) else {
        // The directory is absent only before `scripts/gen-shaders` has ever run.
        return Vec::new();
    };
    let mut jobs: Vec<ShaderJob> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == SOURCE_EXT))
        .filter_map(|path| {
            let stem = path.file_stem()?.to_str()?;
            let profile = stage_profile(stem)?;
            Some(ShaderJob {
                output: path.with_extension(BLOB_EXT),
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

/// The `fxc` target profile for a generated source's stage suffix, and `None` for a
/// file that is not one of shdc's stage sources.
#[cfg(windows)]
fn stage_profile(stem: &str) -> Option<&'static str> {
    match stem.rsplit_once('_')? {
        (_, "vertex") => Some("vs_5_0"),
        (_, "fragment") => Some("ps_5_0"),
        _ => None,
    }
}

/// `metal` reads the stage from the source's own `[[vertex]]` / `[[fragment]]`
/// attribute, so there is no profile to pass — but the suffix still has to be checked,
/// because it is what identifies the file as one of shdc's stage sources.
#[cfg(target_os = "macos")]
fn stage_profile(stem: &str) -> Option<&'static str> {
    match stem.rsplit_once('_')? {
        (_, "vertex" | "fragment") => Some(""),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// The host half: which sources this build owns, and what compiles them
// ---------------------------------------------------------------------------

/// Extension of the generated sources this host compiles, and of the blobs it writes
/// beside them. The other host's files differ in both, which is how the two job lists
/// stay disjoint without a filename convention to parse.
#[cfg(windows)]
const SOURCE_EXT: &str = "hlsl";
#[cfg(windows)]
const BLOB_EXT: &str = "dxbc";
#[cfg(target_os = "macos")]
const SOURCE_EXT: &str = "metal";
#[cfg(target_os = "macos")]
const BLOB_EXT: &str = "metallib";

/// What to call the host's shader compiler in a build warning, and what to tell
/// someone who does not have it.
#[cfg(windows)]
const TOOL_NAME: &str = "fxc";
#[cfg(windows)]
const TOOL_INSTALL_HINT: &str = "Install the Windows SDK.";
#[cfg(target_os = "macos")]
const TOOL_NAME: &str = "the Metal toolchain";
#[cfg(target_os = "macos")]
const TOOL_INSTALL_HINT: &str =
    "Install Xcode and run: xcodebuild -downloadComponent MetalToolchain.";

/// Invoke `fxc` to compile one generated source to a committed `.dxbc` blob. A
/// compile failure is fatal (panics the build) — when fxc *is* present, a broken
/// shader must not slip through as a silently-stale blob.
#[cfg(windows)]
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

/// Invoke `xcrun metal` to compile one generated MSL source straight to a committed
/// `.metallib`. Fatal on failure, for the same reason as the fxc twin.
///
/// One invocation, not the `metal -c` → `metallib` pair: the driver does both, and a
/// single-source library is exactly what `sg_make_shader` loads with
/// `newLibraryWithData:`. `-Werror` is the counterpart of fxc's `/WX`, and holds today
/// — every one of shdc's generated MSL sources compiles warning-free.
#[cfg(target_os = "macos")]
fn compile(xcrun: &Path, job: &ShaderJob) {
    let status = Command::new(xcrun)
        .args(["-sdk", "macosx", "metal", "-Werror", "-O3", "-o"])
        .arg(&job.output)
        .arg(&job.source)
        .status()
        .unwrap_or_else(|err| panic!("failed to launch xcrun ({}): {err}", xcrun.display()));

    if !status.success() {
        panic!(
            "metal failed to compile {} -> {}",
            job.source.display(),
            job.output.display()
        );
    }
}

/// Locate `fxc.exe`: first on `PATH`, then by scanning the Windows SDK's
/// per-version `bin\<ver>\x64` directories and picking the newest. Returns `None`
/// when no SDK is installed (a normal CI box), in which case the committed blobs are
/// used as-is.
#[cfg(windows)]
fn find_tool() -> Option<PathBuf> {
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

/// Confirm the Metal toolchain is really installed, by **running** it.
///
/// `xcrun --find metal` is not the check: Command Line Tools ship a `metal` stub that
/// exists, resolves, and fails with a message about a missing toolchain the moment it
/// is asked to compile anything (`mac-port-plan.md` §7). Asking it for `--version` is
/// the cheapest question that actually goes through the real compiler, so a box
/// without the 839 MB toolchain answers "no" here and builds from the committed
/// `.metallib`s instead of panicking mid-compile.
#[cfg(target_os = "macos")]
fn find_tool() -> Option<PathBuf> {
    let ran = Command::new("xcrun")
        .args(["-sdk", "macosx", "metal", "--version"])
        .output()
        .ok()?;
    ran.status.success().then(|| PathBuf::from("xcrun"))
}
