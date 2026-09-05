//! Offline HLSL → DXBC compilation of the generated Direct3D shader sources.
//!
//! The renderer ships its shaders as **compiled DXBC bytecode** (`include_bytes!`
//! at runtime — no runtime shader compilation, matching the §4 "precompute
//! offline" decision). The compiled `.dxbc` blobs are committed next to their
//! source; this build script regenerates them with `fxc` (the Windows SDK HLSL
//! compiler) — but only when a blob is **freshness-gated stale** (its source is
//! newer, or the blob is missing) and `fxc` is actually available.
//!
//! This mirrors the IBL-bake freshness gate: a normal `cargo build` (or CI's
//! `cargo check`, where `fxc` is usually absent) is a no-op and uses the committed
//! bytecode, so the build never *requires* `fxc`; an active shader edit on a dev
//! box with the SDK on hand recompiles automatically. Set `FXC_FORCE=1` to
//! recompile regardless of timestamps.
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

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

/// One shader to compile: the source file, the `fxc` profile, and the committed
/// output blob (both paths absolute). There is no entry-point field because
/// SPIRV-Cross names every generated entry point `main`.
struct ShaderJob {
    source: PathBuf,
    profile: &'static str,
    output: PathBuf,
}

fn main() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let generated_dir = crate_dir.join("src/shaders/generated");

    println!("cargo:rerun-if-env-changed=FXC_FORCE");
    println!("cargo:rerun-if-changed=build.rs");

    let jobs = generated_jobs(&generated_dir);

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

    for job in &jobs {
        if !(force || is_stale(&job.source, &job.output)) {
            continue;
        }
        let Some(fxc) = fxc.as_ref() else {
            // No fxc on this machine, so the committed blob is what ships. Say so
            // when it is stale (D5's two-host discipline: a shader edited on the
            // other OS leaves this one's bytecode behind) and warn louder when it
            // is missing entirely.
            if job.output.exists() {
                println!(
                    "cargo:warning={} is older than its source and fxc is not available; \
                     building with the committed bytecode.",
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
                source: path,
                profile,
            })
        })
        .collect();
    // `read_dir` order is filesystem-defined; sort so build output is reproducible.
    jobs.sort_unstable_by(|a, b| a.source.cmp(&b.source));
    jobs
}

/// Whether `output` needs to be (re)built from `source`: missing output, or a
/// source modified more recently than the output. A missing/unreadable mtime errs
/// toward rebuilding (when fxc is available).
fn is_stale(source: &Path, output: &Path) -> bool {
    let Ok(output_mtime) = mtime(output) else {
        return true;
    };
    match mtime(source) {
        Ok(source_mtime) => source_mtime > output_mtime,
        Err(_) => true,
    }
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
