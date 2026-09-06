//! The Direct3D 11 device and the DXGI flip-model swapchain sokol_gfx draws through
//! on Windows — invariant 9's sanctioned GPU site, and the only file in the
//! workspace that names D3D11 or DXGI.
//!
//! Everything that used to live in `rhi/` — pipelines, buffers, targets, textures,
//! samplers, every draw — is sokol_gfx's now. What is left is the glue sokol cannot
//! do for itself: create a device, point `sg_environment` at it, create a swapchain
//! on the window, hand over a render-target view per frame, and present. The macOS
//! twin is a `CAMetalLayer` on the same winit window handing over an `MTLDevice` and
//! a per-frame drawable.
//!
//! The backbuffer is plain `R8G8B8A8_UNORM`: the flip model disallows `*_SRGB`
//! swapchain formats, and the composite / egui / Tex shaders encode sRGB themselves
//! (D20). The device is created on the bring-up thread ([`crate::rhi::Gpu::start`])
//! and used from the main thread after the join — D3D11 devices are free-threaded,
//! and only the main thread ever touches the immediate context.

use std::ffi::c_void;

use sokol::gfx as sg;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL,
    D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
#[cfg(feature = "bake")]
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_DEBUG, D3D11_CREATE_DEVICE_FLAG,
    D3D11_QUERY, D3D11_QUERY_DATA_TIMESTAMP_DISJOINT, D3D11_QUERY_DESC, D3D11_QUERY_TIMESTAMP,
    D3D11_QUERY_TIMESTAMP_DISJOINT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
    ID3D11DeviceContext, ID3D11Query, ID3D11RenderTargetView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT, DXGI_FORMAT_BC6H_UF16, DXGI_FORMAT_D32_FLOAT,
    DXGI_FORMAT_R8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
    DXGI_FORMAT_R16G16_FLOAT, DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_FORMAT_UNKNOWN,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET, DXGI_FEATURE_PRESENT_ALLOW_TEARING,
    DXGI_PRESENT, DXGI_PRESENT_ALLOW_TEARING, DXGI_SCALING_NONE, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING, DXGI_SWAP_EFFECT_FLIP_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIAdapter, IDXGIDevice, IDXGIFactory2, IDXGIFactory5,
    IDXGISwapChain1,
};
use windows::core::{BOOL, HRESULT, Interface};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use crate::rhi::error::{GpuError, GpuResult, ResourceContext, ResourceKind};
use crate::rhi::format::Format;
use crate::rhi::gpu_profiler::TIMESTAMP_SLOTS;
use crate::rhi::present::PresentStatus;

/// What the window's backbuffer is created as, and what sokol_gfx is told to expect
/// of a swapchain pass — so the composite, egui and Tex pipelines match it. Plain
/// `Rgba8`: flip-model chains disallow `*_SRGB` swapchain formats, and those shaders
/// encode sRGB themselves (D20). The Metal twin says `Bgra8`, which is why this is
/// the backend's choice and not a shared constant.
pub(crate) const SWAPCHAIN_FORMAT: sg::PixelFormat = sg::PixelFormat::Rgba8;

/// The same choice as a `DXGI_FORMAT`, for the swapchain description and for
/// [`dxgi_format`]'s `Format::Swapchain` arm — one constant so the two cannot drift.
const SWAPCHAIN_DXGI_FORMAT: DXGI_FORMAT = DXGI_FORMAT_R8G8B8A8_UNORM;

/// The process's D3D11 device and its immediate context — what sokol_gfx runs on.
pub(crate) struct Device {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
}

// SAFETY: a D3D11 device is free-threaded by specification. The immediate context is
// not, and it is only ever used by the main thread — the bring-up thread creates both
// and hands them over through a `JoinHandle`, whose join is the synchronization point.
unsafe impl Send for Device {}

