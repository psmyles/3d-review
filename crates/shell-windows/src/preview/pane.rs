//! One preview pane's state: the model loading or loaded, its viewport and
//! camera, and the last frame drawn — everything the window shows, with no
//! Win32 in it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use glam::Vec2;
use review_preview::{
    Button, GpuResult, HostedViewport, Interaction, LoadError, Modifiers, PreviewScene, ViewportId,
};

use crate::host::{with_existing_host, with_host};

/// The largest texture side a preview keeps. The pane is at most a few hundred
/// pixels across, so a 2048 map is already sampled from a mip below its top.
const MAX_TEXTURE_SIDE: u32 = 2048;

/// The longest step a camera glide takes per tick, so a stalled message loop
/// does not make the next tick jump to the end.
const MAX_TICK_SECONDS: f32 = 0.1;

/// What the model is laid over (sRGB): the viewer's own default backdrop
/// (`ViewportBackground::Black`), so the pane shows a model as the viewer
/// would, in either Explorer theme.
pub(super) const BACKGROUND: [u8; 3] = [0, 0, 0];
/// The status line's color on [`BACKGROUND`].
pub(super) const TEXT: [u8; 3] = [160, 160, 160];

enum Status {
    Loading,
    Ready(PreviewScene),
    Failed,
}

/// A loaded frame, already laid over [`BACKGROUND`]: `BGRA` rows, top
/// row first, the layout a 32-bit DIB takes.
pub(super) struct Frame {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) bgra: Vec<u8>,
}

/// What the worker loading the model hands back.
type Loaded = Arc<Mutex<Option<Result<PreviewScene, LoadError>>>>;

pub(super) struct Pane {
    status: Status,
    loaded: Loaded,
    viewport: Option<ViewportId>,
    interaction: Interaction,
    size: (u32, u32),
    frame: Option<Frame>,
    /// A camera glide or the ambient occlusion is still changing the picture.
    settling: bool,
    last_tick: Option<Instant>,
}

impl Pane {
    pub(super) fn new() -> Self {
        Self {
            status: Status::Loading,
            loaded: Arc::default(),
            viewport: None,
            interaction: Interaction::default(),
            size: (1, 1),
            frame: None,
            settling: false,
            last_tick: None,
        }
    }

    /// Load `path` on a worker; `loaded` is called on that worker when it is
    /// done, to wake the window.
    pub(super) fn start_load(&mut self, path: PathBuf, loaded: impl FnOnce() + Send + 'static) {
        let slot = Arc::clone(&self.loaded);
        let spawned = std::thread::Builder::new()
            .name("preview-load".into())
            .spawn(move || {
                let started = Instant::now();
                let result = PreviewScene::load(
                    review_preview::SceneSource::Path(&path),
                    Some(MAX_TEXTURE_SIDE),
                );
                match &result {
                    Ok(scene) => log::debug!(
                        "preview of {} loaded in {:.0} ms",
                        scene.name(),
                        started.elapsed().as_secs_f64() * 1000.0
                    ),
                    Err(error) => log::info!("no preview of {}: {error}", path.display()),
                }
                *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(result);
                loaded();
            });
        if let Err(error) = spawned {
            log::error!("could not start the preview load: {error}");
            self.status = Status::Failed;
        }
    }

