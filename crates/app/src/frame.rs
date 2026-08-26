//! Per-frame orchestration: the `render` method (one redraw of the viewport +
//! egui chrome) and its `build_texture_draw` helper.
//!
//! Split out of `main.rs` to keep the `ApplicationHandler` impl + window bootstrap
//! separate from the frame's draw order. `render` runs the egui pass, paces the next
//! redraw, then draws the scene (Direct3D 11) and the egui chrome on top before
//! presenting. It only reads UI state and applies the resulting `UiOutput` intents
//! through `App`'s own helpers (invariant 2).

use std::sync::Arc;
use std::time::{Duration, Instant};

use review_model::SceneBvh;
use review_render::{
    ActiveMaterial, CameraProjection, Renderer, SceneFrame, TexBackground, TexImage,
};
use review_ui::{TextureBackground, UiOutput, WorkspaceMode, draw_overlay, theme};

use crate::App;
use crate::prof;

impl App {
    pub(crate) fn render(&mut self) {
        let _frame = prof::zone!("Frame");
        let Some(window) = self.window.as_ref().cloned() else {
            return;
        };
        let Some(egui_ctx) = self.egui_ctx.as_ref().cloned() else {
            return;
        };

        window.set_title("3D Review");
        {
            let _z = prof::zone!("Camera Animation");
            self.update_camera_animation();
        }
        // Advance the selection-highlight flash and feed this frame's fade into the
        // UI snapshot the scene callback reads. Done before the egui run below so the
        // viewport reflects the current fade; a change of selection (set by the
        // Outliner last frame) restarts it here.
        self.update_selection_flash();
        // Record any edit the UI committed last frame (selection / hide / material /
        // texture) into the undo history before this frame's egui pass.
        self.observe_edit_state();

        // Bail until the D3D11 device + egui renderer exist (built in `resumed`).
        if self.gpu.is_none() || self.egui_renderer.is_none() {
            return;
        }

        // The bounding-box dimension labels occlude against the mesh through a
        // triangle BVH. Build it lazily the first frame the labels are shown for a
        // given model (and rebuild after a new model loads); reused across frames,
        // so orbiting pays no per-frame triangle cost.
        if self.ui.debug.show_bounding_box && self.occlusion_bvh_revision != self.scene_revision {
            self.occlusion_bvh = Some(SceneBvh::build(&self.scene_model));
            self.occlusion_bvh_revision = self.scene_revision;
        }

        let (full_output, ui_output) = {
            let Some(egui_state) = self.egui_state.as_mut() else {
                return;
            };
            let Some(renderer) = self.renderer.as_ref() else {
                return;
            };

            let raw_input = egui_state.take_egui_input(&window);
            let camera = renderer.camera;
            let scene_model = self.scene_model.clone();
            let occlusion_bvh = self.occlusion_bvh.as_ref();
            // Borrowed as a disjoint field so the egui closure can show the toasts
            // alongside its `&mut self.ui` borrow (the toast system lives in `app`).
            let notifications = &mut self.notifications;
            let mut ui_output = UiOutput::default();
            // The material mode before the egui pass; the toolbar / Material Mode
            // panel mutate it during the pass, so a post-pass mismatch means the
            // user switched modes this frame — surface its name as a toast (app
            // owns the toast facility; the UI only holds the plain value). The
            // active material + buffer view are snapshotted the same way so the
            // Buffers button's cycle (and entering the Buffers view) announces the
            // current buffer.
            let prev_material_mode = self.ui.debug.material_mode;
            let prev_active_material = self.ui.debug.active_material;
            let prev_buffer_view = self.ui.debug.buffer_view;
            let _z = prof::zone!("egui Run");
            let full_output = egui_ctx.run(raw_input, |ctx| {
                ui_output = draw_overlay(ctx, &mut self.ui, camera, &scene_model, occlusion_bvh);
                if self.ui.debug.material_mode != prev_material_mode {
                    notifications.mode(self.ui.debug.material_mode.label());
                }
                // Announce the buffer being viewed when the user switches *into* the
                // Buffers view or cycles to the next buffer (mirrors the material-
                // mode toast above).
                let buffers_now = self.ui.debug.active_material == ActiveMaterial::Buffers;
                let entered_buffers =
                    buffers_now && prev_active_material != ActiveMaterial::Buffers;
                let cycled_buffer = buffers_now && self.ui.debug.buffer_view != prev_buffer_view;
                if entered_buffers || cycled_buffer {
                    notifications.mode(format!("Buffer: {}", self.ui.debug.buffer_view.label()));
                }
                // Toasts paint on the egui Foreground layer, above the chrome.
                notifications.show(ctx);
            });

            egui_state.handle_platform_output(&window, full_output.platform_output.clone());
            (full_output, ui_output)
        };

        {
            let _z = prof::zone!("Apply UI Output");
            self.apply_ui_output(ui_output);
        }

        // Decide when the next frame should be drawn. Continuous motion — a live
        // camera transition, or egui asking to "repaint immediately" (zero delay)
        // — is paced to the monitor's refresh interval so the viewer never renders
        // faster than the display can show it. (We can't rely on the swapchain to
        // pace us: on the Vulkan path `present` does not block on vblank.) A finite
        // egui delay (e.g. a tooltip timer) schedules a single future wake-up, and
        // an infinite delay means everything is idle, so we wait for the next event.
        let repaint_delay = full_output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(Duration::MAX, |output| output.repaint_delay);
        let camera_animating = self
            .renderer
            .as_ref()
            .is_some_and(Renderer::is_camera_animating);
        // The selection flash animates over ~0.5s; keep pacing frames until it
        // finishes so the highlight fades smoothly rather than freezing partway.
        let flash_active = self.selection_flash.is_some();
        // Pump startup warmup frames (Phase B) until the deferred GPU-resource
        // build drains, so the scene pipelines + GTAO pass compile behind
        // the already-shown grid. Paced like the other continuous-redraw sources.
        let warming_up = self.redraw.warmup_frames > 0;
        self.redraw.warmup_frames = self.redraw.warmup_frames.saturating_sub(1);
        self.redraw.repaint_at =
            if repaint_delay.is_zero() || camera_animating || flash_active || warming_up {
                let frame_start = self.redraw.last_render_instant.unwrap_or_else(Instant::now);
                Some(frame_start + self.redraw.refresh_interval)
            } else if repaint_delay == Duration::MAX {
                None
            } else {
                Instant::now().checked_add(repaint_delay)
            };

        // The synced scene inputs the renderer draws this frame (read before the
        // disjoint renderer/gpu borrows below). `debug` carries show_grid / shading
        // / overlay flags (synced during the egui pass); `projection` the
        // perspective/orthographic toggle; the model + revision drive the mesh.
        let debug = self.ui.debug;
        let projection: CameraProjection = self.ui.projection_mode.into();
        let environment = self.ui.environment;
        let gtao = self.ui.gtao;
        let tonemap = self.ui.tonemap;
        let background = self.ui.viewport_background;
        let anti_aliasing = self.ui.anti_aliasing;
        let selection = self.ui.selection_view();
        let hidden_meshes = self.ui.hidden_mesh_nodes();
        let workspace = self.ui.mode;
        let uv_channel = self.ui.uv_view_channel;
        let uv_shading = self.ui.uv_shading_mode;
        // The Tex viewport's draw inputs (background + placed image), resolved from
        // the live UI state only in Texture mode. Built before the renderer borrow
        // below; `pixels_per_point` converts the canvas/placement from egui points to
        // physical pixels for the D3D11 image draw.
        let texture_draw = (workspace == WorkspaceMode::Texture)
            .then(|| self.build_texture_draw(full_output.pixels_per_point));
        let model = self.scene_model.clone();
        let model_revision = self.scene_revision;

        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let Some(egui_renderer) = self.egui_renderer.as_mut() else {
            return;
        };

        {
            let _z = prof::zone!("Paint + Present");
            // Render the 3D scene through Direct3D 11 straight to the backbuffer
            // (the call clears it), then draw the egui chrome on top — the central
            // viewport area of the chrome is transparent, so the scene shows
            // through. egui-directx11 tessellates the shapes internally and manages
            // its own font/texture atlas.
            let render_result = match workspace {
                WorkspaceMode::Uv => renderer.render_uv_scene(
                    gpu,
                    &model,
                    model_revision,
                    uv_channel,
                    uv_shading,
                    anti_aliasing,
                    background,
                ),
                WorkspaceMode::Texture => {
                    let (image, background) = texture_draw.unwrap_or((None, TexBackground::Black));
                    renderer.render_texture(gpu, image, background)
                }
                WorkspaceMode::ThreeD => renderer.render_scene(
                    gpu,
                    &SceneFrame {
                        model: &model,
                        model_revision,
                        debug,
                        projection,
                        environment,
                        gtao,
                        tonemap,
                        anti_aliasing,
                        selection,
                        hidden_meshes: &hidden_meshes,
                        background,
                    },
                ),
            };
            // GPU failures are surfaced as a toast (once per fault, not per
            // frame) — without `--tracy` the prof channel is invisible, and a
            // windowed release build has no console at all.
            if let Err(err) = render_result {
                prof::msg(&format!("scene D3D11 render failed: {err}"));
                if !self.gpu_fault_notified {
                    self.gpu_fault_notified = true;
                    self.notifications
                        .error(format!("Scene render failed: {err}"));
                }
            }
            let egui_output = egui_directx11::RendererOutput {
                textures_delta: full_output.textures_delta,
                shapes: full_output.shapes,
                pixels_per_point: full_output.pixels_per_point,
            };
            if let Some(backbuffer_rtv) = gpu.backbuffer_rtv()
                && let Err(err) =
                    egui_renderer.render(gpu.context(), backbuffer_rtv, &egui_ctx, egui_output)
            {
                prof::msg(&format!("egui D3D11 render failed: {err}"));
                if !self.gpu_fault_notified {
                    self.gpu_fault_notified = true;
                    self.notifications.error(format!("UI render failed: {err}"));
                }
            }
            if let review_render::PresentStatus::DeviceLost { reason } = gpu.present(true) {
                prof::msg(&format!("present failed: device lost ({reason:#x})"));
                if !self.gpu_fault_notified {
                    self.gpu_fault_notified = true;
                    self.notifications.error(format!(
                        "Graphics device lost ({reason:#x}) — restart the viewer"
                    ));
                }
            }
        }

        // Delimit the frame for Tracy's frame view (no-op unless `--tracy`).
        prof::frame_mark();
    }

