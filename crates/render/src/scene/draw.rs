//! The draw verbs: what a pipeline is bound to, and the overlay draws that
//! share the scene pass.

use crate::material::{MaterialDrawRange, MaterialEntry};
use crate::rhi::{
    Bindings, ColorTarget, Frame, IndexBuffer, Pipeline, StorageBuffer, SwapchainJob, Texture,
    VertexBuffer,
};
use crate::shaders::generated;
use crate::{SceneFrame, ShadingMode};

use super::deform_gpu::DeformGpu;
use super::gpu_types::{LineUniforms, PostUniforms, SceneUniforms};

use super::gpu::FULLSCREEN_VERTICES;
use super::gpu::SceneGpu;
use super::targets::{BackbufferRect, TargetSet};

/// Vertices a line program draws per line: two triangles.
const VERTICES_PER_LINE: usize = 6;

impl SceneGpu {
    /// The bindings a draw running `fs_main` needs: the checker, the four IBL maps,
    /// one material's seven slots, the three samplers, and the deform tables.
    pub(super) fn mesh_bindings(&self, checker: &Texture, material: &MaterialEntry) -> Bindings {
        let mut bindings = Bindings::new();
        bindings.texture(generated::VIEW_CHECKER_TEXTURE, checker);
        bindings.sampler(generated::SMP_CHECKER_SAMPLER, &self.checker_sampler);
        self.ibl.bind_shading(&mut bindings);
        bindings.sampler(generated::SMP_IBL_SAMPLER, &self.sampler);
        material.bind_textures(&mut bindings);
        bindings.sampler(generated::SMP_MATERIAL_SAMPLER, self.materials.sampler());
        self.bind_deform(&mut bindings);
        bindings
    }

    /// The bindings a draw that samples nothing needs: only the deform tables its
    /// vertex shader declares — `vs_main` for the selection fill, `vs_line` /
    /// `vs_wire` for the lines, which add their pulled buffers on top. Binding a
    /// texture here would be a validation error, not a harmless extra.
    pub(super) fn line_bindings(&self) -> Bindings {
        let mut bindings = Bindings::new();
        self.bind_deform(&mut bindings);
        bindings
    }

    /// Fill the four deform slots: the dummies first, then whatever real tables the
    /// active model has over the top.
    pub(super) fn bind_deform(&self, bindings: &mut Bindings) {
        self.dummies.bind(bindings);
        if let Some(deform) = self.active_deform() {
            deform.bind(bindings);
        }
    }

    /// The active model's deform tables, if it has any.
    pub(super) fn active_deform(&self) -> Option<&DeformGpu> {
        self.active
            .mesh
            .as_ref()
            .and_then(|mesh| mesh.deform.as_ref())
    }

    /// Which of the active mesh's index lists the frame draws, and its per-material
    /// ranges — the one decision the shaded mesh and the line pass's depth-only redraw
    /// of it must agree on, or a line would be hidden by geometry that is not there.
    ///
    /// In precedence order: solo (isolate the selection) wins; otherwise per-mesh
    /// visibility (the filtered list, present only while some mesh is hidden);
    /// otherwise the whole mesh. All three share the mesh vertex buffer. `None` draws
    /// nothing: Wireframe shading draws no surface, solo with nothing resolved shows
    /// nothing, and an active visibility filter with no list means every mesh is
    /// hidden.
    pub(super) fn mesh_draw_list(
        &self,
        scene: &SceneFrame<'_>,
    ) -> Option<(&IndexBuffer, &[MaterialDrawRange])> {
        let mesh = self.active.mesh.as_ref()?;
        if matches!(scene.debug.shading_mode, ShadingMode::Wireframe) {
            return None;
        }
        let selection = scene.selection;
        if selection.solo && selection.selection.is_active() {
            self.active
                .selection_index
                .as_ref()
                .map(|index| (index, self.active.selection_ranges.as_slice()))
        } else if self.active.visible_active {
            self.active
                .visible_index
                .as_ref()
                .map(|index| (index, self.active.visible_ranges.as_slice()))
        } else {
            Some((&mesh.indices, mesh.ranges.as_slice()))
        }
    }

