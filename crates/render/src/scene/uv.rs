//! The 2D UV viewport.
//!
//! Its own camera, its own targets and its own derived views — the one scene
//! path that shares no state with the 3D pass, which is why `release_uv_views`
//! is called from the 3D path's `sync_frame` rather than from here.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::material::MaterialEntry;
use crate::rhi::{Frame, GpuResult, Texture, VertexBuffer};
use crate::shaders::generated;
use crate::texture::DecodedImage;
use crate::{TexBackground, TonemapSettings, UvCamera, UvFrame, UvTexture, ViewportBackground};

use super::gpu_types::{SceneUniforms, SceneVertex};

use super::slot::SlotId;

use super::gpu::SceneGpu;
use super::targets::BackbufferRect;
use super::uniforms::{post_uniforms, uv_scene_uniforms};

impl SceneGpu {
    /// Render the 2D UV viewport (instead of the 3D scene): the background (a flat
    /// fill, or a checkerboard drawn first), the 0..1 grid, the picked texture
    /// across the square, the optional island fill (Shaded / Islands modes), then
    /// the model's UV edges on top - all framed by the 2D `uv_camera` and
    /// composited with no tone map and no GTAO.
    pub(crate) fn render_uv(
        &mut self,
        frame: &mut Frame<'_>,
        uv: &UvFrame<'_>,
        uv_camera: UvCamera,
    ) -> GpuResult<()> {
        let &UvFrame {
            model,
            model_revision,
            channel,
            shading_mode,
            anti_aliasing,
            background,
            selected_nodes,
            hidden_meshes,
            texture,
        } = uv;
        // The UV viewport shows the source model, so its derived buffers belong in the
        // source slot (see the note in `render`).
        self.activate(SlotId::Source);
        self.release_opt_views();
        let size = frame.size();
        self.sync_targets(frame, size, anti_aliasing.effective_sample_count())?;
        self.sync_uv_texture(texture)?;
        self.sync_uv_view(
            model,
            model_revision,
            channel,
            shading_mode,
            selected_nodes,
            hidden_meshes,
            self.uv_texture.is_some(),
        )?;
        let checker_cell = match background {
            TexBackground::Checker { cell_px } => Some(cell_px.max(1.0)),
            _ => None,
        };
        self.sync_uv_checker(checker_cell, size)?;

        // The UV camera's orthographic view-projection; the rest of the uniform is
        // unused by the UV path (lines and fills return their own vertex colour).
        let uniforms = uv_scene_uniforms(uv_camera);

        // Clear to zero (radiance + coverage); the background is painted in the
        // composite, matching the 3D path.
        let targets = &self.targets;
        frame.begin_offscreen_pass(
            &[&targets.color, &targets.ambient],
            Some(&targets.depth),
            [0.0; 4],
            c"uv scene",
        );

        // The checkerboard, across the whole view and fixed to the screen as the Tex
        // viewport's is: its quad is in clip space, so it is drawn with no camera.
        if let Some(checker) = &self.uv_checker {
            let mut screen = uniforms;
            screen.view_projection = glam::Mat4::IDENTITY.to_cols_array_2d();
            self.draw_flat(frame, &checker.quad, &checker.material, &screen);
        }

        // Reference grid.
        self.draw_lines(frame, &self.scene.line, [&self.uv_grid], &uniforms);

        // The picked texture over the 0..1 square, under the islands, blended by its
        // own alpha where it has one.
        if let Some(backdrop) = &self.uv_texture {
            self.draw_flat(frame, &backdrop.quad, &backdrop.material, &uniforms);
        }

        // Island fill (Shaded / Islands), under the wireframe - faint over a
        // texture. It runs `fs_main`, so it binds the full material set even though
        // the zero-normal branch it takes never uses the sampled values.
        if let Some(fill) = &self.active.views.uv_fill_buf {
            let material = self.materials.fallback();
            let mut bindings = self.mesh_bindings(&self.checker_greyscale, material);
            bindings.mesh_vertices(fill);
            frame.apply_pipeline(&self.scene.uv_fill);
            frame.apply_bindings(&bindings);
            frame.apply_uniforms(generated::UB_SCENE_VS, &uniforms);
            frame.apply_uniforms(generated::UB_SCENE_FS, &uniforms);
            frame.apply_uniforms(generated::UB_MATERIAL, material.uniform());
            frame.draw(0, fill.count());
        }

        // The model's UV edges on top.
        if let Some(wireframe) = &self.active.views.uv_wireframe_buf {
            self.draw_lines(frame, &self.scene.line, [wireframe], &uniforms);
        }
        frame.end_pass();

        // Composite: no GTAO (the flat UV viewport has no depth to occlude) and no
        // tone map. The view is a diagram and a picture, not a lit scene: with the
        // operator off, a texture and the checkerboard show the colours they hold,
        // and the lines and fills the colours they were given. A flat background
        // is painted under it here; the checkerboard already covers every pixel.
        let tonemap = TonemapSettings {
            enabled: false,
            ..TonemapSettings::default()
        };
        let mut post = post_uniforms(ViewportBackground::Black, false, tonemap, false);
        let [r, g, b] = background.clear_color();
        post.bg_top = [r, g, b, 0.0];
        post.bg_bottom = [r, g, b, 0.0];
        self.queue_composite(frame, &post, targets, None, BackbufferRect::full(size));
        Ok(())
    }