impl Device {
    /// Create a hardware device, falling back to the WARP software rasterizer (RDP, a
    /// VM, a box with no usable adapter) rather than leaving the viewer with nothing
    /// to draw through.
    ///
    /// Each driver is tried with the debug layer first, so mis-bound resources
    /// surface as `ID3D11InfoQueue` messages in a debug build; a box without the
    /// debug runtime installed falls back to a plain device rather than failing.
    pub(crate) fn create() -> GpuResult<Device> {
        let base = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
        let mut last: Option<GpuError> = None;
        for (driver, is_warp) in [
            (D3D_DRIVER_TYPE_HARDWARE, false),
            (D3D_DRIVER_TYPE_WARP, true),
        ] {
            let made = create_device(driver, base | debug_flag())
                .or_else(|_| create_device(driver, base))
                .resource(ResourceKind::Device, "D3D11 device");
            match made {
                Ok((device, context)) => {
                    if is_warp {
                        eprintln!(
                            "3d-review: no hardware D3D11 device - using the WARP software renderer"
                        );
                    }
                    return Ok(Device { device, context });
                }
                Err(err) => last = Some(err),
            }
        }
        Err(last.unwrap_or_else(|| GpuError::Backend("no D3D11 driver".into())))
    }

    /// Point `env` at this device and its immediate context, so `sg_setup` runs on
    /// them. sokol AddRefs what it keeps, so neither pointer is retained by us beyond
    /// the device's own lifetime.
    pub(crate) fn fill_environment(&self, env: &mut sg::Environment) {
        env.d3d11 = sg::D3d11Environment {
            device: self.device.as_raw(),
            device_context: self.context.as_raw(),
        };
    }

    /// The MSAA sample counts this adapter supports for **both** given formats — the
    /// subset of `[1, 2, 4, 8, 16]` the scene can actually render at (invariant 4:
    /// capability-gate, never crash). `1` is always included.
    ///
    /// This is a leaf because sokol only reports MSAA as a yes/no per format
    /// (`sg_query_pixelformat().msaa`); the Metal twin answers with
    /// `supportsTextureSampleCount:`, where Apple silicon offers 1/2/4/8 and the 16×
    /// option disappears.
    pub(crate) fn supported_sample_counts(&self, color: Format, depth: Format) -> Vec<u32> {
        [1u32, 2, 4, 8, 16]
            .into_iter()
            .filter(|&count| {
                count == 1 || (self.quality(color, count) > 0 && self.quality(depth, count) > 0)
            })
            .collect()
    }

    /// `CheckMultisampleQualityLevels` for one format — 0 means the adapter cannot
    /// render `count`× MSAA into it.
    fn quality(&self, format: Format, count: u32) -> u32 {
        // SAFETY: `device` is live; `CheckMultisampleQualityLevels` is a pure query.
        unsafe {
            self.device
                .CheckMultisampleQualityLevels(dxgi_format(format), count)
                .unwrap_or(0)
        }
    }
}

/// The window's flip-model swapchain and the render-target view of its current
/// backbuffer.
pub(crate) struct Swapchain {
    device: ID3D11Device,
    swap_chain: IDXGISwapChain1,
    /// The backbuffer's view, created on demand ([`Self::render_view`]) and dropped
    /// on resize — `ResizeBuffers` refuses to run while one is outstanding.
    rtv: Option<ID3D11RenderTargetView>,
    width: u32,
    height: u32,
    /// Whether this adapter/compositor pair allows a tearing present, which is what
    /// makes a vsync-off present actually uncapped. Without it a flip-model
    /// `Present(0, 0)` still queues behind DWM and comes back at the refresh rate —
    /// so `present(false)` would measure the monitor. The shipped viewer always
    /// presents with vsync on and never sees this; the D2 gate is what needs it.
    tearing: bool,
}

