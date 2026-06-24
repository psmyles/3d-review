<#
.SYNOPSIS
    Re-bake the image-based-lighting (IBL) maps that the viewer ships.

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
    Re-run it whenever a source HDR changes (or a new `EnvironmentMap` variant is
    added); the outputs are committed.

    Requires a real GPU (the bake creates a wgpu device) and the MSVC toolchain on
    PATH (the workspace compiles the vendored `ufbx.c`), i.e. run it from the
    "x64 Native Tools Command Prompt for VS 2022" like any other build here.

.EXAMPLE
    pwsh packaging\generate-ibl-bake.ps1
#>
[CmdletBinding()]
param(
    # Build + run the bake tool in release mode (faster readback; slower compile).
    [switch] $Release
)

$ErrorActionPreference = 'Stop'

# Repo root = parent of this script's folder. Run cargo from there so the
# workspace is in scope; the bake tool resolves asset paths itself.
$repoRoot = Split-Path -Parent $PSScriptRoot
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
