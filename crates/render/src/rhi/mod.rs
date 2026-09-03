//! Direct3D 11 / DXGI plumbing — the GPU device, immediate context, and the
//! window swapchain.
//!
//! This module is the **sole** home for Direct3D 11 / DXGI COM (`unsafe`) in the
//! renderer — the sanctioned exception to invariant 9. Everything here is GPU
//! plumbing only; no model geometry, camera math, or material logic lives in
//! `unsafe`: the device/swapchain ([`Gpu`]), the [`Pipeline`] + vertex/constant
//! buffers + targets/textures/samplers the scene path draws with, the `--tracy`
//! GPU timestamp profiler, and the offline bake device.

#[cfg(feature = "bake")]
pub(crate) mod bake;
mod buffer;
pub(crate) mod gpu_profiler;
mod pipeline;
mod sampler;
mod target;
mod texture;

pub(crate) use buffer::{DynamicConstantBuffer, IndexBuffer, StructuredBuffer, VertexBuffer};
pub(crate) use pipeline::{
    BlendMode, Cull, DepthBias, DepthCompare, DepthState, InputElement, Pipeline, PipelineDesc,
    Topology, VertexFormat,
};
pub(crate) use sampler::Sampler;
pub(crate) use target::{ColorTarget, DepthTarget};
pub(crate) use texture::{Texture, bind_ps_textures};

use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CLEAR_DEPTH, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_DEBUG,
    D3D11_CREATE_DEVICE_FLAG, D3D11_SDK_VERSION, D3D11_VIEWPORT, D3D11CreateDevice, ID3D11Device,
    ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11ShaderResourceView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT, DXGI_FORMAT_D32_FLOAT, DXGI_FORMAT_R8G8B8A8_UNORM,
    DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET, DXGI_PRESENT, DXGI_SCALING_NONE,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIAdapter, IDXGIDevice, IDXGIFactory2, IDXGISwapChain1,
};
use windows::core::{BOOL, Interface, Result};

/// The swapchain backbuffer format. Per egui-directx11's contract the render
/// target must be **gamma-space, viewed as non-sRGB-aware** (no `_SRGB`): egui
/// blends in gamma space, and our own scene composite already encodes sRGB in the
/// post pass, so a plain UNORM backbuffer is exactly right.
const BACKBUFFER_FORMAT: DXGI_FORMAT = DXGI_FORMAT_R8G8B8A8_UNORM;

/// The D3D11 device, immediate context, and the window swapchain.
///
/// One immediate context drives all rendering (auto state-tracking; no DX12-style
/// barriers). `app` creates this in `resumed`, hands `&device` to
/// `egui_directx11::Renderer`, and drives the per-frame clear/present.
pub struct Gpu {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    swap_chain: IDXGISwapChain1,
    /// The current backbuffer render-target view. Rebuilt on resize. `None` only
    /// transiently inside `resize` while the old view is released before
    /// `ResizeBuffers`.
    backbuffer_rtv: Option<ID3D11RenderTargetView>,
    size: (u32, u32),
}

impl Gpu {
    /// Create the device + immediate context + a flip-model swapchain on `hwnd`.
    ///
    /// In debug builds the D3D11 debug layer is requested first (so mis-bound
    /// resources surface as `ID3D11InfoQueue` messages, replacing wgpu's validation
    /// safety net); if the debug layer runtime isn't installed, creation falls back
    /// to a non-debug device.
    pub fn new(hwnd: HWND, width: u32, height: u32) -> Result<Self> {
        let width = width.max(1);
        let height = height.max(1);

        let base_flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
        let (device, context) =
            create_device(base_flags | debug_flag()).or_else(|_| create_device(base_flags))?;

        let swap_chain = create_swap_chain(&device, hwnd, width, height)?;
        let backbuffer_rtv = create_backbuffer_rtv(&device, &swap_chain)?;

        Ok(Self {
            device,
            context,
            swap_chain,
            backbuffer_rtv: Some(backbuffer_rtv),
            size: (width, height),
        })
    }

    /// The D3D11 device — handed to `egui_directx11::Renderer::new` and used to
    /// build GPU resources.
    pub fn device(&self) -> &ID3D11Device {
        &self.device
    }