    /// Draw `quad` unlit with `material`'s base color (and opacity, where it has
    /// one): a texture shown as it is. `uniforms` frame it.
    fn draw_flat(
        &self,
        frame: &mut Frame<'_>,
        quad: &VertexBuffer,
        material: &MaterialEntry,
        uniforms: &SceneUniforms,
    ) {
        let mut flat = *uniforms;
        flat.render_options = [
            UNLIT_SHADING,
            0.0,  // no UV checker
            1.0,  // its tiling, unused
            -1.0, // no vertex-colour view
        ];
        // No buffer view (-1), no skin-weight heat map.
        flat.projection_params = [1.0, 0.0, -1.0, 0.0];
        let mut bindings = self.mesh_bindings(&self.checker_greyscale, material);
        bindings.mesh_vertices(quad);
        frame.apply_pipeline(&self.scene.uv_fill);
        frame.apply_bindings(&bindings);
        frame.apply_uniforms(generated::UB_SCENE_VS, &flat);
        frame.apply_uniforms(generated::UB_SCENE_FS, &flat);
        frame.apply_uniforms(generated::UB_MATERIAL, material.uniform());
        frame.draw(0, quad.count());
    }

    /// Upload the texture the UV view is asked to show, if it isn't the one
    /// already uploaded, or drop it when none is asked for.
    fn sync_uv_texture(&mut self, wanted: Option<UvTexture<'_>>) -> GpuResult<()> {
        let Some(wanted) = wanted else {
            self.uv_texture = None;
            return Ok(());
        };
        if self
            .uv_texture
            .as_ref()
            .is_some_and(|current| current.matches(wanted.path, wanted.image))
        {
            return Ok(());
        }
        // sRGB, as a material slot uploads it: the shader works in linear light and
        // the composite encodes back, so the texel on screen is the texel stored.
        // The alpha channel is not sRGB-encoded, so the same upload is the opacity.
        let texture = Texture::rgba8_mipped_or_white(
            wanted.image.width,
            wanted.image.height,
            &wanted.image.rgba,
            true,
            c"uv texture",
        )?;
        let alpha = matches!(wanted.image.source_channels, 2 | 4);
        self.uv_texture = Some(UvTextureGpu {
            path: wanted.path.to_path_buf(),
            image: Arc::clone(wanted.image),
            material: self.materials.image_entry(Arc::new(texture), alpha),
            quad: VertexBuffer::new(&unit_square(), c"uv texture quad")?,
        });
        Ok(())
    }

    /// Keep the checkerboard's quad in step with the view's size and the cell
    /// size asked for, or drop it when the background is not the checkerboard.
    fn sync_uv_checker(&mut self, cell_px: Option<f32>, size: (u32, u32)) -> GpuResult<()> {
        let Some(cell_px) = cell_px else {
            self.uv_checker = None;
            return Ok(());
        };
        let key = (size, cell_px.to_bits());
        if let Some(checker) = self.uv_checker.as_mut() {
            if checker.key != key {
                checker.quad = VertexBuffer::new(&screen_quad(size, cell_px), c"uv checker quad")?;
                checker.key = key;
            }
            return Ok(());
        }
        let texture = Texture::rgba8_mipped_or_white(
            CHECKER_TILE,
            CHECKER_TILE,
            &checker_tile(),
            true,
            c"uv checker",
        )?;
        self.uv_checker = Some(UvCheckerGpu {
            key,
            material: self.materials.image_entry(Arc::new(texture), false),
            quad: VertexBuffer::new(&screen_quad(size, cell_px), c"uv checker quad")?,
        });
        Ok(())
    }
}

