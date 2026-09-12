//! A draw recorded before the swapchain pass exists, and replayed when it opens.
//!
//! Every `Renderer::render_*` runs before that pass and there is only ever one
//! composite per frame, so the composite is deferred while the offscreen passes
//! are issued directly.

use super::*;

/// Most bytes a [`SwapchainJob`] can carry for its uniform block — enough for the
/// largest one any deferred draw uploads (`tex_params`, 64 bytes). Checked at the
/// call site, at compile time, so a block that outgrows it is a build error and not
/// a truncated upload.
pub(super) const MAX_JOB_UNIFORM_BYTES: usize = 64;

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
    pub(super) fn replay(&self, target: (u32, u32)) {
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