    /// The immediate context — the single command stream all rendering records to.
    pub fn context(&self) -> &ID3D11DeviceContext {
        &self.context
    }

    /// The current backbuffer render-target view (the present surface). `None`
    /// only when a failed [`Gpu::resize`] could not rebuild the view (the next
    /// successful resize restores it) — callers skip backbuffer work that frame
    /// instead of panicking.
    pub fn backbuffer_rtv(&self) -> Option<&ID3D11RenderTargetView> {
        self.backbuffer_rtv.as_ref()
    }

    /// The current backbuffer size in physical pixels.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The MSAA sample counts the adapter supports for **both** the scene HDR color
    /// format (`Rgba16Float`) and the scene depth format (`D32_FLOAT`) — the subset
    /// of `[1, 2, 4, 8, 16]` the scene can actually render at (invariant 4:
    /// capability-gate, never crash). `1` (single-sample) is always included. The UI
    /// drops unsupported entries from the Anti-Aliasing menu.
    pub fn supported_msaa_counts(&self) -> Vec<u32> {
        [1u32, 2, 4, 8, 16]
            .into_iter()
            .filter(|&count| count == 1 || self.supports_sample_count(count))
            .collect()
    }

    /// Clamp a requested scene MSAA count to the highest level the adapter
    /// actually supports (≤ the request). Invariant 4: the renderer degrades an
    /// unsupported level (e.g. a persisted AA setting restored on a weaker
    /// adapter) instead of failing target creation for the whole frame.
    pub(crate) fn clamp_msaa(&self, requested: u32) -> u32 {
        let requested = requested.max(1);
        [16u32, 8, 4, 2]
            .into_iter()
            .filter(|&count| count <= requested)
            .find(|&count| self.supports_sample_count(count))
            .unwrap_or(1)
    }

    /// Whether the adapter supports `count`× MSAA for both the scene color + depth
    /// formats (`CheckMultisampleQualityLevels > 0` for each).
    fn supports_sample_count(&self, count: u32) -> bool {
        // SAFETY: `device` is live; `CheckMultisampleQualityLevels` is a pure query.
        let levels = |format: DXGI_FORMAT| unsafe {
            self.device
                .CheckMultisampleQualityLevels(format, count)
                .unwrap_or(0)
        };
        levels(DXGI_FORMAT_R16G16B16A16_FLOAT) > 0 && levels(DXGI_FORMAT_D32_FLOAT) > 0
    }

    /// Clear the backbuffer to `rgba` (gamma-space). Used for the first-frame
    /// black fill (replacing the old GDI hack) and the per-frame scene clear behind
    /// the egui chrome.
    pub fn clear_backbuffer(&self, rgba: [f32; 4]) {
        let Some(rtv) = self.backbuffer_rtv() else {
            return;
        };
        // SAFETY: `rtv` is a live RTV for the current backbuffer; the immediate
        // context owns it for the duration of the call.
        unsafe {
            self.context.ClearRenderTargetView(rtv, &rgba);
        }
    }