    /// Draw the idle slot's mesh as an x-ray ghost over the solid one, inside the
    /// scene pass the caller has already opened: the selection-highlight fill — a flat
    /// tinted colour, alpha-blended, depth-tested but not depth-writing, which is
    /// exactly ghost behaviour — with the tint fed through the same `selection_color`
    /// uniform it always reads. The wireframe ghost is a line and is drawn with the
    /// rest of them ([`Self::draw_line_views`]).
    ///
    /// The deform tables it binds are the **active** slot's, not the idle one's: the
    /// ghost is drawn in its bind pose (its uniform's deform flag is off), so the
    /// tables are never read — they only have to be bound, because `vs_main`
    /// declares them.
    pub(super) fn draw_xray_ghost(&self, frame: &mut Frame<'_>, uniforms: &SceneUniforms) {
        if let Some(mesh) = &self.idle.mesh {
            let mut bindings = self.line_bindings();
            bindings.mesh_vertices(&mesh.vertices);
            bindings.mesh_indices(&mesh.indices);
            frame.apply_pipeline(&self.scene.selection);
            frame.apply_bindings(&bindings);
            frame.apply_uniforms(generated::UB_SCENE_VS, uniforms);
            frame.apply_uniforms(generated::UB_SCENE_FS, uniforms);
            frame.draw(0, mesh.indices.count());
        }
    }

    /// Draw a flat tinted fill over a set of the mesh's own triangles — the
    /// selection highlight and the hover preview, which differ only in which
    /// index list and which colour they are given.
    ///
    /// Runs `fs_selection` through the depth-tested, non-depth-writing selection
    /// pipeline, so the fill is occluded by geometry in front of it and reads as
    /// lying *on* the surface rather than floating over the whole model. The
    /// colour comes from the `selection_color` uniform (the vertices carry the
    /// mesh's own), exactly as [`SceneGpu::draw_wireframe`] does; uniforms are
    /// applied per draw, so the next draw's own call is the restore.
    pub(super) fn draw_highlight(
        &self,
        frame: &mut Frame<'_>,
        indices: &IndexBuffer,
        vertices: &VertexBuffer,
        uniforms: &SceneUniforms,
        color: [f32; 4],
    ) {
        let mut fill_uniforms = *uniforms;
        fill_uniforms.selection_color = color;
        let mut bindings = self.line_bindings();
        bindings.mesh_vertices(vertices);
        bindings.mesh_indices(indices);
        frame.apply_pipeline(&self.scene.selection);
        frame.apply_bindings(&bindings);
        frame.apply_uniforms(generated::UB_SCENE_VS, &fill_uniforms);
        frame.apply_uniforms(generated::UB_SCENE_FS, &fill_uniforms);
        frame.draw(0, indices.count());
    }

    /// Redraw the mesh into the line pass's single-sample depth, writing no colour:
    /// what the lines test against when the scene pass was multisampled, since its
    /// depth cannot be read back. The same draw list, culling and depth bias as the
    /// shaded mesh, so a line is hidden exactly where it would have been in that pass.
    /// Costs a second run of the mesh's vertices and a depth-only fill, against the
    /// per-sample blending the lines no longer pay.
    pub(super) fn draw_depth_prepass(
        &self,
        frame: &mut Frame<'_>,
        scene: &SceneFrame<'_>,
        uniforms: &SceneUniforms,
    ) {
        let (Some(mesh), Some((indices, ranges))) = (&self.active.mesh, self.mesh_draw_list(scene))
        else {
            return;
        };
        let pipeline = if scene.debug.render_backfaces {
            &self.lines.depth_prepass_double_sided
        } else {
            &self.lines.depth_prepass
        };
        let mut bindings = self.line_bindings();
        bindings.mesh_vertices(&mesh.vertices);
        bindings.mesh_indices(indices);
        frame.apply_pipeline(pipeline);
        frame.apply_bindings(&bindings);
        frame.apply_uniforms(generated::UB_SCENE_VS, uniforms);
        frame.apply_uniforms(generated::UB_SCENE_FS, uniforms);
        for range in ranges {
            frame.draw(range.first_index as usize, range.index_count as usize);
        }
    }

    /// Draw the model wireframe: the retained edge list (`wireframe_edge_indices`)
    /// over the mesh's *own* vertex buffer, each edge widened into a screen-space
    /// quad.
    ///
    /// The colour comes from the `selection_color` uniform rather than from the
    /// vertices — the mesh's vertices carry the mesh's own colours. As with the Opt
    /// ghost, nothing needs restoring afterwards: uniforms are applied per draw, so
    /// the next draw's own call is the restore.
    pub(super) fn draw_wireframe(
        &self,
        frame: &mut Frame<'_>,
        uniforms: &SceneUniforms,
        line: &LineUniforms,
        color: [f32; 4],
    ) {
        let (Some(mesh), Some(edges)) = (&self.active.mesh, &self.active.views.wireframe_edges)
        else {
            return;
        };
        let mut wire_scene = *uniforms;
        wire_scene.selection_color = color;
        self.draw_edges(frame, &mesh.vertices, edges, &wire_scene, line);
    }

