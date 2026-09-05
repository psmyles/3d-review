//! The GPU plumbing: sokol_gfx, plus the one per-OS device/swapchain leaf it runs
//! on ([`backend`]).
//!
//! This module is the **sole** home for the drawing API in the renderer, and — in
//! its `backend/` leaf — for the platform GPU `unsafe` that is invariant 9's
//! sanctioned exception. Everything here is GPU plumbing; no model geometry, camera
//! math or material logic lives near it.
//!
//! It is also the sole home for the backend's *types*. Nothing outside `rhi` names an
//! `sg::` type, a pixel format enum or a window handle: every resource is created
//! through a wrapper, every format is a [`Format`], and every failure is a
//! [`GpuError`]. That is what makes the renderer's shape independent of what is
//! underneath it — see `mac-port-plan.md` §3.1.
//!
//! ## The frame
//!
//! sokol_gfx is a process singleton with no thread affinity: [`Gpu::start`] creates
//! the device and calls `sg_setup` on a bring-up thread while the window is being
//! created (D8), and the join is the synchronisation. Everything after runs on the
//! main thread.
//!
//! A frame is **exactly one swapchain pass**, because the Metal backend presents
//! inside `sg_end_pass` and a second one would double-present:
//!
//! ```ignore
//! let Some(mut frame) = gpu.begin_frame() else { return };  // None: nothing to draw into
//! renderer.render_scene(&mut frame, &scene_frame)?;         // offscreen passes
//! egui.prepare(&ctx, output)?;                              // outside any pass
//! frame.begin_swapchain_pass();                             // the pass owns the clear
//! egui.paint(ppp);
//! frame.finish(vsync)                                       // end_pass, commit, present
//! ```

pub(crate) mod backend;
mod bindings;
mod buffer;
mod error;
mod format;
pub(crate) mod gpu_profiler;
mod mips;
mod pipeline;
mod present;
mod sampler;
pub(crate) mod shader;
mod target;
mod texture;

pub(crate) use bindings::Bindings;
pub(crate) use buffer::{IndexBuffer, StorageBuffer, TransientBuffer, VertexBuffer};
pub(crate) use error::ResourceKind;
pub use error::{GpuError, GpuResult};
pub use format::Format;
pub(crate) use format::{SCENE_COLOR_FORMAT, SCENE_DEPTH_FORMAT};
pub(crate) use pipeline::{
    Blend, Cull, Depth, DepthBias, Pipeline, PipelineDesc, Topology, VertexFormat,
};
pub use present::PresentStatus;
pub(crate) use sampler::{Filter, Sampler, Wrap};
pub(crate) use target::{ColorTarget, DepthTarget};
pub(crate) use texture::Texture;

// `SwapchainJob` is declared below rather than in a module of its own: it is half of
// `Frame`'s contract, and the two are read together.

use std::ffi::{CStr, c_char, c_void};

use bytemuck::Pod;
use sokol::gfx as sg;
use winit::window::Window;

/// The GPU bring-up running on its own thread.
///
/// Device creation is the single longest item on the launch path and needs no window
/// (D8), so it starts from the first line of `main` and the window is created
/// alongside it. [`Self::attach`] joins the thread and puts a swapchain on the
/// finished window.
pub struct GpuBringUp(std::thread::JoinHandle<GpuResult<backend::Device>>);

impl GpuBringUp {
    /// Join the bring-up and create the swapchain on `window`, yielding the [`Gpu`]
    /// every later frame draws through.
    ///
    /// A panic on the bring-up thread surfaces as a [`GpuError::Backend`] rather than
    /// being re-raised here: it is a startup failure the shell reports in a dialog,
    /// not something to abort a process that has a window up.
    pub fn attach(self, window: &Window, width: u32, height: u32) -> GpuResult<Gpu> {
        let device = self
            .0
            .join()
            .map_err(|_| GpuError::Backend("the GPU bring-up thread panicked".into()))??;
        let swapchain = backend::Swapchain::new(&device, window, width, height)?;
        Ok(Gpu {
            device,
            swapchain,
            jobs: Vec::new(),
        })
    }
}