    /// Resize the swapchain to `width`×`height`. Releases the old backbuffer view,
    /// resizes the buffers, and rebuilds the view. A no-op for a zero or unchanged
    /// size.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 || (width, height) == self.size {
            return Ok(());
        }
        // The backbuffer view must be released before `ResizeBuffers`.
        self.backbuffer_rtv = None;
        // SAFETY: no outstanding references to the swapchain buffers remain (the
        // RTV was just dropped); 0/UNKNOWN keep the existing buffer count + format.
        let resized = unsafe {
            self.swap_chain.ResizeBuffers(
                0,
                width,
                height,
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            )
        };
        // Rebuild the view over whichever buffers the swapchain now holds — on a
        // failed resize the old buffers remain, so this restores the previous
        // (stale-sized) view rather than leaving the field `None` and skipping
        // every subsequent backbuffer draw until a resize succeeds.
        self.backbuffer_rtv = create_backbuffer_rtv(&self.device, &self.swap_chain).ok();
        resized?;
        if self.backbuffer_rtv.is_none() {
            self.backbuffer_rtv = Some(create_backbuffer_rtv(&self.device, &self.swap_chain)?);
        }
        self.size = (width, height);
        Ok(())
    }

    /// Issue a non-indexed draw of `count` vertices from vertex 0. The pipeline +
    /// vertex buffer + constant buffers must already be bound.
    pub(crate) fn draw(&self, count: u32) {
        // SAFETY: a plain draw on the immediate context; bound state is the
        // caller's responsibility (set just before via the rhi wrappers).
        unsafe {
            self.context.Draw(count, 0);
        }
    }

    /// Issue an indexed draw of `count` indices starting at `start_index` in the
    /// bound index buffer — one per-material draw range over the reordered mesh.
    /// Pass `start_index = 0` to draw the whole buffer.
    pub(crate) fn draw_indexed_range(&self, count: u32, start_index: u32) {
        // SAFETY: indexed draw on the immediate context; bound state is the
        // caller's responsibility.
        unsafe {
            self.context.DrawIndexed(count, start_index, 0);
        }
    }

    /// Begin the offscreen scene pass: bind `colors` as MRT render targets + `depth`
    /// as the DSV, clear color 0 to `clear` and any further colors to transparent,
    /// clear depth to the Reversed-Z far value (0), and set the viewport to the
    /// first color target's size. The scene draws into these; the composite then
    /// samples them.
    pub(crate) fn begin_scene_pass(
        &self,
        colors: &[&ColorTarget],
        depth: &DepthTarget,
        clear: [f32; 4],
    ) {
        // Fixed-size scratch (the scene MRT is at most 2 targets) so the per-frame
        // pass setup allocates nothing.
        debug_assert!(
            colors.len() <= 2,
            "begin_scene_pass supports at most 2 MRTs"
        );
        let mut rtvs: [Option<ID3D11RenderTargetView>; 2] = [const { None }; 2];
        for (rtv, color) in rtvs.iter_mut().zip(colors) {
            *rtv = Some(color.rtv().clone());
        }
        let rtvs = &rtvs[..colors.len().min(2)];
        let (width, height) = colors.first().map_or(self.size, |c| c.size());
        // SAFETY: every RTV + the DSV are live; the local arrays/viewport outlive
        // the calls. The immediate context owns the bound targets.
        unsafe {
            self.context.OMSetRenderTargets(Some(rtvs), depth.view());
            for (index, color) in colors.iter().enumerate() {
                let value = if index == 0 { clear } else { [0.0; 4] };
                self.context.ClearRenderTargetView(color.rtv(), &value);
            }
            self.context
                .ClearDepthStencilView(depth.view(), D3D11_CLEAR_DEPTH.0, 0.0, 0);
            self.context
                .RSSetViewports(Some(&[viewport(width, height)]));
        }
    }

    /// Begin a single-color, depth-less pass into `target` (the GTAO occlusion +
    /// bilateral-blur fullscreen passes): bind its RTV with no DSV and set its
    /// viewport. No clear is issued — the fullscreen triangle overwrites every
    /// pixel of the target.
    pub(crate) fn begin_color_pass(&self, target: &ColorTarget) {
        let (width, height) = target.size();
        // SAFETY: the target RTV is live; the local arrays/viewport outlive the
        // calls. The immediate context owns the bound target.
        unsafe {
            self.context
                .OMSetRenderTargets(Some(&[Some(target.rtv().clone())]), None);
            self.context
                .RSSetViewports(Some(&[viewport(width, height)]));
        }
    }

    /// Begin the backbuffer composite pass: bind the backbuffer RTV (no depth) and
    /// set the full-backbuffer viewport. The composite overwrites every pixel, so
    /// no clear is issued. A no-op while the backbuffer view is unavailable (a
    /// failed resize).
    pub(crate) fn begin_backbuffer_blit(&self) {
        let (width, height) = self.size;
        self.begin_backbuffer_blit_rect(0, 0, width, height);
    }

    /// [`Self::begin_backbuffer_blit`] into a sub-rectangle of the backbuffer.
    ///
    /// The composite draws an oversized triangle covering the whole viewport, and
    /// D3D11 rasterizes only within the bound viewport rect — so restricting the
    /// viewport is all it takes to composite into part of the backbuffer, with no
    /// scissor state and no change to the shader. Pixels outside the rect keep
    /// whatever the previous pass left there, which is what lets the Opt
    /// workspace's split view paint its two halves in two passes.
    pub(crate) fn begin_backbuffer_blit_rect(&self, x: u32, y: u32, width: u32, height: u32) {
        let Some(rtv) = self.backbuffer_rtv() else {
            return;
        };
        // SAFETY: the backbuffer RTV is live; the viewport array outlives the call.
        unsafe {
            self.context
                .OMSetRenderTargets(Some(&[Some(rtv.clone())]), None);
            self.context
                .RSSetViewports(Some(&[viewport_at(x, y, width, height)]));
        }
    }

    /// Unbind `count` pixel-shader shader-resource slots (starting at 0). Called
    /// after the composite so the offscreen color targets aren't still bound as
    /// SRVs when the next frame binds them as render targets (which the D3D11 debug
    /// layer would otherwise flag).
    pub(crate) fn unbind_ps_srvs(&self, count: usize) {
        // A fixed null table (the render path unbinds at most 3 slots) so the
        // per-frame cleanup allocates nothing.
        const NULLS: [Option<ID3D11ShaderResourceView>; 8] = [const { None }; 8];
        debug_assert!(
            count <= NULLS.len(),
            "unbind_ps_srvs supports at most 8 slots"
        );
        // SAFETY: clearing SRV slots with null views; the static array outlives call.
        unsafe {
            self.context
                .PSSetShaderResources(0, Some(&NULLS[..count.min(NULLS.len())]));
        }
    }

    /// Unbind `count` vertex-shader shader-resource slots starting at `first` —
    /// the deform buffers (`t12..t15`), released after the frame so a model swap
    /// can drop them without the debug layer seeing a stale binding.
    pub(crate) fn unbind_vs_srvs(&self, first: u32, count: usize) {
        const NULLS: [Option<ID3D11ShaderResourceView>; 8] = [const { None }; 8];
        debug_assert!(
            count <= NULLS.len(),
            "unbind_vs_srvs supports at most 8 slots"
        );
        // SAFETY: clearing SRV slots with null views; the static array outlives call.
        unsafe {
            self.context
                .VSSetShaderResources(first, Some(&NULLS[..count.min(NULLS.len())]));
        }
    }

    /// Present the backbuffer. `vsync` selects a sync interval of 1 (wait for
    /// vblank) vs 0 (immediate) — mirrors the old `AutoVsync` present mode.
    ///
    /// Status codes (e.g. `DXGI_STATUS_OCCLUDED` while the window is hidden) are
    /// not actionable and report [`PresentStatus::Presented`]; a device
    /// removed/reset HRESULT reports [`PresentStatus::DeviceLost`] so the caller
    /// can surface it — after it, every subsequent frame silently fails.
    pub fn present(&self, vsync: bool) -> PresentStatus {
        // SAFETY: presenting the live swapchain; no resources are mapped.
        let hr = unsafe { self.swap_chain.Present(u32::from(vsync), DXGI_PRESENT(0)) };
        if hr == DXGI_ERROR_DEVICE_REMOVED || hr == DXGI_ERROR_DEVICE_RESET {
            // SAFETY: pure query on the live device; returns the driver's root
            // cause for the removal (hung, reset, driver error, ...).
            let reason = unsafe { self.device.GetDeviceRemovedReason() }
                .err()
                .map(|error| error.code().0)
                .unwrap_or(hr.0);
            return PresentStatus::DeviceLost { reason };
        }
        PresentStatus::Presented
    }
}

