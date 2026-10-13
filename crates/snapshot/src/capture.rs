//! Drawing a subject through the viewer's renderer on a windowless GPU.

use anyhow::{Context, Result, anyhow};
use glam::{Vec2, Vec3};
use image::RgbaImage;
use review_model::Bounds;
use review_render::{
    ActiveMaterial, AntiAliasing, BufferView, CameraProjection, EnvironmentSettings, Gpu,
    GtaoSettings, MaterialMode, OrbitCamera, Renderer, RendererConfig, SceneDebugOptions,
    SceneFrame, SelectionView, ShadingMode, TonemapSettings, ViewportBackground,
};

use crate::cli::{Look, View};
use crate::inputs::Subject;

/// More than ambient occlusion ever takes to converge (24 frames); a frame cap,
/// not a target.
const MAX_FRAMES_PER_VIEW: usize = 64;

pub struct Capture {
    gpu: Gpu,
    renderer: Renderer,
    size: (u32, u32),
    background: ViewportBackground,
    grid: bool,
    /// The renderer caches a mesh's GPU buffers by revision, so every subject
    /// needs its own — the same rule `app` follows for every mesh it hands over.
    next_revision: u64,
    /// The model last drawn (by its `Arc` address) and the revision it was given.
    current: Option<(usize, u64)>,
}

impl Capture {
    pub fn new(size: (u32, u32), background: ViewportBackground, grid: bool) -> Result<Self> {
        let gpu = Gpu::headless(size.0, size.1).context("bringing the GPU up without a window")?;
        let mut renderer = Renderer::new(RendererConfig::default());
        renderer.set_camera_aspect_ratio(size.0 as f32 / size.1 as f32);
        Ok(Self {
            gpu,
            renderer,
            size,
            background,
            grid,
            next_revision: 1,
            current: None,
        })
    }

    /// The camera for `view`, framed on `bounds`.
    pub fn camera(&self, view: &View, bounds: Bounds) -> OrbitCamera {
        let mut camera = OrbitCamera {
            yaw: view.yaw_degrees.to_radians(),
            pitch: view.pitch_degrees.to_radians().clamp(-1.5, 1.5),
            aspect_ratio: self.size.0 as f32 / self.size.1 as f32,
            ..OrbitCamera::default()
        }
        .framed_to_bounds(bounds, Vec2::ONE);
        if let Some(focus) = view.focus {
            camera.target = bounds.min + Vec3::from_array(focus) * bounds.size();
        }
        camera.distance /= view.zoom;
        camera
    }

    /// Render `subject` from `camera` and read the frame back.
    pub fn render(
        &mut self,
        subject: &Subject,
        camera: OrbitCamera,
        look: Look,
    ) -> Result<RgbaImage> {
        let model_key = std::sync::Arc::as_ptr(&subject.model) as usize;
        let revision = match self.current {
            Some((current, revision)) if current == model_key => revision,
            _ => {
                let revision = self.next_revision;
                self.next_revision += 1;
                self.renderer.set_model_materials(&subject.model.materials);
                self.current = Some((model_key, revision));
                revision
            }
        };
        self.renderer.camera = camera;

        let mut debug = SceneDebugOptions {
            show_grid: self.grid,
            ..SceneDebugOptions::default()
        };
        match look {
            Look::Shaded => {}
            Look::Matte => debug.material_mode = MaterialMode::Standard,
            Look::Normals => {
                debug.active_material = ActiveMaterial::Buffers;
                debug.buffer_view = BufferView::GeometricNormal;
            }
            Look::Wire => debug.shading_mode = ShadingMode::Wireframe,
            Look::ShadedWire => {
                debug.material_mode = MaterialMode::Standard;
                debug.wireframe_overlay = true;
            }
            Look::Checker => {
                debug.active_material = ActiveMaterial::UvChecker;
                debug.uv_checker_tiling = 16;
            }
        }
        let scene = SceneFrame {
            model: &subject.model,
            model_revision: revision,
            debug,
            projection: CameraProjection::Perspective,
            environment: EnvironmentSettings::default(),
            gtao: GtaoSettings::default(),
            tonemap: TonemapSettings::default(),
            anti_aliasing: AntiAliasing::default(),
            selection: SelectionView::default(),
            hidden_meshes: &subject.hidden,
            selected_bones: &[],
            selected_nodes: &[],
            hover: None,
            hover_bone: None,
            // The normals look is read back by `compare`, which tells surface from
            // background by the decoded vector's length: black decodes to
            // `(-1, -1, -1)` and can never pass for a normal, a grey can.
            background: if look == Look::Normals {
                ViewportBackground::Black
            } else {
                self.background
            },
            pose: None,
            pose_revision: 0,
            scene_bounds: subject.bounds(),
            pixels_per_point: 1.0,
            audit: None,
        };

        // Ambient occlusion converges over a run of still frames, exactly as it
        // does on screen; keep drawing until it says it is done.
        for _ in 0..MAX_FRAMES_PER_VIEW {
            let mut frame = self
                .gpu
                .begin_frame()
                .ok_or_else(|| anyhow!("the offscreen backbuffer refused a frame"))?;
            self.renderer.render_scene(&mut frame, &scene)?;
            frame.begin_swapchain_pass();
            let _ = frame.finish(false);
            if !self.renderer.is_ao_converging() {
                break;
            }
        }
        let pixels = self.gpu.read_back()?;
        RgbaImage::from_raw(self.size.0, self.size.1, pixels)
            .ok_or_else(|| anyhow!("the read-back frame is the wrong size"))
    }
}
