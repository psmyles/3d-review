//! Hand-rolled Direct3D 11 timestamp-query → Tracy GPU profiler.
//!
//! ## How it times the GPU on D3D11
//!
//! D3D11 has no "begin/end pass with timestamp writes" like wgpu; instead each
//! `ID3D11Query` of type `TIMESTAMP` records the GPU clock at the point `End` is
//! called in the command stream (`Begin` is a no-op for timestamps). A sibling
//! `TIMESTAMP_DISJOINT` query, `Begin`/`End`-bracketed around the whole frame,
//! yields the tick `Frequency` (ticks/sec) and a `Disjoint` flag that invalidates a
//! frame whose clock skipped. So each profiled zone records two timestamps (begin,
//! end) and the elapsed time is `(end - begin) / Frequency`.
//!
//! ## Cross-frame readback ring
//!
//! Query results aren't ready the moment the frame is encoded — the GPU has to
//! finish first. We therefore keep a small ring of per-frame query sets and read a
//! slot back only when we're about to reuse it (`RING` frames later), by which point
//! it is long done. `GetData` through the `windows` wrapper can't distinguish "ready"
//! (`S_OK`) from "not ready" (`S_FALSE`) — both map to `Ok(())` since `S_FALSE` is a
//! success HRESULT — so we **pre-zero** the output structs and treat a zero
//! `Frequency` (which `GetData` leaves untouched when not ready) as "skip this
//! frame". Tracy doesn't care that the numbers arrive a few frames late; it aligns
//! them under the right frame using the GPU's own clock.
//!
//! ## Runtime gating
//!
//! Always compiled in, never active unless `--tracy` was passed: `app` calls
//! [`enable_tracy_gpu`], and the profiler is built lazily by the scene renderer only
//! when the flag is set *and* a Tracy client is running. When it stays absent every
//! scene pass runs exactly as before (no `Begin`/`End` calls at all).

use std::sync::atomic::{AtomicBool, Ordering};

use tracy_client::{Client, GpuContext, GpuContextType};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_QUERY, D3D11_QUERY_DATA_TIMESTAMP_DISJOINT, D3D11_QUERY_DESC, D3D11_QUERY_TIMESTAMP,
    D3D11_QUERY_TIMESTAMP_DISJOINT, ID3D11Device, ID3D11DeviceContext, ID3D11Query,
};
use windows::core::Result;

use super::out_param;

/// Set by `app` on a `--tracy` launch. Read by the scene renderer (which has no
/// channel from `app`) to decide whether to build the profiler. The D3D11 backend is
/// fixed, so unlike the old wgpu profiler this carries no backend value.
static TRACY_GPU_ENABLED: AtomicBool = AtomicBool::new(false);

/// Arm the hand-rolled GPU profiler. Called once from `app` on a `--tracy` launch. A
/// no-op on a normal launch (never called), so the scene renderer never builds the
/// profiler.
pub fn enable_tracy_gpu() {
    TRACY_GPU_ENABLED.store(true, Ordering::Relaxed);
}

/// Whether `app` armed GPU profiling this run (see [`enable_tracy_gpu`]).
fn tracy_gpu_enabled() -> bool {
    TRACY_GPU_ENABLED.load(Ordering::Relaxed)
}

/// Whether the scene renderer should build the profiler this frame: profiling was
/// armed *and* a Tracy client is actually running (so a `--tracy` launch with no
/// connected server still pays nothing for query objects it would never read).
pub(crate) fn should_enable() -> bool {
    tracy_gpu_enabled() && Client::running().is_some()
}

/// Send a plain message to the running Tracy client (a no-op without one) — the
/// only diagnostics channel a `--tracy` session watches.
pub(crate) fn note(text: &str) {
    if let Some(client) = Client::running() {
        client.message(text, 0);
    }
}

/// One profiled scene pass. Each owns a fixed pair of timestamp slots (begin, end)
/// in the per-frame query array, in GPU encode order, so a conditional pass (GTAO)
/// keeps a stable identity in Tracy whether or not it ran a given frame.
#[derive(Clone, Copy)]
pub(crate) enum Zone {
    Scene,
    GtaoGbuffer,
    Gtao,
    GtaoBlur,
    Composite,
}

impl Zone {
    /// Index of this zone's *begin* timestamp; the *end* is +1.
    const fn base(self) -> usize {
        match self {
            Zone::Scene => 0,
            Zone::GtaoGbuffer => 2,
            Zone::Gtao => 4,
            Zone::GtaoBlur => 6,
            Zone::Composite => 8,
        }
    }