/// Outcome of a [`Gpu::present`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentStatus {
    /// The frame presented (including benign status codes like occluded).
    Presented,
    /// The D3D11 device was removed or reset (driver crash/TDR, adapter change).
    /// The swapchain is dead; `reason` is the driver's removal HRESULT.
    DeviceLost { reason: i32 },
}

/// Bind a single SRV to pixel-shader slot `slot` — the one-off form of
/// `bind_ps_textures`, shared by the texture / target / bake wrappers so the
/// one-element bind is written once.
pub(crate) fn bind_ps_srv(ctx: &ID3D11DeviceContext, slot: u32, srv: &ID3D11ShaderResourceView) {
    // SAFETY: the SRV is live; the one-element array outlives the call.
    unsafe {
        ctx.PSSetShaderResources(slot, Some(&[Some(srv.clone())]));
    }
}

/// Unwrap a COM out-param that the preceding successful HRESULT guarantees was
/// written (the `windows` crate models out-params as `Option`s). Practically
/// unreachable; one named helper instead of a bare `unwrap` at every call site.
pub(crate) fn out_param<T>(value: Option<T>) -> T {
    value.expect("COM call succeeded but left its out-param empty")
}

/// An `E_INVALIDARG` [`windows::core::Error`] with a descriptive message — for
/// rhi-side validation failures (bad payload sizes, mismatched layouts) that
/// should surface as render errors rather than fed to the driver.
pub(crate) fn invalid_arg(message: &str) -> windows::core::Error {
    windows::core::Error::new(windows::Win32::Foundation::E_INVALIDARG, message)
}

