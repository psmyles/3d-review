//! The Aud workspace's overdraw views: a count of how much shading each pixel
//! costs, drawn in place of the scene.
//!
//! Both views count into a single-sample `R16F` target by additive blending, then
//! colour the count onto the view's rectangle through a ramp. Every visible triangle
//! is drawn as three *pulled* vertices — an index list in a storage buffer naming
//! corners of the mesh's own vertex buffer — so the geometry is read rather than
//! copied (invariant 1), and it deforms as the shaded mesh does.
//!
//! - **Layered** adds 1 per fragment with no depth test: every surface under a
//!   pixel, hidden ones included.
//! - **Quad** first lays the mesh's depth down, then adds `4 / k` per *visible*
//!   fragment, where `k` is how many of its 2x2 quad's pixel centres its own
//!   triangle covers — the helper lanes a GPU shades because it always shades a
//!   quad whole. 1 is a triangle that fills its quads; 4 is one pixel of a quad.
//!
//! Everything here — the pipelines, the count target and the index list — exists
//! only while a view is on (invariant 3), and only on a device that can blend into
//! a half-float target (invariant 4; `Gpu::supports_overdraw`).

use bytemuck::{Pod, Zeroable};

use crate::geometry::visible_triangle_indices;
use crate::rhi::{
    Bindings, Blend, ColorTarget, Cull, Depth, Format, Frame, GpuResult, Pipeline, PipelineDesc,
    SCENE_DEPTH_FORMAT, StorageBuffer, SwapchainJob, shader,
};
use crate::shaders::generated;
use crate::{OverdrawView, SceneFrame};

use super::gpu::{FULLSCREEN_VERTICES, SceneGpu};
use super::gpu_types::SceneUniforms;
use super::pipelines::MESH_DEPTH_BIAS;
use super::targets::{BackbufferRect, TargetSet};

/// The count target's one attachment.
const COUNT_COLORS: &[Format] = &[Format::R16F];

/// The `overdraw_params` block (`fs_overdraw_ramp`), mirroring `review.glsl`
/// (invariant 11): the view's rectangle, the counts the ramp spans, its four stops
/// and the no-surface colour.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct OverdrawUniforms {
    rect: [f32; 4],
    /// x = the count at the ramp's top, y = at its bottom.
    range: [f32; 4],
    stop0: [f32; 4],
    stop1: [f32; 4],
    stop2: [f32; 4],
    stop3: [f32; 4],
    empty: [f32; 4],
}

const _: () = assert!(size_of::<OverdrawUniforms>() == 112);
const _: () = assert!(size_of::<OverdrawUniforms>() == size_of::<generated::OverdrawParams>());
crate::shaders::assert_same_layout!(OverdrawUniforms => generated::OverdrawParams, {
    rect => rect,
    range => range,
    stop0 => stop0,
    stop1 => stop1,
    stop2 => stop2,
    stop3 => stop3,
    empty => empty,
});

/// The `quad_params` block (`fs_quad_overdraw`): x / y = the count target's size
/// in pixels.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct QuadUniforms {
    params: [f32; 4],
}

const _: () = assert!(size_of::<QuadUniforms>() == 16);
const _: () = assert!(size_of::<QuadUniforms>() == size_of::<generated::QuadParams>());
crate::shaders::assert_same_layout!(QuadUniforms => generated::QuadParams, {
    params => params,
});

/// The four count pipelines, for one culling choice.
struct CountPipelines {
    /// Layered: +1 per fragment, no depth.
    layered: Pipeline,
    /// Quad, first half: the mesh's depth, no colour.
    prepass: Pipeline,
    /// Quad, second half: +4/k per fragment that survives the prepass's depth.
    quad: Pipeline,
}

