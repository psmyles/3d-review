<#
.SYNOPSIS
    Build the optimized 3D Review release exe and package it into a Windows
    installer with Inno Setup.

.DESCRIPTION
    One-command distribution build:
      1. Verifies the MSVC toolchain is on PATH (cc needs `cl`/`rc` to compile
         the vendored ufbx.c). Run from the "x64 Native Tools Command Prompt for
         VS 2022", or from a PowerShell launched within it.
      2. Regenerates the .ico application icon from the source PNG logo *only if*
         it is outdated (the exe resource icon and the installer icon both read
         it from disk at build time, so it must be current before the build).
      3. Regenerates the Environment-dropdown HDR thumbnails (they are baked into
         the exe via include_bytes!, so they must be current before the build).
      4. Re-bakes the BC6H IBL maps *only if* they are outdated (the .bin outputs
         are also include_bytes!'d, so they must be current before the build).
         The freshness gate keeps this a fast no-op unless a source HDR or the IBL
         precompute code changed; an actual re-bake needs a GPU.
      5. cargo build --release -p review-app
      6. Reads product.json (the canonical source of product identity).
      7. Locates ISCC.exe (Inno Setup 6) and compiles packaging\3d-review.iss,
         passing product metadata as /D defines.

    Output: dist\3D-Review-Setup-<version>.exe

.EXAMPLE
    pwsh packaging\build-installer.ps1
#>
[CmdletBinding()]
param(
    [switch] $SkipBuild
)

$ErrorActionPreference = 'Stop'

# Repo root = parent of this script's folder.
$repoRoot = Split-Path -Parent $PSScriptRoot
$productJson = Join-Path $repoRoot 'product.json'
$issScript = Join-Path $PSScriptRoot '3d-review.iss'

# --- 1. Toolchain check --------------------------------------------------------
if (-not $SkipBuild) {
    if (-not (Get-Command cl.exe -ErrorAction SilentlyContinue)) {
        throw "MSVC 'cl.exe' not found on PATH. Run this from the 'x64 Native Tools Command Prompt for VS 2022' so 'cc' can compile ufbx.c."
    }
}

# --- 2. Regenerate the app icon -------------------------------------------------
# The .ico is read from disk by both crates/app/build.rs (exe resource icon) and
# 3d-review.iss (installer icon), so it must be current before either runs.
if (-not $SkipBuild) {
    Write-Host '==> Checking app icon freshness...' -ForegroundColor Cyan
    & (Join-Path $PSScriptRoot 'generate-app-icons.ps1')
}

# --- 3. Regenerate HDR thumbnails ----------------------------------------------
# The Environment-dropdown previews are include_bytes!-embedded, so they must be
# refreshed from the source HDRs before the build bakes them in.
if (-not $SkipBuild) {
    Write-Host '==> Regenerating HDR thumbnails...' -ForegroundColor Cyan
    & (Join-Path $PSScriptRoot 'generate-hdr-thumbnails.ps1')
}

# --- 4. Re-bake IBL maps if outdated -------------------------------------------
# The baked IBL maps (BC6H env/irradiance/prefilter cubes + the BRDF LUT) are
# include_bytes!-embedded too, so they must be current before the build. Unlike
# the thumbnails, baking is a GPU + slow-compile step — so the script only runs
# the actual bake when its .bin outputs are stale vs the source HDRs / IBL code;
# otherwise this is a fast no-op (no GPU touched).
if (-not $SkipBuild) {
    Write-Host '==> Checking IBL bake freshness...' -ForegroundColor Cyan
    & (Join-Path $PSScriptRoot 'generate-ibl-bake.ps1')
}

# --- 5. Build the release exe --------------------------------------------------
if (-not $SkipBuild) {
    Write-Host '==> Building release exe (cargo build --release -p review-app)...' -ForegroundColor Cyan
    Push-Location $repoRoot
    try {
        & cargo build --release -p review-app
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed (exit $LASTEXITCODE)." }
    }
    finally {
        Pop-Location
    }
}

# --- 6. Read product metadata --------------------------------------------------
if (-not (Test-Path $productJson)) { throw "product.json not found at $productJson." }
$meta = Get-Content $productJson -Raw | ConvertFrom-Json

$appName = $meta.productName
$appVersion = $meta.version
$appPublisher = $meta.publisher
$appExe = "$($meta.exeName).exe"
$appProgId = $meta.fileAssociation.progId
$appTypeName = $meta.fileAssociation.typeName
$appUrl = $meta.homepage

$exePath = Join-Path $repoRoot "target\release\$appExe"
if (-not (Test-Path $exePath)) {
    throw "Release exe not found at $exePath. Run without -SkipBuild, or build first."
}

# --- 7. Locate ISCC and compile the installer ----------------------------------
$iscc = (Get-Command 'ISCC.exe' -ErrorAction SilentlyContinue)?.Source
if (-not $iscc) {
    $candidates = @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
    )
    $iscc = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $iscc) {
    throw "ISCC.exe (Inno Setup 6) not found. Install from https://jrsoftware.org/isdl.php or add it to PATH."
}

$distDir = Join-Path $repoRoot 'dist'
New-Item -ItemType Directory -Force -Path $distDir | Out-Null

Write-Host "==> Packaging installer for $appName $appVersion..." -ForegroundColor Cyan
& $iscc `
    "/DMyAppName=$appName" `
    "/DMyAppVersion=$appVersion" `
    "/DMyAppPublisher=$appPublisher" `
    "/DMyAppExe=$appExe" `
    "/DMyAppProgId=$appProgId" `
    "/DMyAppTypeName=$appTypeName" `
    "/DMyAppUrl=$appUrl" `
    $issScript
if ($LASTEXITCODE -ne 0) { throw "ISCC failed (exit $LASTEXITCODE)." }

$installer = Join-Path $distDir "3D-Review-Setup-$appVersion.exe"
Write-Host "==> Done: $installer" -ForegroundColor Green