    /// Human-friendly Tracy zone name (invariant: all markers are readable strings).
    const fn name(self) -> &'static str {
        match self {
            Zone::Scene => "Scene Geometry",
            Zone::GtaoGbuffer => "GTAO G-Buffer",
            Zone::Gtao => "GTAO Occlusion",
            Zone::GtaoBlur => "GTAO Blur",
            Zone::Composite => "Composite",
        }
    }

    /// Bit in the per-frame active mask.
    const fn bit(self) -> u16 {
        1 << (self.base() / 2)
    }
}

/// Encode order, used to walk timestamps when feeding Tracy.
const ZONES: [Zone; 5] = [
    Zone::Scene,
    Zone::GtaoGbuffer,
    Zone::Gtao,
    Zone::GtaoBlur,
    Zone::Composite,
];

/// Mask bit set whenever the GTAO passes ran (gbuffer + occlusion + blur encode
/// together).
const GTAO_MASK: u16 = Zone::GtaoGbuffer.bit() | Zone::Gtao.bit() | Zone::GtaoBlur.bit();

/// The set of zones a frame encodes: always the scene + composite, plus the three
/// GTAO passes when AO is active. Passed to [`GpuProfiler::begin_frame`].
pub(crate) fn frame_mask(gtao_active: bool) -> u16 {
    let base = Zone::Scene.bit() | Zone::Composite.bit();
    if gtao_active { base | GTAO_MASK } else { base }
}

/// Number of timestamp slots per frame (zones × 2).
const SLOTS: usize = 10;
/// Ring depth. Generous enough that a slot written at frame N is read and free well
/// before it is reused at N+RING, so its query results are always ready by then.
const RING: usize = 4;

/// One ring entry: a disjoint query (frequency + validity for the frame) plus the
/// per-zone begin/end timestamp queries, and the bookkeeping to drain it a few
/// frames after it was written.
struct RingSlot {
    disjoint: ID3D11Query,
    /// `SLOTS` timestamp queries (zone begin/end pairs).
    timestamps: Vec<ID3D11Query>,
    /// `true` once this slot's queries have been `End`-ed for a frame and not yet
    /// read back.
    pending: bool,
    /// Which zones were armed this slot, so the reader skips unran passes (whose
    /// timestamp queries hold stale data from an earlier frame).
    mask: u16,
}

/// The GPU profiler: a ring of per-frame query sets + the Tracy GPU context the
/// readbacks feed.
pub(crate) struct GpuProfiler {
    ring: Vec<RingSlot>,
    /// Created lazily on the first successful readback so its calibration baseline +
    /// tick period come from a real disjoint query.
    ctx: Option<GpuContext>,
    frame_no: u64,
    /// Ring slot encoded this frame.
    cur_slot: usize,
}

impl GpuProfiler {
    /// Build the query ring. Timestamp + disjoint queries are core to D3D11 feature
    /// level 11_0+, but `CreateQuery` can still fail on an exotic driver, so this is
    /// fallible and the caller treats `Err` as "no profiling this run".
    pub(crate) fn new(device: &ID3D11Device) -> Result<Self> {
        let mut ring = Vec::with_capacity(RING);
        for _ in 0..RING {
            let disjoint = create_query(device, D3D11_QUERY_TIMESTAMP_DISJOINT)?;
            let mut timestamps = Vec::with_capacity(SLOTS);
            for _ in 0..SLOTS {
                timestamps.push(create_query(device, D3D11_QUERY_TIMESTAMP)?);
            }
            ring.push(RingSlot {
                disjoint,
                timestamps,
                pending: false,
                mask: 0,
            });
        }
        Ok(Self {
            ring,
            ctx: None,
            frame_no: 0,
            cur_slot: 0,
        })
    }

    /// Start a frame: pick this frame's ring slot, drain it if it still holds an
    /// unread result (it was written `RING` frames ago, so it is done), then begin
    /// the disjoint query for the new frame. `mask` is the set of zones about to be
    /// encoded. Call once per `render`, before encoding.
    pub(crate) fn begin_frame(&mut self, ctx: &ID3D11DeviceContext, mask: u16) {
        self.frame_no += 1;
        let slot = (self.frame_no % RING as u64) as usize;
        self.cur_slot = slot;
        if self.ring[slot].pending {
            self.read_slot(ctx, slot);
        }
        self.ring[slot].mask = mask;
        // SAFETY: `disjoint` is a live query owned by the ring; the immediate context
        // records into it. `Begin` is valid for a disjoint query.
        unsafe { ctx.Begin(&self.ring[slot].disjoint) };
    }

    /// Record this zone's *begin* timestamp at the current point in the command
    /// stream. Call right before the zone's draws.
    pub(crate) fn zone_begin(&self, ctx: &ID3D11DeviceContext, zone: Zone) {
        self.end_timestamp(ctx, zone.base());
    }

