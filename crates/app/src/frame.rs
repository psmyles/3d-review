//! Per-frame orchestration: the `render` method (one redraw of the viewport +
//! egui chrome) and its `build_texture_draw` helper.
//!
//! Split out of `main.rs` to keep the `ApplicationHandler` impl + window bootstrap
//! separate from the frame's draw order. `render` runs the egui pass, paces the next
//! redraw, then draws the scene and the egui chrome on top before presenting. It only
//! reads UI state and applies the resulting `UiOutput` intents through `App`'s own
//! helpers (invariant 2).
//!
//! The draw order is the one sokol_gfx's pass model imposes (`docs/ARCHITECTURE.md`,
//! Platform decisions: one swapchain pass):
//! acquire the frame, let the renderer record its offscreen passes, tessellate the
//! chrome *outside* any pass, then open the **one** swapchain pass — Metal presents
//! inside `sg_end_pass`, so a second one would double-present — composite into it,
//! paint the chrome over that, and finish.

use std::sync::Arc;
use std::time::{Duration, Instant};

use review_model::ModelData;
use review_render::{
    ActiveMaterial, CameraProjection, OptSceneFrame, OptView, ProcessedModelRef, Renderer,
    SceneFrame, SceneViewport, TexBackground, TexImage, UvFrame, UvTexture,
};
use review_ui::{
    ComparisonSide, OptLayout, OptOverlayLevel, OptOverlayView, TextureBackground, UiOutput,
    WorkspaceMode, draw_overlay, theme,
};

use crate::App;
use crate::keys;
use crate::prof;

/// The notice slot every GPU fault report shares, so a device that faults over
/// and over rewrites one card instead of stacking a new one per frame.
const GPU_FAULT_NOTICE: &str = "gpu-fault";

impl App {
    pub(crate) fn render(&mut self) {
        let _frame = prof::zone!("Frame");
        // Timestamps the frame and steps the scripted orbit; no-op without
        // `--gate-out` (`gate.rs`).
        self.gate_frame_begin();
        let gate_active = self.gate_active();
        let vsync = self.gate_vsync();
        let Some(window) = self.window.as_ref().cloned() else {
            return;
        };
        let Some(egui_ctx) = self.egui_ctx.as_ref().cloned() else {
            return;
        };

        {
            let _z = prof::zone!("Camera Animation");
            self.update_camera_animation();
            // Move the flycam by this frame's share of whatever direction keys are
            // held (`flycam.rs`). Beside the transition step because it is the
            // other thing that moves the camera without an input event of its own.
            self.step_flycam();
        }
        // Advance the animation clock and re-evaluate the pose when the clip or
        // time moved (the palette upload is keyed by its revision).
        self.tick_animation();
        // Record any edit the UI committed last frame (selection / hide / material /
        // texture) into the undo history before this frame's egui pass.
        self.observe_edit_state();

        // Bail until the GPU + egui renderer exist (built in `resumed`).
        if self.gpu.is_none() || self.egui_renderer.is_none() {
            return;
        }

        // A minimized window has a zero-sized backbuffer, and there is nothing to
        // draw into until it is restored. `begin_frame` skips such a frame too, but
        // the bail belongs *here*: everything between the two — the egui pass, the
        // chrome layout, the scene rect the viewport is derived from — would
        // otherwise run against a zero-sized screen, which is not a size any of it
        // is meaningful at. Restoring the window resizes the swapchain and requests
        // the redraw that resumes drawing.
        let (backbuffer_width, backbuffer_height) =
            self.gpu.as_ref().map_or((0, 0), review_render::Gpu::size);
        if backbuffer_width == 0 || backbuffer_height == 0 {
            return;
        }

        // Hand the Log window what has been logged since last frame, if it is up.
        self.feed_log_window();

        let Some((full_output, ui_output)) = self.run_egui_pass(&window, &egui_ctx) else {
            return;
        };

        // The pass above is what decides whether egui took the last press — a
        // panel divider or a window's resize edge grabs it here, a frame after
        // `egui_winit` had to guess. Resolve the proposal before anything reads
        // `drag_mode`.
        self.settle_pending_drag(&egui_ctx);

        {
            let _z = prof::zone!("Apply UI Output");
            self.apply_ui_output(ui_output);
        }
        // The pass may have opened or closed the Log window.
        self.watch_log_window();

        // Reconcile the Opt workspace with the stack the egui pass just edited:
        // create its subsystem on first entry, schedule a run for any change, and
        // manage the "still working" notice. Only while the workspace is active —
        // a session that never opens it never builds any of this.
        if self.ui.mode == WorkspaceMode::Opt {
            let _z = prof::zone!("Sync Opt");
            self.sync_opt();
        }
        // The audit runs on every load, whichever workspace is up: the toolbar
        // shows its count from anywhere.
        self.sync_audit();
        self.sync_audit_highlight();

        self.schedule_next_frame(&full_output);

        let Some(before_present) = self.paint(full_output, &egui_ctx, gate_active, vsync) else {
            return;
        };
        self.gate_after_present(before_present);

        // Delimit the frame for Tracy's frame view (no-op unless `--tracy`).
        prof::frame_mark();
    }