impl CountPipelines {
    fn new(cull: Cull) -> GpuResult<Self> {
        let count_shader = |label| {
            shader::make(
                generated::overdraw_count_shader_desc,
                shader::bytecode!("overdraw_count"),
                label,
            )
        };
        let layered = Pipeline::new(&PipelineDesc {
            cull,
            blend: Blend::Additive,
            ..PipelineDesc::offscreen(
                count_shader(c"overdraw layered")?,
                COUNT_COLORS,
                c"overdraw layered",
            )
        })?;
        let prepass = Pipeline::new(&PipelineDesc {
            cull,
            depth: Depth::WRITE,
            depth_bias: MESH_DEPTH_BIAS,
            depth_format: Some(SCENE_DEPTH_FORMAT),
            color_write: false,
            ..PipelineDesc::offscreen(
                count_shader(c"overdraw prepass")?,
                COUNT_COLORS,
                c"overdraw prepass",
            )
        })?;
        let quad = Pipeline::new(&PipelineDesc {
            cull,
            blend: Blend::Additive,
            depth: Depth::TEST_ONLY,
            depth_format: Some(SCENE_DEPTH_FORMAT),
            ..PipelineDesc::offscreen(
                shader::make(
                    generated::quad_overdraw_shader_desc,
                    shader::bytecode!("quad_overdraw"),
                    c"overdraw quad",
                )?,
                COUNT_COLORS,
                c"overdraw quad",
            )
        })?;
        Ok(Self {
            layered,
            prepass,
            quad,
        })
    }
}

/// What the overdraw views draw with, while one is on.
pub(super) struct OverdrawGpu {
    /// The count pipelines for the culling the frame asks for, and which that is.
    counts: CountPipelines,
    double_sided: bool,
    /// The count, as a colour, into the view's rectangle.
    ramp: Pipeline,
    /// The per-pixel count, sized to the view's targets.
    count: ColorTarget,
    /// Three corner indices per visible triangle, and what they were built from.
    indices: Option<StorageBuffer<u32>>,
    indices_key: Option<(u64, Vec<u32>)>,
}

impl OverdrawGpu {
    fn new(size: (u32, u32), double_sided: bool) -> GpuResult<Self> {
        let ramp = Pipeline::new(&PipelineDesc::swapchain(
            shader::make(
                generated::overdraw_ramp_shader_desc,
                shader::bytecode!("overdraw_ramp"),
                c"overdraw ramp",
            )?,
            c"overdraw ramp",
        ))?;
        Ok(Self {
            counts: CountPipelines::new(cull(double_sided))?,
            double_sided,
            ramp,
            count: ColorTarget::single_channel(size.0, size.1, Format::R16F, c"overdraw count")?,
            indices: None,
            indices_key: None,
        })
    }
}

fn cull(double_sided: bool) -> Cull {
    if double_sided { Cull::None } else { Cull::Back }
}

impl SceneGpu {
    /// Drop the overdraw view's resources (invariant 3) — from the viewports that
    /// never draw it, since leaving Aud is not an event the renderer sees.
    pub(crate) fn release_overdraw(&mut self) {
        self.overdraw = None;
    }

    /// Build (or free) the overdraw view's resources for this frame: the pipelines
    /// for its culling, the count target at `size`, and the visible index list.
    pub(super) fn sync_overdraw(
        &mut self,
        scene: &SceneFrame<'_>,
        size: (u32, u32),
    ) -> GpuResult<()> {
        if !scene.debug.overdraw.is_active() {
            self.overdraw = None;
            return Ok(());
        }
        let double_sided = scene.debug.render_backfaces;
        let overdraw = match &mut self.overdraw {
            Some(overdraw) => overdraw,
            slot @ None => slot.insert(OverdrawGpu::new(size, double_sided)?),
        };
        if overdraw.double_sided != double_sided {
            overdraw.counts = CountPipelines::new(cull(double_sided))?;
            overdraw.double_sided = double_sided;
        }
        if overdraw.count.size() != (size.0.max(1), size.1.max(1)) {
            overdraw.count =
                ColorTarget::single_channel(size.0, size.1, Format::R16F, c"overdraw count")?;
        }
        let current = overdraw
            .indices_key
            .as_ref()
            .is_some_and(|(revision, hidden)| {
                *revision == scene.model_revision && hidden.as_slice() == scene.hidden_meshes
            });
        if !current {
            let indices = visible_triangle_indices(scene.model, scene.hidden_meshes);
            overdraw.indices = if indices.is_empty() {
                None
            } else {
                Some(StorageBuffer::immutable(&indices, c"overdraw indices")?)
            };
            overdraw.indices_key = Some((scene.model_revision, scene.hidden_meshes.to_vec()));
        }
        Ok(())
    }

