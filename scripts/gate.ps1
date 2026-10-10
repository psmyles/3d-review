<#
.SYNOPSIS
    The D2 gate: measure two builds of the viewer against each other on this box.

.DESCRIPTION
    Platform decision D2 (`docs/ARCHITECTURE.md`) let the Windows migration land
    only if it cost nothing measurable against `main`: same box, same fixture,
    release build, three measurements — startup, frame time, memory — with a
    budget on each.

    Each launch runs the viewer's own `--gate-out <file>` mode (`crates/app/src/gate.rs`):
    it loads the fixture, plays a scripted orbit, writes one JSON stamp and then sits
    idle with the model resident so this script can sample memory from outside. The two
    builds are launched **interleaved** — A, B, A, B — rather than all of A then all of
    B, so a machine that warms up or throttles part way through the run does it to both.
    One discarded warm-up launch of each comes first; the first launch after a build is
    an outlier (page faults on a cold binary, a cold shader cache).

    The three measurements:

      * startup  process creation -> the first present with the model on screen.
                 The far end is the stamp's wall clock; the near end is
                 `Process.StartTime` (`GetProcessTimes`), read here rather than in the
                 viewer, which is `forbid(unsafe_code)`.
      * frame    median present-to-present over 300 frames, vsync off. Meaningful only
                 because the swapchain allows tearing — a flip-model present with a
                 sync interval of 0 is still paced by DWM, and measured the monitor
                 (a median of exactly 8.34 ms on both builds) until that was fixed.
                 CPU-only frame time is reported beside it: a regression in one and not
                 the other says different things.
      * memory   private bytes + GPU dedicated bytes, sampled while the process idles
                 with the model up.

    Budgets are on the **delta** (B - A), not on absolutes: 10 ms startup, 0.3 ms
    frame, 5 % memory. Exits non-zero if any is exceeded.

.EXAMPLE
    scripts\gate.ps1 -A ..\3d-review-main\target\release\3d-review.exe `
                     -B .\target\release\3d-review.exe `
                     -Fixture .\assets\test_models\SK_Player_01.fbx -Runs 5
#>
[CmdletBinding()]
param(
    # The baseline build (`main`).
    [Parameter(Mandatory)][string]$A,
    # The build under test (the port).
    [Parameter(Mandatory)][string]$B,
    [Parameter(Mandatory)][string]$Fixture,
    # Measured launches per build. Five is enough for a median to settle; the run
    # costs about 15 s per launch.
    [int]$Runs = 5,
    # Where the stamps are written. Defaults to a temp directory.
    [string]$Out = (Join-Path $env:TEMP "3d-review-gate"),
    # Budgets on B - A.
    [double]$StartupBudgetMs = 10.0,
    [double]$FrameBudgetMs = 0.3,
    [double]$MemoryBudgetPercent = 5.0
)

$ErrorActionPreference = 'Stop'

foreach ($path in @($A, $B, $Fixture)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "not found: $path" }
}
$A = (Resolve-Path -LiteralPath $A).Path
$B = (Resolve-Path -LiteralPath $B).Path
$Fixture = (Resolve-Path -LiteralPath $Fixture).Path
New-Item -ItemType Directory -Force $Out | Out-Null

# The Unix epoch in UTC, to turn the stamp's nanoseconds back into a DateTime.
$epoch = [DateTime]::SpecifyKind([DateTime]::new(1970, 1, 1), [DateTimeKind]::Utc)