/// The process's GPU: the device sokol_gfx runs on and the window's swapchain.
///
/// One of these exists for the life of the process. Its `Drop` shuts sokol_gfx down,
/// so it must be the **last** GPU-holding field of `App` to be dropped — every
/// wrapper's own `Drop` guards on `sg::isvalid()` for the same reason.
pub struct Gpu {
    device: backend::Device,
    swapchain: backend::Swapchain,
    /// This frame's deferred draws, waiting for the swapchain pass to open (see
    /// [`SwapchainJob`]). Held here rather than in [`Frame`] so the allocation
    /// survives the frame that grew it and a steady-state frame allocates nothing.
    jobs: Vec<SwapchainJob>,
}

impl Gpu {
    /// Start bringing the GPU up on a worker thread, right now.
    ///
    /// The device and `sg_setup` need no window, and together they are the longest
    /// single item on the launch path, so they run alongside window creation instead
    /// of after it (D8). sokol_gfx has no thread affinity — it is set up here and
    /// used from the main thread after [`GpuBringUp::attach`] joins.
    pub fn start() -> GpuBringUp {
        GpuBringUp(
            std::thread::Builder::new()
                .name("gpu-bring-up".into())
                .spawn(Self::bring_up)
                .expect("the GPU bring-up thread must start"),
        )
    }

    /// The device, and sokol_gfx set up on it. Runs on the bring-up thread.
    fn bring_up() -> GpuResult<backend::Device> {
        let device = backend::Device::create()?;

        let mut desc = sg::Desc::new();
        desc.environment.defaults = sg::EnvironmentDefaults {
            color_format: backend::SWAPCHAIN_FORMAT,
            depth_format: sg::PixelFormat::None,
            sample_count: 1,
        };
        device.fill_environment(&mut desc.environment);
        // Validation failures are otherwise completely silent — sokol reports the
        // *reason* a resource came back invalid only through this channel, and a
        // windowed release build has no console for the default logger to print to.
        desc.logger = sg::Logger {
            func: Some(log_sokol),
            user_data: std::ptr::null_mut(),
        };
        sg::setup(&desc);
        if !sg::isvalid() {
            return Err(GpuError::Resource {
                kind: ResourceKind::Device,
                label: "sokol_gfx".into(),
                detail: "sg_setup did not come up on the graphics device".into(),
            });
        }
        // Per-frame draw/pipeline/bindings counts, plotted alongside the GPU zones on
        // a `--tracy` run. Off otherwise: sokol tallies them on every call.
        if gpu_profiler::should_enable() {
            sg::enable_stats();
        }
        Ok(device)
    }

    /// The current backbuffer size in physical pixels.
    pub fn size(&self) -> (u32, u32) {
        self.swapchain.size()
    }

    /// Resize the swapchain. Infallible: a zero size (a minimized window) is
    /// remembered and the frame skipped, and a backend resize that fails leaves the
    /// old buffers for the next frame — neither is anything `input.rs` could act on.
    pub fn resize(&mut self, width: u32, height: u32) {
        if (width, height) != self.swapchain.size() {
            self.swapchain.resize(width, height);
        }
    }

    /// Forward a `ScaleFactorChanged` to the backend. A no-op on Windows, where the
    /// swapchain's buffers *are* the window's pixels; on macOS it sets the layer's
    /// `contentsScale`, without which a window dragged between displays of different
    /// backing scale goes soft.
    pub fn set_scale_factor(&mut self, scale: f64) {
        self.swapchain.set_scale_factor(scale);
    }

    /// The MSAA sample counts the adapter supports for **both** scene formats — the
    /// subset of `[1, 2, 4, 8, 16]` the scene can actually render at (invariant 4:
    /// capability-gate, never crash). `1` is always included; the UI drops the rest
    /// from the Anti-Aliasing menu.
    pub fn supported_msaa_counts(&self) -> Vec<u32> {
        self.device
            .supported_sample_counts(SCENE_COLOR_FORMAT, SCENE_DEPTH_FORMAT)
    }

