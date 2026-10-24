//! The windowless GPU and renderer a preview draws through, and the image it
//! reads back.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use glam::Vec2;
use review_render::{
    AntiAliasing, CameraProjection, EnvironmentSettings, FramingArea, Gpu, GpuError, GpuResult,
    GtaoSettings, MsaaSamples, OrbitCamera, Renderer, RendererConfig, SceneDebugOptions,
    SceneFrame, SelectionView, TonemapSettings, ViewportBackground,
};

use crate::PreviewScene;

/// How much of the view a framed model fills. A little under all of it, so a
/// model never touches the edge of a thumbnail tile or the pane.
const FILL: f32 = 0.9;

/// One frame, read back: tight RGBA8 rows, top row first, in *straight* alpha —
/// the color is the surface's own and the alpha its coverage, over nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    /// The same pixels as `BGRA`, the byte order a Windows bitmap stores.
    pub fn to_bgra(&self) -> Vec<u8> {
        let mut bgra = self.rgba.clone();
        for texel in bgra.as_chunks_mut::<4>().0 {
            texel.swap(0, 2);
        }
        bgra
    }

    /// This image laid over an opaque `background` (sRGB bytes) — what a pane
    /// that cannot show alpha draws. The blend is in display space, exactly
    /// as the viewer's own composite lays the scene over its background.
    pub fn over(&self, background: [u8; 3]) -> Image {
        let mut rgba = self.rgba.clone();
        for texel in rgba.as_chunks_mut::<4>().0 {
            let alpha = u32::from(texel[3]);
            for (channel, &back) in texel[..3].iter_mut().zip(&background) {
                let blended = u32::from(*channel) * alpha + u32::from(back) * (255 - alpha);
                *channel = ((blended + 127) / 255) as u8;
            }
            texel[3] = 255;
        }
        Image {
            width: self.width,
            height: self.height,
            rgba,
        }
    }

    /// Half the size in each direction, each texel the alpha-weighted mean of
    /// four — so a supersampled render keeps its edges and its transparent
    /// texels lend nothing to the color beside them. An odd last row or column
    /// is dropped.
    pub(crate) fn halved(&self) -> Image {
        let width = (self.width / 2).max(1);
        let height = (self.height / 2).max(1);
        let source = |x: u32, y: u32| {
            let x = x.min(self.width - 1) as usize;
            let y = y.min(self.height - 1) as usize;
            let at = (y * self.width as usize + x) * 4;
            &self.rgba[at..at + 4]
        };
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            for x in 0..width {
                let taps = [
                    source(x * 2, y * 2),
                    source(x * 2 + 1, y * 2),
                    source(x * 2, y * 2 + 1),
                    source(x * 2 + 1, y * 2 + 1),
                ];
                let alpha: u32 = taps.iter().map(|tap| u32::from(tap[3])).sum();
                for channel in 0..3 {
                    let weighted: u32 = taps
                        .iter()
                        .map(|tap| u32::from(tap[channel]) * u32::from(tap[3]))
                        .sum();
                    // Nothing covers a fully transparent texel, so its color is 0.
                    let mean = (weighted + alpha / 2).checked_div(alpha).unwrap_or(0);
                    rgba.push(mean as u8);
                }
                rgba.push(((alpha + 2) / 4) as u8);
            }
        }
        Image {
            width,
            height,
            rgba,
        }
    }
}

/// Identifies a [`Viewport`] inside a [`Host`]. Never reused within a process,
/// so an id that outlived the host it was opened on finds nothing in the next
/// one rather than someone else's viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ViewportId(u64);

static NEXT_VIEWPORT: AtomicU64 = AtomicU64::new(1);

/// The process's windowless GPU and every viewport drawing through it.
///
/// sokol_gfx is a process singleton, and a viewport's GPU resources are only
/// meaningful on the device they were made on — a sokol id from a device since
/// shut down can alias a resource of the next one. So the viewports live
/// *inside* the host: dropping it drops them first (field order), nothing can
/// keep one past it, and the one `&mut Host` a shell holds under its lock is
/// what serializes every GPU call. A shell hosting several previews opens a
/// viewport for each — each has its own camera — and draws them in turn.
pub struct Host {
    viewports: HashMap<ViewportId, Viewport>,
    /// The viewport thumbnails are drawn through, made on first use.
    thumbnails: Option<Viewport>,
    gpu: Gpu,
}

