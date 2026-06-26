//! Hand-rolled wgpu timestamp-query → Tracy GPU profiler.
//!
//! ## Why a cross-frame readback ring
//!
//! The scene is encoded inside an egui paint callback, and **egui owns
//! `queue.submit` + present** — our code never submits. So we cannot do the
//! textbook "write timestamps, submit, block, read" sequence: we can only *record*
//! timestamp writes onto egui's encoder and append a resolve/copy at the end of the
//! same `prepare`. The values become readable only after egui submits (later) and
//! the GPU finishes. We therefore read each frame's timestamps a few frames later
//! through a small ring of resolve+readback buffers, polling the device
//! **non-blocking** once per frame. Tracy doesn't care that the numbers arrive
//! late — it aligns them under the right frame using the GPU's own clock.
//!
//! ## Runtime gating
//!
//! Always compiled in, never active unless `--tracy` was passed: `app` calls
//! [`enable_tracy_gpu`] (which also records the GPU backend for the Tracy context
//! type), and the device is only asked for `TIMESTAMP_QUERY` on a `--tracy` launch.
//! The profiler is created lazily in `prepare` only when all of: the flag is set,
//! the device actually has `TIMESTAMP_QUERY`, and a Tracy client is running. When
//! it stays `None`, every pass keeps `timestamp_writes: None` exactly as before.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use tracy_client::{Client, GpuContext, GpuContextType};

/// Set by `app` on a `--tracy` launch once the adapter is known. Read by the scene
/// callback (which has no channel from `app`) to decide whether to build the
/// profiler. Also stores the GPU backend so the Tracy GPU context reports the
/// right API.
static TRACY_GPU_ENABLED: AtomicBool = AtomicBool::new(false);
static TRACY_GPU_BACKEND: AtomicU8 = AtomicU8::new(GpuContextType::Invalid as u8);

/// Arm the hand-rolled GPU profiler and record the backend for the Tracy GPU
/// context. Called once from `app` on a `--tracy` launch, after the adapter is
/// chosen and the device was requested with `TIMESTAMP_QUERY`. A no-op on a normal
/// launch (never called), so the scene callback never builds the profiler.
pub fn enable_tracy_gpu(backend: wgpu::Backend) {
    let ty = match backend {
        wgpu::Backend::Vulkan => GpuContextType::Vulkan,
        wgpu::Backend::Dx12 => GpuContextType::Direct3D12,
        wgpu::Backend::Gl => GpuContextType::OpenGL,
        _ => GpuContextType::Invalid,
    };
    TRACY_GPU_BACKEND.store(ty as u8, Ordering::Relaxed);
    TRACY_GPU_ENABLED.store(true, Ordering::Relaxed);
}

/// Whether `app` armed GPU profiling this run (see [`enable_tracy_gpu`]).
pub(crate) fn tracy_gpu_enabled() -> bool {
    TRACY_GPU_ENABLED.load(Ordering::Relaxed)
}

fn backend_context_type() -> GpuContextType {
    match TRACY_GPU_BACKEND.load(Ordering::Relaxed) {
        x if x == GpuContextType::Vulkan as u8 => GpuContextType::Vulkan,
        x if x == GpuContextType::Direct3D12 as u8 => GpuContextType::Direct3D12,
        x if x == GpuContextType::OpenGL as u8 => GpuContextType::OpenGL,
        _ => GpuContextType::Invalid,
    }
}

/// One profiled scene pass. Each owns a fixed pair of timestamp-query slots (begin,
/// end) in the shared query set, in GPU encode order, so a conditional pass (GTAO)
/// keeps a stable identity in Tracy whether or not it ran a given frame.
#[derive(Clone, Copy)]
pub(crate) enum Zone {
    Scene,
    GtaoGbuffer,
    Gtao,
    GtaoBlur,
}

impl Zone {
    /// Index of this zone's *begin* timestamp in the query set; the *end* is +1.
    const fn base(self) -> u32 {
        match self {
            Zone::Scene => 0,
            Zone::GtaoGbuffer => 2,
            Zone::Gtao => 4,
            Zone::GtaoBlur => 6,
        }
    }

    /// Human-friendly Tracy zone name (invariant: all markers are readable strings).
    const fn name(self) -> &'static str {
        match self {
            Zone::Scene => "Scene Geometry",
            Zone::GtaoGbuffer => "GTAO G-Buffer",
            Zone::Gtao => "GTAO Occlusion",
            Zone::GtaoBlur => "GTAO Blur",
        }
    }

    /// Bit in the per-frame active mask.
    const fn bit(self) -> u16 {
        1 << (self.base() / 2)
    }
}