/// A full-target viewport (top-left origin, depth 0..1) at `width`×`height`.
fn viewport(width: u32, height: u32) -> D3D11_VIEWPORT {
    viewport_at(0, 0, width, height)
}

/// A viewport covering `width`×`height` pixels at `(x, y)`, depth 0..1.
fn viewport_at(x: u32, y: u32, width: u32, height: u32) -> D3D11_VIEWPORT {
    D3D11_VIEWPORT {
        TopLeftX: x as f32,
        TopLeftY: y as f32,
        Width: width as f32,
        Height: height as f32,
        MinDepth: 0.0,
        MaxDepth: 1.0,
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

/// Create a hardware D3D11 device + immediate context at feature level 11_1 (or
/// 11_0 fallback).
fn create_device(flags: D3D11_CREATE_DEVICE_FLAG) -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let feature_levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    let mut obtained: D3D_FEATURE_LEVEL = D3D_FEATURE_LEVEL_11_0;
    // SAFETY: out-params are owned `Option`s populated by the call; the feature
    // level slice outlives the call.
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            flags,
            Some(&feature_levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            Some(&mut obtained),
            Some(&mut context),
        )?;
    }
    Ok((out_param(device), out_param(context)))
}

/// Create a flip-model swapchain on `hwnd` from `device`'s DXGI factory.
fn create_swap_chain(
    device: &ID3D11Device,
    hwnd: HWND,
    width: u32,
    height: u32,
) -> Result<IDXGISwapChain1> {
    // Walk device -> DXGI device -> adapter -> factory.
    let dxgi_device: IDXGIDevice = device.cast()?;
    // SAFETY: a live DXGI device always has an adapter.
    let adapter: IDXGIAdapter = unsafe { dxgi_device.GetAdapter()? };
    // SAFETY: a live adapter's parent is always the DXGI factory that made it.
    let factory: IDXGIFactory2 = unsafe { adapter.GetParent()? };

    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: width,
        Height: height,
        Format: BACKBUFFER_FORMAT,
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
        Flags: 0,
    };

    // SAFETY: `device` is a valid IUnknown; `hwnd` is the live window; `desc` is a
    // well-formed flip-model description.
    unsafe { factory.CreateSwapChainForHwnd(device, hwnd, &desc, None, None) }
}

/// Create the render-target view over swapchain buffer 0.
fn create_backbuffer_rtv(
    device: &ID3D11Device,
    swap_chain: &IDXGISwapChain1,
) -> Result<ID3D11RenderTargetView> {
    // SAFETY: buffer 0 always exists on a created swapchain.
    let backbuffer: ID3D11Texture2D = unsafe { swap_chain.GetBuffer(0)? };
    let mut rtv: Option<ID3D11RenderTargetView> = None;
    // SAFETY: `backbuffer` is a render-target-capable texture; default view desc.
    unsafe {
        device.CreateRenderTargetView(&backbuffer, None, Some(&mut rtv))?;
    }
    Ok(out_param(rtv))
}
