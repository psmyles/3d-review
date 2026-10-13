//! Drawing a subject through the viewer's renderer on a windowless GPU.

use anyhow::{Context, Result, anyhow};
use glam::{Vec2, Vec3};
use image::RgbaImage;
use review_model::Bounds;
use review_render::{
    ActiveMaterial, AntiAliasing, BufferView, CameraProjection, EnvironmentSettings, FramingArea,
    Gpu, GtaoSettings, MaterialMode, OrbitCamera, Renderer, RendererConfig, SceneDebugOptions,
    SceneFrame, SelectionView, ShadingMode, TonemapSettings, TrajectoryOverlay, TrajectoryStyle,
    ViewportBackground,
};

use crate::animation::Posed;
use crate::cli::{Look, View};
use crate::inputs::Subject;

/// More than ambient occlusion ever takes to converge (24 frames); a frame cap,
/// not a target.
const MAX_FRAMES_PER_VIEW: usize = 64;

pub struct Capture {
    gpu: Gpu,
    renderer: Renderer,
    size: (u32, u32),
    /// Pixels covered on the left, top, right and bottom, as the viewer's chrome
    /// covers its scene: framed around, then dimmed.
    chrome: [u32; 4],
    background: ViewportBackground,
    grid: bool,
    skeleton: bool,
    curve_style: TrajectoryStyle,
    /// The renderer caches a mesh's GPU buffers by revision, so every subject
    /// needs its own — the same rule `app` follows for every mesh it hands over.
    next_revision: u64,
    /// The model last drawn (by its `Arc` address) and the revision it was given.
    current: Option<(usize, u64)>,
}

impl Capture {
    pub fn new(
        size: (u32, u32),
        chrome: [u32; 4],
        background: ViewportBackground,
        grid: bool,
        skeleton: bool,
        curve_style: TrajectoryStyle,
    ) -> Result<Self> {
        let gpu = Gpu::headless(size.0, size.1).context("bringing the GPU up without a window")?;
        let mut renderer = Renderer::new(RendererConfig::default());
        renderer.set_camera_aspect_ratio(size.0 as f32 / size.1 as f32);
        Ok(Self {
            gpu,
            renderer,
            size,
            chrome,
            background,
            grid,
            skeleton,
            curve_style,
            next_revision: 1,
            current: None,
        })
    }

    /// The camera for `view`, framed on `bounds` into what the chrome leaves.
    pub fn camera(&self, view: &View, bounds: Bounds) -> OrbitCamera {
        let (width, height) = (self.size.0 as f32, self.size.1 as f32);
        let [left, top, right, bottom] = self.chrome.map(|inset| inset as f32);
        let area = FramingArea {
            aspect_ratio: width / height,
            visible_min: Vec2::new(left / width, top / height),
            visible_max: Vec2::new(1.0 - right / width, 1.0 - bottom / height),
        };
        let mut camera = OrbitCamera {
            yaw: view.yaw_degrees.to_radians(),
            pitch: view.pitch_degrees.to_radians().clamp(-1.5, 1.5),
            aspect_ratio: self.size.0 as f32 / self.size.1 as f32,
            ..OrbitCamera::default()
        }
        .framed_to_bounds(bounds, area, CameraProjection::Perspective);
        if let Some(focus) = view.focus {
            camera.target = bounds.min + Vec3::from_array(focus) * bounds.size();
        }
        camera.distance /= view.zoom;
        camera
    }

    /// Render `subject` from `camera` - posed, with its overlays, when `posed` is
    /// given - and read the frame back.
    pub fn render(
        &mut self,
        subject: &Subject,
        posed: Option<&Posed>,
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
            show_skeleton: self.skeleton,
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
            selected_bones: posed.map_or(&[], |posed| &posed.selected_bones),
            selected_nodes: posed.map_or(&[], |posed| &posed.selected_nodes),
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
            // One pose per subject, and the palette lives with the subject's own
            // mesh, so a fixed revision is enough to upload it once.
            pose: posed.map(|posed| &posed.deform),
            pose_revision: 1,
            scene_bounds: posed
                .and_then(|posed| posed.envelope)
                .or_else(|| subject.bounds()),
            pixels_per_point: 1.0,
            audit: None,
            // Keyed on the subject's revision: each has its own paths.
            trajectories: posed.filter(|posed| !posed.paths.is_empty()).map(|posed| {
                TrajectoryOverlay {
                    revision,
                    paths: &posed.paths,
                    shown: &posed.shown,
                    style: self.curve_style,
                }
            }),
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
        let mut image = RgbaImage::from_raw(self.size.0, self.size.1, pixels)
            .ok_or_else(|| anyhow!("the read-back frame is the wrong size"))?;
        // Not the normals look: `compare` reads that one back pixel by pixel.
        if look != Look::Normals {
            self.dim_chrome(&mut image);
        }
        Ok(image)
    }

    /// Darken what the chrome covers, leaving enough of it to see what a
    /// framing put underneath.
    fn dim_chrome(&self, image: &mut RgbaImage) {
        let [left, top, right, bottom] = self.chrome;
        let (width, height) = self.size;
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            if x < left || y < top || x >= width - right || y >= height - bottom {
                for channel in &mut pixel.0[..3] {
                    *channel = (u16::from(*channel) * 3 / 10 + 20) as u8;
                }
            }
        }
    }
}
