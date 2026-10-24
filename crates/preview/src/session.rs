//! One interactive preview's state: the model loading or loaded, its viewport
//! and camera, and the last frame drawn — everything a shell's view shows, with
//! no platform in it.
//!
//! A shell owns the window, turns its input into calls here, and shows
//! [`Session::frame`]. Every draw goes through the process's one host
//! ([`crate::shared`]), so any number of sessions can be open at once.
//!
//! The macOS Quick Look extension drives this. The Explorer extension
//! (`shell-windows`) predates it and keeps its own copy as `preview/pane.rs`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use glam::Vec2;

use crate::shared::{with_existing_host, with_host};
use crate::{
    Button, GpuResult, HostedViewport, Image, Interaction, LoadError, Modifiers, PreviewScene,
    SceneSource, ViewportId,
};

/// The largest texture side a preview keeps. A preview is at most a window
/// across, so a 2048 map is already sampled from a mip below its top.
const MAX_TEXTURE_SIDE: u32 = 2048;

/// The longest step a camera glide takes per tick, so a stalled run loop does
/// not make the next tick jump to the end.
const MAX_TICK_SECONDS: f32 = 0.1;

/// What a session has to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The model is still being read: say so.
    Loading,
    /// There is a model, and [`Session::frame`] is (or will be) its picture.
    Ready,
    /// The model could not be read, or has nothing to draw: say so. The log
    /// names the reason.
    Failed,
}

enum State {
    Loading,
    Ready(PreviewScene),
    Failed,
}

/// What the worker loading the model hands back.
type Loaded = Arc<Mutex<Option<Result<PreviewScene, LoadError>>>>;

pub struct Session {
    state: State,
    loaded: Loaded,
    viewport: Option<ViewportId>,
    interaction: Interaction,
    size: (u32, u32),
    /// What the model is laid over (sRGB): the frame is opaque.
    background: [u8; 3],
    frame: Option<Image>,
    /// A camera glide or the ambient occlusion is still changing the picture.
    settling: bool,
    last_tick: Option<Instant>,
}

impl Session {
    /// A session whose frames are laid over `background` (sRGB bytes).
    pub fn new(background: [u8; 3]) -> Self {
        Self {
            state: State::Loading,
            loaded: Arc::default(),
            viewport: None,
            interaction: Interaction::default(),
            size: (1, 1),
            background,
            frame: None,
            settling: false,
            last_tick: None,
        }
    }

    /// Load `path` on a worker. `loaded` is called exactly once when it is done
    /// — on that worker, or here if no worker could start — to wake the shell,
    /// which then calls [`Self::finish_load`] on its own thread.
    pub fn start_load(&mut self, path: PathBuf, loaded: impl FnOnce() + Send + 'static) {
        let slot = Arc::clone(&self.loaded);
        // Shared with the worker so a failed spawn can still call it here.
        let loaded = Arc::new(Mutex::new(Some(loaded)));
        let notify = {
            let loaded = Arc::clone(&loaded);
            move || {
                let taken = loaded.lock().unwrap_or_else(PoisonError::into_inner).take();
                if let Some(loaded) = taken {
                    loaded();
                }
            }
        };
        let worker_notify = notify.clone();
        let spawned = std::thread::Builder::new()
            .name("preview-load".into())
            .spawn(move || {
                let started = Instant::now();
                let result = PreviewScene::load(SceneSource::Path(&path), Some(MAX_TEXTURE_SIDE));
                match &result {
                    Ok(scene) => log::debug!(
                        "preview of {} loaded in {:.0} ms",
                        scene.name(),
                        started.elapsed().as_secs_f64() * 1000.0
                    ),
                    Err(error) => log::info!("no preview of {}: {error}", path.display()),
                }
                *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(result);
                worker_notify();
            });
        if let Err(error) = spawned {
            log::error!("could not start the preview load: {error}");
            self.state = State::Failed;
            notify();
        }
    }