    /// Run this frame's egui pass: lay out the chrome over the current state,
    /// collect the intents it emits, and announce the mode switches it made.
    /// `None` until the egui state and the renderer exist.
    fn run_egui_pass(
        &mut self,
        window: &winit::window::Window,
        egui_ctx: &egui::Context,
    ) -> Option<(egui::FullOutput, UiOutput)> {
        // The Opt workspace's processed level, resolved *before* the egui pass so its
        // half of the split can be labelled with its own box and its own camera. Like
        // every other input the chrome reads, this is the state as of the start of the
        // frame: `sync_opt` below may land a newer result that this frame's render
        // then draws, leaving the labels one frame behind on that frame alone. The
        // revision call is idempotent within a frame, so the render's own call further
        // down still resolves the same level.
        let opt_level = (self.ui.mode == WorkspaceMode::Opt)
            .then(|| self.opt_processed_revision())
            .flatten()
            .zip(self.opt.as_ref().and_then(|opt| opt.processed.clone()));
        // The same preference the scene draw makes, so the labels describe the
        // mesh actually on screen rather than the one behind it.
        let opt_overlay_preview = opt_level
            .is_some()
            .then_some(self.opt.as_ref())
            .flatten()
            .and_then(|opt| opt.level_mesh(self.ui.opt.active_lod));
        let opt_level_model = opt_level.as_ref().and_then(|(revision, result)| {
            opt_overlay_preview
                .as_deref()
                .or_else(|| result.lod(self.ui.opt.active_lod).map(|lod| &lod.model))
                .map(|model| (model, *revision))
        });
        let (full_output, ui_output) = {
            let egui_state = self.egui_state.as_mut()?;
            let renderer = self.renderer.as_ref()?;

            let raw_input = egui_state.take_egui_input(window);
            let camera = renderer.camera;
            let scene_model = self.scene_model.clone();
            // Both the dimension labels' occlusion and the viewport pick read
            // this one index, built on the import worker; `None` until it lands.
            let occlusion_bvh = self.scene_bvh.as_deref();
            // Supplied for the whole Opt workspace, not only once a level exists:
            // the split lays out two halves either way, drawing the source into both
            // until a run lands, so its labels need that half's camera regardless.
            let opt_overlay = (self.ui.mode == WorkspaceMode::Opt).then(|| OptOverlayView {
                camera: renderer.opt_camera,
                level: opt_level_model.map(|(model, revision)| OptOverlayLevel {
                    model,
                    bvh: self
                        .opt
                        .as_ref()
                        .and_then(|opt| opt.level_bvhs.get(self.ui.opt.active_lod))
                        .map(Arc::as_ref),
                    revision,
                }),
            });
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
            let announced_tool = &mut self.announced_tool;
            let prev_active_material = self.ui.debug.active_material;
            let prev_buffer_view = self.ui.debug.buffer_view;
            let _z = prof::zone!("egui Run");
            // `run_ui` hands the closure the frame's root `Ui` — egui shows panels
            // into a `Ui` rather than onto the `Context` — and the chrome carves its
            // bands out of it. Floating layers still address `ui.ctx()`.
            let full_output = egui_ctx.run_ui(raw_input, |ui| {
                ui_output = draw_overlay(
                    ui,
                    &mut self.ui,
                    camera,
                    &scene_model,
                    occlusion_bvh,
                    opt_overlay,
                );
                if self.ui.debug.material_mode != prev_material_mode {
                    notifications.mode(review_ui::material_mode_name(self.ui.debug.material_mode));
                }
                // Both ways of switching land here: the toolbar button was
                // clicked during this pass, and a `Q` press between frames
                // changed the field before it. Comparing against what was last
                // *announced* catches either.
                if self.ui.tool != *announced_tool {
                    *announced_tool = self.ui.tool;
                    notifications.mode(review_ui::viewport_tool_name(self.ui.tool));
                }
                // Announce the buffer being viewed when the user switches *into* the
                // Buffers view or cycles to the next buffer (mirrors the material-
                // mode toast above).
                let buffers_now = self.ui.debug.active_material == ActiveMaterial::Buffers;
                let entered_buffers =
                    buffers_now && prev_active_material != ActiveMaterial::Buffers;
                let cycled_buffer = buffers_now && self.ui.debug.buffer_view != prev_buffer_view;
                if entered_buffers || cycled_buffer {
                    notifications.mode(keys::app_notifications::buffer_mode(
                        review_ui::buffer_view_name(self.ui.debug.buffer_view),
                    ));
                }
                // The notice column paints on the egui Foreground layer, above
                // the chrome. It takes the side panels' widths — measured by
                // `draw_overlay` a few lines up — so it centres on the free
                // viewport rather than on the window.
                notifications.show(ui.ctx(), self.ui.chrome_insets);
            });

            egui_state.handle_platform_output(window, full_output.platform_output.clone());
            (full_output, ui_output)
        };
        Some((full_output, ui_output))
    }

