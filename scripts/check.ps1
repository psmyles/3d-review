# The gate: everything that must pass before a change is claimed done.
#
# The same four checks CLAUDE.md and README.md have always named, in one command
# so there is no set to remember and no chance of running three of them. Nothing
# here is new policy — the point is that "did you run the gate" has one answer.
#
# The PowerShell twin of `check.sh`; keep the two in step.
#
# Fixture-dependent suites are run with REVIEW_REQUIRE_FIXTURES=1, so a missing
# asset or an un-vendored dependency fails loudly instead of being skipped.
# Skipping is right for a fresh checkout and wrong here: without it a green run
# means "nothing was found to run" just as readily as "everything passed". Pass
# -AllowSkips for a checkout that genuinely lacks the fixtures.
#
# Needs the x64 Native Tools Command Prompt for VS 2022 (MSVC `cl` on PATH) so
# `cc` can build the vendored C.
[CmdletBinding()]
param(
    # Don't fail on a skipped fixture suite — for a checkout without
    # assets/test_models or a vendored tree.
    [switch]$AllowSkips,
    # Skip the shader bytecode freshness check (it needs no GPU, only the files).
    [switch]$NoShaderCheck
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Push-Location $repo
try {
    $failures = @()

    function Invoke-Step {
        param([string]$Name, [scriptblock]$Body)
        Write-Host ""
        Write-Host "=== $Name ===" -ForegroundColor Cyan
        # Reset first: a step that runs a *script* rather than a native command
        # leaves $LASTEXITCODE at whatever the previous native command set, so a
        # passing shader check would inherit a failing cargo test's 101.
        $global:LASTEXITCODE = 0
        $ok = $true
        try {
            & $Body
        } catch {
            Write-Host $_ -ForegroundColor Red
            $ok = $false
        }
        if (-not $ok -or $LASTEXITCODE -ne 0) {
            $script:failures += $Name
            Write-Host "$Name FAILED" -ForegroundColor Red
        }
    }

    Invoke-Step 'cargo fmt --check' { cargo fmt --all -- --check }
    Invoke-Step 'cargo clippy' { cargo clippy --workspace --all-targets -- -D warnings }

    if (-not $AllowSkips) {
        $env:REVIEW_REQUIRE_FIXTURES = '1'
    }
    Invoke-Step 'cargo test' { cargo test --workspace }
    Remove-Item Env:\REVIEW_REQUIRE_FIXTURES -ErrorAction SilentlyContinue

    if (-not $NoShaderCheck) {
        Invoke-Step 'shader bytecode' {
            & (Join-Path $PSScriptRoot '../packaging/check-shader-bytecode.ps1')
        }
    }

    Write-Host ""
    if ($failures.Count -gt 0) {
        Write-Host ("FAILED: " + ($failures -join ', ')) -ForegroundColor Red
        exit 1
    }
    Write-Host "All checks passed." -ForegroundColor Green
} finally {
    Pop-Location
}