    /// The largest 2D texture this device can create, for `egui-winit`'s texture-size
    /// cap — a real query now rather than the hardcoded 16384 the D3D11 path assumed.
    pub fn max_texture_size(&self) -> usize {
        sg::query_limits().max_image_size_2d.max(0) as usize
    }

    /// Begin a frame: acquire the backbuffer and hand back the [`Frame`] every pass
    /// this frame is recorded through.
    ///
    /// `None` means there is nothing to draw into — a zero-sized (minimized) window,
    /// or a drawable the device refused after a reset — and the caller skips the
    /// frame rather than drawing into nothing.
    pub fn begin_frame(&mut self) -> Option<Frame<'_>> {
        let (width, height) = self.swapchain.size();
        if width == 0 || height == 0 {
            return None;
        }
        let mut swapchain = sg::Swapchain::new();
        swapchain.width = width as i32;
        swapchain.height = height as i32;
        swapchain.sample_count = 1;
        swapchain.color_format = backend::SWAPCHAIN_FORMAT;
        // The scene renders offscreen, so the swapchain pass needs no depth buffer.
        swapchain.depth_format = sg::PixelFormat::None;
        if !self.swapchain.acquire(&mut swapchain) {
            return None;
        }
        // A frame that queued jobs and then never opened its pass (an error
        // propagated out past `finish`) must not leave them for the next one.
        self.jobs.clear();
        Some(Frame {
            gpu: self,
            swapchain,
            clear: [0.0, 0.0, 0.0, 1.0],
            pass_open: false,
            finished: false,
        })
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        if sg::isvalid() {
            sg::shutdown();
        }
    }
}

impl std::fmt::Debug for Gpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (width, height) = self.swapchain.size();
        f.debug_struct("Gpu")
            .field("backend", &sg::query_backend())
            .field("size", &(width, height))
            .finish()
    }
}

/// Most bytes a [`SwapchainJob`] can carry for its uniform block — enough for the
/// largest one any deferred draw uploads (`tex_params`, 64 bytes). Checked at the
/// call site, at compile time, so a block that outgrows it is a build error and not
/// a truncated upload.
const MAX_JOB_UNIFORM_BYTES: usize = 64;

/// One draw recorded before the swapchain pass exists and replayed once it opens.
///
/// The renderer's entry points run *before* [`Frame::begin_swapchain_pass`] — there
/// is exactly one such pass per frame and it has to contain the chrome as well — so
/// anything they draw to the backbuffer (the Tex viewport's two fullscreen draws,
/// and the scene composite when that stage lands) is queued as one of these instead
/// of issued directly.
///
/// Every field is a `Copy` sokol id or inline bytes: a job keeps **no borrow** of the
/// resources it names, so a queued draw does not pin the renderer's pipelines for the
/// life of the frame. What it does rely on is that they outlive the frame — which
/// they do, since the [`crate::Renderer`] that owns them outlives every frame it
/// draws.
pub(crate) struct SwapchainJob {
    pipeline: sg::Pipeline,
    /// `None` for a shader that reads nothing: sokol validates bindings against what
    /// the shader declared, so binding an unexpected slot is as wrong as leaving a
    /// declared one empty.
    bindings: Option<sg::Bindings>,
    uniform_slot: usize,
    uniforms: [u8; MAX_JOB_UNIFORM_BYTES],
    uniform_len: usize,
    /// Vertices to draw, non-indexed — every deferred draw so far is a fullscreen
    /// triangle whose corners come from `gl_VertexIndex`.
    vertex_count: usize,
    /// The sub-rectangle of the backbuffer this job rasterizes into, or `None` for
    /// the whole thing. The Opt split is what needs it: each half composites into
    /// its own rect of the same backbuffer.
    viewport: Option<[i32; 4]>,
}

