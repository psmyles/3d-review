//! The Metal device and the `CAMetalLayer` sokol_gfx draws through on macOS —
//! invariant 9's sanctioned GPU site, and the only file in the workspace that names
//! Metal or Core Animation.
//!
//! It is the twin of [`super::d3d11`]: same contract, same shape, no trait between them
//! (`docs/ARCHITECTURE.md`, Platform decisions: platform leaves). What sokol_gfx cannot
//! do for itself is create a device, point `sg_environment` at it, put a drawable
//! surface on the window, hand over a render target per frame, and present — and that
//! is all this module is.
//!
//! Four things differ from the D3D11 side, and each is load-bearing:
//!
//! * **The backbuffer is `BGRA8Unorm`, not `RGBA8Unorm`.** A `CAMetalLayer` accepts
//!   only a short list of formats and RGBA8 is not on it. That is a channel *order*
//!   difference in storage only: the shaders still write `float4` RGBA and Metal
//!   swizzles on the way out, so D20 holds unchanged — a plain UNORM target the
//!   composite / egui / Tex shaders sRGB-encode themselves.
//! * **sokol presents the drawable, not us.** `sg_end_pass` calls `presentDrawable:`
//!   on the frame's command buffer and `sg_commit` commits it, so [`Swapchain::present`]
//!   only releases our own hold on the drawable. Presenting again here would be a
//!   double present.
//! * **The frame blocks at acquire, not at present.** `nextDrawable` waits until the
//!   display has taken an earlier frame; D3D11 waits inside `Present(1, 0)` instead.
//! * **Vsync is a property of the layer, not an argument to present.** `vsync` is
//!   therefore applied to `displaySyncEnabled` and takes effect from the next frame;
//!   the shipped viewer always presents with vsync on and never changes it.
//!
//! The device is created on the bring-up thread ([`crate::rhi::Gpu::start`]) and used
//! from the main thread after the join. The *layer* is not: Core Animation and
//! `NSView` are main-thread-only, so [`Swapchain::new`] must be called there, as its
//! caller ([`crate::rhi::GpuBringUp::attach`], from `resumed`) is.

#![allow(
    unsafe_code,
    reason = "invariant 9: the macOS device and swapchain leaf"
)]

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::NSView;
use objc2_foundation::CGSize;
#[cfg(feature = "bake")]
use objc2_metal::{
    MTLBlitCommandEncoder, MTLBuffer, MTLCommandEncoder, MTLOrigin, MTLResourceOptions, MTLSize,
    MTLTexture,
};
use objc2_metal::{
    MTLCommandBuffer, MTLCommandBufferStatus, MTLCommandQueue, MTLCreateSystemDefaultDevice,
    MTLDevice, MTLPixelFormat,
};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use sokol::gfx as sg;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use crate::rhi::error::{GpuError, GpuResult};
use crate::rhi::format::Format;
use crate::rhi::gpu_profiler::{TIMESTAMP_SLOTS, Zone};
use crate::rhi::present::PresentStatus;

/// What the window's backbuffer is created as, and what sokol_gfx is told to expect
/// of a swapchain pass — so the composite, egui and Tex pipelines match it. `Bgra8`,
/// not the D3D11 side's `Rgba8`: see the module header. This being the backend's
/// choice rather than a shared constant is exactly why it lives here.
pub(crate) const SWAPCHAIN_FORMAT: sg::PixelFormat = sg::PixelFormat::Bgra8;

/// How many drawables may be in flight. Two, mirroring the D3D11 side's two-buffer
/// flip chain: with three, `nextDrawable` only blocks once three frames are queued,
/// which buys latency for throughput a model viewer does not need.
const MAX_DRAWABLES: usize = 2;

/// The process's Metal device — what sokol_gfx runs on.
pub(crate) struct Device {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
}

// SAFETY: an `MTLDevice` is thread-safe by specification — Metal explicitly allows
// creating and using a device from any thread. The bring-up thread creates it and
// hands it to the main thread through a `JoinHandle`, whose join is the
// synchronization point.
unsafe impl Send for Device {}

