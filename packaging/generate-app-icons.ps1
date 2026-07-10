<#
.SYNOPSIS
    Regenerate the Windows .ico application icon from the source PNG logo.

.DESCRIPTION
    `assets/icons/application-logo.ico` is a multi-resolution icon derived from
    `assets/icons/application-logo.png`. It's used for the exe resource icon
    (crates/app/build.rs, what Explorer/pinned taskbar shortcuts show) and the
    Inno Setup installer icon (packaging/3d-review.iss). Both are read straight
    off disk at build time (no include_bytes!), but they must still be current
    whenever the source PNG changes, so this mirrors
    generate-hdr-thumbnails.ps1's freshness-gate pattern.

    Requires the ImageMagick CLI (`magick`). It is searched for on PATH first,
    then in packaging\magick\ (the copy the installer bundles).

.EXAMPLE
    pwsh packaging\generate-app-icons.ps1
#>
[CmdletBinding()]
param(
    # Force-rebuild the icon even if it is newer than the source PNG.
    [switch] $Force
)

$ErrorActionPreference = 'Stop'

# Source PNG and generated .ico, both relative to the repo root (parent of this
# script's folder).
$repoRoot = Split-Path -Parent $PSScriptRoot
$iconDir  = Join-Path $repoRoot 'assets\icons'
$srcPng   = Join-Path $iconDir 'application-logo.png'
$dstIco   = Join-Path $iconDir 'application-logo.ico'

# Sizes baked into the .ico, largest first, matching Windows' expectations for
# Explorer thumbnails (256), taskbar/shortcut (48), Alt+Tab (32), and title bar
# small icons (16).
$sizes = '256,48,32,16'

# --- Locate the ImageMagick CLI ------------------------------------------------
$magick = (Get-Command 'magick.exe' -ErrorAction SilentlyContinue)?.Source
if (-not $magick) {
    $bundled = Join-Path $PSScriptRoot 'magick\magick.exe'
    if (Test-Path $bundled) { $magick = $bundled }
}
if (-not $magick) {
    throw "ImageMagick 'magick.exe' not found on PATH or in packaging\magick\. Install from https://imagemagick.org/ or stage it in packaging\magick\."
}

if (-not (Test-Path $srcPng)) {
    throw "Source logo not found at $srcPng."
}

# --- Regenerate the .ico if stale ----------------------------------------------
if (-not $Force -and (Test-Path $dstIco) -and `
    (Get-Item $dstIco).LastWriteTimeUtc -ge (Get-Item $srcPng).LastWriteTimeUtc) {
    Write-Host '==> Up to date: application-logo.ico' -ForegroundColor DarkGray
    return
}

Write-Host '==> Regenerating application-logo.ico from application-logo.png...' -ForegroundColor Cyan
& $magick $srcPng -define "icon:auto-resize=$sizes" -background none $dstIco
if ($LASTEXITCODE -ne 0) { throw "magick failed to build application-logo.ico (exit $LASTEXITCODE)." }

Write-Host "==> App icon up to date: $dstIco" -ForegroundColor Green