impl SwapchainJob {
    /// A draw of `vertex_count` vertices through `pipeline`, with one uniform block.
    ///
    /// `T` is the `#[repr(C)]` mirror of that block, size-asserted against shdc's
    /// generated struct beside its declaration (invariant 11).
    pub(crate) fn new<T: Pod>(
        pipeline: &Pipeline,
        vertex_count: usize,
        uniform_slot: usize,
        value: &T,
    ) -> Self {
        const {
            assert!(
                size_of::<T>() <= MAX_JOB_UNIFORM_BYTES,
                "a deferred draw's uniform block outgrew MAX_JOB_UNIFORM_BYTES"
            );
        }
        let mut uniforms = [0u8; MAX_JOB_UNIFORM_BYTES];
        uniforms[..size_of::<T>()].copy_from_slice(bytemuck::bytes_of(value));
        Self {
            pipeline: pipeline.handle(),
            bindings: None,
            uniform_slot,
            uniforms,
            uniform_len: size_of::<T>(),
            vertex_count,
            viewport: None,
        }
    }

    /// Give the draw the textures and samplers it reads.
    pub(crate) fn with_bindings(mut self, bindings: &Bindings) -> Self {
        self.bindings = Some(*bindings.raw());
        self
    }

    /// Restrict the draw to a sub-rectangle of the backbuffer, in physical pixels
    /// from the top-left.
    pub(crate) fn with_viewport(mut self, x: u32, y: u32, width: u32, height: u32) -> Self {
        self.viewport = Some([x as i32, y as i32, width as i32, height as i32]);
        self
    }

    /// Issue the draw. Called from inside the swapchain pass and nowhere else.
    fn replay(&self, target: (u32, u32)) {
        sg::apply_pipeline(self.pipeline);
        // Always set one, even for a whole-backbuffer job: sokol carries pass state
        // forward, so a job following one that narrowed the viewport would otherwise
        // inherit it.
        let [x, y, width, height] =
            self.viewport
                .unwrap_or([0, 0, target.0 as i32, target.1 as i32]);
        sg::apply_viewport(x, y, width, height, true);
        if let Some(bindings) = &self.bindings {
            sg::apply_bindings(bindings);
        }
        sg::apply_uniforms(
            self.uniform_slot,
            &sg::slice_as_range(&self.uniforms[..self.uniform_len]),
        );
        sg::draw(0, self.vertex_count, 1);
    }
}

/// One frame in flight: the acquired backbuffer, the clear it opens with, and the
/// single swapchain pass everything on-screen is drawn into.
///
/// It borrows the [`Gpu`] for its whole life, which is what makes "one frame at a
/// time" a compile-time fact rather than a convention.
pub struct Frame<'gpu> {
    gpu: &'gpu mut Gpu,
    swapchain: sg::Swapchain,
    clear: [f32; 4],
    pass_open: bool,
    finished: bool,
}

