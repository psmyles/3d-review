# Stack benchmark: pure DX11+Win32 vs winit+wgpu+egui

A controlled A/B/C to quantify how much **startup time, RAM, VRAM, binary size,
and build cost** are attributable to the windowing/GPU/UI stack itself — the
question raised after a sister project got faster startup + lower resource use by
dropping to a pure DX11 + Win32 native UI.

Windows-only, by design (Windows is the primary target).

## What's measured

| crate / target | stack |
| --- | --- |
| `dx11min`  | Pure Win32 + Direct3D 11 (`windows` crate). No winit, wgpu, or egui. |
| `wgpumin`  | winit + wgpu + egui, pinned to the **exact** versions review-app locks. Empty window: clear + one egui label. |
| `3d-review` | The real viewer (full renderer, IBL bakes, pipelines) — the shipped reality. |

`dx11min` and `wgpumin` are deliberately equivalent: a 1280×720 window that
clears to the same colour and vsync-presents continuously, idle until closed. The
only difference is the stack, so `wgpumin − dx11min` is the pure abstraction-layer
cost, and `3d-review − wgpumin` is the cost of the actual viewer's payload.

`windows` crate (not `windows-sys`) is used for `dx11min` because windows-sys
ships no D3D11 COM vtables. After `strip` + thin-LTO the unused bindings are
dead-code-eliminated, so the runtime footprint matches a hand-written C++ DX11
app; only the build is slightly heavier than raw windows-sys would be.

## How it's measured

`measure.ps1` measures everything **externally from the OS** (no in-process
instrumentation, so the method is identical across all three binaries):

- **Startup** — kernel process-create time (`Process.StartTime`) → first visible
  top-level window (`MainWindowHandle != 0`). 1 warmup run discarded, then N warm
  runs; median / min / max reported.
- **RAM** — `WorkingSet64` + `PrivateMemorySize64` after a settle period.
- **VRAM** — `GPU Process Memory` perf counters (Dedicated + Shared), summed over
  the PID's instances.
- **Binary size** — stripped release `.exe` size on disk.
- **Build cost** — clean release build wall-time + unique dependency count.

Caveat: review-app shows a black-filled window slightly *before* its real content
paints (an intentional anti-flash; see `startup_paint.rs`), so its window-visible
number is a floor, not its time-to-rendered-content.

## Run

```
cargo build --release -p dx11min -p wgpumin --manifest-path experiments/stack-bench/Cargo.toml
cargo build --release -p review-app            # if not already built
pwsh -File experiments/stack-bench/measure.ps1 -Runs 8
```

Results land in `results.json`; see `RESULTS.md` for the captured run + analysis.
