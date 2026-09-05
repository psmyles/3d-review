//! The Metal device and the `CAMetalLayer` sokol_gfx draws through on macOS —
//! invariant 9's sanctioned GPU site, and the only file in the workspace that names
//! Metal or Core Animation.
//!
//! It is the twin of [`super::d3d11`]: same contract, same shape, no trait between
//! them (`mac-port-plan.md` §5). What sokol_gfx cannot do for itself is create a
//! device, point `sg_environment` at it, put a drawable surface on the window, hand
//! over a render target per frame, and present — and that is all this module is.
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

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::NSView;
use objc2_foundation::CGSize;
use objc2_metal::{MTLCreateSystemDefaultDevice, MTLDevice, MTLPixelFormat};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use sokol::gfx as sg;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use crate::rhi::error::{GpuError, GpuResult};
use crate::rhi::format::Format;
use crate::rhi::gpu_profiler::TIMESTAMP_SLOTS;
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
    /// **Infallible by design** (`mac-port-plan.md` §3.1), exactly as on D3D11: a zero
    /// dimension (a minimized window) is remembered but not applied — Core Animation
    /// refuses it — and the frame is skipped instead.
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
// The GPU-timing leaf (`mac-port-plan.md` D18)
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

/// GPU timing on Metal — **not implemented yet** (`mac-port-plan.md` Phase 2 step 5).
///
/// Per-pass zones are not achievable at all here: sokol owns the command buffer and
/// the encoder, and Apple silicon has no draw-boundary counter sampling, so the most
/// this leaf can ever report is the frame as a whole — two sentinel command buffers
/// on `sg_mtl_command_queue()` and the difference of their `GPUEndTime`s. Until that
/// lands, [`Self::new`] fails, which the caller already treats as "no GPU profiling
/// this run" and reports once: `--tracy` still gives CPU zones and sokol's own
/// per-frame call counts, just no GPU timeline.
pub(crate) struct GpuTimer {
    _private: (),
}

impl GpuTimer {
    pub(crate) fn new(device: &Device) -> GpuResult<Self> {
        let _ = device;
        Err(GpuError::Backend(
            "GPU timing on Metal is not implemented yet (mac-port-plan.md Phase 2 step 5)".into(),
        ))
    }

    pub(crate) fn begin_frame(&mut self) -> Option<FrameTimings> {
        None
    }

    pub(crate) fn timestamp(&mut self, index: usize) {
        let _ = index;
    }

    pub(crate) fn end_frame(&mut self) {}
}

// ---------------------------------------------------------------------------
// The readback leaf, for the offline bake only (`mac-port-plan.md` D19)
// ---------------------------------------------------------------------------

/// Copy one subresource of a sokol image back to the CPU as tight, row-padding-free
/// bytes — **not implemented yet** (`mac-port-plan.md` Phase 2 step 5).
///
/// The design is settled: blit the private texture into a shared `MTLBuffer` on our
/// own command buffer from `sg_mtl_command_queue()`, committed *after* `sg_commit`
/// (queue order is commit order) and waited on. Until then `bake_ibl` fails loudly on
/// this Mac rather than writing a file of zeroes over the committed maps; the viewer
/// never compiles this, and the committed `.bin`s are what ships.
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
    let _ = (image, mip, slice, width, height, format, bpp);
    Err(GpuError::Backend(
        "GPU readback on Metal is not implemented yet (mac-port-plan.md Phase 2 step 5)".into(),
    ))
}