impl Frame<'_> {
    /// The backbuffer size in physical pixels.
    pub fn size(&self) -> (u32, u32) {
        (
            self.swapchain.width.max(0) as u32,
            self.swapchain.height.max(0) as u32,
        )
    }

    /// Set the colour the swapchain pass clears to (linear 0..1, gamma-space since
    /// the backbuffer is plain UNORM). Black unless something asks otherwise — the
    /// Tex viewport's solid backgrounds are exactly this and need no draw of their
    /// own. Must be called before [`Self::begin_swapchain_pass`].
    pub(crate) fn set_clear(&mut self, rgb: [f32; 3]) {
        self.clear = [rgb[0], rgb[1], rgb[2], 1.0];
    }

    /// Queue a draw for the swapchain pass. See [`SwapchainJob`].
    pub(crate) fn queue(&mut self, job: SwapchainJob) {
        debug_assert!(
            !self.pass_open,
            "a job queued after the pass opened would never be replayed"
        );
        self.gpu.jobs.push(job);
    }

    /// Open an offscreen pass over `colors` (+ `depth`), clearing the colour
    /// attachments to `clear` and depth to 0 — Reversed-Z's "infinitely far", which
    /// is what `GreaterEqual` tests against.
    ///
    /// Offscreen passes run *before* [`Self::begin_swapchain_pass`], which is why
    /// they are issued immediately while the composite that reads them is deferred
    /// as a [`SwapchainJob`]: there is exactly one swapchain pass per frame and the
    /// chrome has to be inside it too.
    pub(crate) fn begin_offscreen_pass(
        &mut self,
        colors: &[&ColorTarget],
        depth: Option<&DepthTarget>,
        clear: [f32; 4],
        label: &'static CStr,
    ) {
        debug_assert!(!self.pass_open, "a pass is already open");
        debug_assert!(!colors.is_empty(), "a pass needs at least one attachment");
        let mut pass = sg::Pass::new();
        for (slot, target) in colors.iter().enumerate() {
            pass.attachments.colors[slot] = target.attachment();
            pass.action.colors[slot] = sg::ColorAttachmentAction {
                load_action: sg::LoadAction::Clear,
                store_action: sg::StoreAction::Store,
                clear_value: sg::Color {
                    r: clear[0],
                    g: clear[1],
                    b: clear[2],
                    a: clear[3],
                },
            };
        }
        if let Some(depth) = depth {
            pass.attachments.depth_stencil = depth.attachment();
            pass.action.depth = sg::DepthAttachmentAction {
                load_action: sg::LoadAction::Clear,
                store_action: sg::StoreAction::Dontcare,
                clear_value: 0.0,
            };
        }
        pass.label = label.as_ptr();
        sg::begin_pass(&pass);
        self.pass_open = true;
    }

    /// Close the open offscreen pass. The swapchain pass is closed by
    /// [`Self::finish`] instead, which also commits and presents.
    pub(crate) fn end_pass(&mut self) {
        debug_assert!(self.pass_open, "no pass is open");
        sg::end_pass();
        self.pass_open = false;
    }

    /// Open the one swapchain pass, clearing the backbuffer, and replay whatever the
    /// renderer queued into it.
    ///
    /// Everything drawn on-screen goes between this and [`Self::finish`]. There is
    /// exactly one per frame: Metal presents inside `sg_end_pass`, so a second pass
    /// would double-present. The startup black frame is this pass with nothing drawn
    /// into it — the same code path, not a special case.
    pub fn begin_swapchain_pass(&mut self) {
        debug_assert!(!self.pass_open, "the swapchain pass is already open");
        let mut pass = sg::Pass::new();
        pass.swapchain = self.swapchain;
        pass.action.colors[0] = sg::ColorAttachmentAction {
            load_action: sg::LoadAction::Clear,
            store_action: sg::StoreAction::Store,
            clear_value: sg::Color {
                r: self.clear[0],
                g: self.clear[1],
                b: self.clear[2],
                a: self.clear[3],
            },
        };
        pass.label = c"swapchain".as_ptr();
        sg::begin_pass(&pass);
        self.pass_open = true;
        let target = (
            self.swapchain.width.max(0) as u32,
            self.swapchain.height.max(0) as u32,
        );
        for job in &self.gpu.jobs {
            job.replay(target);
        }
        self.gpu.jobs.clear();
    }

    /// Make `pipeline` current for the following draws.
    pub(crate) fn apply_pipeline(&self, pipeline: &Pipeline) {
        debug_assert!(self.pass_open, "a draw needs an open pass");
        pipeline.apply();
    }

    /// Bind what the following draws read. sokol resets bindings at every
    /// `apply_pipeline`, so this belongs *after* one, not once per pass.
    pub(crate) fn apply_bindings(&self, bindings: &Bindings) {
        debug_assert!(self.pass_open, "a draw needs an open pass");
        sg::apply_bindings(bindings.raw());
    }

    /// Upload one uniform block.
    ///
    /// `T` is a `#[repr(C)]` mirror of the block in `review.glsl`, size-asserted
    /// against shdc's generated struct beside its declaration (invariant 11) — which
    /// is what makes this safe to hand over as raw bytes.
    pub(crate) fn apply_uniforms<T: Pod>(&self, slot: usize, value: &T) {
        debug_assert!(self.pass_open, "a uniform upload needs an open pass");
        sg::apply_uniforms(slot, &sg::value_as_range(value));
    }

    /// Restrict rasterization to a sub-rectangle of the target, in physical pixels
    /// from the top-left. The composite's Opt split uses this; egui restores the full
    /// rect before painting.
    pub(crate) fn set_viewport(&self, x: i32, y: i32, width: i32, height: i32) {
        debug_assert!(self.pass_open, "a viewport needs an open pass");
        sg::apply_viewport(x, y, width, height, true);
    }

    /// Clip the following draws to a rectangle, in physical pixels from the top-left
    /// — egui's per-mesh `clip_rect`.
    pub(crate) fn set_scissor(&self, x: i32, y: i32, width: i32, height: i32) {
        debug_assert!(self.pass_open, "a scissor needs an open pass");
        sg::apply_scissor_rect(x, y, width, height, true);
    }

    /// Draw `count` elements — indices when the pipeline is indexed, vertices when
    /// it is not — starting at `base`.
    pub(crate) fn draw(&self, base: usize, count: usize) {
        debug_assert!(self.pass_open, "a draw needs an open pass");
        sg::draw(base, count, 1);
    }

    /// Close the pass, submit the frame and present it.
    ///
    /// `vsync` selects a sync interval of 1 (wait for vblank) vs 0. The redraw pacer
    /// in `app` does not rely on this blocking — on Metal the wait is at
    /// `nextDrawable`, not at present.
    pub fn finish(&mut self, vsync: bool) -> PresentStatus {
        self.end_and_commit();
        self.finished = true;
        self.gpu.swapchain.present(vsync)
    }

    /// End any open pass and submit. Shared by [`Self::finish`] and the `Drop` guard.
    fn end_and_commit(&mut self) {
        if self.pass_open {
            sg::end_pass();
            self.pass_open = false;
        }
        sg::commit();
    }
}

