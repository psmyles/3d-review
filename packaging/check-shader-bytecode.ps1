<#
.SYNOPSIS
    Fail a release build whose committed shader bytecode does not match the shaders
    in the tree (`docs/ARCHITECTURE.md`, Platform decisions D5).

.DESCRIPTION
    The viewer compiles no shader at run time: it `include_bytes!`s blobs that are
    committed beside the generated sources, and `crates/render/build.rs` rebuilds one
    only when it does not match the source it was compiled from. That gate is enough
    on a dev box with `fxc`. It is not enough for a *release*, for two reasons:

      * a box without the Windows SDK builds happily from whatever blobs are
        committed, and says so only in a `cargo:warning` that scrolls past;
      * from D5 on there are two hosts. A shader edited and rebuilt on the Mac
        arrives here as a new generated source next to a `.dxbc` that is a shader
        revision behind, and `git checkout` gives both the same mtime — so the
        question "is the blob stale?" cannot be answered from timestamps at all on
        the one occasion it matters.

    So this checks content, against the manifest `build.rs` writes: every generated
    source for *this* host must have a row, and both its source and its blob must
    still hash to what that row records. Anything else fails the packaging build.

    The other host's rows are checked too, but only ever **warn**: a Windows
    installer cannot ship a `.metallib`, so a stale one is the Mac's to fix. The
    Mac's own packaging script is what hard-fails on those.

.PARAMETER RepoRoot
    Defaults to the parent of this script's folder.
#>
[CmdletBinding()]
param(
    [string]$RepoRoot
)

$ErrorActionPreference = 'Stop'

# Repo root = parent of this script's folder, as in the other packaging scripts.
# Resolved here rather than as a `param` default: under Windows PowerShell,
# `powershell -File <script>` binds parameters before $PSScriptRoot is populated, so
# the default would come out empty.
if (-not $RepoRoot) { $RepoRoot = Split-Path -Parent $PSScriptRoot }

$generated = Join-Path $RepoRoot 'crates\render\src\shaders\generated'
$manifestPath = Join-Path $generated 'bytecode.manifest'

if (-not (Test-Path -LiteralPath $generated)) {
    throw "not found: $generated (run scripts\gen-shaders.ps1)"
}
if (-not (Test-Path -LiteralPath $manifestPath)) {
    throw @"
$manifestPath is missing.
It is written by crates\render\build.rs and committed beside the blobs; build the
render crate once on a machine with fxc, then commit what it writes.
"@
}

# source file name -> @{ SourceHash; SourceLength; BlobHash; BlobLength }
$manifest = @{}
foreach ($line in Get-Content -LiteralPath $manifestPath) {
    if ($line -match '^\s*(#|$)') { continue }
    $f = $line -split '\s+'
    if ($f.Count -lt 5) { throw "malformed manifest row: $line" }
    $manifest[$f[0]] = @{
        SourceHash   = $f[1].ToLowerInvariant()
        SourceLength = [int64]$f[2]
        BlobHash     = $f[3].ToLowerInvariant()
        BlobLength   = [int64]$f[4]
    }
}

function Test-ShaderSet {
    <#
      Check one host's generated sources against the manifest. Returns the list of
      problems as strings; empty means everything matches.
    #>
    param(
        # e.g. 'hlsl5' with extension 'hlsl', whose blobs are '.dxbc'.
        [string]$Slang,
        [string]$SourceExtension,
        [string]$BlobExtension
    )

    $problems = @()
    $sources = Get-ChildItem -LiteralPath $generated -Filter "review_*_$($Slang)_*.$SourceExtension" |
        Sort-Object Name
    foreach ($source in $sources) {
        $row = $manifest[$source.Name]
        if (-not $row) {
            $problems += "$($source.Name): no manifest row (never compiled here)"
            continue
        }
        $sourceHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $source.FullName).Hash.ToLowerInvariant()
        if ($sourceHash -ne $row.SourceHash) {
            $problems += "$($source.Name): changed since its blob was built"
            continue
        }

        $blob = Join-Path $generated ([IO.Path]::ChangeExtension($source.Name, $BlobExtension))
        if (-not (Test-Path -LiteralPath $blob)) {
            $problems += "$([IO.Path]::GetFileName($blob)): missing"
            continue
        }
        $blobHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $blob).Hash.ToLowerInvariant()
        if ($blobHash -ne $row.BlobHash) {
            $problems += "$([IO.Path]::GetFileName($blob)): does not match the manifest"
        }
    }
    # A row whose source is gone is stale bookkeeping, not a shipping hazard, but it
    # means the manifest was hand-edited or a regeneration was half-committed.
    foreach ($name in $manifest.Keys) {
        if ($name -notlike "review_*_$($Slang)_*.$SourceExtension") { continue }
        if (-not (Test-Path -LiteralPath (Join-Path $generated $name))) {
            $problems += "$name`: manifest row with no source"
        }
    }
    return $problems
}

Write-Host '==> Checking shader bytecode against the committed sources...' -ForegroundColor Cyan

$hlsl = @(Test-ShaderSet -Slang 'hlsl5' -SourceExtension 'hlsl' -BlobExtension 'dxbc')
if ($hlsl.Count -gt 0) {
    Write-Host 'Shader bytecode is out of date:' -ForegroundColor Red
    $hlsl | ForEach-Object { Write-Host "  $_" -ForegroundColor Red }
    throw @"
The committed DXBC does not match the shaders in the tree, so this build would ship
bytecode that is not the shader source beside it. Build the render crate on a machine
with fxc (any `cargo build -p review-render`), then commit the regenerated .dxbc and
bytecode.manifest.
"@
}

# The Mac half: warn only, and stay silent on a tree that carries no Metal blobs.
$metalBlobs = @(Get-ChildItem -LiteralPath $generated -Filter '*.metallib' -ErrorAction SilentlyContinue)
if ($metalBlobs.Count -gt 0) {
    $metal = @(Test-ShaderSet -Slang 'metal_macos' -SourceExtension 'metal' -BlobExtension 'metallib')
    if ($metal.Count -gt 0) {
        Write-Warning 'The macOS shader bytecode is out of date (this installer does not ship it):'
        $metal | ForEach-Object { Write-Warning "  $_" }
        Write-Warning 'Rebuild on the Mac before shipping a macOS build.'
    }
}

$count = (Get-ChildItem -LiteralPath $generated -Filter 'review_*_hlsl5_*.hlsl').Count
Write-Host "==> Shader bytecode is current ($count programs)." -ForegroundColor Green
