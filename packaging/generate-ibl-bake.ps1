<#
.SYNOPSIS
    Re-bake the image-based-lighting (IBL) maps that the viewer ships, but only
    when they are outdated.

.DESCRIPTION
    Shaded mode is lit by precomputed IBL maps (env cube / diffuse irradiance /
    prefiltered specular per environment, plus a shared BRDF integration LUT).
    These are baked offline into `assets/ibl_baked/*.bin` and embedded by the
    `render` crate via `include_bytes!`, so the shipping viewer loads them by a
    plain GPU upload instead of running the ~40-pass precompute at startup. The
    three HDR cubes are BC6H block-compressed (`Bc6hRgbUfloat`, ~8× smaller than
    raw f16); the shared BRDF LUT stays raw little-endian f16 (`Rg16Float`). They
    must exist on disk before `cargo build`.

    This script runs the `bake_ibl` tool (the render crate's `bake` feature),
    which decodes every `assets/textures/T_HDR_*.hdr`, runs the precompute on a
    headless GPU device, reads the results back, BC6H-encodes the cubes (via
    `intel_tex_2`) and writes the `.bin` files.

    Because the bake needs a real GPU and a slow compile, it is *freshness-gated*:
    the bake only runs when a baked `.bin` output is missing or older than an
    input that determines its bytes (a source HDR, or the IBL precompute / encode
    code — `ibl.rs`, `ibl.wgsl`, `bake_ibl.rs`). Otherwise the script is a fast
    no-op (no GPU touched), which is what lets the installer build call it
    unconditionally. Pass `-Force` to re-bake regardless. The mtime idiom mirrors
    `generate-hdr-thumbnails.ps1`.

    Requires a real GPU (the bake creates a wgpu device) and the MSVC toolchain on
    PATH (the workspace compiles the vendored `ufbx.c`), i.e. run it from the
    "x64 Native Tools Command Prompt for VS 2022" like any other build here.

.EXAMPLE
    pwsh packaging\generate-ibl-bake.ps1

.EXAMPLE
    pwsh packaging\generate-ibl-bake.ps1 -Force
#>
[CmdletBinding()]
param(
    # Build + run the bake tool in release mode (faster readback; slower compile).
    [switch] $Release,
    # Re-bake even when the committed .bin outputs already look up to date.
    [switch] $Force
)

$ErrorActionPreference = 'Stop'

# Repo root = parent of this script's folder. Run cargo from there so the
# workspace is in scope; the bake tool resolves asset paths itself.
$repoRoot = Split-Path -Parent $PSScriptRoot

# --- Freshness check -----------------------------------------------------------
# Skip the (GPU + slow-compile) bake when the committed assets\ibl_baked\*.bin
# outputs are already newer than every input that determines their bytes: the
# source HDRs plus the bake/precompute code (constants, precompute shader, BC6H
# encode path). A missing output is stale. Same mtime idiom as
# generate-hdr-thumbnails.ps1.
function Test-IblBakeStale {
    $outputs = @(Get-ChildItem -Path (Join-Path $repoRoot 'assets\ibl_baked\T_IBL_*.bin') -ErrorAction SilentlyContinue)
    if ($outputs.Count -eq 0) { return $true }   # nothing baked yet -> stale

    $inputs = @()
    $inputs += Get-ChildItem -Path (Join-Path $repoRoot 'assets\textures\T_HDR_*.hdr') -ErrorAction SilentlyContinue
    foreach ($rel in @('crates\render\src\ibl.rs', 'crates\render\src\ibl.wgsl', 'crates\render\src\bin\bake_ibl.rs')) {
        $inputs += Get-Item -Path (Join-Path $repoRoot $rel) -ErrorAction SilentlyContinue
    }
    if ($inputs.Count -eq 0) { return $false }   # no inputs resolved -> treat as fresh

    $newestInput  = ($inputs  | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1).LastWriteTimeUtc
    $oldestOutput = ($outputs | Sort-Object LastWriteTimeUtc | Select-Object -First 1).LastWriteTimeUtc
    return $newestInput -gt $oldestOutput
}

if (-not $Force -and -not (Test-IblBakeStale)) {
    Write-Host '==> IBL maps already up to date (skipping bake; pass -Force to re-bake).' -ForegroundColor DarkGray
    return
}

# --- Run the bake --------------------------------------------------------------
Push-Location $repoRoot
try {
    $cargoArgs = @('run', '-p', 'review-render', '--features', 'bake', '--bin', 'bake_ibl')
    if ($Release) { $cargoArgs += '--release' }

    Write-Host "==> Baking IBL maps: cargo $($cargoArgs -join ' ')" -ForegroundColor Cyan
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw "bake_ibl failed (exit $LASTEXITCODE)." }

    Write-Host "==> IBL maps baked into assets\ibl_baked" -ForegroundColor Green
}
finally {
    Pop-Location
}
