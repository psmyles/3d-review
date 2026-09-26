#!/usr/bin/env bash
# The macOS measurement harness: the twin of `scripts/gate.ps1` (docs/ARCHITECTURE.md,
# Platform decisions D2).
#
# One difference of substance, and it is a decision rather than an omission: **there
# is no cross-OS budget.** D2's three budgets exist to gate the *Windows* migration
# against `main` on one box; comparing a Mac against a Windows PC would compare two
# machines, not two builds. So the number to beat here is the next Mac build's, and
# this script has two modes:
#
#   scripts/gate.sh -B target/release/3d-review -f assets/test_models/SK_Player_01.fbx
#       Measure one build and print its numbers. This is the baseline you record.
#
#   scripts/gate.sh -A <older build> -B <newer build> -f <fixture>
#       Measure two, interleaved, and print the delta with D2's budgets applied.
#
# Each launch runs the viewer's own `--gate-out <file>` mode (`crates/app/src/gate.rs`):
# it loads the fixture, plays a scripted orbit, writes one JSON stamp and then sits
# idle with the model resident so this script can sample memory from outside. In the
# two-build mode the launches are **interleaved** — A, B, A, B — rather than all of A
# then all of B, so a machine that warms up or throttles part way through does it to
# both. One discarded warm-up launch of each comes first: the first launch after a
# build is an outlier (a cold binary, a cold shader cache).
#
# The measurements, and where each end of them comes from:
#
#   startup  process creation -> the first present with the model on screen. The far
#            end is the stamp's wall clock; the near end is `pbi_start_tvsec` from
#            `proc_pidinfo`, read here rather than in the viewer, which is
#            `forbid(unsafe_code)` — the same split as the Windows script's
#            `Process.StartTime`. Both are gettimeofday values, so they subtract.
#   frame    median present-to-present over 300 frames, vsync off (the viewer sets
#            `displaySyncEnabled` from the flag, and the gate window is a fixed size
#            so both builds rasterize the same pixels). CPU-only frame time is
#            reported beside it: a regression in one and not the other says different
#            things.
#   memory   physical footprint, sampled while the process idles with the model up —
#            what Activity Monitor calls "Memory", and the analogue of the Windows
#            script's private bytes. **There is no separate VRAM row**: Apple silicon
#            has unified memory, so what the Windows counter reports as GPU dedicated
#            bytes is already inside this figure. A row of `n/a` would have been a
#            worse answer than none.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
baseline=""
build=""
fixture=""
runs=5
out="${TMPDIR:-/tmp}/3d-review-gate"
startup_budget_ms=10.0
frame_budget_ms=0.3
memory_budget_percent=5.0

usage() { sed -n '2,44p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        -A|--baseline) baseline="$2"; shift 2 ;;
        -B|--build)    build="$2"; shift 2 ;;
        -f|--fixture)  fixture="$2"; shift 2 ;;
        -n|--runs)     runs="$2"; shift 2 ;;
        -o|--out)      out="$2"; shift 2 ;;
        -h|--help)     usage; exit 0 ;;
        *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

[[ -n "$build" && -n "$fixture" ]] || { echo "-B and -f are required" >&2; usage >&2; exit 2; }
for path in ${baseline:+"$baseline"} "$build" "$fixture"; do
    [[ -e "$path" ]] || { echo "not found: $path" >&2; exit 1; }
done
build="$(cd "$(dirname "$build")" && pwd)/$(basename "$build")"
fixture="$(cd "$(dirname "$fixture")" && pwd)/$(basename "$fixture")"
if [[ -n "$baseline" ]]; then
    baseline="$(cd "$(dirname "$baseline")" && pwd)/$(basename "$baseline")"
fi
mkdir -p "$out"

# The process's creation time in Unix nanoseconds, from the kernel rather than from
# the shell's clock before the fork — which would have folded this script's own
# `fork`/`exec` into every startup figure.
#
# `proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, sizeof info)` fills a
# `struct proc_bsdinfo` (sys/proc_info.h) whose last two fields are the start time as
# seconds + microseconds; 136 bytes, with `pbi_start_tvsec` at offset 120. ctypes
# rather than a compiled helper so this script needs no build step of its own.
process_start_ns() {
    python3 - "$1" <<'PY'
import ctypes, struct, sys

PROC_PIDTBSDINFO = 3
SIZE = 136
START_TVSEC_OFFSET = 120

libc = ctypes.CDLL("/usr/lib/libSystem.dylib", use_errno=True)
buf = ctypes.create_string_buffer(SIZE)
written = libc.proc_pidinfo(int(sys.argv[1]), PROC_PIDTBSDINFO, ctypes.c_uint64(0), buf, SIZE)
if written != SIZE:
    sys.exit("proc_pidinfo returned %d, expected %d" % (written, SIZE))
sec, usec = struct.unpack_from("<QQ", buf.raw, START_TVSEC_OFFSET)
print(sec * 1_000_000_000 + usec * 1_000)
PY
}