    /// Decide when the next frame should be drawn (invariant 6): paced to the
    /// display while something is moving, at egui's own deadline while it waits
    /// on a timer, and not at all while everything is idle.
    fn schedule_next_frame(&mut self, full_output: &egui::FullOutput) {
        // Decide when the next frame should be drawn. Continuous motion — a live
        // camera transition, or egui asking to "repaint immediately" (zero delay)
        // — is paced to the monitor's refresh interval so the viewer never renders
        // faster than the display can show it. (We don't leave that to the
        // swapchain: a vsynced present only blocks once its queue of frames is full
        // — inside `Present` on D3D11, at `nextDrawable` on Metal — so on its own it
        // would let a burst of redraws run frames ahead of the display.) A finite
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
        // A playing clip keeps pacing frames until it pauses or stops.
        let anim_playing = self.animation_playing();
        // A held flycam key is a live interaction (invariant 6): keep pacing
        // frames so movement is continuous, and stop the moment it is let go.
        let flying = self.flycam_active();
        // Pump startup warmup frames until the deferred GPU-resource
        // build drains, so the scene pipelines + GTAO pass compile behind
        // the already-shown grid. Paced like the other continuous-redraw sources.
        let warming_up = self.redraw.warmup_frames > 0;
        self.redraw.warmup_frames = self.redraw.warmup_frames.saturating_sub(1);
        self.redraw.repaint_at = if repaint_delay.is_zero()
            || camera_animating
            || anim_playing
            || flying
            || warming_up
        {
            let frame_start = self.redraw.last_render_instant.unwrap_or_else(Instant::now);
            Some(frame_start + self.redraw.refresh_interval)
        } else if repaint_delay == Duration::MAX {
            None
        } else {
            Instant::now().checked_add(repaint_delay)
        };
    }

