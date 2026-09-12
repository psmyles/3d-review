//! Redraw pacing: when the next frame is drawn, and why.
//!
//! Redraw is driven by winit events and active camera animation — never by UI
//! state mutation (invariant 6). A burst of high-frequency input coalesces into
//! a single redraw capped at the monitor refresh rate; continuous redraw happens
//! only while a camera transition, an interaction or playback is live.

use std::time::{Duration, Instant};

use winit::event_loop::{ActiveEventLoop, ControlFlow};

use crate::gate::GATE_POLL;
use crate::{App, FALLBACK_REFRESH_HZ};

/// The on-demand redraw scheduler (invariant 6): everything that decides *when*
/// the next frame is drawn, grouped out of [`App`].
pub(crate) struct RedrawScheduler {
    pub(crate) last_render_instant: Option<Instant>,
    /// When egui has asked to be repainted at a future time (e.g. a UI fade
    /// animation). Drives `ControlFlow::WaitUntil` so the loop sleeps until then
    /// instead of spinning. `None` = wait for the next input/redraw event.
    pub(crate) repaint_at: Option<Instant>,
    /// An interactive event (drag, hover, wheel, key) has requested a redraw.
    /// Folded into the paced `repaint_at` schedule in `about_to_wait` rather than
    /// triggering an immediate `request_redraw`, so a high-polling-rate mouse or
    /// key auto-repeat can't drive rendering faster than the monitor refresh.
    pub(crate) requested: bool,
    /// Remaining startup "warmup" frames to pump (Phase B). The first frame builds
    /// only the cheap core scene resources; the deferred scene pipelines + GTAO
    /// pass then compile one stage per subsequent frame. While this is non-zero,
    /// `render` keeps scheduling the next frame so the build drains behind the
    /// already-shown grid, then stops. Seeded once in `resumed`; `app` can't see
    /// the render-side build state (invariant 2), so it pumps a fixed count.
    pub(crate) warmup_frames: u32,
    /// Minimum spacing between continuously-rendered frames, derived from the
    /// active monitor's refresh rate. Caps redraw to the display so animation
    /// doesn't render faster than it can be shown. Defaults to 60 Hz until a
    /// monitor is known.
    pub(crate) refresh_interval: Duration,
}

impl Default for RedrawScheduler {
    fn default() -> Self {
        Self {
            last_render_instant: None,
            repaint_at: None,
            requested: false,
            warmup_frames: 0,
            refresh_interval: Duration::from_secs_f64(1.0 / FALLBACK_REFRESH_HZ),
        }
    }
}

impl App {
    pub(crate) fn pace_next_frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();

        // A gate run drives itself: frames back to back until the stamp is written
        // (the pacer below would cap it at the refresh rate, which is the thing
        // being measured), then idle until the hold ends and the process exits.
        if self.gate_active() {
            if self.gate_should_exit() {
                event_loop.exit();
            } else if self.gate_wants_redraw() {
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
                event_loop.set_control_flow(ControlFlow::Poll);
            } else {
                event_loop.set_control_flow(ControlFlow::WaitUntil(now + GATE_POLL));
            }
            return;
        }

        // Sample windowed bounds once the event burst has settled, so a maximize
        // (whose `Moved` arrives before the maximized flag is set) doesn't poison
        // the saved placement with fullscreen geometry. `record_windowed_bounds`
        // skips while maximized, so the pre-maximize bounds survive.
        if self.placement.bounds_dirty {
            self.placement.bounds_dirty = false;
            self.record_windowed_bounds();
        }

        // Fold a pending interactive redraw (drag, hover, wheel, key) into the
        // paced schedule. The earliest we'll draw is one refresh interval after
        // the last frame, so a burst of high-frequency input events coalesces
        // into a single redraw capped at the monitor refresh rate.
        if self.redraw.requested {
            self.redraw.requested = false;
            let earliest = self
                .redraw
                .last_render_instant
                .map_or(now, |last| last + self.redraw.refresh_interval);
            self.redraw.repaint_at = Some(
                self.redraw
                    .repaint_at
                    .map_or(earliest, |at| at.min(earliest)),
            );
        }

        // Sleep until the next scheduled repaint (if any), otherwise block until
        // the next input event. When the scheduled time arrives, fire one redraw
        // and fall back to waiting.
        match self.redraw.repaint_at {
            Some(wake) if now >= wake => {
                self.redraw.repaint_at = None;
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            Some(wake) => event_loop.set_control_flow(ControlFlow::WaitUntil(wake)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }
}
