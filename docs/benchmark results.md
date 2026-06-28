# Stack benchmark results

**Question:** how much of startup time and resource use is attributable to the
windowing/GPU/UI **stack** (winit + wgpu + egui) versus a pure Win32 + Direct3D 11
native path?

**TL;DR:** the pure DX11 path reaches its first rendered frame **~2.2× faster**
(136 ms vs 295 ms), uses **~4.7× less resident RAM** (40 MB vs 190 MB), **~19×
less dedicated VRAM** (18 MB vs 342 MB), ships a **~43× smaller binary** (0.15 MB
vs 6.4 MB), and builds **~6× faster** (6.8 s vs 43 s) than the equivalent
winit+wgpu+egui app doing the *same* thing (open a window, clear, present). The
gap is dominated by wgpu's **DX12 adapter enumeration** at startup and its GPU
memory/heap reservations — real, intrinsic costs of a portable abstraction over
the native API.

## Machine

- CPU: AMD Ryzen 9 7900 (12C/24T)
- GPU: NVIDIA GeForce RTX 4080 (DX12) — **3 graphics adapters present** (4080 +
  Ryzen iGPU + a virtual monitor), which is what makes DX12 enumeration costly.
- RAM: 64 GB · Windows 11 · 1280×720 window · warm disk cache.

Absolute milliseconds are machine/driver-specific; the **ratios** are the result.

## Results (medians of 10 runs; release builds, identical profile)

| Metric | dx11min (Win32+D3D11) | wgpumin (winit+wgpu+egui) | review-app (full viewer) |
| --- | ---: | ---: | ---: |
| **First frame** (main→first present, ms) | **135.6** | **295.0** | **344.3** |
|   – min / max | 132.9 / 138.4 | 288.6 / 298.6 | 339.5 / 349.7 |
| Window visible (create→window, ms) | 125.0 | 13.5 | 28.2 |
| RAM — working set (resident, MB) | **40.2** | **189.8** | 236.8 |
| RAM — private commit (MB) | 51.3 | 491.4 | 1037.6 |
| VRAM — dedicated (MB) | **18.3** | **341.6** | 867.6 |
| VRAM — shared (MB) | 0.9 | 100.8 | 102.8 |
| Binary size (stripped, MB) | **0.15** | **6.42** | 14.96 |
| Clean release build (s) | **6.8** | 42.6 | 43.7 |
| Dependencies (normal crates) | 13 | 149 | 172 |

### Ratios vs the pure DX11 baseline

| | wgpumin | review-app |
| --- | ---: | ---: |
| First frame | 2.18× slower | 2.54× slower |
| Resident RAM | 4.7× | 5.9× |
| Dedicated VRAM | 18.7× | 47× |
| Binary size | 43× | 100× |
| Build time | 6.3× | 6.5× |
| Dependencies | 11.5× | 13.2× |

## Reading it

**Startup — two different stories, both true.**
- *Time to a window on screen*: winit shows a window in **~13 ms**, far faster
  than DX11's 125 ms — but it's a **blank** window; the real content lands
  ~280 ms later. DX11 shows its window and first rendered frame **together** at
  ~136 ms.
- *Time to rendered content* (the honest startup metric): **DX11 wins, 136 ms vs
  295 ms.** The 159 ms the wgpu stack spends is almost entirely wgpu's first
  `set_window`: enumerating adapters (DX12 creates an `ID3D12Device` per adapter
  to probe features — 3 adapters here) + `request_device`. review-app's own code
  comments measure this at ~197 ms enumerate + ~47 ms request_device on this box.
  DX11 asks for `D3D_DRIVER_TYPE_HARDWARE` directly and enumerates nothing.
- So the perceived snappiness of the sister project's DX11 rewrite is real: a
  native viewer is interactive-with-content in ~half the time.

**Pure stack overhead vs the viewer's own payload.** Splitting the deltas:
- `wgpumin − dx11min` = the abstraction layers alone: **+159 ms** first frame,
  **+150 MB** resident RAM, **+323 MB** VRAM, **+6.3 MB** binary, **+136 deps**.
- `review-app − wgpumin` = the actual viewer's work (pipelines, IBL cubes, MRT
  HDR render targets at window res × MSAA, skybox): **+49 ms** first frame,
  **+47 MB** RAM, **+526 MB** VRAM, **+8.5 MB** binary. Most of review-app's VRAM
  is rendering targets, not the stack.

**Build cost.** The full viewer builds in basically the same time as the empty
wgpu window (43.7 s vs 42.6 s): the wgpu/naga/winit/egui graph dominates compile
time and is shared, so the entire rest of the app is nearly free to compile on top
of it. The DX11 app builds in 6.8 s because it pulls none of that graph (13 crates
vs 149).

## Caveats / fairness