/// Encode order, used to walk timestamps when feeding Tracy.
const ZONES: [Zone; 4] = [Zone::Scene, Zone::GtaoGbuffer, Zone::Gtao, Zone::GtaoBlur];

/// Mask bit set whenever the GTAO passes ran (gbuffer + occlusion + blur encode
/// together), used to size the resolve range.
const GTAO_MASK: u16 = Zone::GtaoGbuffer.bit() | Zone::Gtao.bit() | Zone::GtaoBlur.bit();

/// The set of zones a frame encodes: always the scene pass, plus the three GTAO
/// passes when AO is active. Passed to [`GpuProfiler::begin`].
pub(crate) fn frame_mask(gtao_active: bool) -> u16 {
    if gtao_active {
        Zone::Scene.bit() | GTAO_MASK
    } else {
        Zone::Scene.bit()
    }
}

/// Number of timestamp slots in the shared query set (zones × 2). The last pair is
/// reserved for a future bloom pass so its addition won't renumber existing zones.
const SLOTS: u32 = 10;
const SLOT_BYTES: u64 = SLOTS as u64 * 8;
/// Ring depth. Generous enough that a slot written at frame N is read and free well
/// before it is reused at N+RING, regardless of map-callback latency.
const RING: usize = 4;

const MAP_IDLE: u8 = 0;
const MAP_MAPPING: u8 = 1;
const MAP_MAPPED: u8 = 2;

/// One ring entry: a GPU-side resolve buffer plus a CPU-mappable readback buffer,
/// and the bookkeeping to drain it a few frames after it was written.
struct RingSlot {
    /// `resolve_query_set` destination (`QUERY_RESOLVE | COPY_SRC`).
    resolve: wgpu::Buffer,
    /// `MAP_READ | COPY_DST` copy target the CPU reads.
    readback: wgpu::Buffer,
    /// Shared with the `map_async` callback (which runs on the poll thread and must
    /// not borrow `self`): `IDLE → MAPPING → MAPPED`.
    state: Arc<AtomicU8>,
    /// `true` once timestamps have been resolved+copied here and not yet read back.
    pending: bool,
    /// Frame number the resolve was encoded at; a slot is only mapped once a later
    /// frame has run (so egui has submitted the copy).
    resolved_at: u64,
    /// Which zones were written this slot (so the reader skips unran passes).
    active_mask: u16,
    /// Contiguous timestamp count actually resolved (2 for scene-only, 8 with GTAO).
    used: u32,
}

/// The GPU profiler: a single shared timestamp query set + a ring of
/// resolve/readback buffers + the Tracy GPU context the readbacks feed.
pub(crate) struct GpuProfiler {
    query_set: wgpu::QuerySet,
    ring: Vec<RingSlot>,
    /// Nanoseconds per GPU tick, from `Queue::get_timestamp_period`.
    period: f32,
    /// Created lazily on the first successful readback so its calibration baseline
    /// is a real GPU tick.
    ctx: Option<GpuContext>,
    frame_no: u64,
    /// Ring slot encoded this frame.
    cur_slot: usize,
    /// Mask of zones armed this frame.
    cur_mask: u16,
    /// Whether this frame's slot was free, so it is safe to write timestamps. When
    /// `false`, `writes` returns `None` and the frame is simply not profiled.
    armed: bool,
}

