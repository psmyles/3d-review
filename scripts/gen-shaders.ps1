# Regenerate the per-backend shader sources and the sokol_gfx reflection from the one
# annotated-GLSL source, `crates/render/src/shaders/review.glsl` (mac-port-plan D4).
#
# The PowerShell twin of `gen-shaders.sh` — same inputs, same outputs, so a Windows shell
# without bash can regenerate. Keep the two in step.
#
# Everything written under `crates/render/src/shaders/generated/` is checked in, so a plain
# `cargo build` never needs sokol-shdc: build.rs only compiles the generated source for the
# host's backend to bytecode (fxc -> DXBC here). Run this after editing review.glsl and
# commit the result; the output is platform-independent text.
#
# Regenerating is only half of it: `cargo build -p review-render` then recompiles the
# bytecode for this host and updates `generated\bytecode.manifest`, and both go in the
# same commit. The other host's blobs are visibly stale until it does the same (D5),
# which is what packaging\check-shader-bytecode.ps1 and its Mac twin exist to catch.
#
# sokol-shdc is a prebuilt binary from floooh/sokol-tools-bin. Set $env:SOKOL_SHDC to point
# at a copy, or let this script fetch one into the repo-ignored `.tools/`.
$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$src = 'crates/render/src/shaders/review.glsl'
$gen = 'crates/render/src/shaders/generated'
$slangs = 'hlsl5:metal_macos'
Set-Location $repo

# --- locate sokol-shdc ------------------------------------------------------
$shdc = $env:SOKOL_SHDC
if (-not $shdc) {
    # `win32` is sokol-tools-bin's name for the Windows build (an x86_64 exe, despite the
    # directory name).
    $shdc = '.tools/sokol-shdc.exe'
    if (-not (Test-Path $shdc)) {
        Write-Host 'fetching sokol-shdc (win32) into .tools/ ...'
        New-Item -ItemType Directory -Force '.tools' | Out-Null
        Invoke-WebRequest -UseBasicParsing -OutFile $shdc `
            'https://raw.githubusercontent.com/floooh/sokol-tools-bin/master/bin/win32/sokol-shdc.exe'
    }
}

New-Item -ItemType Directory -Force $gen | Out-Null

# --- 1. per-backend shader sources, for build.rs to compile to bytecode ------
& $shdc -i $src -o "$gen/review" -l $slangs -f bare
if ($LASTEXITCODE -ne 0) { throw "sokol-shdc failed ($LASTEXITCODE)" }

# --- 2. the sokol_gfx reflection (bindings + entry points) as a Rust module ---
# Without --bytecode on purpose: this file is checked in and must be identical whichever OS
# regenerates it, so it carries *source* and build.rs supplies the bytecode.
& $shdc -i $src -o "$gen/review.rs" -l $slangs -f sokol_rust
if ($LASTEXITCODE -ne 0) { throw "sokol-shdc failed ($LASTEXITCODE)" }

# --- 3. make review.rs the same file whoever regenerated it ------------------
# The Rust emitter's formatting and the absolute -o path shdc echoes into its header are
# machine-specific and change no byte of the shader; left alone they churn thousands of
# lines whenever the regenerating OS changes. `newline_style` is pinned because shdc writes
# CRLF here and LF elsewhere.
if (-not (Get-Command rustfmt -ErrorAction SilentlyContinue)) {
    throw 'rustfmt not found; it ships with the toolchain (rustup component add rustfmt)'
}
rustfmt --edition 2024 --config newline_style=Unix "$gen/review.rs"
if ($LASTEXITCODE -ne 0) { throw "rustfmt failed ($LASTEXITCODE)" }
# The -o path, rewritten repo-relative. Anchored to the Cmdline line so nothing else matches.
$text = [IO.File]::ReadAllText("$repo/$gen/review.rs")
$text = [Regex]::Replace($text, '(sokol-shdc -i .*?)-o \S+', "`$1-o $gen/review.rs")
[IO.File]::WriteAllText("$repo/$gen/review.rs", $text)

Write-Host "regenerated $((Get-ChildItem $gen).Count) files in $gen"