impl Swapchain {
    /// Create a `width`×`height` flip-model swapchain on `window`'s client area.
    pub(crate) fn new(
        device: &Device,
        window: &Window,
        width: u32,
        height: u32,
    ) -> GpuResult<Self> {
        let hwnd = match window.window_handle().map(|handle| handle.as_raw()) {
            Ok(RawWindowHandle::Win32(handle)) => HWND(handle.hwnd.get() as *mut c_void),
            _ => {
                return Err(GpuError::InvalidArg(
                    "the window has no Win32 handle".into(),
                ));
            }
        };
        let (width, height) = (width.max(1), height.max(1));
        // Queried before the description is built, because supporting tearing means
        // creating the chain with the flag — it cannot be turned on at present time.
        let tearing = tearing_supported(device);
        let flags = if tearing {
            DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING.0 as u32
        } else {
            0
        };
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: SWAPCHAIN_DXGI_FORMAT,
            Stereo: BOOL(0),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_NONE,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            Flags: flags,
        };
        // SAFETY: plain COM traversal device -> DXGI device -> adapter -> factory on
        // a live device; `desc` is fully initialized and `hwnd` is the live window.
        let swap_chain = unsafe {
            let dxgi_device: IDXGIDevice = device
                .device
                .cast()
                .resource(ResourceKind::Swapchain, "DXGI device")?;
            let adapter: IDXGIAdapter = dxgi_device
                .GetAdapter()
                .resource(ResourceKind::Swapchain, "DXGI adapter")?;
            let factory: IDXGIFactory2 = adapter
                .GetParent()
                .resource(ResourceKind::Swapchain, "DXGI factory")?;
            factory
                .CreateSwapChainForHwnd(&device.device, hwnd, &desc, None, None)
                .resource(ResourceKind::Swapchain, "window swapchain")?
        };
        Ok(Swapchain {
            device: device.device.clone(),
            swap_chain,
            rtv: None,
            width,
            height,
            tearing,
        })
    }

    /// The backbuffer size in physical pixels.
    pub(crate) fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Drop the view and resize the backbuffers.
    ///
    /// **Infallible by design** (`mac-port-plan.md` §3.1): a zero dimension (a
    /// minimized window) is remembered but not applied — DXGI refuses it — and the
    /// frame is skipped instead, and a `ResizeBuffers` that fails leaves the old
    /// buffers in place for the next frame to draw into. Neither is anything the
    /// caller in `input.rs` could act on, so neither is a `Result`.
    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.rtv = None;
        if width == 0 || height == 0 {
            return;
        }
        // The flags must repeat what the chain was created with — `ResizeBuffers`
        // treats them as the new flag set, so passing 0 here would quietly drop
        // tearing support on the first resize.
        let flags = if self.tearing {
            DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING
        } else {
            DXGI_SWAP_CHAIN_FLAG(0)
        };
        // SAFETY: the view — the only outstanding reference to a backbuffer — was
        // released above; 0/UNKNOWN keep the existing buffer count and format.
        if let Err(err) = unsafe {
            self.swap_chain
                .ResizeBuffers(0, width, height, DXGI_FORMAT_UNKNOWN, flags)
        } {
            eprintln!("3d-review: swapchain ResizeBuffers failed: {err}");
        }
    }

    /// The twin of the Metal swapchain's `set_scale_factor`, and deliberately empty:
    /// a DXGI swapchain's buffers *are* the window's pixels, with no scale between
    /// the two for a DPI change to invalidate. The call exists on both so the caller
    /// stays free of `cfg`.
    pub(crate) fn set_scale_factor(&mut self, scale: f64) {
        let _ = scale;
    }

    /// Acquire this frame's render target and point `sc` at it, reporting whether
    /// there is a frame to draw. `false` (logged) means the device refused — a
    /// device-removed reset — and the frame is skipped rather than drawn into
    /// nothing. Nothing here waits on the display; a D3D11 frame blocks in
    /// [`Self::present`] instead.
    pub(crate) fn acquire(&mut self, sc: &mut sg::Swapchain) -> bool {
        match self.render_view() {
            Some(view) => {
                sc.d3d11.render_view = view;
                true
            }
            None => false,
        }
    }

    /// The render-target view of the current backbuffer, as the raw pointer
    /// `sg_swapchain` takes, creating it if a resize dropped it.
    fn render_view(&mut self) -> Option<*const c_void> {
        if self.rtv.is_none() {
            // SAFETY: buffer 0 of a live flip-model swapchain; the out-pointer is a
            // live local, written only on success.
            self.rtv = unsafe {
                let back: ID3D11Texture2D = match self.swap_chain.GetBuffer(0) {
                    Ok(back) => back,
                    Err(err) => {
                        eprintln!("3d-review: swapchain GetBuffer failed: {err}");
                        return None;
                    }
                };
                let mut rtv: Option<ID3D11RenderTargetView> = None;
                match self
                    .device
                    .CreateRenderTargetView(&back, None, Some(&mut rtv))
                {
                    Ok(()) => rtv,
                    Err(err) => {
                        eprintln!("3d-review: CreateRenderTargetView failed: {err}");
                        None
                    }
                }
            };
        }
        self.rtv.as_ref().map(|rtv| rtv.as_raw().cast_const())
    }

    /// Present the completed frame. `vsync` selects a sync interval of 1 (wait for
    /// vblank) vs 0 (immediate).
    ///
    /// Benign status codes (e.g. `DXGI_STATUS_OCCLUDED` while the window is hidden)
    /// are not actionable and report [`PresentStatus::Presented`]; a device
    /// removed/reset HRESULT reports [`PresentStatus::DeviceLost`] with the driver's
    /// root cause, so the caller can surface it — after one, every subsequent frame
    /// silently fails.
    pub(crate) fn present(&mut self, vsync: bool) -> PresentStatus {
        // Tearing is only legal with a sync interval of 0, and only on a chain
        // created with the matching flag.
        let flags = if !vsync && self.tearing {
            DXGI_PRESENT_ALLOW_TEARING
        } else {
            DXGI_PRESENT(0)
        };
        // SAFETY: presenting the live swapchain; no resources are mapped.
        let hr = unsafe { self.swap_chain.Present(u32::from(vsync), flags) };
        if hr == DXGI_ERROR_DEVICE_REMOVED || hr == DXGI_ERROR_DEVICE_RESET {
            // SAFETY: pure query on the live device; returns the driver's root cause
            // for the removal (hung, reset, driver error, ...).
            let reason = unsafe { self.device.GetDeviceRemovedReason() }
                .err()
                .map_or(hr.0, |err| err.code().0);
            return PresentStatus::DeviceLost { reason };
        }
        PresentStatus::Presented
    }
}