    /// Record and present the frame: resolve this frame's scene inputs from the
    /// UI state, have the renderer draw the active workspace, paint the chrome
    /// over it, present, and report any GPU fault. Returns when the CPU side of
    /// the frame ended (for the gate), or `None` when there was nothing to draw
    /// into.
    fn paint(
        &mut self,
        full_output: egui::FullOutput,
        egui_ctx: &egui::Context,
        gate_active: bool,
        vsync: bool,
    ) -> Option<Option<Instant>> {
        let (backbuffer_width, backbuffer_height) =
            self.gpu.as_ref().map_or((0, 0), review_render::Gpu::size);
        // The synced scene inputs the renderer draws this frame (read before the
        // disjoint renderer/gpu borrows below). `debug` carries show_grid / shading
        // / overlay flags (synced during the egui pass); `projection` the
        // perspective/orthographic toggle; the model + revision drive the mesh.
        let mut debug = self.ui.debug;
        // Aud's focus look: while a finding is highlighted the model turns neutral
        // clay, so the severity colours are the only colour on it.
        let audit_focused = self.ui.mode == WorkspaceMode::Aud
            && !self.audit.highlight.is_empty()
            && self.ui.aud.view == review_audit::DiagnosticView::Issues;
        if audit_focused {
            debug.material_mode = review_render::MaterialMode::Standard;
            debug.active_material = ActiveMaterial::Source;
        }
        let projection: CameraProjection = self.ui.projection_mode.into();
        let environment = self.ui.environment;
        let gtao = self.ui.gtao;
        let tonemap = self.ui.tonemap;
        let background = self.ui.viewport_background;
        let anti_aliasing = self.ui.anti_aliasing;
        let selection = self.ui.selection_view();
        let hidden_meshes = self.ui.hidden_mesh_nodes();
        let selected_bones = self.ui.selected_bone_nodes();
        let selected_nodes = self.ui.selected_node_set();
        // The pick's preview, split by what it targets: the skeleton overlay
        // decides whether a click would take a bone or a mesh part, so only one
        // of the two is ever `Some`.
        let (hover, hover_bone) = match self.ui.hover {
            Some(review_ui::HoverTarget::Node(node)) => (Some(node as u32), None),
            Some(review_ui::HoverTarget::Bone(bone)) => (None, Some(bone as u32)),
            None => (None, None),
        };
        let workspace = self.ui.mode;
        let uv_channel = self.ui.uv_view_channel;
        // What the UV workspace lays out: the Outliner's mesh/group selection, or
        // every visible node when there is none. Built only for that workspace.
        let uv_nodes = if workspace == WorkspaceMode::Uv {
            self.ui.uv_scope_nodes()
        } else {
            Vec::new()
        };
        // The texture picked in the Textures tab, which the UV workspace draws
        // behind the layout until another is picked or `Esc` clears it. Cloned handles (a path and an `Arc`), so nothing of
        // `self.ui` stays borrowed across the paint.
        let uv_texture = (workspace == WorkspaceMode::Uv)
            .then_some(self.ui.uv_texture.as_ref())
            .flatten()
            .and_then(|path| {
                self.ui
                    .texture_pool
                    .iter()
                    .find(|entry| &entry.path == path)
            })
            .map(|entry| (entry.path.clone(), Arc::clone(&entry.image)));
        let uv_background = tex_background(self.ui.uv_background, full_output.pixels_per_point);
        let uv_shading = self.ui.uv_shading_mode;
        // The Tex viewport's draw inputs (background + placed image), resolved from
        // the live UI state only in Texture mode. Built before the renderer borrow
        // below; `pixels_per_point` converts the canvas/placement from egui points to
        // physical pixels for the renderer's image draw.
        let texture_draw = (workspace == WorkspaceMode::Texture)
            .then(|| self.build_texture_draw(full_output.pixels_per_point));

        // The Opt workspace hands the renderer both meshes and lets it lay them
        // out; every other workspace draws the source alone. The processed mesh
        // goes through the same `SceneFrame` settings as the source, so shading,
        // wireframe, normals, selection, AA, AO and tone mapping all apply to it
        // unchanged.
        let opt_revision = (workspace == WorkspaceMode::Opt)
            .then(|| self.opt_processed_revision())
            .flatten();
        // The `Arc` is cloned so the borrow below outlives the `self` reborrow the
        // renderer takes, exactly as the source model's is.
        let opt_result = opt_revision
            .and(self.opt.as_ref())
            .and_then(|opt| opt.processed.clone());
        // A run in flight publishes level 0 as it improves it, and that is what
        // the viewport shows until the run lands — so a rebuild is watched
        // settling rather than waited out. Held as its own `Arc` for the same
        // reason the result is: the borrow below outlives the `self` reborrow.
        let opt_preview = opt_revision
            .and(self.opt.as_ref())
            .and_then(|opt| opt.level_mesh(self.ui.opt.active_lod));
        // The chrome-free area the split lays its two views out in, in physical
        // pixels. The UI measures it in points during the pass above; without it
        // (the first frame, before the chrome has been laid out) the whole
        // backbuffer stands in.
        let gpu_size = (backbuffer_width, backbuffer_height);
        let opt_viewport = self
            .ui
            .scene_viewport
            .map(|rect| scene_viewport_px(rect, full_output.pixels_per_point, gpu_size))
            .unwrap_or_else(|| SceneViewport::full(gpu_size));
        let active_lod = self.ui.opt.active_lod;
        let opt_layout = self.ui.opt.layout;
        let opt_ghost = self.ui.opt.ghost_style;
        // The chrome owns the ghost's colour (invariant 8) and shows the same one
        // in the overlay legend; the renderer decides its alpha.
        let ghost_tint = {
            let [r, g, b, _] = theme::color::GHOST_XRAY.to_normalized_gamma_f32();
            [r, g, b]
        };
        let opt_swap = self.ui.opt.side == ComparisonSide::Source;

        let source_model = self.scene_model.clone();
        let model: &ModelData = &source_model;
        let model_revision = self.scene_revision;
        // The evaluated pose, for the 3D workspace only: Opt compares static
        // geometry and deliberately shows the bind pose. Borrowed as a disjoint
        // field so it can outlive the renderer borrow below.
        let pose =
            (workspace.shows_pose() && self.animation.active).then_some(&self.animation.deform);
        let pose_revision = self.animation.pose_revision;
        let scene_bounds = self.ui.bounds;

        // The Aud focus's offenders, borrowed from the highlight `app` resolved.
        let highlight = &self.audit.highlight;
        let audit_overlay = (workspace == WorkspaceMode::Aud && !highlight.is_empty()).then(|| {
            let color = |token: egui::Color32| {
                let [r, g, b, _] = token.to_normalized_gamma_f32();
                [r, g, b, theme::color::AUDIT_FILL_OPACITY]
            };
            review_render::AuditOverlay {
                revision: highlight.revision,
                fills: [
                    &highlight.fills[0],
                    &highlight.fills[1],
                    &highlight.fills[2],
                ],
                edges: [
                    &highlight.edges[0],
                    &highlight.edges[1],
                    &highlight.edges[2],
                ],
                corner_dots: [
                    &highlight.corner_dots[0],
                    &highlight.corner_dots[1],
                    &highlight.corner_dots[2],
                ],
                point_dots: [
                    &highlight.point_dots[0],
                    &highlight.point_dots[1],
                    &highlight.point_dots[2],
                ],
                colors: [
                    color(theme::color::SEVERITY_INFO),
                    color(theme::color::SEVERITY_WARNING),
                    color(theme::color::SEVERITY_ERROR),
                ],
                hidden_alpha: theme::color::AUDIT_HIDDEN_OPACITY,
            }
        });
        let renderer = self.renderer.as_mut()?;
        let gpu = self.gpu.as_mut()?;
        let egui_renderer = self.egui_renderer.as_mut()?;

        // GPU faults hit while painting. Collected rather than reported inline:
        // `gpu` and `egui_renderer` are borrowed out of `self` for the whole block,
        // so the reporter — which needs all of `self` — runs once it closes. An
        // empty `Vec` allocates nothing, so a clean frame pays for none of this.
        let mut faults: Vec<(GpuFault, String)> = Vec::new();
        let before_present: Option<Instant>;
        {
            let _z = prof::zone!("Paint + Present");
            // Acquire this frame's backbuffer. `None` means there is nothing to draw
            // into — a minimized window, or a drawable the device refused after a
            // reset — so the frame is skipped rather than drawn into nothing.
            let mut frame = gpu.begin_frame()?;

            // The renderer records its offscreen passes and leaves the composite for
            // the swapchain pass below (§3.2).
            let render_result = match workspace {
                WorkspaceMode::Uv => renderer.render_uv_scene(
                    &mut frame,
                    &UvFrame {
                        model,
                        model_revision,
                        channel: uv_channel,
                        shading_mode: uv_shading,
                        anti_aliasing,
                        background: uv_background,
                        selected_nodes: &uv_nodes,
                        hidden_meshes: &hidden_meshes,
                        texture: uv_texture.as_ref().map(|(path, image)| UvTexture {
                            path: path.as_path(),
                            image,
                        }),
                        pixels_per_point: full_output.pixels_per_point,
                    },
                ),
                WorkspaceMode::Texture => {
                    let (image, background) = texture_draw.unwrap_or((None, TexBackground::Black));
                    renderer.render_texture(&mut frame, image, background)
                }
                WorkspaceMode::ThreeD | WorkspaceMode::Opt | WorkspaceMode::Aud => {
                    let scene_frame = SceneFrame {
                        model,
                        model_revision,
                        debug,
                        projection,
                        environment,
                        gtao,
                        tonemap,
                        anti_aliasing,
                        selection,
                        hidden_meshes: &hidden_meshes,
                        selected_bones: &selected_bones,
                        selected_nodes: &selected_nodes,
                        hover,
                        hover_bone,
                        background,
                        pose,
                        pose_revision,
                        scene_bounds,
                        pixels_per_point: full_output.pixels_per_point,
                        audit: audit_overlay,
                    };
                    if workspace == WorkspaceMode::Opt {
                        // Read the cameras out before the call: the arguments are
                        // evaluated after the `&mut renderer` receiver is borrowed.
                        let source_camera = renderer.camera;
                        let processed_camera = renderer.opt_camera;
                        let processed = opt_preview
                            .as_deref()
                            .or_else(|| {
                                opt_result
                                    .as_ref()
                                    .and_then(|result| result.lod(active_lod))
                                    .map(|lod| &lod.model)
                            })
                            .zip(opt_revision)
                            .map(|(model, revision)| ProcessedModelRef { model, revision });
                        renderer.render_opt_scene(
                            &mut frame,
                            &OptSceneFrame {
                                base: scene_frame,
                                processed,
                                viewport: opt_viewport,
                                view: match opt_layout {
                                    OptLayout::Split => OptView::Split,
                                    OptLayout::Overlay => OptView::Overlay {
                                        ghost: opt_ghost.into(),
                                        swap: opt_swap,
                                        tint: ghost_tint,
                                    },
                                },
                                source_camera,
                                processed_camera,
                            },
                        )
                    } else {
                        renderer.render_scene(&mut frame, &scene_frame)
                    }
                }
            };
            if let Err(err) = render_result {
                faults.push((GpuFault::Scene, err.to_string()));
            }

            // Ambient occlusion averages frames whenever the view holds still, and
            // asks for one more until it has converged (~0.4 s at 60 Hz, then it
            // stops on its own). This is a continuous-redraw source like a camera
            // transition, but it cannot join the disjunction above: that runs before
            // the render, so it would read the state from *before* this frame reset
            // the average — and a one-off redraw, like the frame after a slider tick,
            // would never schedule the follow-up that converges it. Requesting here
            // routes it through the same coalescing path input events use, so it is
            // paced to the refresh rate exactly as `camera_animating` is.
            if renderer.is_ao_converging() {
                self.redraw.requested = true;
            }

            // Tessellate the chrome and upload its geometry + texture deltas. Outside
            // any pass on purpose: both are resource updates sokol forbids inside one.
            if let Err(err) = egui_renderer.prepare(
                egui_ctx,
                full_output.shapes,
                full_output.textures_delta,
                full_output.pixels_per_point,
                frame.size(),
            ) {
                faults.push((GpuFault::Ui, err.to_string()));
            }

            // The one swapchain pass: it owns the clear, replays whatever the
            // renderer composited, and then the chrome paints over that.
            frame.begin_swapchain_pass();
            egui_renderer.paint(&frame, full_output.pixels_per_point);
            // The CPU half of the gate's frame figure ends here: everything after
            // this is the present itself.
            before_present = gate_active.then(Instant::now);
            if let review_render::PresentStatus::DeviceLost { reason } = frame.finish(vsync) {
                faults.push((GpuFault::DeviceLost, format!("{reason:#x}")));
            }
            // After the frame: a texture egui freed may still have been drawn from it.
            egui_renderer.free_textures();
        }
        for (fault, detail) in faults {
            self.report_gpu_fault(fault, detail);
        }
        Some(before_present)
    }

