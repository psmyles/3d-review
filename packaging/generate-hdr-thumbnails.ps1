<#
.SYNOPSIS
    Regenerate the Environment-dropdown HDR preview thumbnails from the source
    HDR environments.

.DESCRIPTION
    The Environment panel shows a small preview beside each HDR option. Those
    previews are PNGs the `ui` crate bakes in via `include_bytes!`, so they must
    exist on disk before `cargo build`. This script tone-maps every
    `assets/textures/T_HDR_*.hdr` down to a matching
    `assets/thumbnails/<name>.png`, so adding/replacing an HDR (and wiring up a
    new `EnvironmentMap` variant) just needs a re-run.

    Tone map: resize in linear light (`-colorspace RGB`), then encode to sRGB
    (`-colorspace sRGB`). Values above 1.0 (a bright sun) clip to white, which is
    fine for a thumbnail. Output is a fixed THUMB_W x THUMB_H (2:1) PNG, matching
    the dropdown's 2:1 thumbnail slot.

    Requires the ImageMagick CLI (`magick`) on the BUILD machine only (this is a
    dev-time asset-gen step; the app no longer bundles or ships ImageMagick). It is
    searched for on PATH first, then in packaging\magick\.

.EXAMPLE
    pwsh packaging\generate-hdr-thumbnails.ps1
#>
[CmdletBinding()]
param(
    # Force-rebuild every thumbnail even if it is newer than its source HDR.
    [switch] $Force
)

$ErrorActionPreference = 'Stop'

# Source HDRs and their generated previews, both relative to the repo root
# (parent of this script's folder).
$repoRoot   = Split-Path -Parent $PSScriptRoot
$textureDir = Join-Path $repoRoot 'assets\textures'
$thumbDir   = Join-Path $repoRoot 'assets\thumbnails'

# Thumbnail dimensions (2:1). The dropdown displays them smaller (see the
# ENV_THUMB tokens in crates/ui/src/theme.rs); these are generated at 2x so they
# stay crisp on high-DPI displays.
$thumbW = 128
$thumbH = 64

# --- Locate the ImageMagick CLI ------------------------------------------------
$magick = (Get-Command 'magick.exe' -ErrorAction SilentlyContinue)?.Source
if (-not $magick) {
    $bundled = Join-Path $PSScriptRoot 'magick\magick.exe'
    if (Test-Path $bundled) { $magick = $bundled }
}
if (-not $magick) {
    throw "ImageMagick 'magick.exe' not found on PATH or in packaging\magick\. Install from https://imagemagick.org/ or stage it in packaging\magick\."
}

# --- Generate one PNG per source HDR -------------------------------------------
New-Item -ItemType Directory -Force -Path $thumbDir | Out-Null

$sources = Get-ChildItem -Path (Join-Path $textureDir 'T_HDR_*.hdr') -ErrorAction SilentlyContinue
if (-not $sources) {
    throw "No T_HDR_*.hdr files found in $textureDir."
}

foreach ($src in $sources) {
    $dst = Join-Path $thumbDir ($src.BaseName + '.png')

    if (-not $Force -and (Test-Path $dst) -and `
        (Get-Item $dst).LastWriteTimeUtc -ge $src.LastWriteTimeUtc) {
        Write-Host "==> Up to date: $($src.BaseName).png" -ForegroundColor DarkGray
        continue
    }

    Write-Host "==> Thumbnail: $($src.Name) -> thumbnails\$($src.BaseName).png" -ForegroundColor Cyan
    & $magick $src.FullName -colorspace RGB -resize "${thumbW}x${thumbH}!" -colorspace sRGB $dst
    if ($LASTEXITCODE -ne 0) { throw "magick failed for $($src.Name) (exit $LASTEXITCODE)." }
}

Write-Host "==> HDR thumbnails up to date in $thumbDir" -ForegroundColor Green