impl Device {
    /// The system's default GPU.
    ///
    /// There is no WARP-style software fallback to try, as there is on D3D11: every
    /// Mac that runs a supported macOS has a Metal-capable GPU, and a nil device here
    /// means the process cannot render at all.
    pub(crate) fn create() -> GpuResult<Device> {
        // SAFETY: the standard Metal entry point, valid to call from any thread. It
        // returns a +1 reference (a `Create`-rule function), so `Retained::from_raw`
        // takes that ownership rather than retaining again.
        let device = unsafe { Retained::from_raw(MTLCreateSystemDefaultDevice()) };
        device
            .map(|device| Device { device })
            .ok_or_else(|| GpuError::Resource {
                kind: crate::rhi::error::ResourceKind::Device,
                label: "Metal device".into(),
                detail: "no Metal device: this GPU cannot run 3D Review".into(),
            })
    }

    /// Point `env` at this device, so `sg_setup` runs on it. sokol_gfx retains what it
    /// keeps, so the pointer is not retained by us beyond the device's own lifetime.
    pub(crate) fn fill_environment(&self, env: &mut sg::Environment) {
        env.metal = sg::MetalEnvironment {
            device: Retained::as_ptr(&self.device).cast(),
        };
    }

    /// The device's name as Metal reports it (`Apple M2`). The D3D11 twin reads
    /// the DXGI adapter description.
    pub(crate) fn adapter_name(&self) -> String {
        self.device.name().to_string()
    }

    /// The MSAA sample counts this device supports — the subset of `[1, 2, 4, 8, 16]`
    /// the scene can actually render at (invariant 4: capability-gate, never crash).
    /// `1` is always included.
    ///
    /// The formats are ignored, and that is the honest answer rather than a shortcut:
    /// `supportsTextureSampleCount:` is a device-wide question in Metal, where the
    /// D3D11 twin's `CheckMultisampleQualityLevels` is asked per format. Apple silicon
    /// answers yes to 1/2/4/8, so the 16× option simply disappears from the menu.
    pub(crate) fn supported_sample_counts(&self, color: Format, depth: Format) -> Vec<u32> {
        let _ = (color, depth);
        [1u32, 2, 4, 8, 16]
            .into_iter()
            .filter(|&count| count == 1 || self.device.supportsTextureSampleCount(count as usize))
            .collect()
    }
}

/// The window's `CAMetalLayer` and the drawable it is currently rendering into.
pub(crate) struct Swapchain {
    layer: Retained<CAMetalLayer>,
    /// This frame's drawable, held from [`Self::acquire`] to [`Self::present`]. sokol
    /// keeps only a borrowed pointer to it across the pass, so it must stay alive
    /// until the command buffer that presents it has been committed.
    drawable: Option<Retained<ProtocolObject<dyn CAMetalDrawable>>>,
    /// What `displaySyncEnabled` is currently set to, so a frame that asks for the
    /// same thing does not touch Core Animation at all.
    vsync: bool,
    width: u32,
    height: u32,
}

impl Swapchain {
    /// Attach a `width`×`height` Metal layer to `window`'s view: vsync-paced, two
    /// drawables in flight, `BGRA8Unorm` — the closest equivalent of the flip-model
    /// chain the D3D11 side makes.
    ///
    /// Must be called on the main thread: it touches `NSView` and Core Animation.
    pub(crate) fn new(
        device: &Device,
        window: &Window,
        width: u32,
        height: u32,
    ) -> GpuResult<Self> {
        let ns_view = match window.window_handle().map(|handle| handle.as_raw()) {
            Ok(RawWindowHandle::AppKit(handle)) => handle.ns_view,
            _ => {
                return Err(GpuError::InvalidArg(
                    "the window has no AppKit handle".into(),
                ));
            }
        };
        let (width, height) = (width.max(1), height.max(1));

        // SAFETY: `CAMetalLayer::new` is a plain `+[CAMetalLayer layer]`-style
        // allocation, and every setter below is a documented property on the layer it
        // was just handed.
        let layer = unsafe {
            let layer = CAMetalLayer::new();
            layer.setDevice(Some(&device.device));
            layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
            // We only ever render into the drawable and never read it back, which
            // lets Core Animation skip making it readable.
            layer.setFramebufferOnly(true);
            layer.setMaximumDrawableCount(MAX_DRAWABLES);
            // Vsync: what makes `nextDrawable` block, and so what paces the frame.
            layer.setDisplaySyncEnabled(true);
            layer.setContentsScale(window.scale_factor());
            layer.setDrawableSize(CGSize::new(f64::from(width), f64::from(height)));
            layer
        };

        // SAFETY: winit hands out a live `NSView` for the window it owns, and this
        // runs on the main thread (see the doc comment). Setting the layer *before*
        // `setWantsLayer:` makes the view layer-*hosting* — it draws only what we
        // render — rather than layer-backed, where AppKit would own and redraw the
        // layer's contents itself.
        unsafe {
            let view: &NSView = ns_view.cast().as_ref();
            view.setLayer(Some(&layer));
            view.setWantsLayer(true);
        }

        Ok(Swapchain {
            layer,
            drawable: None,
            vsync: true,
            width,
            height,
        })
    }

