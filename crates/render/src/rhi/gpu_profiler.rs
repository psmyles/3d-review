//! The `--tracy` GPU profiler: what the renderer times, and how those timings reach
//! Tracy (`mac-port-plan.md` D18).
//!
//! Everything here is platform-independent — which zones exist, which of them a
//! given frame encodes, and how a resolved tick pair becomes a Tracy GPU span. The
//! measuring itself is a backend leaf ([`backend::GpuTimer`]): a ring of D3D11
//! timestamp queries on Windows, and on macOS a pair of sentinel command buffers
//! bracketing sokol's, which can only report the frame as a whole. Both hand back a
//! [`backend::FrameTimings`], so this file is the same on either.
//!
//! Nothing here is built unless `--tracy` was passed **and** a Tracy server is
//! actually connected: a normal launch creates no queries and issues no extra GPU
//! calls at all.

use std::sync::atomic::{AtomicBool, Ordering};

use sokol::gfx as sg;
use tracy_client::{Client, GpuContext, GpuContextType};

use super::backend;
use super::error::GpuResult;

/// Set by `app` on a `--tracy` launch. Read by the renderer, which has no channel
/// from `app`, to decide whether to build profiling state.
static TRACY_GPU_ENABLED: AtomicBool = AtomicBool::new(false);

/// Arm GPU profiling. Called once from `app` on a `--tracy` launch; never called on
/// a normal one, so nothing profiling-related is ever built.
pub fn enable_tracy_gpu() {
    TRACY_GPU_ENABLED.store(true, Ordering::Relaxed);
}

/// Give up on GPU profiling for the rest of the run — the device would not give us
/// the queries. Called once, so the attempt is not repeated every frame.
pub(crate) fn disable_tracy_gpu() {
    TRACY_GPU_ENABLED.store(false, Ordering::Relaxed);
}

/// Whether to build profiling state: it was armed *and* a Tracy client is actually
/// running, so a `--tracy` launch with no connected server still pays nothing.
pub(crate) fn should_enable() -> bool {
    TRACY_GPU_ENABLED.load(Ordering::Relaxed) && Client::running().is_some()
}

/// One profiled GPU pass. Each owns a fixed pair of timestamp slots (begin, end) in
/// the per-frame array, in encode order, so a conditional pass (GTAO) keeps a stable
/// identity in Tracy whether or not it ran in a given frame.
#[derive(Clone, Copy)]
pub(crate) enum Zone {
    Scene,
    GtaoGbuffer,
    GtaoDepthMips,
    Gtao,
    GtaoDenoise,
    Composite,
    /// The frame as a whole — **written by the backend leaf, never by a pass.**
    ///
    /// It exists because Metal cannot answer the per-pass question at all (D18):
    /// sokol owns the command buffer and the encoder, and Apple silicon has no
    /// draw-boundary counter sampling, so the honest measurement there is two
    /// sentinel command buffers bracketing sokol's. D3D11, which *can* time each
    /// pass, never writes these slots — and since which zones a frame reported is
    /// measured rather than declared, a capture on either OS simply shows what that
    /// OS could actually measure, with no `cfg` anywhere in this file.
    Frame,
}

impl Zone {
    /// Index of this zone's *begin* timestamp; the *end* is +1.
    ///
    /// `pub(crate)` for [`Zone::Frame`]'s sake: the backend leaf that writes those
    /// slots has to know where they are.
    pub(crate) const fn base(self) -> usize {
        match self {
            Zone::Scene => 0,
            Zone::GtaoGbuffer => 2,
            Zone::GtaoDepthMips => 4,
            Zone::Gtao => 6,
            Zone::GtaoDenoise => 8,
            Zone::Composite => 10,
            Zone::Frame => 12,
        }
    }

    /// Human-friendly Tracy zone name (invariant: all markers are readable strings).
    const fn name(self) -> &'static str {
        match self {
            Zone::Scene => "Scene Geometry",
            Zone::GtaoGbuffer => "GTAO G-Buffer",
            Zone::GtaoDepthMips => "GTAO Depth Mips",
            Zone::Gtao => "GTAO Occlusion",
            Zone::GtaoDenoise => "GTAO Denoise",
            Zone::Composite => "Composite",
            Zone::Frame => "GPU Frame",
        }
    }

    /// The two timestamp-slot bits this zone occupies, for testing against the
    /// frame's measured `written` set — and, in the backend leaf, for declaring
    /// which slots it just wrote.
    pub(crate) const fn slot_bits(self) -> u16 {
        (1 << self.base()) | (1 << (self.base() + 1))
    }
}

/// Encode order, used to walk a frame's timestamps when feeding Tracy.
const ZONES: [Zone; 7] = [
    Zone::Scene,
    Zone::GtaoGbuffer,
    Zone::GtaoDepthMips,
    Zone::Gtao,
    Zone::GtaoDenoise,
    Zone::Composite,
    Zone::Frame,
];

/// Number of timestamp slots a frame needs (zones × 2). The backend leaf sizes its
/// per-frame storage from this.
pub(crate) const TIMESTAMP_SLOTS: usize = ZONES.len() * 2;

// Both backends track which slots a frame wrote in a `u16` bitmask, so the zone list
// cannot outgrow 8 entries without widening that field. At 7 there is room for one
// more; this is the check that turns the eighth into a compile error rather than a
// zone that silently never reports.
const _: () = assert!(
    TIMESTAMP_SLOTS <= u16::BITS as usize,
    "a zone's slot bits must fit the backends' `written` mask"
);