# Physical footprint in bytes, while the process is alive. `footprint` is the tool
# Activity Monitor's number comes from; `vmmap --summary` reports the same figure and
# is the fallback if `footprint` is unavailable.
# Both tools scale the unit to the size — a 400 MB process reports "403 MB" from
# `footprint` and "403.0M" from `vmmap`, not kilobytes — so the unit is parsed rather
# than assumed. Assuming KB is how this first returned `n/a` for every run.
process_footprint_bytes() {
    local pid="$1" reading=""
    reading="$(footprint -p "$pid" 2>/dev/null |
               sed -n 's/.*Footprint: *\([0-9.]*\) *\([KMG]\)B.*/\1 \2/p' | head -1)"
    if [[ -z "$reading" ]]; then
        reading="$(vmmap --summary "$pid" 2>/dev/null |
                   sed -n 's/^Physical footprint: *\([0-9.]*\)\([KMG]\).*/\1 \2/p' | head -1)"
    fi
    [[ -n "$reading" ]] || { echo ""; return; }
    python3 -c 'import sys; v,u=sys.argv[1:3]; print(int(float(v)*{"K":1024,"M":1024**2,"G":1024**3}[u]))' $reading
}

# One launch: run the viewer in gate mode, sample memory during the idle hold it
# leaves for exactly this, and write a one-line record for the report step.
gate_run() {
    local exe="$1" label="$2" index="$3"
    local stamp="$out/$label-$index.json"
    rm -f "$stamp"

    "$exe" "$fixture" --gate-out "$stamp" >"$out/$label-$index.log" 2>&1 &
    local pid=$!

    # Wait for the stamp. The generous ceiling covers a cold first launch on a busy
    # box; a hung viewer is killed rather than left behind.
    local waited=0
    while [[ ! -s "$stamp" ]]; do
        if ! kill -0 "$pid" 2>/dev/null; then
            echo "$label run $index exited before writing a stamp" >&2
            cat "$out/$label-$index.log" >&2
            exit 1
        fi
        if (( waited > 4800 )); then
            kill -9 "$pid" 2>/dev/null || true
            echo "$label run $index timed out waiting for its stamp" >&2
            exit 1
        fi
        perl -e 'select undef, undef, undef, 0.025'
        waited=$((waited + 1))
    done

    # The stamp is written before the hold begins, so the process is idle with the
    # model resident right now — which is the state being asked about.
    local started footprint
    started="$(process_start_ns "$pid" || echo "")"
    footprint="$(process_footprint_bytes "$pid")"

    wait "$pid" || true
    echo "$label $stamp ${started:-0} ${footprint:-0}" >> "$out/runs.txt"
}

echo
if [[ -n "$baseline" ]]; then
    echo "gate: two builds, interleaved"
    echo "  A (baseline):   $baseline"
    echo "  B (under test): $build"
else
    echo "gate: one build"
    echo "  B: $build"
fi
echo "  fixture: $fixture"
echo "  runs: $runs measured, 1 warm-up each"
echo

: > "$out/runs.txt"

# Warm-up: discarded. The first launch after a build pays for a cold binary and a
# cold driver shader cache, and would land entirely on whichever build went first.
echo "warming up..."
[[ -n "$baseline" ]] && gate_run "$baseline" "warmup-A" 0
gate_run "$build" "warmup-B" 0
: > "$out/runs.txt"

for ((i = 1; i <= runs; i++)); do
    echo "run $i/$runs"
    [[ -n "$baseline" ]] && gate_run "$baseline" "A" "$i"
    gate_run "$build" "B" "$i"
done

python3 - "$out/runs.txt" "$startup_budget_ms" "$frame_budget_ms" "$memory_budget_percent" <<'PY'
import json, sys, statistics