/// Whether the DXGI factory reports `PRESENT_ALLOW_TEARING`. `IDXGIFactory5` and
/// the feature both arrived in Windows 10; anything older, or a `cast` that fails,
/// answers no and the swapchain is created exactly as before.
fn tearing_supported(device: &Device) -> bool {
    // SAFETY: plain COM traversal on a live device, then a pure feature query whose
    // out-parameter is a `BOOL` sized by `size_of`.
    unsafe {
        let Ok(dxgi_device) = device.device.cast::<IDXGIDevice>() else {
            return false;
        };
        let Ok(adapter) = dxgi_device.GetAdapter() else {
            return false;
        };
        let Ok(factory) = adapter.GetParent::<IDXGIFactory5>() else {
            return false;
        };
        let mut allowed = BOOL(0);
        factory
            .CheckFeatureSupport(
                DXGI_FEATURE_PRESENT_ALLOW_TEARING,
                std::ptr::from_mut(&mut allowed).cast(),
                u32::try_from(size_of::<BOOL>()).unwrap_or(4),
            )
            .is_ok()
            && allowed.as_bool()
    }
}

/// The renderer's [`Format`] as the DXGI format the MSAA capability query speaks.
/// Only the formats that query is ever asked about are listed — every other format is
/// sokol's business now, named as a `sg::PixelFormat` and never as a `DXGI_FORMAT`.
fn dxgi_format(format: Format) -> DXGI_FORMAT {
    // Deliberately exhaustive, with no catch-all arm: this used to end in
    // `_ => R8G8B8A8_UNORM`, which was harmless while `supported_sample_counts` was
    // the only caller (it asks about the two scene formats and nothing else) and
    // silently wrong the moment the bake's readback reused it. A staging texture in
    // the wrong format makes `CopySubresourceRegion` a no-op that reports success,
    // so the BRDF LUT read back as its own cleared zeroes and looked exactly like a
    // pass that had not run. Adding a `Format` must be a compile error here.
    match format {
        Format::Rgba8 => DXGI_FORMAT_R8G8B8A8_UNORM,
        Format::Rgba8Srgb => DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
        Format::Rgba16F => DXGI_FORMAT_R16G16B16A16_FLOAT,
        Format::Rg16F => DXGI_FORMAT_R16G16_FLOAT,
        Format::R8 => DXGI_FORMAT_R8_UNORM,
        Format::Bc6hUf16 => DXGI_FORMAT_BC6H_UF16,
        Format::Depth32F => DXGI_FORMAT_D32_FLOAT,
        // Nothing creates a texture in the swapchain format — it only ever describes
        // a *pipeline's* colour target — so the concrete one is what the chain was
        // made with.
        Format::Swapchain => SWAPCHAIN_DXGI_FORMAT,
    }
}