    /// Resolve the Tex viewport's D3D11 draw inputs from the live UI state: the
    /// background fill, plus — when a texture is selected and the canvas has been laid
    /// out — the image placed by the canvas center + pan/zoom (egui points → physical
    /// pixels via `ppp`). The UI emits only plain values (invariant 2); `app` owns the
    /// pool and resolves placement here.
    fn build_texture_draw(&self, ppp: f32) -> (Option<TexImage>, TexBackground) {
        let background = match self.ui.texture_view.background {
            TextureBackground::Black => TexBackground::Black,
            TextureBackground::White => TexBackground::White,
            TextureBackground::Grey => TexBackground::Grey,
            TextureBackground::Checker => TexBackground::Checker {
                // The UI theme's checker cell (points), scaled to physical pixels.
                cell_px: theme::size::TEXTURE_CHECKER_CELL * ppp,
            },
        };

        let view = &self.ui.texture_view;
        let (Some(canvas), Some(entry)) = (
            self.ui.texture_canvas,
            self.ui.texture_pool.get(view.selected),
        ) else {
            return (None, background);
        };

        // The image is centered at the canvas center + pan, sized by the zoom (image
        // texels → points). The shader discards fragments outside this rect, so the
        // background shows around it.
        let image = &entry.image;
        let img_px = egui::vec2(image.width.max(1) as f32, image.height.max(1) as f32);
        let size_pts = img_px * view.zoom;
        let center = canvas.center() + view.pan;
        let min_pts = center - size_pts * 0.5;
        let tex_image = TexImage {
            path: entry.path.clone(),
            image: Arc::clone(&entry.image),
            channel: view.channel.shader_index(),
            min_px: [min_pts.x * ppp, min_pts.y * ppp],
            size_px: [size_pts.x * ppp, size_pts.y * ppp],
        };
        (Some(tex_image), background)
    }
}