impl Host {
    /// Bring the GPU up (hardware, or WARP where there is none).
    pub fn new() -> GpuResult<Self> {
        Ok(Self {
            viewports: HashMap::new(),
            thumbnails: None,
            gpu: Gpu::headless(1, 1)?,
        })
    }

    /// The graphics adapter's name, for a log line.
    pub fn adapter_name(&self) -> String {
        self.gpu.adapter_name()
    }

    /// A new viewport, with a camera of its own.
    pub fn open(&mut self) -> ViewportId {
        let id = ViewportId(NEXT_VIEWPORT.fetch_add(1, Ordering::Relaxed));
        self.viewports.insert(id, Viewport::new());
        id
    }

    /// Release a viewport's GPU resources.
    pub fn close(&mut self, id: ViewportId) {
        self.viewports.remove(&id);
    }

    /// Whether no viewport is open — what lets a shell drop the host, and the
    /// GPU memory it holds, once it has been idle a while.
    pub fn is_idle(&self) -> bool {
        self.viewports.is_empty()
    }

    /// The viewport `id`, with the GPU it draws through. `None` for an id this
    /// host never opened (or has closed).
    pub fn viewport(&mut self, id: ViewportId) -> Option<HostedViewport<'_>> {
        let viewport = self.viewports.get_mut(&id)?;
        Some(HostedViewport {
            gpu: &mut self.gpu,
            viewport,
        })
    }

    /// The viewport thumbnails draw through.
    pub(crate) fn thumbnails(&mut self) -> HostedViewport<'_> {
        HostedViewport {
            gpu: &mut self.gpu,
            viewport: self.thumbnails.get_or_insert_with(Viewport::new),
        }
    }
}

/// A viewport and the GPU it draws through, borrowed from its [`Host`].
pub struct HostedViewport<'a> {
    gpu: &'a mut Gpu,
    viewport: &'a mut Viewport,
}

impl HostedViewport<'_> {
    /// The camera half, which needs no GPU: what an [`crate::Interaction`]
    /// moves.
    pub fn camera(&mut self) -> &mut Viewport {
        self.viewport
    }

    /// Draw one frame of `scene` into a `size` view. [`Self::read_back`] returns
    /// it.
    pub fn render(&mut self, scene: &PreviewScene, size: (u32, u32)) -> GpuResult<()> {
        self.viewport.render(self.gpu, scene, size)
    }

    /// The last frame [`Self::render`] drew.
    pub fn read_back(&mut self) -> GpuResult<Image> {
        let (width, height) = self.gpu.size();
        Ok(Image {
            width,
            height,
            rgba: self.gpu.read_back()?,
        })
    }
}

/// One view's renderer: its camera, and the material table of the scene it
/// last drew. Lives inside a [`Host`], which supplies the GPU.
pub struct Viewport {
    renderer: Renderer,
    /// The scene whose materials and textures the renderer's table holds.
    bound: Option<u64>,
}

impl Viewport {
    fn new() -> Self {
        Self {
            renderer: Renderer::new(RendererConfig::default()),
            bound: None,
        }
    }

    /// Point the camera along the viewer's default view — the front-right
    /// three-quarter angle a model opens at — and frame `scene` into a `size`
    /// view.
    pub fn frame(&mut self, scene: &PreviewScene, size: (u32, u32)) {
        self.set_view_size(size);
        let home = OrbitCamera {
            aspect_ratio: aspect(size),
            ..OrbitCamera::default()
        };
        self.renderer.camera = match scene.bounds() {
            Some(bounds) => {
                home.framed_to_bounds(bounds, framing_area(size), CameraProjection::Perspective)
            }
            None => home,
        };
    }

    /// Glide back to a framing of `scene` from wherever the camera is — `F` in
    /// the viewer. [`Self::advance`] moves it.
    pub fn reframe(&mut self, scene: &PreviewScene) {
        if let Some(bounds) = scene.bounds() {
            self.renderer.animate_camera_to_bounds(bounds);
        }
    }

    /// Size the camera to a `size` view without drawing — a pane that was
    /// resized before its first frame frames into its real shape.
    pub fn resize(&mut self, size: (u32, u32)) {
        self.set_view_size(size);
    }