/// The GPU profiler: the backend's timer plus the Tracy GPU context its readbacks
/// feed.
pub(crate) struct GpuProfiler {
    timer: backend::GpuTimer,
    /// Created on the first successful readback, so its calibration baseline and
    /// tick period come from a real measurement rather than a guess.
    ctx: Option<GpuContext>,
}

impl GpuProfiler {
    /// Build the profiler, or fail — the caller treats an error as "no GPU profiling
    /// this run" rather than as a render failure.
    pub(crate) fn new(device: &backend::Device) -> GpuResult<Self> {
        Ok(Self {
            timer: backend::GpuTimer::new(device)?,
            ctx: None,
        })
    }

    /// Open a frame, and report whatever older frame's timings came back with it.
    ///
    /// Which zones the frame will encode is **not** declared here — it is measured,
    /// by which timestamps actually get recorded before the frame closes. Declaring
    /// it was the obvious design and the wrong one: a frame that skips a pass leaves
    /// that pass's queries holding the values they were last written with, four
    /// frames ago, and a declared-but-unwritten zone reports those stale ticks as a
    /// perfectly plausible duration.
    pub(crate) fn begin_frame(&mut self) {
        if let Some(timings) = self.timer.begin_frame() {
            self.emit(&timings);
        }
    }

    /// Record `zone`'s *begin* timestamp at this point in the command stream. Call
    /// immediately before the zone's pass.
    pub(crate) fn zone_begin(&mut self, zone: Zone) {
        self.timer.timestamp(zone.base());
    }

    /// Record `zone`'s *end* timestamp. Call immediately after the zone's pass.
    pub(crate) fn zone_end(&mut self, zone: Zone) {
        self.timer.timestamp(zone.base() + 1);
    }

    /// Close the frame and plot sokol's own per-frame call counts beside the zones.
    ///
    /// The counts come from the frame that just *ended*, which is why they are read
    /// here rather than at `begin_frame`: `sg_query_stats().prev_frame` is only
    /// complete after `sg_commit`.
    pub(crate) fn end_frame(&mut self) {
        self.timer.end_frame();
        let Some(client) = Client::running() else {
            return;
        };
        let stats = sg::query_stats().prev_frame;
        client.plot(
            tracy_client::plot_name!("GPU passes"),
            stats.num_passes.into(),
        );
        client.plot(tracy_client::plot_name!("GPU draws"), stats.num_draw.into());
        client.plot(
            tracy_client::plot_name!("GPU pipeline binds"),
            stats.num_apply_pipeline.into(),
        );
        client.plot(
            tracy_client::plot_name!("GPU bindings"),
            stats.num_apply_bindings.into(),
        );
        client.plot(
            tracy_client::plot_name!("GPU uniform bytes"),
            stats.size_apply_uniforms.into(),
        );
    }

    /// Turn one frame's resolved timestamps into Tracy GPU zones.
    fn emit(&mut self, timings: &backend::FrameTimings) {
        if self.ctx.is_none() {
            let Some(client) = Client::running() else {
                return;
            };
            // ns per tick = 1e9 / frequency; the baseline is the first tick this
            // frame actually *recorded*, so the GPU timeline lines up with the CPU
            // one. "First recorded", not `times[0]`: slot 0 is the scene pass's
            // begin, which the D3D11 leaf always writes and the Metal one never
            // does — taking it unconditionally would calibrate that backend's whole
            // timeline against a zero.
            let period = 1.0e9 / timings.frequency as f32;
            let baseline = ZONES
                .iter()
                .filter(|zone| timings.written & zone.slot_bits() == zone.slot_bits())
                .map(|zone| timings.times[zone.base()])
                .min()
                .unwrap_or(0) as i64;
            self.ctx = client
                .new_gpu_context(Some(CONTEXT_NAME), CONTEXT_TYPE, baseline, period)
                .ok();
        }
        let Some(ctx) = self.ctx.as_ref() else {
            return;
        };

        for zone in ZONES {
            // Both halves must have been written *by this frame*; a pass it did not
            // run has stale ticks in its queries, not zeroes.
            if timings.written & zone.slot_bits() != zone.slot_bits() {
                continue;
            }
            let (begin, end) = (timings.times[zone.base()], timings.times[zone.base() + 1]);
            // A counter that did not advance says nothing worth plotting.
            if end <= begin {
                continue;
            }
            if let Ok(mut span) = ctx.span_alloc(zone.name(), "scene_render", file!(), line!()) {
                span.end_zone();
                span.upload_timestamp_start(begin as i64);
                span.upload_timestamp_end(end as i64);
            }
        }
    }
}

/// What Tracy calls this timeline, and which API it attributes it to. Per-OS because
/// the measurement is: D3D11 counts real per-pass timestamps, Metal will only be able
/// to bracket the whole frame (D18), and a capture should say which it is looking at.
#[cfg(windows)]
const CONTEXT_NAME: &str = "GPU (D3D11 scene)";
#[cfg(windows)]
const CONTEXT_TYPE: GpuContextType = GpuContextType::Direct3D11;
#[cfg(target_os = "macos")]
const CONTEXT_NAME: &str = "GPU (Metal frame)";
/// Tracy has no Metal context type, and picking a neighbouring API's would make a
/// capture claim something untrue about what it is looking at. `Invalid` is the
/// enum's own "stand in for other types of contexts", which is exactly the case.
///
/// Tracy's timeline labels the row **"GPU (Metal frame) — Invalid context 0"**. That
/// is the enum name leaking into the UI, not a fault: the context is live and its
/// zones are there beside it. Do not "fix" it by claiming Vulkan or D3D12.
#[cfg(target_os = "macos")]
const CONTEXT_TYPE: GpuContextType = GpuContextType::Invalid;