/// The shading-mode value `fs_main` reads as unlit (`render_options.x < 1.5`).
const UNLIT_SHADING: f32 = 0.0;

/// The checkerboard's two greys, as sRGB bytes: the Tex viewport's
/// (`tex::gpu`'s `CHECKER_LIGHT` / `CHECKER_DARK`, `from_gray(170)` / `(110)`).
const CHECKER_LIGHT: u8 = 170;
const CHECKER_DARK: u8 = 110;
/// Side of the checkerboard's texture, in texels: two cells each way. Big enough
/// that linear filtering softens only the texel at each cell edge.
const CHECKER_TILE: u32 = 32;

/// The UV viewport's texture: its material (the upload in its base-color and
/// opacity slots) and the 0..1 quad it is drawn on, kept with the path and decoded
/// image it was built from so an unchanged frame uploads nothing.
pub(crate) struct UvTextureGpu {
    path: PathBuf,
    image: Arc<DecodedImage>,
    material: MaterialEntry,
    quad: VertexBuffer,
}

impl UvTextureGpu {
    fn matches(&self, path: &Path, image: &Arc<DecodedImage>) -> bool {
        self.path == path && Arc::ptr_eq(&self.image, image)
    }
}

/// The UV viewport's checkerboard: a two-by-two-cell tile and the clip-space quad
/// that repeats it across the view, rebuilt when the view's size or the cell size
/// (`key`) changes.
pub(crate) struct UvCheckerGpu {
    key: ((u32, u32), u32),
    material: MaterialEntry,
    quad: VertexBuffer,
}

/// Two-by-two cells of the checkerboard's greys, `CHECKER_TILE` texels square.
fn checker_tile() -> Vec<u8> {
    let cell = CHECKER_TILE / 2;
    let mut rgba = Vec::with_capacity((CHECKER_TILE * CHECKER_TILE * 4) as usize);
    for y in 0..CHECKER_TILE {
        for x in 0..CHECKER_TILE {
            let grey = if (x / cell + y / cell).is_multiple_of(2) {
                CHECKER_LIGHT
            } else {
                CHECKER_DARK
            };
            rgba.extend_from_slice(&[grey, grey, grey, 255]);
        }
    }
    rgba
}

/// A quad covering the whole view in clip space, with UVs that put one tile -
/// two cells - in every `2 * cell_px` physical pixels.
fn screen_quad(size: (u32, u32), cell_px: f32) -> [SceneVertex; 6] {
    let tile = 2.0 * cell_px;
    let (u, v) = (size.0 as f32 / tile, size.1 as f32 / tile);
    let corner = |x: f32, y: f32, tu: f32, tv: f32| flat_vertex([x, y, 0.0], [tu, tv]);
    [
        corner(-1.0, -1.0, 0.0, 0.0),
        corner(1.0, -1.0, u, 0.0),
        corner(1.0, 1.0, u, v),
        corner(-1.0, -1.0, 0.0, 0.0),
        corner(1.0, 1.0, u, v),
        corner(-1.0, 1.0, 0.0, v),
    ]
}

/// The 0..1 UV square as two triangles, at z = 0 with UVs equal to positions.
fn unit_square() -> [SceneVertex; 6] {
    let corner = |u: f32, v: f32| flat_vertex([u, v, 0.0], [u, v]);
    [
        corner(0.0, 0.0),
        corner(1.0, 0.0),
        corner(1.0, 1.0),
        corner(0.0, 0.0),
        corner(1.0, 1.0),
        corner(0.0, 1.0),
    ]
}

/// A vertex `draw_flat` can draw: a real normal facing the UV camera, so `fs_main`
/// takes its surface path rather than the overlay one.
fn flat_vertex(position: [f32; 3], uv: [f32; 2]) -> SceneVertex {
    SceneVertex {
        position,
        normal: [0.0, 0.0, 1.0],
        uv,
        tangent: [1.0, 0.0, 0.0, 1.0],
        vertex_color: [1.0; 4],
        deform: [0; 4],
    }
}