/// The debug-device flag in debug builds, none in release.
fn debug_flag() -> D3D11_CREATE_DEVICE_FLAG {
    if cfg!(debug_assertions) {
        D3D11_CREATE_DEVICE_DEBUG
    } else {
        D3D11_CREATE_DEVICE_FLAG(0)
    }
}

/// Create a D3D11 device + immediate context on `driver` at feature level 11_1 (or
/// the 11_0 fallback).
fn create_device(
    driver: D3D_DRIVER_TYPE,
    flags: D3D11_CREATE_DEVICE_FLAG,
) -> windows::core::Result<(ID3D11Device, ID3D11DeviceContext)> {
    let feature_levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    let mut obtained: D3D_FEATURE_LEVEL = D3D_FEATURE_LEVEL_11_0;
    // SAFETY: the out-params are owned `Option`s populated by the call; the feature
    // level slice outlives it.
    unsafe {
        D3D11CreateDevice(
            None,
            driver,
            Default::default(),
            flags,
            Some(&feature_levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            Some(&mut obtained),
            Some(&mut context),
        )?;
    }
    match (device, context) {
        (Some(device), Some(context)) => Ok((device, context)),
        // Unreachable by contract: a successful HRESULT writes both out-params.
        _ => Err(windows::core::Error::from(HRESULT(-1))),
    }
}

// ---------------------------------------------------------------------------
// The GPU-timing leaf (`mac-port-plan.md` D18)
// ---------------------------------------------------------------------------

/// One frame's resolved GPU timestamps, handed back to [`crate::rhi::gpu_profiler`]
/// to become Tracy zones.
///
/// The split is deliberate: everything platform-specific about *measuring* GPU time
/// is here, everything about *reporting* it is shared. The Metal twin will fill one
/// of these from two sentinel command buffers instead of a query ring, and the Tracy
/// side will not know the difference.
pub(crate) struct FrameTimings {
    /// Bit *per timestamp slot* that the frame actually recorded.
    ///
    /// This is measured, not declared. Timestamp queries are reused across ring
    /// slots, so a query left un-`End`-ed this frame still reads back whatever it
    /// held when it was last written four frames ago — a stale pair that looks
    /// perfectly valid. Only the slots in here were written by the frame these
    /// timings belong to.
    pub(crate) written: u16,
    /// Ticks per second, from the disjoint query.
    pub(crate) frequency: u64,
    /// The raw begin/end tick pairs, in [`crate::rhi::gpu_profiler::Zone`] order.
    pub(crate) times: [u64; TIMESTAMP_SLOTS],
}

/// One ring entry: a disjoint query (frequency + validity for the frame), the
/// per-zone begin/end timestamp queries, and the bookkeeping to drain it a few
/// frames after it was written.
struct RingSlot {
    disjoint: ID3D11Query,
    /// `TIMESTAMP_SLOTS` timestamp queries (zone begin/end pairs).
    timestamps: Vec<ID3D11Query>,
    /// `true` once this slot's queries have been `End`-ed for a frame and not yet
    /// read back.
    pending: bool,
    /// Which timestamp slots this frame actually wrote.
    written: u16,
}

/// Ring depth. Generous enough that a slot written at frame N is read and freed well
/// before it is reused at N+RING, so its results are always ready by then.
const RING: usize = 4;

/// GPU timing on Direct3D 11: a ring of per-frame timestamp queries.
///
/// D3D11 has no "begin/end pass with timestamp writes". Each `ID3D11Query` of type
/// `TIMESTAMP` records the GPU clock at the point `End` is called in the command
/// stream (`Begin` is a no-op for timestamps), and a sibling `TIMESTAMP_DISJOINT`
/// query bracketing the whole frame yields the tick `Frequency` plus a `Disjoint`
/// flag that invalidates a frame whose clock skipped.
///
/// Results are not ready the moment a frame is encoded — the GPU has to finish
/// first — so a slot is read back only when it is about to be reused, `RING` frames
/// later. Tracy does not mind that the numbers arrive late; it places them by the
/// GPU's own clock.
///
/// It times sokol_gfx's work by recording into the **same immediate context** sokol
/// submits through: `ctx.End(query)` between sokol calls is valid, since sokol issues
/// its own commands on that context and neither library holds state the other
/// disturbs. The handles come from our own [`Device`] rather than
/// `sg_d3d11_device_context()` — they are the same objects, because we created them
/// and handed them to `sg_setup`, and using ours avoids rebuilding a COM pointer from
/// a `*const c_void`.
pub(crate) struct GpuTimer {
    /// A cloned context handle (refcounted, cheap). Held so the per-zone calls take
    /// no parameters, which is what keeps the call sites at the passes readable.
    context: ID3D11DeviceContext,
    ring: Vec<RingSlot>,
    frame_no: u64,
    /// The ring slot being encoded this frame.
    cur_slot: usize,
    /// Whether a frame is open. Not every frame renders a 3D scene — the Tex
    /// viewport draws two fullscreen jobs and nothing else — and `End`-ing a
    /// disjoint query that was never `Begin`-ed is a validation error, so the
    /// timestamp and close calls are no-ops until a frame is opened.
    open: bool,
}

impl GpuTimer {
    /// Build the query ring. Timestamp + disjoint queries are core to D3D11 feature
    /// level 11_0+, but `CreateQuery` can still fail on an exotic driver, so this is
    /// fallible and the caller treats `Err` as "no GPU profiling this run".
    pub(crate) fn new(device: &Device) -> GpuResult<Self> {
        let mut ring = Vec::with_capacity(RING);
        for _ in 0..RING {
            let disjoint = create_query(&device.device, D3D11_QUERY_TIMESTAMP_DISJOINT)
                .resource(ResourceKind::Query, "disjoint timestamp")?;
            let mut timestamps = Vec::with_capacity(TIMESTAMP_SLOTS);
            for _ in 0..TIMESTAMP_SLOTS {
                timestamps.push(
                    create_query(&device.device, D3D11_QUERY_TIMESTAMP)
                        .resource(ResourceKind::Query, "timestamp")?,
                );
            }
            ring.push(RingSlot {
                disjoint,
                timestamps,
                pending: false,
                written: 0,
            });
        }
        Ok(Self {
            context: device.context.clone(),
            ring,
            frame_no: 0,
            cur_slot: 0,
            open: false,
        })
    }

    /// Start a frame: pick this frame's ring slot, drain it if it still holds an
    /// unread result (written `RING` frames ago, so it is done), and open the
    /// disjoint query. Anything returned belongs to that older frame.
    pub(crate) fn begin_frame(&mut self) -> Option<FrameTimings> {
        self.frame_no += 1;
        let slot = (self.frame_no % RING as u64) as usize;
        self.cur_slot = slot;
        let drained = if self.ring[slot].pending {
            self.read_slot(slot)
        } else {
            None
        };
        self.ring[slot].written = 0;
        // SAFETY: `disjoint` is a live query owned by the ring; the immediate context
        // records into it, and `Begin` is valid for a disjoint query.
        unsafe { self.context.Begin(&self.ring[slot].disjoint) };
        self.open = true;
        drained
    }

    /// Record the timestamp at `index` in this frame's slot: `End` on a timestamp
    /// query writes the GPU clock at that point in the command stream.
    pub(crate) fn timestamp(&mut self, index: usize) {
        if !self.open {
            return;
        }
        let slot = self.cur_slot;
        // SAFETY: the timestamp query is live and belongs to the slot `begin_frame`
        // selected this frame; `index` is bounds-checked by the slice index.
        unsafe { self.context.End(&self.ring[slot].timestamps[index]) };
        self.ring[slot].written |= 1 << index;
    }

    /// Close the frame's timing window and mark the slot for a later readback.
    pub(crate) fn end_frame(&mut self) {
        if !self.open {
            return;
        }
        self.open = false;
        let slot = self.cur_slot;
        // SAFETY: the disjoint query is live and was `Begin`-ed this frame.
        unsafe { self.context.End(&self.ring[slot].disjoint) };
        self.ring[slot].pending = true;
    }

    /// Read one slot's disjoint + timestamps back. A frame whose clock was disjoint,
    /// or whose results are somehow not ready, yields `None` and is dropped.
    fn read_slot(&mut self, slot: usize) -> Option<FrameTimings> {
        self.ring[slot].pending = false;

        // Pre-zeroed so a not-ready `GetData` reads as `Frequency == 0` and is
        // skipped: the `windows` wrapper cannot tell "ready" (`S_OK`) from "not
        // ready" (`S_FALSE`) — both are success HRESULTs and map to `Ok(())` — and
        // `S_FALSE` leaves the output untouched.
        let mut disjoint = D3D11_QUERY_DATA_TIMESTAMP_DISJOINT::default();
        // SAFETY: the disjoint query is live; the output struct is sized exactly and
        // outlives the call.
        let _ = unsafe {
            self.context.GetData(
                &self.ring[slot].disjoint,
                Some((&mut disjoint as *mut D3D11_QUERY_DATA_TIMESTAMP_DISJOINT).cast()),
                size_of::<D3D11_QUERY_DATA_TIMESTAMP_DISJOINT>() as u32,
                0,
            )
        };
        if disjoint.Frequency == 0 || disjoint.Disjoint.as_bool() {
            return None;
        }

        let mut times = [0u64; TIMESTAMP_SLOTS];
        for (index, value) in times.iter_mut().enumerate() {
            // SAFETY: each timestamp query is live; `value` is a `u64` output slot.
            let _ = unsafe {
                self.context.GetData(
                    &self.ring[slot].timestamps[index],
                    Some((value as *mut u64).cast()),
                    size_of::<u64>() as u32,
                    0,
                )
            };
        }
        Some(FrameTimings {
            written: self.ring[slot].written,
            frequency: disjoint.Frequency,
            times,
        })
    }
}

/// Create an `ID3D11Query` of `query_type`.
fn create_query(
    device: &ID3D11Device,
    query_type: D3D11_QUERY,
) -> windows::core::Result<ID3D11Query> {
    let desc = D3D11_QUERY_DESC {
        Query: query_type,
        MiscFlags: 0,
    };
    let mut query = None;
    // SAFETY: `desc` is a well-formed query description; the out-param is populated.
    unsafe { device.CreateQuery(&desc, Some(&mut query))? };
    query.ok_or_else(|| windows::core::Error::from(HRESULT(-1)))
}

// ---------------------------------------------------------------------------
// The readback leaf, for the offline bake only (`mac-port-plan.md` D19)
// ---------------------------------------------------------------------------

/// Copy one subresource of a sokol image back to the CPU as tight, row-padding-free
/// bytes.
///
/// sokol_gfx has no readback of any kind — it is a rendering API, and every path
/// that draws needs none — so this is a leaf, and it is compiled only into the
/// `bake` tool. The Metal twin blits the private texture into a shared `MTLBuffer`
/// on a command buffer from `sg_mtl_command_queue()`, committed after `sg_commit`.
///
/// `slice` is the cube face (or array layer). D3D11 indexes a cube's subresources
/// `mip + face * mip_count`, which is why the mip count is queried here rather than
/// asked of the caller: the two must agree, and only one of them can be wrong.
///
/// Nothing needs unbinding first, unlike the D3D11 original: a sokol pass is closed
/// by `sg_end_pass`, so by the time this runs the image is no longer a bound render
/// target and the RTV/SRV hazard that once baked six flat black cubes cannot arise.
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
    let info = sg::d3d11_query_image_info(image);
    if info.tex2d.is_null() {
        return Err(GpuError::invalid_arg(
            "readback source is not a D3D11 2D texture",
        ));
    }
    // sokol's native-handle getters are `*const`, and `from_raw_borrowed` wants
    // `&*mut`; the constness carries no meaning across the C ABI here.
    let texture_ptr = info.tex2d.cast_mut();
    let device_ptr = sg::d3d11_device().cast_mut();
    let context_ptr = sg::d3d11_device_context().cast_mut();
    // SAFETY: sokol hands back borrowed COM pointers it owns for the life of the
    // image and the device; `from_raw_borrowed` does not take ownership, so nothing
    // is released here. The image outlives this call — the bake holds every target
    // until it has written the file — and the bake's single thread is the only user
    // of the immediate context.
    let (source, device, context) = unsafe {
        (
            ID3D11Texture2D::from_raw_borrowed(&texture_ptr),
            ID3D11Device::from_raw_borrowed(&device_ptr),
            ID3D11DeviceContext::from_raw_borrowed(&context_ptr),
        )
    };
    let (Some(source), Some(device), Some(context)) = (source, device, context) else {
        return Err(GpuError::invalid_arg(
            "no D3D11 texture, device or context for readback",
        ));
    };

    let mips = sg::query_image_num_mipmaps(image).max(1) as u32;
    let subresource = mip + slice * mips;

    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: dxgi_format(format),
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let mut staging = None;
    // SAFETY: a CPU-readable staging texture with no initial data; the out-param is
    // populated on success.
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut staging)) }
        .resource(ResourceKind::Texture, "readback staging")?;
    let staging =
        staging.ok_or_else(|| GpuError::invalid_arg("readback staging texture was not created"))?;

    // SAFETY: the source subresource matches the staging texture in format and size;
    // a null box copies the whole subresource.
    unsafe {
        context.CopySubresourceRegion(&staging, 0, 0, 0, 0, source, subresource, None);
    }

    let tight_row = (width as usize) * (bpp as usize);
    let mut out = Vec::with_capacity(tight_row * height as usize);
    // SAFETY: `staging` is mappable; the mapped range is at least `RowPitch * height`
    // bytes, so every tight-row copy stays in bounds. `Unmap` is paired with `Map`.
    unsafe {
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        context
            .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .resource(ResourceKind::Texture, "readback map")?;
        let base = mapped.pData as *const u8;
        let row_pitch = mapped.RowPitch as usize;
        for row in 0..height as usize {
            out.extend_from_slice(std::slice::from_raw_parts(
                base.add(row * row_pitch),
                tight_row,
            ));
        }
        context.Unmap(&staging, 0);
    }
    Ok(out)
}