    /// The backbuffer size in physical pixels.
    pub(crate) fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Resize the layer's drawables.
    ///
    /// **Infallible by design** (Platform decisions: `rhi` owns the backend), exactly
    /// as on D3D11: a zero dimension (a minimized window) is remembered but not applied
    /// — Core Animation refuses it — and the frame is skipped instead.
    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        if width == 0 || height == 0 {
            return;
        }
        // SAFETY: a documented property on a live layer.
        unsafe {
            self.layer
                .setDrawableSize(CGSize::new(f64::from(width), f64::from(height)));
        }
    }

    /// Follow the window onto a display with a different backing scale.
    ///
    /// `contentsScale` is how Core Animation maps the layer's *point* bounds — which
    /// AppKit sets from the view — onto the drawable's pixels. [`Self::resize`] keeps
    /// the drawable itself right, but on its own that is not enough: left at the old
    /// display's scale, CA would believe the layer needs twice (or half) the pixels
    /// the drawable actually has and rescale it to fit, so a window dragged from a
    /// Retina display to a 1× one would go soft rather than sharp. The two have to
    /// move together, and winit reports them as two events.
    ///
    /// The D3D11 twin's is empty: DXGI has no notion of a scale between the swapchain
    /// and the window, so its buffer size is the whole story.
    pub(crate) fn set_scale_factor(&mut self, scale: f64) {
        self.layer.setContentsScale(scale);
    }

    /// Acquire this frame's drawable and point `sc` at it, reporting whether there is
    /// a frame to draw. `false` (the drawable timed out, or the window is off-screen)
    /// means skip the frame rather than draw into nothing.
    ///
    /// This is where a Metal frame waits on the display; the D3D11 twin waits in
    /// `Present` instead.
    pub(crate) fn acquire(&mut self, sc: &mut sg::Swapchain) -> bool {
        // SAFETY: a documented method on a live layer. `nextDrawable` returns nil
        // rather than blocking forever when the layer is off-screen or the wait times
        // out.
        let drawable = unsafe { self.layer.nextDrawable() };
        let Some(drawable) = drawable else {
            self.drawable = None;
            return false;
        };
        sc.metal.current_drawable = Retained::as_ptr(&drawable).cast();
        // Held until `present`: sokol_gfx only borrows the pointer across the pass.
        self.drawable = Some(drawable);
        true
    }

    /// Finish the frame.
    ///
    /// There is no present call here: `sg_end_pass` already scheduled
    /// `presentDrawable:` on the frame's command buffer and `sg_commit` committed it.
    /// All that is left is to release our own hold on the drawable, which that command
    /// buffer retains for as long as it needs.
    ///
    /// `vsync` is a layer property on Metal rather than an argument to a present call,
    /// so it is applied here and takes effect from the next frame. The viewer never
    /// changes it; the gate harness is what turns it off.
    ///
    /// There is no `DeviceLost` to report: Metal has no device-removed HRESULT, and a
    /// GPU fault kills the process rather than handing back a status.
    pub(crate) fn present(&mut self, vsync: bool) -> PresentStatus {
        self.drawable = None;
        if vsync != self.vsync {
            self.vsync = vsync;
            // SAFETY: a documented property on a live layer.
            unsafe { self.layer.setDisplaySyncEnabled(vsync) };
        }
        PresentStatus::Presented
    }
}

// ---------------------------------------------------------------------------
// The GPU-timing leaf (Platform decisions D18)
// ---------------------------------------------------------------------------