impl GpuProfiler {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("review_gpu_profiler_timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: SLOTS,
        });
        let ring = (0..RING)
            .map(|_| RingSlot {
                resolve: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("review_gpu_profiler_resolve"),
                    size: SLOT_BYTES,
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                }),
                readback: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("review_gpu_profiler_readback"),
                    size: SLOT_BYTES,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                state: Arc::new(AtomicU8::new(MAP_IDLE)),
                pending: false,
                resolved_at: 0,
                active_mask: 0,
                used: 0,
            })
            .collect::<Vec<_>>();

        Self {
            query_set,
            ring,
            period: queue.get_timestamp_period(),
            ctx: None,
            frame_no: 0,
            cur_slot: 0,
            cur_mask: 0,
            armed: false,
        }
    }

    /// Start a frame: advance the frame counter, read back any slots whose data is
    /// ready (and request maps for those whose copy has been submitted), then arm
    /// this frame's slot if it is free. `mask` is the set of zones about to be
    /// encoded. Must be called once per `prepare`, before encoding.
    pub(crate) fn begin(&mut self, device: &wgpu::Device, mask: u16) {
        self.frame_no += 1;
        self.pump(device);

        let slot = (self.frame_no % RING as u64) as usize;
        let free =
            !self.ring[slot].pending && self.ring[slot].state.load(Ordering::Acquire) == MAP_IDLE;
        self.cur_slot = slot;
        self.armed = free;
        self.cur_mask = if free { mask } else { 0 };
    }

    /// Timestamp writes for `zone`, or `None` when this frame isn't being profiled
    /// (the inactive path keeps `timestamp_writes: None`, unchanged behavior).
    pub(crate) fn writes(&self, zone: Zone) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        if !self.armed || self.cur_mask & zone.bit() == 0 {
            return None;
        }
        let base = zone.base();
        Some(wgpu::RenderPassTimestampWrites {
            query_set: &self.query_set,
            beginning_of_pass_write_index: Some(base),
            end_of_pass_write_index: Some(base + 1),
        })
    }

    /// Append the resolve + copy for this frame's slot onto egui's encoder (egui
    /// submits it later in the same submission as the timestamp writes). Must be
    /// called once per `prepare`, after encoding.
    pub(crate) fn finish(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if !self.armed {
            return;
        }
        let used = if self.cur_mask & GTAO_MASK != 0 { 8 } else { 2 };
        let slot = self.cur_slot;
        encoder.resolve_query_set(&self.query_set, 0..used, &self.ring[slot].resolve, 0);
        encoder.copy_buffer_to_buffer(
            &self.ring[slot].resolve,
            0,
            &self.ring[slot].readback,
            0,
            used as u64 * 8,
        );
        let s = &mut self.ring[slot];
        s.pending = true;
        s.resolved_at = self.frame_no;
        s.active_mask = self.cur_mask;
        s.used = used;
        s.state.store(MAP_IDLE, Ordering::Release);
    }

    /// Drain ready readbacks and request maps for submitted-but-unmapped slots, then
    /// poll the device once (non-blocking — never `Wait`).
    fn pump(&mut self, device: &wgpu::Device) {
        for slot in 0..self.ring.len() {
            if !self.ring[slot].pending {
                continue;
            }
            match self.ring[slot].state.load(Ordering::Acquire) {
                MAP_MAPPED => self.read_slot(slot),
                MAP_IDLE if self.ring[slot].resolved_at < self.frame_no => {
                    // The copy was encoded on an earlier frame's encoder, so egui has
                    // since submitted it; safe to map now.
                    let state = self.ring[slot].state.clone();
                    self.ring[slot].state.store(MAP_MAPPING, Ordering::Release);
                    self.ring[slot].readback.slice(..).map_async(
                        wgpu::MapMode::Read,
                        move |result| {
                            state.store(
                                if result.is_ok() { MAP_MAPPED } else { MAP_IDLE },
                                Ordering::Release,
                            );
                        },
                    );
                }
                _ => {}
            }
        }
        let _ = device.poll(wgpu::Maintain::Poll);
    }

    /// Read one mapped slot's timestamps, feed them to Tracy, and free the slot.
    fn read_slot(&mut self, slot: usize) {
        let used = self.ring[slot].used as usize;
        let mask = self.ring[slot].active_mask;
        // Copy out before touching `&mut self` again (the mapped view borrows the
        // buffer, hence `self.ring`).
        let mut times = [0u64; SLOTS as usize];
        {
            let view = self.ring[slot]
                .readback
                .slice(0..self.ring[slot].used as u64 * 8)
                .get_mapped_range();
            let src: &[u64] = bytemuck::cast_slice(&view);
            let n = src.len().min(times.len());
            times[..n].copy_from_slice(&src[..n]);
        }
        self.ring[slot].readback.unmap();
        self.ring[slot].state.store(MAP_IDLE, Ordering::Release);
        self.ring[slot].pending = false;

        self.emit_zones(&times, mask, used);
    }

    /// Turn a frame's resolved timestamps into Tracy GPU zones.
    fn emit_zones(&mut self, times: &[u64], mask: u16, used: usize) {
        if self.ctx.is_none() {
            // Use the first timestamp as the context's calibration baseline so the
            // GPU timeline lines up with the CPU one.
            let Some(client) = Client::running() else {
                return;
            };
            let baseline = times.first().copied().unwrap_or(0) as i64;
            self.ctx = client
                .new_gpu_context(
                    Some("GPU (wgpu scene)"),
                    backend_context_type(),
                    baseline,
                    self.period,
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
            let begin_idx = zone.base() as usize;
            let end_idx = begin_idx + 1;
            if end_idx >= used {
                continue;
            }
            let (begin, end) = (times[begin_idx], times[end_idx]);
            // Skip unwritten/garbage pairs (a pass that early-returned, or a counter
            // that hasn't advanced).
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