    /// Report a GPU fault: on screen and in the log the first time one happens
    /// this session, down the prof channel every time after. A wedged device
    /// fails again on every frame, so the `gpu_fault_notified` latch reports the
    /// *first* fault and lets the rest pass — which is the one that says what
    /// went wrong, the later ones being its consequences. The notice is keyed as
    /// well, so even if that latch is ever relaxed the viewport gets one card
    /// rather than a column of them. The first fault reaches the log through its
    /// card, which logs itself; a line a frame after it would fill the day's log
    /// file with the same sentence.
    pub(crate) fn report_gpu_fault(&mut self, fault: GpuFault, detail: String) {
        let message = match fault {
            GpuFault::Scene => keys::app_notifications::gpu_scene_failed(detail),
            GpuFault::Ui => keys::app_notifications::gpu_ui_failed(detail),
            GpuFault::DeviceLost => keys::app_notifications::gpu_device_lost(detail),
        };
        if self.gpu_fault_notified {
            prof::msg(&message);
        } else {
            self.gpu_fault_notified = true;
            self.notifications.error_keyed(GPU_FAULT_NOTICE, message);
        }
    }

    /// Resolve the Tex viewport's draw inputs from the live UI state: the
    /// background fill, plus — when a texture is selected and the canvas has been laid
    /// out — the image placed by the canvas center + pan/zoom (egui points → physical
    /// pixels via `ppp`). The UI emits only plain values (invariant 2); `app` owns the
    /// pool and resolves placement here.
    fn build_texture_draw(&self, ppp: f32) -> (Option<TexImage>, TexBackground) {
        let background = tex_background(self.ui.texture_view.background, ppp);

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

/// The renderer's form of a Tex / UV background fill: the checker's cell (the UI
/// theme's, in points) scaled to physical pixels by `ppp`.
fn tex_background(background: TextureBackground, ppp: f32) -> TexBackground {
    match background {
        TextureBackground::Black => TexBackground::Black,
        TextureBackground::White => TexBackground::White,
        TextureBackground::Grey => TexBackground::Grey,
        TextureBackground::Checker => TexBackground::Checker {
            cell_px: theme::size::TEXTURE_CHECKER_CELL * ppp,
        },
    }
}

/// Convert the UI's measured scene area (egui points) into the renderer's
/// physical-pixel [`SceneViewport`], clamped to the backbuffer.
///
/// The clamp is not defensive tidiness: the rect is measured during the egui
/// pass, so a resize landing between that and the draw would otherwise hand the
/// rasterizer a viewport reaching past the backbuffer.
fn scene_viewport_px(rect: egui::Rect, ppp: f32, size: (u32, u32)) -> SceneViewport {
    let (width, height) = size;
    // A degenerate backbuffer — a minimized window — has no rect to clamp into, and
    // the clamps below would have no room to place even a single pixel in. `render`
    // skips those frames before reaching here; this keeps the conversion total
    // rather than a panic waiting for the next caller.
    if width == 0 || height == 0 {
        return SceneViewport {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
    }
    let left = (rect.left() * ppp).round().max(0.0) as u32;
    let top = (rect.top() * ppp).round().max(0.0) as u32;
    let right = (rect.right() * ppp).round().max(0.0) as u32;
    let bottom = (rect.bottom() * ppp).round().max(0.0) as u32;
    let x = left.min(width.saturating_sub(1));
    let y = top.min(height.saturating_sub(1));
    SceneViewport {
        x,
        y,
        width: right.clamp(x + 1, width) - x,
        height: bottom.clamp(y + 1, height) - y,
    }
}

impl App {
    fn update_camera_animation(&mut self) {
        let now = Instant::now();
        let delta_seconds = self
            .redraw
            .last_render_instant
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32());
        self.redraw.last_render_instant = Some(now);

        // The viewer redraws on demand, so FPS is only meaningful across
        // consecutive frames (camera animation / interaction). Ignore the long
        // gaps after an idle period and exponentially smooth the live rate.
        if (0.0..0.25).contains(&delta_seconds) && delta_seconds > 0.0 {
            let instant_fps = 1.0 / delta_seconds;
            self.ui.fps = if self.ui.fps > 0.0 {
                self.ui.fps * 0.9 + instant_fps * 0.1
            } else {
                instant_fps
            };
        }

        // Tracy plots (no-op unless `--tracy`): the live smoothed frame rate plus
        // the measured model stats, so they read alongside the timeline.
        prof::plot!("FPS", self.ui.fps);
        prof::plot!("Triangles", self.ui.stats.triangle_count as f64);
        prof::plot!("Draw Calls", self.ui.stats.draw_count as f64);

        // Advance any live camera transition. The follow-up redraw is scheduled
        // by the paced `repaint_at` logic in `render` (which checks
        // `is_camera_animating`), so we don't request one directly here — doing so
        // would bypass the refresh-rate cap.
        //
        // Cap the step: the viewer redraws on demand, so after an idle period
        // `last_render_instant` is stale and the first frame's delta is the whole
        // idle gap. Advancing a transition by that would fast-forward it to the
        // end in one frame (skipping the animation entirely) — most visible on the
        // short 0.1 s WASD orbits, where almost any delta exceeds the duration.
        // One ~30 Hz frame is plenty to keep motion smooth.
        const MAX_ANIMATION_STEP: f32 = 1.0 / 30.0;
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.update_camera_animation(delta_seconds.min(MAX_ANIMATION_STEP));
        }
    }
}
#[cfg(test)]
mod tests {
    use super::scene_viewport_px;