    /// Take what the worker loaded. True when there is something new to show.
    pub fn finish_load(&mut self) -> bool {
        let Some(result) = self
            .loaded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return false;
        };
        self.state = match result {
            Ok(scene) => State::Ready(scene),
            Err(_) => State::Failed,
        };
        true
    }

    pub fn status(&self) -> Status {
        match self.state {
            State::Loading => Status::Loading,
            State::Ready(_) => Status::Ready,
            State::Failed => Status::Failed,
        }
    }

    /// The last frame drawn, laid over the background — or `None` when there is
    /// none at the current size yet.
    pub fn frame(&self) -> Option<&Image> {
        self.frame
            .as_ref()
            .filter(|frame| (frame.width, frame.height) == self.size)
    }

    /// Whether another frame would look different with nothing else changing:
    /// what a shell runs its settle timer for, calling [`Self::tick`].
    pub fn is_settling(&self) -> bool {
        self.settling
    }

    /// Whether a button is down: what a shell holds the pointer capture for.
    pub fn is_dragging(&self) -> bool {
        self.interaction.is_dragging()
    }

    /// The view is now `size` pixels. Draws nothing: the shell asks for a frame
    /// at the new size with [`Self::draw`].
    pub fn resize(&mut self, size: (u32, u32)) {
        self.size = (size.0.max(1), size.1.max(1));
    }

    /// A button went down at `position` (pixels from the view's top left).
    pub fn press(&mut self, button: Button, position: Vec2, modifiers: Modifiers) {
        self.interaction.press(button, position, modifiers);
    }

    pub fn release(&mut self, button: Button) {
        self.interaction.release(button);
    }

    /// The pointer went elsewhere: whatever drag was running is over.
    pub fn cancel_drag(&mut self) {
        for button in [Button::Left, Button::Right, Button::Middle] {
            self.interaction.release(button);
        }
    }

    /// The pointer moved; draws a frame when the camera moved with it, and says
    /// so.
    pub fn pointer_moved(&mut self, position: Vec2) -> bool {
        let extent = Vec2::new(self.size.0 as f32, self.size.1 as f32);
        let moved = self
            .with_view(|view, _, interaction| interaction.moved(view.camera(), position, extent));
        if moved == Some(true) {
            self.draw();
            return true;
        }
        false
    }

    /// The wheel turned `notches` (positive away from the user, which zooms
    /// in); draws.
    pub fn wheel_notches(&mut self, notches: f32) {
        let zoomed = self.with_view(|view, _, interaction| {
            interaction.wheel_notches(view.camera(), notches);
        });
        if zoomed.is_some() {
            self.draw();
        }
    }

    /// A pixel-precise scroll of `pixels` (a trackpad); draws.
    pub fn wheel_pixels(&mut self, pixels: f32) {
        let zoomed = self.with_view(|view, _, interaction| {
            interaction.wheel_pixels(view.camera(), pixels);
        });
        if zoomed.is_some() {
            self.draw();
        }
    }

    /// A trackpad pinch of `scale` (positive spreads, zooming in); draws.
    pub fn pinch(&mut self, scale: f32) {
        let zoomed = self.with_view(|view, _, interaction| {
            interaction.pinch(view.camera(), scale);
        });
        if zoomed.is_some() {
            self.draw();
        }
    }

    /// Glide back to a framing of the whole model; draws.
    pub fn reframe(&mut self) {
        if self
            .with_view(|view, scene, _| view.camera().reframe(scene))
            .is_some()
        {
            self.last_tick = Some(Instant::now());
            self.draw();
        }
    }

    /// One tick of the settle timer: step the camera glide, draw.
    pub fn tick(&mut self) {
        let now = Instant::now();
        let seconds = self
            .last_tick
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32())
            .min(MAX_TICK_SECONDS);
        self.last_tick = Some(now);
        let _ = self.with_view(|view, _, _| view.camera().advance(seconds));
        self.draw();
    }

    /// Draw the model at the view's size into [`Self::frame`].
    pub fn draw(&mut self) {
        let size = self.size;
        let drawn = self.with_view(|view, scene, _| -> GpuResult<_> {
            view.render(scene, size)?;
            let image = view.read_back()?;
            Ok((image, view.camera().is_settling()))
        });
        match drawn {
            Some(Ok((image, settling))) => {
                self.frame = Some(image.over(self.background));
                self.settling = settling;
            }
            Some(Err(error)) => {
                log::error!("preview did not draw: {error}");
                self.settling = false;
            }
            None => self.settling = false,
        }
    }

    /// Run `body` with this session's viewport, its scene and its pointer
    /// state, under the host's lock, opening the viewport (and framing the
    /// model in it) the first time. `None` while there is no model to show or no
    /// GPU to show it on.
    fn with_view<T>(
        &mut self,
        body: impl FnOnce(&mut HostedViewport<'_>, &PreviewScene, &mut Interaction) -> T,
    ) -> Option<T> {
        let State::Ready(scene) = &self.state else {
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

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(id) = self.viewport.take() {
            with_existing_host(|host| host.close(id));
        }
    }
}