    /// Draw the idle slot's wireframe as the Opt overlay's wireframe ghost, in the
    /// tint `uniforms` carries. Its own vertex buffer and edge list; the deform tables
    /// bound are the active slot's, never read (see [`Self::draw_xray_ghost`]).
    pub(super) fn draw_wireframe_ghost(
        &self,
        frame: &mut Frame<'_>,
        uniforms: &SceneUniforms,
        line: &LineUniforms,
    ) {
        if let (Some(mesh), Some(edges)) = (&self.idle.mesh, &self.idle.ghost_wireframe_edges) {
            self.draw_edges(frame, &mesh.vertices, edges, uniforms, line);
        }
    }

    /// One wireframe draw through the `wire` program: six vertices per edge and no
    /// vertex input — `vs_wire` pulls each edge's two corner indices from `edges`
    /// and the corners themselves from `vertices`' storage view, so both are bound as
    /// storage rather than as the input stream.
    fn draw_edges(
        &self,
        frame: &mut Frame<'_>,
        vertices: &VertexBuffer,
        edges: &StorageBuffer<u32>,
        uniforms: &SceneUniforms,
        line: &LineUniforms,
    ) {
        /// Two corner indices per edge in the list.
        const INDICES_PER_EDGE: usize = 2;

        let mut bindings = self.line_bindings();
        if !bindings.pulled_vertices(generated::VIEW_LINE_VERTICES, vertices) {
            return;
        }
        bindings.storage(generated::VIEW_LINE_INDICES, edges);
        frame.apply_pipeline(&self.lines.wireframe);
        frame.apply_bindings(&bindings);
        // `fs_line` reads its colour from the vertex stage, so shdc strips the
        // fragment block and only the vertex block (and the line block) are declared.
        frame.apply_uniforms(generated::UB_SCENE_VS, uniforms);
        frame.apply_uniforms(generated::UB_LINE_PARAMS, line);
        frame.draw(0, edges.capacity() / INDICES_PER_EDGE * VERTICES_PER_LINE);
    }

    /// Draw a set of line lists through one line pipeline. They share everything but
    /// the vertex buffer, so the pipeline and the uniforms are applied once.
    pub(super) fn draw_lines<'b>(
        &self,
        frame: &mut Frame<'_>,
        pipeline: &Pipeline,
        buffers: impl IntoIterator<Item = &'b VertexBuffer>,
        uniforms: &SceneUniforms,
        line: &LineUniforms,
    ) {
        /// Two vertices per line in a line list.
        const VERTICES_PER_LIST_LINE: usize = 2;

        // An iterator rather than a slice, so the caller's list of optional views
        // is filtered in place instead of collected into a `Vec` every frame.
        let mut buffers = buffers.into_iter().peekable();
        if buffers.peek().is_none() {
            return;
        }
        frame.apply_pipeline(pipeline);
        // `fs_line` reads nothing from the fragment block, so shdc strips it and only
        // the vertex block (and the line block) are declared.
        frame.apply_uniforms(generated::UB_SCENE_VS, uniforms);
        frame.apply_uniforms(generated::UB_LINE_PARAMS, line);
        for buffer in buffers {
            let mut bindings = self.line_bindings();
            if !bindings.pulled_vertices(generated::VIEW_LINE_VERTICES, buffer) {
                continue;
            }
            frame.apply_bindings(&bindings);
            frame.draw(
                0,
                buffer.count() / VERTICES_PER_LIST_LINE * VERTICES_PER_LINE,
            );
        }
    }

    /// Queue the composite into the frame's swapchain pass: the AO-darkened ambient +
    /// tone map + sRGB encode over the viewport background, into `dest`.
    ///
    /// Deferred rather than drawn, because that pass has not opened yet and there is
    /// only one of it per frame (`docs/ARCHITECTURE.md`, Platform decisions: one
    /// swapchain pass). With GTAO off, `ao` is `None` and its slot is bound with the
    /// ambient target as a harmless placeholder — the shader ignores it while
    /// `gtao_enabled` is 0, but every declared view must still be bound.
    pub(super) fn queue_composite(
        &self,
        frame: &mut Frame<'_>,
        post: &PostUniforms,
        targets: &TargetSet,
        ao: Option<&ColorTarget>,
        dest: BackbufferRect,
    ) {
        let mut bindings = Bindings::new();
        bindings.target(generated::VIEW_SCENE_COLOR, &targets.color);
        bindings.target(generated::VIEW_GTAO_TEXTURE, ao.unwrap_or(&targets.ambient));
        bindings.target(generated::VIEW_AMBIENT_TEXTURE, &targets.ambient);
        bindings.sampler(generated::SMP_SCENE_SAMPLER, &self.sampler);
        frame.queue(
            SwapchainJob::new(
                &self.composite,
                FULLSCREEN_VERTICES,
                generated::UB_POST_PARAMS,
                post,
            )
            .with_bindings(&bindings)
            .with_viewport(dest.x, dest.y, dest.width, dest.height),
        );
    }
}
