//! Offline HLSL → DXBC compilation for the Direct3D 11 scene path.
//!
//! The renderer ships its shaders as **compiled DXBC bytecode** (`include_bytes!`
//! at runtime — no runtime shader compilation, matching the §4 "precompute
//! offline" decision). The compiled `.dxbc` blobs are committed next to their
//! `.hlsl` source under `src/hlsl/`; this build script regenerates them with
//! `fxc` (the Windows SDK HLSL compiler) — but only when it is **freshness-gated
//! stale** (the `.hlsl` is newer than its `.dxbc`, or the `.dxbc` is missing) and
//! `fxc` is actually available.
//!
//! This mirrors the IBL-bake freshness gate: a normal `cargo build` (or CI's
//! `cargo check`, where `fxc` is usually absent) is a no-op and uses the committed
//! bytecode, so the build never *requires* `fxc`; an active shader edit on a dev
//! box with the SDK on hand recompiles automatically. Set `FXC_FORCE=1` to
//! recompile regardless of timestamps.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

/// One shader entry point to compile: the source file (under `src/hlsl/`), the
/// `fxc` profile, the entry-point function, and the committed output blob.
struct ShaderJob {
    source: &'static str,
    profile: &'static str,
    entry: &'static str,
    output: &'static str,
}

/// Every HLSL entry point the renderer compiles. Grows as scene passes are ported
/// off wgpu (migration Phases 1–6); Phase 1 ships only the grid/line vertex +
/// fragment shaders.
const SHADERS: &[ShaderJob] = &[
    ShaderJob {
        source: "scene.hlsl",
        profile: "vs_5_0",
        entry: "vs_main",
        output: "scene.vs.dxbc",
    },
    ShaderJob {
        source: "scene.hlsl",
        profile: "ps_5_0",
        entry: "fs_line",
        output: "scene.line.ps.dxbc",
    },
    ShaderJob {
        source: "scene.hlsl",
        profile: "ps_5_0",
        entry: "fs_main",
        output: "scene.mesh.ps.dxbc",
    },
    ShaderJob {
        source: "scene.hlsl",
        profile: "vs_5_0",
        entry: "vs_skybox",
        output: "scene.skybox.vs.dxbc",
    },
    ShaderJob {
        source: "scene.hlsl",
        profile: "ps_5_0",
        entry: "fs_skybox",
        output: "scene.skybox.ps.dxbc",
    },
    ShaderJob {
        source: "post.hlsl",
        profile: "vs_5_0",
        entry: "vs_fullscreen",
        output: "post.vs.dxbc",
    },
    ShaderJob {
        source: "post.hlsl",
        profile: "ps_5_0",
        entry: "fs_post",
        output: "post.ps.dxbc",
    },
];

fn main() {
    let hlsl_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/hlsl");

    // Re-run when any HLSL source changes (so an edit triggers recompilation when
    // fxc is present). The committed .dxbc outputs are not watched — we write them.
    for source in distinct_sources() {
        println!("cargo:rerun-if-changed={}", hlsl_dir.join(source).display());
    }
    println!("cargo:rerun-if-env-changed=FXC_FORCE");

    let force = std::env::var_os("FXC_FORCE").is_some();
    let fxc = find_fxc();

    for job in SHADERS {
        let source_path = hlsl_dir.join(job.source);
        let output_path = hlsl_dir.join(job.output);

        let stale = force || is_stale(&source_path, &output_path);
        if !stale {
            continue;
        }

        let Some(fxc) = fxc.as_ref() else {
            // No fxc on this machine. If the committed blob exists, use it
            // silently; only warn if it's missing (that *is* a build break — the
            // runtime `include_bytes!` will fail).
            if !output_path.exists() {
                println!(
                    "cargo:warning=fxc not found and {} is missing; the build will fail. \
                     Install the Windows SDK or run packaging/compile-hlsl on a machine with fxc.",
                    output_path.display()
                );
            }
            continue;
        };

        compile(fxc, &source_path, &output_path, job);
    }
}

/// The distinct source files referenced by [`SHADERS`], for `rerun-if-changed`.
fn distinct_sources() -> Vec<&'static str> {
    let mut sources: Vec<&'static str> = SHADERS.iter().map(|job| job.source).collect();
    sources.sort_unstable();
    sources.dedup();
    sources
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

/// Invoke `fxc` to compile one entry point to a committed `.dxbc` blob. A compile
/// failure is fatal (panics the build) — when fxc *is* present, a broken shader
/// must not slip through as a silently-stale blob.
fn compile(fxc: &Path, source: &Path, output: &Path, job: &ShaderJob) {
    // /O3 highest optimization; /WX warnings-as-errors so a sloppy translation
    // fails the build rather than shipping; default (column-major) matrix packing.
    let status = Command::new(fxc)
        .arg("/nologo")
        .arg("/T")
        .arg(job.profile)
        .arg("/E")
        .arg(job.entry)
        .arg("/O3")
        .arg("/WX")
        .arg("/Fo")
        .arg(output)
        .arg(source)
        .status()
        .unwrap_or_else(|err| panic!("failed to launch fxc ({}): {err}", fxc.display()));

    if !status.success() {
        panic!(
            "fxc failed to compile {} entry `{}` ({}) -> {}",
            source.display(),
            job.entry,
            job.profile,
            output.display()
        );
    }
}

/// Locate `fxc.exe`: first on `PATH`, then by scanning the Windows SDK's
/// per-version `bin\<ver>\x64` directories and picking the newest. Returns `None`
/// when no SDK is installed (a normal CI box), in which case the committed blobs
/// are used as-is.
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
