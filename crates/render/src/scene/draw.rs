//! The draw verbs: what a pipeline is bound to, and the overlay draws that
//! share the scene pass.

use crate::GhostStyle;
use crate::material::MaterialEntry;
use crate::rhi::{
    Bindings, ColorTarget, Frame, IndexBuffer, Pipeline, SwapchainJob, Texture, VertexBuffer,
};
use crate::shaders::generated;

use super::deform_gpu::DeformGpu;
use super::gpu_types::{PostUniforms, SceneUniforms};

use super::gpu::FULLSCREEN_VERTICES;
use super::gpu::SceneGpu;
use super::targets::{BackbufferRect, TargetSet};

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

    /// The bindings a draw running `fs_line` or `fs_selection` needs: nothing but the
    /// deform tables `vs_main` declares. Binding a texture here would be a validation
    /// error, not a harmless extra.
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

    /// Draw the idle slot's mesh as a see-through ghost over the solid one, inside the
    /// scene pass the caller has already opened.
    ///
    /// Both styles reuse pipelines that already exist. The x-ray is the
    /// selection-highlight fill — a flat tinted colour, alpha-blended, depth-tested but
    /// not depth-writing, which is exactly ghost behaviour — with the tint fed through
    /// the same `selection_color` uniform it always reads. The wireframe ghost is the
    /// same fragment shader over the same mesh vertex buffer, drawn as an indexed
    /// `LineList` through the wireframe pipeline, so it reads that tint too. Neither
    /// needs a shader change, so the committed bytecode stays valid.
    ///
    /// The deform tables it binds are the **active** slot's, not the idle one's: the
    /// ghost is drawn in its bind pose (its uniform's deform flag is off), so the
    /// tables are never read — they only have to be bound, because `vs_main` declares
    /// them.
    pub(super) fn draw_ghost(
        &self,
        frame: &mut Frame<'_>,
        style: GhostStyle,
        uniforms: &SceneUniforms,
    ) {
        match style {
            GhostStyle::Xray => {
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
            GhostStyle::Wireframe => {
                if let (Some(mesh), Some(edges)) =
                    (&self.idle.mesh, &self.idle.ghost_wireframe_index)
                {
                    let mut bindings = self.line_bindings();
                    bindings.mesh_vertices(&mesh.vertices);
                    bindings.mesh_indices(edges);
                    frame.apply_pipeline(&self.scene.wireframe);
                    frame.apply_bindings(&bindings);
                    frame.apply_uniforms(generated::UB_SCENE_VS, uniforms);
                    frame.apply_uniforms(generated::UB_SCENE_FS, uniforms);
                    frame.draw(0, edges.count());
                }
            }
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

    /// Draw the model wireframe: a `LineList` over the mesh's *own* vertex buffer,
    /// indexed by the retained edge list (`wireframe_edge_indices`).
    ///
    /// It runs `fs_selection`, so the colour comes from the `selection_color`
    /// uniform rather than from the vertices — the mesh's vertices carry the mesh's
    /// own colours. As with the Opt ghost, nothing needs restoring afterwards:
    /// uniforms are applied per draw, so the next draw's own call is the restore.
    pub(super) fn draw_wireframe(
        &self,
        frame: &mut Frame<'_>,
        uniforms: &SceneUniforms,
        color: [f32; 4],
    ) {
        let (Some(mesh), Some(edges)) = (&self.active.mesh, &self.active.views.wireframe_index)
        else {
            return;
        };
        let mut wire_uniforms = *uniforms;
        wire_uniforms.selection_color = color;
        let mut bindings = self.line_bindings();
        bindings.mesh_vertices(&mesh.vertices);
        bindings.mesh_indices(edges);
        frame.apply_pipeline(&self.scene.wireframe);
        frame.apply_bindings(&bindings);
        frame.apply_uniforms(generated::UB_SCENE_VS, &wire_uniforms);
        frame.apply_uniforms(generated::UB_SCENE_FS, &wire_uniforms);
        frame.draw(0, edges.count());
    }

    /// Draw a set of line buffers through one pipeline. They share everything but the
    /// vertex stream, so the pipeline and the scene uniforms are applied once.
    pub(super) fn draw_lines<'b>(
        &self,
        frame: &mut Frame<'_>,
        pipeline: &Pipeline,
        buffers: impl IntoIterator<Item = &'b VertexBuffer>,
        uniforms: &SceneUniforms,
    ) {
        // An iterator rather than a slice, so the caller's list of optional views
        // is filtered in place instead of collected into a `Vec` every frame.
        let mut buffers = buffers.into_iter().peekable();
        if buffers.peek().is_none() {
            return;
        }
        frame.apply_pipeline(pipeline);
        // `fs_line` reads nothing from the fragment block, so shdc strips it and only
        // the vertex block is declared.
        frame.apply_uniforms(generated::UB_SCENE_VS, uniforms);
        for buffer in buffers {
            let mut bindings = self.line_bindings();
            bindings.mesh_vertices(buffer);
            frame.apply_bindings(&bindings);
            frame.draw(0, buffer.count());
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