function Get-GpuDedicatedBytes {
    <#
      GPU memory for one process, summed over its adapters. The counter instance is
      named `pid_<pid>_luid_...`, one per adapter the process has allocated on, so a
      hybrid-graphics box reports more than one and they add up. Returns $null when
      the counter set is unavailable (an older Windows, or a disabled provider)
      rather than failing the run — the RAM half of the budget still applies.
    #>
    param([int]$ProcessId)
    try {
        $samples = (Get-Counter "\GPU Process Memory(pid_$($ProcessId)_*)\Dedicated Usage" `
                -ErrorAction Stop).CounterSamples
        return ($samples | Measure-Object -Property CookedValue -Sum).Sum
    } catch {
        return $null
    }
}

function Invoke-GateRun {
    <#
      One launch: run the viewer in gate mode, sample memory during the idle hold it
      leaves for exactly this, and return the parsed stamp plus what was sampled.
    #>
    param([string]$Exe, [string]$Label, [int]$Index)

    $stamp = Join-Path $Out ("{0}-{1}.json" -f $Label, $Index)
    Remove-Item -LiteralPath $stamp -ErrorAction SilentlyContinue

    $proc = Start-Process -FilePath $Exe -PassThru `
        -ArgumentList @($Fixture, '--gate-out', $stamp)

    # Wait for the stamp. The generous ceiling covers a cold first launch on a busy
    # box; a hung viewer is killed rather than left behind.
    $deadline = (Get-Date).AddSeconds(120)
    while (-not (Test-Path -LiteralPath $stamp)) {
        if ($proc.HasExited) { throw "$Label run $Index exited before writing a stamp" }
        if ((Get-Date) -gt $deadline) {
            $proc.Kill()
            throw "$Label run $Index timed out waiting for its stamp"
        }
        Start-Sleep -Milliseconds 25
    }

    # The stamp is written before the hold begins, so the process is idle with the
    # model resident right now — which is the state D2 asks for.
    $private = $null
    $gpu = $null
    try {
        $live = Get-Process -Id $proc.Id -ErrorAction Stop
        $gpu = Get-GpuDedicatedBytes -ProcessId $proc.Id
        $live.Refresh()
        $private = $live.PrivateMemorySize64
    } catch {
        Write-Warning "$Label run ${Index}: memory sample failed ($($_.Exception.Message))"
    }

    if (-not $proc.WaitForExit(30000)) {
        Write-Warning "$Label run ${Index}: did not exit on its own; killing"
        $proc.Kill()
        $proc.WaitForExit(5000) | Out-Null
    }

    $json = Get-Content -LiteralPath $stamp -Raw | ConvertFrom-Json
    if ($json.frames -lt 1) { throw "$Label run $Index measured no frames" }

    # Startup: process creation (GetProcessTimes, via .NET) to the stamped first
    # present. Both are the same system clock, so the subtraction is valid.
    $firstPresent = $epoch.AddTicks([long]([double]$json.first_present_unix_ns / 100.0))
    $startupMs = ($firstPresent - $proc.StartTime.ToUniversalTime()).TotalMilliseconds

    [pscustomobject]@{
        Label        = $Label
        Index        = $Index
        StartupMs    = $startupMs
        FrameMs      = [double]$json.frame_ms.median
        FrameP95Ms   = [double]$json.frame_ms.p95
        CpuMs        = [double]$json.cpu_ms.median
        PrivateBytes = $private
        GpuBytes     = $gpu
        Settings     = $json.settings
        Model        = $json.model
        # Absent from a build that predates the audit, null when none landed.
        AuditMs      = if ($json.audit) { [double]$json.audit.ms } else { $null }
        AuditReportBytes = if ($json.audit) { [double]$json.audit.report_bytes } else { $null }
    }
}

function Get-Median {
    param([double[]]$Values)
    if (-not $Values -or $Values.Count -eq 0) { return $null }
    $sorted = $Values | Sort-Object
    $mid = [int][Math]::Floor($sorted.Count / 2)
    if ($sorted.Count % 2 -eq 1) { return $sorted[$mid] }
    return ($sorted[$mid - 1] + $sorted[$mid]) / 2.0
}

function Get-MedianOf {
    param([object[]]$Rows, [string]$Property)
    $values = $Rows | ForEach-Object { $_.$Property } | Where-Object { $null -ne $_ }
    if (-not $values) { return $null }
    return Get-Median -Values ([double[]]$values)
}

Write-Host ""
Write-Host "D2 gate" -ForegroundColor Cyan
Write-Host "  A (baseline): $A"
Write-Host "  B (under test): $B"
Write-Host "  fixture: $Fixture"
Write-Host "  runs: $Runs measured, 1 warm-up each, interleaved"
Write-Host ""

# Warm-up: discarded. The first launch after a build pays for a cold binary and a
# cold driver shader cache, and would land entirely on whichever build went first.
Write-Host "warming up..." -ForegroundColor DarkGray
Invoke-GateRun -Exe $A -Label 'warmup-A' -Index 0 | Out-Null
Invoke-GateRun -Exe $B -Label 'warmup-B' -Index 0 | Out-Null

$rowsA = @()
$rowsB = @()
for ($i = 1; $i -le $Runs; $i++) {
    Write-Host ("run {0}/{1}" -f $i, $Runs) -ForegroundColor DarkGray
    $rowsA += Invoke-GateRun -Exe $A -Label 'A' -Index $i
    $rowsB += Invoke-GateRun -Exe $B -Label 'B' -Index $i
}

# Written before anything is formatted, so the measurements survive a reporting
# bug — a gate run is minutes of launches and must not have to be repeated for one.
($rowsA + $rowsB) | ConvertTo-Json -Depth 5 |
    Set-Content -LiteralPath (Join-Path $Out 'runs.json') -Encoding utf8

# The two builds must have measured the same thing, or the comparison is decoration.
$sa = $rowsA[0].Settings
$sb = $rowsB[0].Settings
foreach ($field in @('msaa', 'gtao', 'clip_playing', 'width', 'height', 'vsync')) {
    if ($sa.$field -ne $sb.$field) {
        throw "settings differ between builds ($field): A=$($sa.$field) B=$($sb.$field)"
    }
}
if ($rowsA[0].Model.triangles -ne $rowsB[0].Model.triangles) {
    throw "the two builds imported different geometry from the same fixture"
}

Write-Host ""
Write-Host ("conditions: {0}x{1}, {2}x MSAA, GTAO {3}, clip playing {4}, {5} triangles, {6} clips" -f `
        $sa.width, $sa.height, $sa.msaa, $sa.gtao, $sa.clip_playing,
    $rowsA[0].Model.triangles, $rowsA[0].Model.clips) -ForegroundColor DarkGray
Write-Host ""

$measures = @(
    @{ Name = 'startup (ms)'; Property = 'StartupMs'; Budget = $StartupBudgetMs; Relative = $false; Format = 'N1' }
    @{ Name = 'frame (ms)'; Property = 'FrameMs'; Budget = $FrameBudgetMs; Relative = $false; Format = 'N3' }
    @{ Name = 'frame p95 (ms)'; Property = 'FrameP95Ms'; Budget = $null; Relative = $false; Format = 'N3' }
    @{ Name = 'cpu/frame (ms)'; Property = 'CpuMs'; Budget = $null; Relative = $false; Format = 'N3' }
    @{ Name = 'private (MB)'; Property = 'PrivateBytes'; Budget = $MemoryBudgetPercent; Relative = $true; Format = 'N1' }
    @{ Name = 'gpu (MB)'; Property = 'GpuBytes'; Budget = $MemoryBudgetPercent; Relative = $true; Format = 'N1' }
    @{ Name = 'audit (ms)'; Property = 'AuditMs'; Budget = $null; Relative = $false; Format = 'N1' }
    @{ Name = 'audit report (MB)'; Property = 'AuditReportBytes'; Budget = $null; Relative = $false; Format = 'N3' }
)

$failures = @()
$table = foreach ($m in $measures) {
    $medA = Get-MedianOf -Rows $rowsA -Property $m.Property
    $medB = Get-MedianOf -Rows $rowsB -Property $m.Property
    if ($null -eq $medA -or $null -eq $medB) {
        # A figure only one build records (the baseline predates it) is still
        # worth reading; there is just nothing to subtract.
        $scale = if ($m.Property -like '*Bytes') { 1MB } else { 1.0 }
        $shownA = if ($null -ne $medA) { "{0:$($m.Format)}" -f ([double]$medA / $scale) } else { 'n/a' }
        $shownB = if ($null -ne $medB) { "{0:$($m.Format)}" -f ([double]$medB / $scale) } else { 'n/a' }
        [pscustomobject]@{ Measure = $m.Name; A = $shownA; B = $shownB; Delta = 'n/a'; Budget = 'n/a'; Verdict = 'skipped' }
        continue
    }
    $scale = if ($m.Property -like '*Bytes') { 1MB } else { 1.0 }
    $a = [double]$medA / $scale
    $b = [double]$medB / $scale
    $delta = $b - $a
    $percent = if ($a -ne 0) { 100.0 * $delta / $a } else { 0.0 }
    # `-f` rather than `.ToString($format)`: the format string is per-measure data,
    # and a numeric type that does not carry the overload (a summed counter comes
    # back boxed) turns a reporting detail into a failed run at the last step.
    $fmt = '{0:' + $m.Format + '}'

    $verdict = '-'
    if ($null -ne $m.Budget) {
        $over = if ($m.Relative) { $percent -gt $m.Budget } else { $delta -gt $m.Budget }
        $verdict = if ($over) { 'OVER' } else { 'ok' }
        if ($over) { $failures += ("{0}: {1:N3} over a budget of {2}" -f $m.Name, $delta, $m.Budget) }
    }
    $budgetText = if ($null -eq $m.Budget) { '-' }
    elseif ($m.Relative) { "+$($m.Budget)%" }
    else { "+$($m.Budget)" }

    [pscustomobject]@{
        Measure = $m.Name
        A       = ($fmt -f $a)
        B       = ($fmt -f $b)
        Delta   = ("{0}{1} ({2}{3:N1}%)" -f $(if ($delta -ge 0) { '+' } else { '' }), ($fmt -f $delta),
            $(if ($percent -ge 0) { '+' } else { '' }), $percent)
        Budget  = $budgetText
        Verdict = $verdict
    }
}

$table | Format-Table -AutoSize

if ($failures.Count -gt 0) {
    Write-Host "GATE FAILED" -ForegroundColor Red
    $failures | ForEach-Object { Write-Host "  $_" -ForegroundColor Red }
    exit 1
}

Write-Host "GATE PASSED" -ForegroundColor Green
Write-Host "  stamps: $Out"
exit 0