impl Drop for Frame<'_> {
    fn drop(&mut self) {
        // A frame abandoned mid-flight — a render error propagated out past
        // `finish` — must still close its pass and commit. Leaving sokol_gfx inside
        // a pass makes the *next* frame's `begin_pass` a validation failure, which
        // would turn one bad frame into a permanently dead viewport.
        if !self.finished && sg::isvalid() {
            self.end_and_commit();
        }
    }
}

/// sokol_gfx's log/validation channel.
///
/// Everything sokol has to say about a resource that came back invalid, a pass that
/// was mis-configured or a binding that was left unbound arrives here and nowhere
/// else, so it goes to stderr *and* to the Tracy message channel — a windowed release
/// build has no console, and `--tracy` is the only place a running session can be
/// watched. Level 0 is sokol's own "panic": the library cannot continue, so neither
/// do we.
extern "C" fn log_sokol(
    tag: *const c_char,
    level: u32,
    item: u32,
    message: *const c_char,
    line: u32,
    file: *const c_char,
    _user_data: *mut c_void,
) {
    // SAFETY: sokol passes NUL-terminated C string literals from its own static
    // storage, or null. `from_ptr` is only reached for a non-null pointer, and the
    // borrow ends inside this call.
    let text = |ptr: *const c_char| -> &str {
        if ptr.is_null() {
            ""
        } else {
            unsafe { CStr::from_ptr(ptr) }.to_str().unwrap_or("")
        }
    };
    let severity = match level {
        0 => "panic",
        1 => "error",
        2 => "warning",
        _ => "info",
    };
    let line = format!(
        "{}: {severity} [id {item}] {} ({}:{line})",
        text(tag),
        text(message),
        text(file),
    );
    eprintln!("{line}");
    gpu_profiler::note(&line);
    if level == 0 {
        panic!("{line}");
    }
}
