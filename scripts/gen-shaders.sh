#!/usr/bin/env bash
# Regenerate the per-backend shader sources and the sokol_gfx reflection from the one
# annotated-GLSL source, `crates/render/src/shaders/review.glsl` (mac-port-plan D4).
#
# Everything this writes under `crates/render/src/shaders/generated/` is checked in, so a
# plain `cargo build` never needs sokol-shdc — build.rs only compiles the generated source
# for the host's backend to bytecode (fxc -> DXBC on Windows, `xcrun metal` -> .metallib on
# macOS). Run this after editing review.glsl, on either OS, and commit the result: the
# output is platform-independent text.
#
# The PowerShell twin `gen-shaders.ps1` does the same thing for a Windows shell without
# bash. Keep the two in step.
#
# sokol-shdc is a prebuilt binary from floooh/sokol-tools-bin. Point SOKOL_SHDC at a copy,
# or let this script fetch one into the repo-ignored `.tools/`.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
src="crates/render/src/shaders/review.glsl"
gen="crates/render/src/shaders/generated"
slangs="hlsl5:metal_macos"
cd "$repo"

# --- locate sokol-shdc ------------------------------------------------------
shdc="${SOKOL_SHDC:-}"
if [[ -z "$shdc" ]]; then
    # One binary per platform directory in sokol-tools-bin. `win32` is that repo's name for
    # the Windows build (an x86_64 exe, despite the directory name) and the only one
    # carrying a file suffix, so the suffix travels with the platform rather than being
    # appended at each use site.
    exe=""
    case "$(uname -s)-$(uname -m)" in
        Darwin-arm64) plat=osx_arm64 ;;
        Darwin-x86_64) plat=osx ;;
        Linux-x86_64) plat=linux ;;
        # Git Bash / MSYS2 / Cygwin, whose `uname -s` is `MINGW64_NT-10.0-26200` and kin.
        MINGW*|MSYS*|CYGWIN*) plat=win32; exe=.exe ;;
        *) echo "no sokol-shdc build known for $(uname -s)-$(uname -m); set SOKOL_SHDC" >&2; exit 1 ;;
    esac
    shdc=".tools/sokol-shdc$exe"
    if [[ ! -x "$shdc" ]]; then
        echo "fetching sokol-shdc ($plat) into .tools/ ..."
        mkdir -p .tools
        curl -sSLf -o "$shdc" \
            "https://raw.githubusercontent.com/floooh/sokol-tools-bin/master/bin/$plat/sokol-shdc$exe"
        chmod +x "$shdc"
    fi
fi

mkdir -p "$gen"

# --- 1. per-backend shader sources, for build.rs to compile to bytecode ------
"$shdc" -i "$src" -o "$gen/review" -l "$slangs" -f bare

# --- 2. the sokol_gfx reflection (bindings + entry points) as a Rust module ---
# Generated without --bytecode on purpose: this file is checked in and must be identical
# whichever OS regenerates it, so it carries *source* and build.rs supplies the bytecode.
"$shdc" -i "$src" -o "$gen/review.rs" -l "$slangs" -f sokol_rust

# --- 3. make review.rs the same file whoever regenerated it ------------------
# Two things in the sokol_rust output are machine-specific, and neither changes a byte of
# the shader: the -i/-o paths shdc echoes into its header comment (absolute for -o), and
# the Rust emitter's formatting, which differs between the per-platform sokol-shdc binaries
# because they are built from different upstream revisions. Left alone they churn thousands
# of lines every time the OS regenerating this changes, burying the handful of lines a real
# shader edit moves.
#
# rustfmt settles the formatting half (the repo carries no rustfmt.toml, so this is plain
# defaults). `newline_style` is pinned rather than left on Auto because shdc writes CRLF on
# Windows and LF elsewhere, and Auto would preserve each.
if ! command -v rustfmt >/dev/null; then
    echo "rustfmt not found; it ships with the toolchain (rustup component add rustfmt)" >&2
    exit 1
fi
rustfmt --edition 2024 --config newline_style=Unix "$gen/review.rs"
# The -o path, rewritten repo-relative. Anchored to the Cmdline line so nothing else can match.
sed -E "/sokol-shdc -i /s#-o [^ ]*#-o $gen/review.rs#" "$gen/review.rs" > "$gen/review.rs.tmp"
mv "$gen/review.rs.tmp" "$gen/review.rs"

echo "regenerated $(ls -1 "$gen" | wc -l) files in $gen"