    /// Draw the overdraw view in place of the scene: count into the target, then
    /// queue the ramp into `dest`. `false` when no view is on, so the caller draws
    /// the scene instead.
    pub(super) fn record_overdraw(
        &self,
        frame: &mut Frame<'_>,
        scene: &SceneFrame<'_>,
        targets: &TargetSet,
        uniforms: &SceneUniforms,
        dest: BackbufferRect,
    ) -> bool {
        let view = scene.debug.overdraw;
        let Some(overdraw) = self.overdraw.as_ref().filter(|_| view.is_active()) else {
            return false;
        };
        let quad = view == OverdrawView::Quad;
        frame.begin_offscreen_pass(
            &[&overdraw.count],
            quad.then_some(&targets.scratch_depth),
            [0.0; 4],
            c"overdraw count",
        );
        if let (Some(mesh), Some(indices)) = (&self.active.mesh, &overdraw.indices) {
            let mut bindings = self.line_bindings();
            if bindings.pulled_vertices(generated::VIEW_LINE_VERTICES, &mesh.vertices) {
                bindings.storage(generated::VIEW_LINE_INDICES, indices);
                let draw = |pipeline: &Pipeline, quad_size: Option<QuadUniforms>| {
                    frame.apply_pipeline(pipeline);
                    frame.apply_bindings(&bindings);
                    frame.apply_uniforms(generated::UB_SCENE_VS, uniforms);
                    if let Some(quad_size) = quad_size {
                        frame.apply_uniforms(generated::UB_QUAD_PARAMS, &quad_size);
                    }
                    frame.draw(0, indices.capacity());
                };
                if quad {
                    let (width, height) = overdraw.count.size();
                    draw(&overdraw.counts.prepass, None);
                    draw(
                        &overdraw.counts.quad,
                        Some(QuadUniforms {
                            params: [width as f32, height as f32, 0.0, 0.0],
                        }),
                    );
                } else {
                    draw(&overdraw.counts.layered, None);
                }
            }
        }
        frame.end_pass();

        let (low, high) = view.range();
        let ramp = scene.debug.overdraw_ramp;
        let stop = |rgb: [f32; 3]| [rgb[0], rgb[1], rgb[2], 1.0];
        let mut bindings = Bindings::new();
        bindings.target(generated::VIEW_COUNT_TEX, &overdraw.count);
        bindings.sampler(generated::SMP_COUNT_SMP, &self.gtao_sampler);
        frame.queue(
            SwapchainJob::new(
                &overdraw.ramp,
                FULLSCREEN_VERTICES,
                generated::UB_OVERDRAW_PARAMS,
                &OverdrawUniforms {
                    rect: [
                        dest.x as f32,
                        dest.y as f32,
                        dest.width as f32,
                        dest.height as f32,
                    ],
                    range: [high, low, 0.0, 0.0],
                    stop0: stop(ramp.stops[0]),
                    stop1: stop(ramp.stops[1]),
                    stop2: stop(ramp.stops[2]),
                    stop3: stop(ramp.stops[3]),
                    empty: stop(ramp.empty),
                },
            )
            .with_bindings(&bindings)
            .with_viewport(dest.x, dest.y, dest.width, dest.height),
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use glam::{Vec2, Vec3};

    /// `fs_quad_overdraw`'s count, on the CPU: what pixel `pixel` of a `size`
    /// target adds for the triangle whose clip-space corners (x, y, w) are
    /// `corners`. Mirrors the shader line for line, so a change to one is a
    /// change to both.
    fn quad_coverage(corners: [Vec3; 3], pixel: Vec2, size: Vec2) -> f32 {
        let [a, b, c] = corners;
        let (e0, e1, e2) = (b.cross(c), c.cross(a), a.cross(b));
        let det = a.dot(e0);
        let size = size.max(Vec2::ONE);
        let pixel = pixel.floor();
        let quad = pixel - pixel % 2.0;
        let mut covered: f32 = 0.0;
        for i in 0..4 {
            let centre = quad + Vec2::new((i & 1) as f32, (i >> 1) as f32) + 0.5;
            let p = Vec3::new(
                centre.x / size.x * 2.0 - 1.0,
                1.0 - centre.y / size.y * 2.0,
                1.0,
            );
            let sign = if det > 0.0 {
                1.0
            } else if det < 0.0 {
                -1.0
            } else {
                0.0
            };
            let edges = Vec3::new(e0.dot(p), e1.dot(p), e2.dot(p)) * sign;
            if edges.min_element() >= 0.0 {
                covered += 1.0;
            }
        }
        let k: f32 = if det.abs() > 1e-12 {
            covered.max(1.0)
        } else {
            4.0
        };
        4.0 / k
    }

    /// Corners in NDC at w = 1, as an orthographic draw hands them over.
    fn flat(points: [(f32, f32); 3]) -> [Vec3; 3] {
        points.map(|(x, y)| Vec3::new(x, y, 1.0))
    }

    const SIZE: Vec2 = Vec2::new(2.0, 2.0);

    /// A triangle that fills its quads costs one shading per pixel, whichever
    /// way it winds.
    #[test]
    fn a_covering_triangle_costs_one() {
        let big = flat([(-5.0, -5.0), (5.0, -5.0), (0.0, 5.0)]);
        assert_eq!(quad_coverage(big, Vec2::ZERO, SIZE), 1.0);
        let reversed = [big[0], big[2], big[1]];
        assert_eq!(quad_coverage(reversed, Vec2::new(1.0, 1.0), SIZE), 1.0);
    }

    /// A triangle covering half a quad pays for the half it does not.
    #[test]
    fn half_a_quad_costs_two() {
        // Everything left of x = 0: the quad's left column.
        let left = flat([(0.0, -10.0), (0.0, 10.0), (-10.0, 0.0)]);
        assert_eq!(quad_coverage(left, Vec2::new(0.0, 1.0), SIZE), 2.0);
    }

    /// A triangle covering one pixel centre pays for the whole quad.
    #[test]
    fn one_pixel_costs_four() {
        // Around the top-left pixel's centre, (-0.5, 0.5) in NDC.
        let tiny = flat([(-0.6, 0.4), (-0.4, 0.4), (-0.5, 0.6)]);
        assert_eq!(quad_coverage(tiny, Vec2::ZERO, SIZE), 4.0);
    }

    /// An edge-on triangle cannot say which centres it covers, so it is taken
    /// to fill its quad: it never inflates the count. (A truly degenerate one
    /// rasterizes no fragment to ask anyway.)
    #[test]
    fn an_edge_on_triangle_counts_as_filling_its_quad() {
        let edge_on = flat([(-1.0, 0.0), (0.0, 0.0), (1.0, 0.0)]);
        assert_eq!(quad_coverage(edge_on, Vec2::ZERO, SIZE), 1.0);
    }

    /// Homogeneous corners need no divide: the same triangle scaled by w
    /// counts the same.
    #[test]
    fn coverage_does_not_depend_on_w() {
        let left = flat([(0.0, -10.0), (0.0, 10.0), (-10.0, 0.0)]);
        let scaled = left.map(|corner| corner * 3.5);
        assert_eq!(
            quad_coverage(scaled, Vec2::new(0.0, 1.0), SIZE),
            quad_coverage(left, Vec2::new(0.0, 1.0), SIZE),
        );
    }
}