records_path, startup_budget, frame_budget, memory_budget = sys.argv[1:5]
startup_budget, frame_budget, memory_budget = map(float, (startup_budget, frame_budget, memory_budget))

rows = {"A": [], "B": []}
for line in open(records_path):
    label, stamp_path, started_ns, footprint = line.split()
    stamp = json.load(open(stamp_path))
    if stamp["frames"] < 1:
        sys.exit(f"{label} measured no frames")
    started_ns = int(started_ns)
    rows[label].append({
        # Both ends are gettimeofday values, so the subtraction is valid.
        "startup_ms": (stamp["first_present_unix_ns"] - started_ns) / 1e6 if started_ns else None,
        "frame_ms": stamp["frame_ms"]["median"],
        "frame_p95_ms": stamp["frame_ms"]["p95"],
        "cpu_ms": stamp["cpu_ms"]["median"],
        "footprint_mb": int(footprint) / (1024 * 1024) if int(footprint) else None,
        "settings": stamp["settings"],
        "model": stamp["model"],
    })

if not rows["B"]:
    sys.exit("no measured runs")

# The two builds must have measured the same thing, or the comparison is decoration.
if rows["A"]:
    a0, b0 = rows["A"][0], rows["B"][0]
    for field in ("msaa", "gtao", "clip_playing", "width", "height", "vsync"):
        if a0["settings"][field] != b0["settings"][field]:
            sys.exit(f"settings differ between builds ({field}): "
                     f"A={a0['settings'][field]} B={b0['settings'][field]}")
    if a0["model"]["triangles"] != b0["model"]["triangles"]:
        sys.exit("the two builds imported different geometry from the same fixture")

s, m = rows["B"][0]["settings"], rows["B"][0]["model"]
print(f"\nconditions: {s['width']}x{s['height']}, {s['msaa']}x MSAA, GTAO {s['gtao']}, "
      f"clip playing {s['clip_playing']}, {m['triangles']} triangles, {m['clips']} clips\n")

def median_of(label, key):
    values = [r[key] for r in rows[label] if r[key] is not None]
    return statistics.median(values) if values else None

MEASURES = [
    ("startup (ms)",   "startup_ms",   startup_budget, False, 1),
    ("frame (ms)",     "frame_ms",     frame_budget,   False, 3),
    ("frame p95 (ms)", "frame_p95_ms", None,           False, 3),
    ("cpu/frame (ms)", "cpu_ms",       None,           False, 3),
    ("footprint (MB)", "footprint_mb", memory_budget,  True,  1),
]

failures = []
if rows["A"]:
    print(f"{'measure':<16}{'A':>12}{'B':>12}{'delta':>22}{'budget':>10}  verdict")
    for name, key, budget, relative, digits in MEASURES:
        a, b = median_of("A", key), median_of("B", key)
        if a is None or b is None:
            print(f"{name:<16}{'n/a':>12}{'n/a':>12}{'n/a':>22}{'n/a':>10}  skipped")
            continue
        delta = b - a
        percent = 100.0 * delta / a if a else 0.0
        verdict, budget_text = "-", "-"
        if budget is not None:
            over = percent > budget if relative else delta > budget
            verdict = "OVER" if over else "ok"
            budget_text = f"+{budget}%" if relative else f"+{budget}"
            if over:
                failures.append(f"{name}: {delta:+.3f} over a budget of {budget_text}")
        print(f"{name:<16}{a:>12.{digits}f}{b:>12.{digits}f}"
              f"{f'{delta:+.{digits}f} ({percent:+.1f}%)':>22}{budget_text:>10}  {verdict}")
else:
    print(f"{'measure':<16}{'median':>12}{'min':>12}{'max':>12}")
    for name, key, _budget, _relative, digits in MEASURES:
        values = [r[key] for r in rows["B"] if r[key] is not None]
        if not values:
            print(f"{name:<16}{'n/a':>12}{'n/a':>12}{'n/a':>12}")
            continue
        print(f"{name:<16}{statistics.median(values):>12.{digits}f}"
              f"{min(values):>12.{digits}f}{max(values):>12.{digits}f}")

if failures:
    print("\nGATE FAILED")
    for failure in failures:
        print(f"  {failure}")
    sys.exit(1)
print(f"\n{'GATE PASSED' if rows['A'] else 'recorded'}")
PY

echo "  stamps: $out"