    /// Take what the worker loaded. True when there is something new to show.
    pub(super) fn finish_load(&mut self) -> bool {
        let Some(result) = self
            .loaded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return false;
        };
        self.status = match result {
            Ok(scene) => Status::Ready(scene),
            Err(_) => Status::Failed,
        };
        true
    }

    /// The status line, while there is no picture to show.
    pub(super) fn status_text(&self) -> Option<String> {
        match self.status {
            Status::Loading => {
                Some(review_localization::tr(crate::keys::shell_preview::LOADING).into_owned())
            }
            Status::Failed => {
                Some(review_localization::tr(crate::keys::shell_preview::FAILED).into_owned())
            }
            Status::Ready(_) => None,
        }
    }

    pub(super) fn frame(&self) -> Option<&Frame> {
        self.frame
            .as_ref()
            .filter(|frame| (frame.width, frame.height) == self.size)
    }

    pub(super) fn is_settling(&self) -> bool {
        self.settling
    }

    pub(super) fn is_dragging(&self) -> bool {
        self.interaction.is_dragging()
    }

    pub(super) fn resize(&mut self, size: (u32, u32)) {
        self.size = (size.0.max(1), size.1.max(1));
    }

    pub(super) fn press(&mut self, button: Button, position: Vec2, modifiers: Modifiers) {
        self.interaction.press(button, position, modifiers);
    }

    pub(super) fn release(&mut self, button: Button) {
        self.interaction.release(button);
    }

    /// The pointer capture went elsewhere: whatever drag was running is over.
    pub(super) fn cancel_drag(&mut self) {
        for button in [Button::Left, Button::Right, Button::Middle] {
            self.interaction.release(button);
        }
    }

    /// The pointer moved; draws a frame when the camera moved with it.
    pub(super) fn pointer_moved(&mut self, position: Vec2) -> bool {
        let extent = Vec2::new(self.size.0 as f32, self.size.1 as f32);
        let moved = self
            .with_view(|view, _, interaction| interaction.moved(view.camera(), position, extent));
        if moved == Some(true) {
            self.draw();
            return true;
        }
        false
    }

    pub(super) fn wheel(&mut self, notches: f32) {
        let zoomed = self.with_view(|view, _, interaction| {
            interaction.wheel_notches(view.camera(), notches);
        });
        if zoomed.is_some() {
            self.draw();
        }
    }

    /// Glide back to a framing of the whole model.
    pub(super) fn reframe(&mut self) {
        if self
            .with_view(|view, scene, _| view.camera().reframe(scene))
            .is_some()
        {
            self.last_tick = Some(Instant::now());
            self.draw();
        }
    }

    /// One tick of the settle timer: step the camera glide, draw.
    pub(super) fn tick(&mut self) {
        let now = Instant::now();
        let seconds = self
            .last_tick
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32())
            .min(MAX_TICK_SECONDS);
        self.last_tick = Some(now);
        let _ = self.with_view(|view, _, _| view.camera().advance(seconds));
        self.draw();
    }

    /// Draw the model at the pane's size into [`Self::frame`].
    pub(super) fn draw(&mut self) {
        let size = self.size;
        let drawn = self.with_view(|view, scene, _| -> GpuResult<_> {
            view.render(scene, size)?;
            let image = view.read_back()?;
            Ok((image, view.camera().is_settling()))
        });
        match drawn {
            Some(Ok((image, settling))) => {
                let over = image.over(BACKGROUND);
                self.frame = Some(Frame {
                    width: over.width,
                    height: over.height,
                    bgra: over.to_bgra(),
                });
                self.settling = settling;
            }
            Some(Err(error)) => {
                log::error!("preview did not draw: {error}");
                self.settling = false;
            }
            None => self.settling = false,
        }
    }

    /// Run `body` with this pane's viewport, its scene and its pointer state,
    /// under the host's lock, opening the viewport (and framing the model in it)
    /// the first time. `None` while there is no model to show or no GPU to show
    /// it on.
    fn with_view<T>(
        &mut self,
        body: impl FnOnce(&mut HostedViewport<'_>, &PreviewScene, &mut Interaction) -> T,
    ) -> Option<T> {
        let Status::Ready(scene) = &self.status else {
            return None;
        };
        let interaction = &mut self.interaction;
        let size = self.size;
        let current = self.viewport;
        let result = with_host(|host| {
            let id = match current.filter(|&id| host.viewport(id).is_some()) {
                Some(id) => id,
                None => {
                    let id = host.open();
                    if let Some(mut view) = host.viewport(id) {
                        view.camera().frame(scene, size);
                    }
                    id
                }
            };
            let value = host.viewport(id).map(|mut view| {
                view.camera().resize(size);
                body(&mut view, scene, interaction)
            });
            (id, value)
        });
        match result {
            Ok((id, value)) => {
                self.viewport = Some(id);
                value
            }
            Err(_) => None,
        }
    }
}

impl Drop for Pane {
    fn drop(&mut self) {
        if let Some(id) = self.viewport.take() {
            with_existing_host(|host| host.close(id));
        }
    }
}