    /// The split's divider has to land in the middle of what the user can see,
    /// so an open side panel must move the rect, not just narrow it.
    #[test]
    fn the_scene_rect_converts_to_pixels_at_scale() {
        let rect = egui::Rect::from_min_max(egui::pos2(100.0, 20.0), egui::pos2(500.0, 300.0));
        let view = scene_viewport_px(rect, 2.0, (1200, 800));
        assert_eq!((view.x, view.y), (200, 40));
        assert_eq!((view.width, view.height), (800, 560));
    }

    /// A minimized window's backbuffer is zero-sized, which is not a rect the
    /// clamps can place a pixel in: the conversion answers with an empty viewport
    /// rather than panicking. `render` skips those frames before they get here.
    #[test]
    fn a_zero_sized_backbuffer_yields_an_empty_viewport() {
        let rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1280.0, 720.0));
        let view = scene_viewport_px(rect, 1.0, (0, 0));
        assert_eq!((view.x, view.y), (0, 0));
        assert_eq!((view.width, view.height), (0, 0));
    }

    /// A rect measured before a shrinking resize must not reach past the
    /// backbuffer it is about to be rasterized into.
    #[test]
    fn an_oversized_rect_is_clamped_to_the_backbuffer() {
        let rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(4000.0, 3000.0));
        let view = scene_viewport_px(rect, 1.0, (1280, 720));
        assert_eq!((view.x, view.y), (0, 0));
        assert_eq!((view.width, view.height), (1280, 720));
    }
}

/// Which part of a frame failed on the GPU: what the error notice leads with.
/// The detail that follows it is the renderer's own diagnostic.
#[derive(Clone, Copy)]
pub(crate) enum GpuFault {
    /// The scene's offscreen passes or its composite.
    Scene,
    /// The chrome's tessellation upload.
    Ui,
    /// The device went away; the detail is its reason code.
    DeviceLost,
}