/// One frame's resolved GPU timestamps, handed back to [`crate::rhi::gpu_profiler`]
/// to become Tracy zones.
///
/// The same struct the D3D11 leaf fills from its query ring, so the Tracy side is
/// identical on both. What differs is how much of it can be filled: see [`GpuTimer`].
pub(crate) struct FrameTimings {
    /// Bit per timestamp slot that the frame actually recorded.
    pub(crate) written: u16,
    /// Ticks per second.
    pub(crate) frequency: u64,
    /// The raw begin/end tick pairs, in [`crate::rhi::gpu_profiler::Zone`] order.
    pub(crate) times: [u64; TIMESTAMP_SLOTS],
}

/// Ring depth. Generous enough that a frame's sentinels have long completed by the
/// time their slot comes round again, so a readback never has to block. The same
/// number, for the same reason, as the D3D11 leaf's query ring.
const RING: usize = 4;

/// One ring entry: the two sentinel command buffers bracketing one frame's work.
#[derive(Default)]
struct RingSlot {
    /// Committed before sokol's frame buffer, so the GPU reaches it first.
    begin: Option<Retained<ProtocolObject<dyn MTLCommandBuffer>>>,
    /// Committed after `sg_commit`, so the GPU reaches it last.
    end: Option<Retained<ProtocolObject<dyn MTLCommandBuffer>>>,
}

/// GPU timing on Metal: two empty command buffers per frame, on sokol's own queue
/// (Platform decisions D18).
///
/// **This measures the frame, not the passes, and that is the ceiling rather than a
/// shortcut.** Timing a pass would mean sampling counters at its boundaries, which
/// needs the command buffer and encoder sokol owns and does not hand out; Apple
/// silicon additionally has no draw-boundary counter sampling at all. Xcode's Metal
/// profiler is what covers per-pass timing on this OS.
///
/// What *is* available is the queue's own ordering. Command buffers on one
/// `MTLCommandQueue` execute in commit order, so an empty buffer committed before
/// sokol's frame and another committed after it bracket that frame on the GPU
/// timeline, and each reports a `GPUEndTime` the driver filled in. The difference is
/// the frame's GPU cost — reported as the one [`Zone::Frame`] the shared profiler
/// knows about. Using sokol's queue rather than one of our own is what makes the
/// ordering a guarantee instead of a hope.
///
/// [`Zone::Frame`]: crate::rhi::gpu_profiler::Zone::Frame
pub(crate) struct GpuTimer {
    /// sokol's command queue, retained. The same object `sg_commit` submits through.
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    ring: Vec<RingSlot>,
    frame_no: u64,
    /// The ring slot being recorded this frame.
    cur_slot: usize,
    /// Whether a frame is open. Not every frame renders a 3D scene, and a frame that
    /// opened no window of its own must not close one.
    open: bool,
}

impl GpuTimer {
    /// Take a retained handle on sokol's queue. Fails if sokol is not on Metal or has
    /// not been set up, which the caller treats as "no GPU profiling this run".
    pub(crate) fn new(device: &Device) -> GpuResult<Self> {
        // The device is not needed: the queue is sokol's, and creating one of our own
        // would put our sentinels on a timeline with no ordering relationship to the
        // frame they are supposed to bracket. Taken as a parameter anyway, because
        // the D3D11 twin needs it and the two are aliased rather than behind a trait.
        let _ = device;
        let queue = sg::mtl_command_queue().cast_mut().cast();
        // SAFETY: sokol hands back a pointer to the queue it owns for the life of the
        // device. `retain` takes a reference of our own rather than ownership of
        // sokol's, so dropping this timer does not take sokol's queue with it.
        let queue = unsafe { Retained::retain(queue) }.ok_or_else(|| {
            GpuError::Backend("sokol_gfx is not running on a Metal command queue".into())
        })?;
        Ok(Self {
            queue,
            ring: (0..RING).map(|_| RingSlot::default()).collect(),
            frame_no: 0,
            cur_slot: 0,
            open: false,
        })
    }

