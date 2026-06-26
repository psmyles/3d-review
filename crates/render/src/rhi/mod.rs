//! Direct3D 11 / DXGI plumbing — the GPU device, immediate context, and the
//! window swapchain.
//!
//! This module is the **sole** home for Direct3D 11 / DXGI COM (`unsafe`) in the
//! renderer — the second sanctioned exception to invariant 9 (the first being
//! `app`'s window/swapchain bootstrap). Everything here is GPU plumbing only; no
//! model geometry, camera math, or material logic lives in `unsafe`. As the scene
//! passes are ported off wgpu (migration Phases 1–5) the pipeline/buffer/pass/
//! upload wrappers join this module; it now also carries the [`Pipeline`],
//! vertex/constant buffers, and the depth target the scene path draws with.

mod buffer;
mod pipeline;
mod sampler;
mod target;
mod texture;

pub(crate) use buffer::{DynamicConstantBuffer, IndexBuffer, VertexBuffer};
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
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_UNKNOWN,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_PRESENT, DXGI_SCALING_NONE, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIAdapter, IDXGIDevice,
    IDXGIFactory2, IDXGISwapChain1,
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

    /// The current backbuffer render-target view (the present surface).
    pub fn backbuffer_rtv(&self) -> &ID3D11RenderTargetView {
        self.backbuffer_rtv
            .as_ref()
            .expect("backbuffer_rtv is only None transiently during resize")
    }

    /// The current backbuffer size in physical pixels.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Clear the backbuffer to `rgba` (gamma-space). Used for the first-frame
    /// black fill (replacing the old GDI hack) and the per-frame scene clear behind
    /// the egui chrome.
    pub fn clear_backbuffer(&self, rgba: [f32; 4]) {
        // SAFETY: `backbuffer_rtv` is a live RTV for the current backbuffer; the
        // immediate context owns it for the duration of the call.
        unsafe {
            self.context
                .ClearRenderTargetView(self.backbuffer_rtv(), &rgba);
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
        unsafe {
            self.swap_chain.ResizeBuffers(
                0,
                width,
                height,
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            )?;
        }
        self.backbuffer_rtv = Some(create_backbuffer_rtv(&self.device, &self.swap_chain)?);
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
        let rtvs: Vec<Option<ID3D11RenderTargetView>> =
            colors.iter().map(|c| Some(c.rtv().clone())).collect();
        let (width, height) = colors.first().map_or(self.size, |c| c.size());
        // SAFETY: every RTV + the DSV are live; the local arrays/viewport outlive
        // the calls. The immediate context owns the bound targets.
        unsafe {
            self.context.OMSetRenderTargets(Some(&rtvs), depth.view());
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
    /// no clear is issued.
    pub(crate) fn begin_backbuffer_blit(&self) {
        let (width, height) = self.size;
        // SAFETY: the backbuffer RTV is live; the viewport array outlives the call.
        unsafe {
            self.context
                .OMSetRenderTargets(Some(&[Some(self.backbuffer_rtv().clone())]), None);
            self.context
                .RSSetViewports(Some(&[viewport(width, height)]));
        }
    }

    /// Unbind `count` pixel-shader shader-resource slots (starting at 0). Called
    /// after the composite so the offscreen color targets aren't still bound as
    /// SRVs when the next frame binds them as render targets (which the D3D11 debug
    /// layer would otherwise flag).
    pub(crate) fn unbind_ps_srvs(&self, count: usize) {
        let nulls: Vec<Option<ID3D11ShaderResourceView>> = vec![None; count];
        // SAFETY: clearing SRV slots with null views; the local array outlives call.
        unsafe {
            self.context.PSSetShaderResources(0, Some(&nulls));
        }
    }

    /// Present the backbuffer. `vsync` selects a sync interval of 1 (wait for
    /// vblank) vs 0 (immediate) — mirrors the old `AutoVsync` present mode.
    pub fn present(&self, vsync: bool) {
        // SAFETY: presenting the live swapchain; no resources are mapped.
        unsafe {
            // Present returns an HRESULT (e.g. DXGI_STATUS_OCCLUDED when the window
            // is hidden); none is actionable here, so it's deliberately ignored.
            let _ = self.swap_chain.Present(u32::from(vsync), DXGI_PRESENT(0));
        }
    }
}

/// A full-target viewport (top-left origin, depth 0..1) at `width`×`height`.
fn viewport(width: u32, height: u32) -> D3D11_VIEWPORT {
    D3D11_VIEWPORT {
        TopLeftX: 0.0,
        TopLeftY: 0.0,
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
    Ok((device.unwrap(), context.unwrap()))
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
    // SAFETY: a live DXGI device always has an adapter, and the adapter a factory.
    let adapter: IDXGIAdapter = unsafe { dxgi_device.GetAdapter()? };
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
    Ok(rtv.unwrap())
}