- **VRAM is the noisiest metric** — driver heap reservations vary run to run
  (wgpu's DX12 allocator reserves memory in large blocks). Treat the MB as
  approximate; the *ordering* (18 ≪ 342 < 868) is robust.
- **Private commit ≫ working set** for the GPU apps because wgpu/D3D12 reserve
  large virtual heaps that aren't all resident. Working set (resident) is the
  truer "RAM in use" number; both are listed.
- **dx11min uses the `windows` crate**, not `windows-sys` (windows-sys ships no
  D3D11 COM vtables). After strip+LTO the runtime footprint matches a hand-written
  C++ DX11 app; only its *build* is a touch heavier than raw windows-sys/C++ would
  be (still 6× faster than the wgpu stack).
- **Apples-to-apples scope:** dx11min and wgpumin both just clear+present an empty
  1280×720 window. A *real* DX11 image viewer would add texture upload, a textured
  quad, and hand-rolled UI — clawing back some RAM/VRAM/startup — but far less than
  the wgpu/egui machinery it replaces (an RGBA8 image is a small upload; there is
  no adapter enumeration, no shader/pipeline graph, no HDR MRT chain).
- Measured warm-cache. Cold first-launch (driver DLL load) inflates all three but
  hits the wgpu stack hardest (more/bigger DLLs).

## Follow-up: how far does *tuning wgpu* (not leaving it) get us?

Before committing to a rewrite, we measured the two wgpu-side levers against the
**same shipped binary**, env-gated via `$STACKBENCH_TUNE` (see
`measure-tuning.ps1`, `results-tuning.json`): the GPU **memory hint**
(`MemoryHints::default` → `MemoryUsage`) and the default **MSAA** level (4× → 2×
/ off). Same box/method as above; 10 runs each.

| mode | First frame (ms) | RAM WS (MB) | Private (MB) | Dedicated VRAM (MB) | Shared VRAM (MB) |
| --- | ---: | ---: | ---: | ---: | ---: |
| baseline (4×, default hints) | 347.7 | 236.5 | 1037.6 | **867.6** | 102.8 |
| `memhints` (MemoryUsage)     | 348.8 | 197.4 |  735.0 | **608.5** |  42.8 |
| `msaa2` (2×)                 | 353.3 | 236.4 | 1037.0 | **867.6** | 102.8 |
| `msaaoff` (no AA)            | 350.9 | 237.1 |  780.0 | **611.6** | 102.8 |
| `all` (memhints + 2×)        | 352.6 | 197.2 |  610.9 | **488.5** |  42.8 |

- **`memory_hints` is the real VRAM lever — and a near free lunch.** `MemoryUsage`
  alone cut dedicated VRAM **868 → 608 MB (−30%)** and resident RAM **236 → 197 MB**,
  with first frame flat and no quality cost. Confirms the "VRAM is allocator
  reservation, not texels" caveat above: the reservation *is* the mass, and the
  hint shrinks it.
- **MSAA is NOT a VRAM lever under the default allocator.** 4×→2× moved dedicated
  VRAM by **0 MB** — the ~37 MB of texel savings fell inside slack the large-block
  allocator had already reserved. (Single-sample crosses a block threshold and
  drops −256, but that's a quality sacrifice, not a free win.)
- **The two levers are synergistic, not additive.** `msaa2` alone = 0, but
  `memhints + msaa2` beats `memhints` alone by **120 MB** (608 → 488): once
  `MemoryUsage` tightens the blocks, the MSAA savings finally materialize (and
  amplify via released slack). MSAA trimming is only worth doing *with* the hint.
- **Tuning does not touch startup.** First frame is flat across all five modes
  (347–353 ms, within noise) — adapter enumeration is intrinsic, as the startup
  comment in `crates/app/src/main.rs` already found. Binary/build are likewise
  unchanged (the wgpu/naga graph stays).

**Bottom line:** the best measured wgpu-tuned config (`all`) is **488 MB VRAM /
197 MB RAM / 353 ms first frame** — VRAM now *undercuts* the projected DX11 viewer
(~550–620 MB), so the most lopsided axis in this benchmark (47× empty) is no
longer a reason to leave wgpu. What tuning can't recover — startup (~2×), resident
RAM (~2×), binary (~5×), build (~3×) — is the entire remaining case for a native
DX11 path. Separately, `MemoryHints::MemoryUsage` is a one-line, zero-quality-cost
−259 MB VRAM / −39 MB RAM win that is worth shipping as the default regardless of
that decision (it favours smaller allocation blocks, so validate once under a heavy
model load first).

## Conclusion

For a Windows-only, image-centric viewer, dropping to pure Win32 + D3D11 buys a
genuinely faster, lighter app: ~2× quicker to usable content, ~5× less RAM, an
order of magnitude less VRAM, a ~40× smaller binary, and a ~6× faster build. The
price is portability and developer ergonomics — manual COM, no winit/egui
niceties, hand-rolled UI — which, given Windows is the stated primary target, is
the trade the sister project already found worthwhile. The data backs the
instinct.