    /// Open a frame: pick its ring slot, read back whatever older frame that slot
    /// still holds, and commit the opening sentinel.
    pub(crate) fn begin_frame(&mut self) -> Option<FrameTimings> {
        self.frame_no += 1;
        self.cur_slot = (self.frame_no % RING as u64) as usize;
        let drained = self.read_slot(self.cur_slot);
        self.ring[self.cur_slot] = RingSlot {
            begin: self.sentinel(),
            end: None,
        };
        self.open = self.ring[self.cur_slot].begin.is_some();
        drained
    }

    /// A no-op: there are no per-pass timestamps to record here. The shared profiler
    /// calls this at every pass boundary on both OSes; on this one the answer is that
    /// the boundary is not observable — see the type's documentation.
    pub(crate) fn timestamp(&mut self, index: usize) {
        let _ = index;
    }

    /// Close the frame by committing the trailing sentinel. Called after `sg_commit`,
    /// so this buffer is behind the frame's own work in the queue.
    pub(crate) fn end_frame(&mut self) {
        if !self.open {
            return;
        }
        self.open = false;
        self.ring[self.cur_slot].end = self.sentinel();
    }

    /// An empty command buffer, committed immediately. It encodes nothing; the only
    /// thing wanted from it is the `GPUEndTime` the driver stamps when the GPU
    /// reaches it.
    fn sentinel(&self) -> Option<Retained<ProtocolObject<dyn MTLCommandBuffer>>> {
        let buffer = self.queue.commandBuffer()?;
        buffer.commit();
        Some(buffer)
    }

    /// Read one slot's pair back, if the frame it belongs to has finished on the GPU.
    ///
    /// A slot whose buffers are not both `Completed` yields `None` and is dropped
    /// rather than waited on: a profiler that stalls the frame it is measuring
    /// measures the stall. With a ring of four that should never happen.
    fn read_slot(&mut self, slot: usize) -> Option<FrameTimings> {
        let entry = std::mem::take(&mut self.ring[slot]);
        let (begin, end) = (entry.begin?, entry.end?);
        if begin.status() != MTLCommandBufferStatus::Completed
            || end.status() != MTLCommandBufferStatus::Completed
        {
            return None;
        }
        // SAFETY: reading the timing properties of two completed command buffers,
        // which is exactly when the driver has filled them in.
        let (begin_time, end_time) = unsafe { (begin.GPUEndTime(), end.GPUEndTime()) };

        let mut times = [0u64; TIMESTAMP_SLOTS];
        let base = Zone::Frame.base();
        times[base] = seconds_to_nanos(begin_time);
        times[base + 1] = seconds_to_nanos(end_time);
        Some(FrameTimings {
            written: Zone::Frame.slot_bits(),
            // `GPUEndTime` is already seconds, so the "ticks" handed to the shared
            // profiler are plain nanoseconds and the tick rate is 1 GHz.
            frequency: 1_000_000_000,
            times,
        })
    }
}

/// A `CFTimeInterval` (seconds on the mach timebase) as whole nanoseconds. Negative
/// and non-finite values clamp to zero — the shared profiler drops a zone whose end
/// does not exceed its begin, so a driver that filled in nothing reports nothing.
fn seconds_to_nanos(seconds: f64) -> u64 {
    if !seconds.is_finite() || seconds <= 0.0 {
        return 0;
    }
    (seconds * 1.0e9) as u64
}

// ---------------------------------------------------------------------------
// The readback leaf, for the offline bake only (Platform decisions D19)
// ---------------------------------------------------------------------------

/// Row alignment for the destination of a texture→buffer blit.
///
/// Metal states `destinationBytesPerRow` must be a multiple of the texture's pixel
/// size, and requires 256 on some device families. 256 satisfies every one of them
/// and costs only the padding stripped off below — the same shape as the D3D11 twin,
/// which de-pads the staging texture's `RowPitch`.
#[cfg(feature = "bake")]
const BLIT_ROW_ALIGNMENT: usize = 256;