    pub(crate) fn orbit(&mut self, delta: Vec2) {
        self.renderer.orbit_camera(delta);
    }

    pub(crate) fn pan(&mut self, delta: Vec2, view: Vec2) {
        self.renderer.pan_camera(delta, view);
    }

    pub(crate) fn zoom(&mut self, amount: f32) {
        self.renderer.zoom_camera(amount);
    }

    /// Step a camera glide by `seconds`. True while one is running.
    pub fn advance(&mut self, seconds: f32) -> bool {
        self.renderer.update_camera_animation(seconds)
    }

    /// Whether another frame would look different with nothing else changing:
    /// a camera glide is under way, or the ambient occlusion is still averaging
    /// in. Read after [`Self::render`] — that is the frame it describes.
    pub fn is_settling(&self) -> bool {
        self.renderer.is_camera_animating() || self.renderer.is_ao_converging()
    }

    /// Draw one frame of `scene` into a `size` view on `gpu`.
    fn render(&mut self, gpu: &mut Gpu, scene: &PreviewScene, size: (u32, u32)) -> GpuResult<()> {
        self.set_view_size(size);
        gpu.resize(size.0.max(1), size.1.max(1));
        if self.bound != Some(scene.id) {
            self.renderer.set_model_materials(&scene.model.materials);
            self.renderer.bind_imported_textures(&scene.textures);
            self.bound = Some(scene.id);
        }
        let frame = SceneFrame {
            model: &scene.model,
            model_revision: scene.id,
            debug: SceneDebugOptions {
                // A model alone: no floor under it in a tile or a pane.
                show_grid: false,
                ..SceneDebugOptions::default()
            },
            projection: CameraProjection::Perspective,
            environment: EnvironmentSettings::default(),
            gtao: GtaoSettings::default(),
            tonemap: TonemapSettings::default(),
            anti_aliasing: AntiAliasing {
                enabled: true,
                msaa: MsaaSamples::X4,
            },
            selection: SelectionView::default(),
            hidden_meshes: &[],
            selected_bones: &[],
            selected_nodes: &[],
            hover: None,
            hover_bone: None,
            background: ViewportBackground::Black,
            transparent_background: true,
            pose: scene.pose.as_ref(),
            // One pose per scene, so its id identifies it.
            pose_revision: scene.id,
            scene_bounds: scene.bounds(),
            pixels_per_point: 1.0,
            audit: None,
            trajectories: None,
        };
        let mut gpu_frame = gpu
            .begin_frame()
            .ok_or_else(|| GpuError::Backend("the offscreen target refused a frame".into()))?;
        self.renderer.render_scene(&mut gpu_frame, &frame)?;
        gpu_frame.begin_swapchain_pass();
        let _ = gpu_frame.finish(false);
        Ok(())
    }

    /// Size the camera and the framing to a `size` view.
    fn set_view_size(&mut self, size: (u32, u32)) {
        self.renderer.set_camera_aspect_ratio(aspect(size));
        self.renderer
            .set_framing_area(framing_area(size), CameraProjection::Perspective);
    }
}

fn aspect((width, height): (u32, u32)) -> f32 {
    width.max(1) as f32 / height.max(1) as f32
}

fn framing_area(size: (u32, u32)) -> FramingArea {
    FramingArea::whole(aspect(size)).shrunk(FILL)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shell keeps the one host behind a lock that any of its threads can
    /// take, and loads scenes on workers.
    #[test]
    fn a_host_and_a_scene_can_move_between_threads() {
        fn send<T: Send>() {}
        send::<Host>();
        send::<PreviewScene>();
    }

    #[test]
    fn halving_weights_color_by_coverage() {
        // A red texel beside three transparent ones: the result is red, a
        // quarter covered — not a quarter of the way from black to red.
        let image = Image {
            width: 2,
            height: 2,
            rgba: vec![200, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        };
        assert_eq!(image.halved().rgba, vec![200, 0, 0, 64]);
    }

    #[test]
    fn laying_over_a_background_makes_it_opaque() {
        let image = Image {
            width: 2,
            height: 1,
            rgba: vec![255, 255, 255, 255, 255, 255, 255, 0],
        };
        let over = image.over([10, 20, 30]);
        assert_eq!(over.rgba, vec![255, 255, 255, 255, 10, 20, 30, 255]);
    }
}
