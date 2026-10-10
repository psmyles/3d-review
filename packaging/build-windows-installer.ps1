<#
.SYNOPSIS
    Build the optimized 3D Review release exe and package it into a Windows
    installer with Inno Setup.

.DESCRIPTION
    One-command distribution build:
      1. Verifies the MSVC toolchain is on PATH (cc needs `cl`/`rc` to compile
         the vendored ufbx.c). Run from the "x64 Native Tools Command Prompt for
         VS 2022", or from a PowerShell launched within it.
      2. Reads product.json (the canonical source of product identity).
      3. Writes product.json's version into crates/app/Cargo.toml and refreshes
         Cargo.lock, so neither can drift from the version the exe reports.
      4. Regenerates the .ico application icon from the source PNG logo *only if*
         it is outdated (the exe resource icon and the installer icon both read
         it from disk at build time, so it must be current before the build).
      5. Regenerates the Environment-dropdown HDR thumbnails (they are baked into
         the exe via include_bytes!, so they must be current before the build).
      6. Re-bakes the BC6H IBL maps *only if* they are outdated (the .bin outputs
         are also include_bytes!'d, so they must be current before the build).
         The freshness gate keeps this a fast no-op unless a source HDR or the IBL
         precompute code changed; an actual re-bake needs a GPU.
      7. cargo build --release -p review-app
      8. Verifies the committed shader bytecode matches the shader beside it.
      9. Checks the built exe is there, locates ISCC.exe (Inno Setup 6) and
         compiles packaging\3d-review.iss, passing product metadata as /D defines.

    Output: dist\<exeName>-<version>.exe (e.g. dist\3d-review-0.5.1.exe), the same
    naming as the macOS .dmg.

    It edits two tracked files — crates/app/Cargo.toml and Cargo.lock — which is
    step 3; the macOS twin (scripts/build-mac.sh) does the same. Everything else
    it writes goes to target\ and dist\.

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

# --- 2. Read product metadata ---------------------------------------------------
# Read before anything is built: the version below is what the crate manifest, the
# exe's version resource and the installer are all synced to.
if (-not (Test-Path $productJson)) { throw "product.json not found at $productJson." }
$meta = Get-Content $productJson -Raw | ConvertFrom-Json

$appName = $meta.productName
$appVersion = $meta.version
$appPublisher = $meta.publisher
$appFileStem = $meta.exeName
$appExe = "$appFileStem.exe"
$appProgId = $meta.fileAssociation.progId
$appTypeName = $meta.fileAssociation.typeName
$appUrl = $meta.homepage

# --- 3. Sync the crate version to product.json ----------------------------------
# product.json is the single source of the version, but nothing in a `cargo build`
# reads it into the manifest — crates\app\build.rs only *warns* when the two
# disagree, in a `cargo:warning` that scrolls past. So the release syncs it, exactly
# as scripts/build-mac.sh does.
#
# Cargo.lock records the member's version too, and is committed. `cargo update
# --workspace` re-resolves the workspace members *only*, so the lock picks the new
# version up while every registry dependency stays pinned exactly as it was — which a
# bare `cargo generate-lockfile` would not guarantee. Deliberately not `--offline`: on
# a machine whose registry cache is cold that would fail here, before the build that
# would have populated it.
Write-Host "==> Syncing crates\app\Cargo.toml version to $appVersion..." -ForegroundColor Cyan
$appToml = Join-Path $repoRoot 'crates\app\Cargo.toml'
if (-not (Test-Path $appToml)) { throw "crates\app\Cargo.toml not found at $appToml." }
$tomlText = Get-Content $appToml -Raw
# The [package] version is the file's first `version = "..."` line; a dependency's is
# written inline beside its name, so a line-anchored match cannot reach one.
$versionRx = [regex] '(?m)^version\s*=\s*"([^"]*)"'
$versionMatch = $versionRx.Match($tomlText)
if (-not $versionMatch.Success) { throw "no version found in $appToml." }
$currentVersion = $versionMatch.Groups[1].Value
if ($currentVersion -eq $appVersion) {
    Write-Host "    already in sync."
}
else {
    # WriteAllText with an explicit no-BOM encoding: Set-Content -Encoding utf8 adds a
    # BOM under Windows PowerShell 5.1, and the manifest is committed.
    [System.IO.File]::WriteAllText(
        $appToml,
        $versionRx.Replace($tomlText, "version = `"$appVersion`"", 1),
        (New-Object System.Text.UTF8Encoding $false))
    Write-Host "    updated: $currentVersion -> $appVersion."
    if ($SkipBuild) {
        Write-Warning "-SkipBuild, so target\release still holds a $currentVersion exe."
    }
}

# Run unconditionally: the manifest can already be in sync while the lock is not
# (someone edited Cargo.toml by hand), and with both in sync it is a no-op.
& cargo update --manifest-path (Join-Path $repoRoot 'Cargo.toml') --workspace --quiet
if ($LASTEXITCODE -ne 0) { throw "cargo update failed (exit $LASTEXITCODE) — Cargo.lock is not synced to $appVersion." }
$lockText = Get-Content (Join-Path $repoRoot 'Cargo.lock') -Raw
$lockMatch = [regex]::Match($lockText, '(?m)^name = "review-app"\r?\n^version = "([^"]*)"')
if (-not $lockMatch.Success) { throw "no review-app package found in Cargo.lock." }
if ($lockMatch.Groups[1].Value -ne $appVersion) {
    throw "Cargo.lock still records review-app $($lockMatch.Groups[1].Value), not $appVersion."
}
Write-Host "    Cargo.lock: review-app $appVersion."

# --- 4. Regenerate the app icon -------------------------------------------------
# The .ico is read from disk by both crates/app/build.rs (exe resource icon) and
# 3d-review.iss (installer icon), so it must be current before either runs.
if (-not $SkipBuild) {
    Write-Host '==> Checking app icon freshness...' -ForegroundColor Cyan
    & (Join-Path $PSScriptRoot 'generate-app-icons.ps1')
}

# --- 5. Regenerate HDR thumbnails ----------------------------------------------
# The Environment-dropdown previews are include_bytes!-embedded, so they must be
# refreshed from the source HDRs before the build bakes them in.
if (-not $SkipBuild) {
    Write-Host '==> Regenerating HDR thumbnails...' -ForegroundColor Cyan
    & (Join-Path $PSScriptRoot 'generate-hdr-thumbnails.ps1')
}

# --- 6. Re-bake IBL maps if outdated -------------------------------------------
# The baked IBL maps (BC6H env/irradiance/prefilter cubes + the BRDF LUT) are
# include_bytes!-embedded too, so they must be current before the build. Unlike
# the thumbnails, baking is a GPU + slow-compile step — so the script only runs
# the actual bake when its .bin outputs are stale vs the source HDRs / IBL code;
# otherwise this is a fast no-op (no GPU touched).
if (-not $SkipBuild) {
    Write-Host '==> Checking IBL bake freshness...' -ForegroundColor Cyan
    & (Join-Path $PSScriptRoot 'generate-ibl-bake.ps1')
}

# --- 7. Build the release exe --------------------------------------------------
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

# --- 8. Verify the committed shader bytecode -----------------------------------
# The viewer `include_bytes!`s blobs committed beside the generated shader sources
# and compiles nothing at run time, so an installer must not ship bytecode that is
# not the shader beside it. build.rs rebuilds a stale blob where fxc is present (the
# step above) and only *warns* where it is not; this fails instead — and it is what
# catches a shader rebuilt on the Mac and not here, which no timestamp comparison can
# (`docs/ARCHITECTURE.md`, Platform decisions D5).
Write-Host '==> Checking shader bytecode...' -ForegroundColor Cyan
& (Join-Path $PSScriptRoot 'check-shader-bytecode.ps1')

# --- 8b. Verify the license notices are present ---------------------------------
# 3d-review.iss installs these beside the exe. The statically linked exe carries
# code from many other projects whose licenses require their notices to travel
# with the binary, so an installer without them is a compliance bug, not a
# cosmetic one. Fail here with a clear message rather than letting ISCC report a
# missing source file.
Write-Host '==> Verifying license notices...' -ForegroundColor Cyan
$noticeFiles = 'LICENSE', 'docs\THIRD-PARTY-NOTICES.md', 'docs\CREDITS.md'
$absent = $noticeFiles | Where-Object { -not (Test-Path (Join-Path $repoRoot $_)) }
if ($absent) { throw "Missing license file(s) the installer must ship: $($absent -join ', ')" }
$licenseTexts = @(Get-ChildItem -Path (Join-Path $repoRoot 'licenses') -Filter '*.txt' -ErrorAction SilentlyContinue)
if ($licenseTexts.Count -eq 0) { throw "No license texts found in $repoRoot\licenses\ (the installer ships licenses\*.txt)" }
Write-Host "    $($noticeFiles.Count) notice files + $($licenseTexts.Count) license texts."

# --- 9. Package the installer ---------------------------------------------------
$exePath = Join-Path $repoRoot "target\release\$appExe"
if (-not (Test-Path $exePath)) {
    throw "Release exe not found at $exePath. Run without -SkipBuild, or build first."
}

# ISCC (Inno Setup 6) reads packaging\3d-review.iss and takes the product metadata
# above as /D defines.
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
    "/DMyAppFileStem=$appFileStem" `
    "/DMyAppProgId=$appProgId" `
    "/DMyAppTypeName=$appTypeName" `
    "/DMyAppUrl=$appUrl" `
    $issScript
if ($LASTEXITCODE -ne 0) { throw "ISCC failed (exit $LASTEXITCODE)." }

$installer = Join-Path $distDir "$appFileStem-$appVersion.exe"
Write-Host "==> Done: $installer" -ForegroundColor Green