/// Copy one subresource of a sokol image back to the CPU as tight, row-padding-free
/// bytes.
///
/// sokol_gfx has no readback of any kind — it is a rendering API, and every path that
/// draws needs none — so this is a leaf, and it is compiled only into the `bake` tool.
///
/// The blit runs on **sokol's own queue** and is committed *after* `sg_commit` (the
/// bake calls `Baker::flush` before every readback), so queue order is what guarantees
/// the passes that filled this image have finished. `waitUntilCompleted` then blocks
/// until the copy itself is done, which is exactly right for an offline tool.
///
/// `slice` is the cube face (or array layer). Unlike the D3D11 twin there is no
/// subresource arithmetic to get wrong: Metal addresses the face and the mip as two
/// separate arguments.
#[cfg(feature = "bake")]
pub(crate) fn read_image_subresource(
    image: sg::Image,
    mip: u32,
    slice: u32,
    width: u32,
    height: u32,
    format: Format,
    bpp: u32,
) -> GpuResult<Vec<u8>> {
    // Unused here, and deliberately still in the signature: the D3D11 twin needs it
    // to size a staging texture, and the two modules are aliased as one `backend`
    // rather than kept apart by a trait. Metal reads the format from the texture.
    let _ = format;

    let info = sg::mtl_query_image_info(image);
    let texture = info
        .tex
        .get(info.active_slot.max(0) as usize)
        .copied()
        .unwrap_or(std::ptr::null());
    // SAFETY: sokol hands back pointers to the objects it owns for the life of the
    // image and the device; `retain` takes references of our own, so nothing sokol
    // owns is released when these are dropped. The bake holds every target until it
    // has written its file, so the image outlives this call.
    let (texture, device, queue) = unsafe {
        (
            Retained::retain(texture.cast_mut().cast::<ProtocolObject<dyn MTLTexture>>()),
            Retained::retain(
                sg::mtl_device()
                    .cast_mut()
                    .cast::<ProtocolObject<dyn MTLDevice>>(),
            ),
            Retained::retain(
                sg::mtl_command_queue()
                    .cast_mut()
                    .cast::<ProtocolObject<dyn MTLCommandQueue>>(),
            ),
        )
    };
    let (Some(texture), Some(device), Some(queue)) = (texture, device, queue) else {
        return Err(GpuError::invalid_arg(
            "no Metal texture, device or queue for readback",
        ));
    };

    let tight_row = (width as usize) * (bpp as usize);
    let padded_row = tight_row.div_ceil(BLIT_ROW_ALIGNMENT) * BLIT_ROW_ALIGNMENT;
    let total = padded_row
        .checked_mul(height as usize)
        .ok_or_else(|| GpuError::invalid_arg("readback destination size overflowed"))?;

    // Shared storage, sized exactly as computed above, so the CPU read at the end
    // stays inside it.
    let buffer = device
        .newBufferWithLength_options(total, MTLResourceOptions::MTLResourceStorageModeShared)
        .ok_or_else(|| GpuError::invalid_arg("readback buffer was not created"))?;

    let command_buffer = queue
        .commandBuffer()
        .ok_or_else(|| GpuError::invalid_arg("no Metal command buffer for readback"))?;
    let blit = command_buffer
        .blitCommandEncoder()
        .ok_or_else(|| GpuError::invalid_arg("no Metal blit encoder for readback"))?;
    // SAFETY: the source rectangle is the whole subresource the caller named, and the
    // destination is the buffer just sized for exactly `padded_row * height` bytes.
    unsafe {
        blit.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toBuffer_destinationOffset_destinationBytesPerRow_destinationBytesPerImage(
            &texture,
            slice as usize,
            mip as usize,
            MTLOrigin { x: 0, y: 0, z: 0 },
            MTLSize {
                width: width as usize,
                height: height as usize,
                depth: 1,
            },
            &buffer,
            0,
            padded_row,
            total,
        );
    }
    blit.endEncoding();
    command_buffer.commit();
    // SAFETY: blocking on a committed command buffer. This is an offline tool; there
    // is no frame to stall.
    unsafe { command_buffer.waitUntilCompleted() };

    let mut out = Vec::with_capacity(tight_row * height as usize);
    let base = buffer.contents().as_ptr().cast::<u8>();
    // SAFETY: the buffer is shared-storage and the copy above has completed, so its
    // contents are visible to the CPU; every tight row read starts inside the
    // `padded_row * height` bytes it was created with.
    unsafe {
        for row in 0..height as usize {
            out.extend_from_slice(std::slice::from_raw_parts(
                base.add(row * padded_row),
                tight_row,
            ));
        }
    }
    Ok(out)
}