    /// Record this zone's *end* timestamp. Call right after the zone's draws.
    pub(crate) fn zone_end(&self, ctx: &ID3D11DeviceContext, zone: Zone) {
        self.end_timestamp(ctx, zone.base() + 1);
    }

    /// `End` the timestamp query at `index` in this frame's slot (records the GPU
    /// clock there).
    fn end_timestamp(&self, ctx: &ID3D11DeviceContext, index: usize) {
        // SAFETY: the timestamp query is live; `End` records the timestamp. The slot
        // is the one `begin_frame` selected this frame.
        unsafe { ctx.End(&self.ring[self.cur_slot].timestamps[index]) };
    }

    /// End the disjoint query, closing the frame's timing window, and mark the slot
    /// pending so a later `begin_frame` reads it back. Call once per `render`, after
    /// encoding.
    pub(crate) fn end_frame(&mut self, ctx: &ID3D11DeviceContext) {
        let slot = self.cur_slot;
        // SAFETY: the disjoint query is live and was `Begin`-ed this frame.
        unsafe { ctx.End(&self.ring[slot].disjoint) };
        self.ring[slot].pending = true;
    }

    /// Read one slot's disjoint + timestamps (which are ready by now), feed the zones
    /// to Tracy, and free the slot. A not-ready or disjoint frame is skipped.
    fn read_slot(&mut self, ctx: &ID3D11DeviceContext, slot: usize) {
        self.ring[slot].pending = false;
        let mask = self.ring[slot].mask;

        // Pre-zeroed so a not-ready `GetData` (which leaves the output untouched and
        // still returns `Ok` because `S_FALSE` is a success HRESULT) reads as
        // `Frequency == 0` and is skipped below.
        let mut disjoint = D3D11_QUERY_DATA_TIMESTAMP_DISJOINT::default();
        // SAFETY: the disjoint query is live; the output struct is sized exactly and
        // outlives the call. `GetData` may return `S_FALSE` (mapped to `Ok`), which
        // leaves `disjoint` zeroed — handled by the `Frequency == 0` check.
        let _ = unsafe {
            ctx.GetData(
                &self.ring[slot].disjoint,
                Some((&mut disjoint as *mut D3D11_QUERY_DATA_TIMESTAMP_DISJOINT).cast()),
                size_of::<D3D11_QUERY_DATA_TIMESTAMP_DISJOINT>() as u32,
                0,
            )
        };
        if disjoint.Frequency == 0 || disjoint.Disjoint.as_bool() {
            return;
        }

        let mut times = [0u64; SLOTS];
        for (index, value) in times.iter_mut().enumerate() {
            // SAFETY: each timestamp query is live; `value` is a `u64` output slot.
            let _ = unsafe {
                ctx.GetData(
                    &self.ring[slot].timestamps[index],
                    Some((value as *mut u64).cast()),
                    size_of::<u64>() as u32,
                    0,
                )
            };
        }

        self.emit_zones(&times, mask, disjoint.Frequency);
    }

    /// Turn a frame's resolved timestamps into Tracy GPU zones.
    fn emit_zones(&mut self, times: &[u64; SLOTS], mask: u16, frequency: u64) {
        if self.ctx.is_none() {
            let Some(client) = Client::running() else {
                return;
            };
            // ns per tick = 1e9 / frequency; baseline = first recorded tick so the GPU
            // timeline lines up with the CPU one.
            let period = 1.0e9 / frequency as f32;
            let baseline = times.first().copied().unwrap_or(0) as i64;
            self.ctx = client
                .new_gpu_context(
                    Some("GPU (D3D11 scene)"),
                    GpuContextType::Direct3D11,
                    baseline,
                    period,
                )
                .ok();
        }
        let Some(ctx) = self.ctx.as_ref() else {
            return;
        };

        for zone in ZONES {
            if mask & zone.bit() == 0 {
                continue;
            }
            let (begin, end) = (times[zone.base()], times[zone.base() + 1]);
            // Skip unwritten/garbage pairs (a pass that didn't run, or a counter that
            // hasn't advanced).
            if begin == 0 || end <= begin {
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

/// Create an `ID3D11Query` of `query_type`.
fn create_query(device: &ID3D11Device, query_type: D3D11_QUERY) -> Result<ID3D11Query> {
    let desc = D3D11_QUERY_DESC {
        Query: query_type,
        MiscFlags: 0,
    };
    let mut query = None;
    // SAFETY: `desc` is a well-formed query description; the out-param is populated.
    unsafe { device.CreateQuery(&desc, Some(&mut query))? };
    Ok(out_param(query))
}
