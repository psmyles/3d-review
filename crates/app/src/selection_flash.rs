//! The selection-highlight flash: a brief bright fill over a newly selected
//! node/material that fades out.
//!
//! Split out of `main.rs` as its own animation concern — a state type, its
//! timing constants, the easing curve, and the per-frame advance. `frame.rs`
//! calls [`App::update_selection_flash`] once per redraw and keeps pacing frames
//! while `App::selection_flash` is live; nothing else touches it. The fade lands
//! in the UI snapshot the scene callback reads, so data still flows one way
//! (invariant 2) and the redraw loop stays in `app` (invariant 6).

use std::time::{Duration, Instant};

use crate::App;

/// State of the selection-highlight flash: a brief bright fill over a newly
/// selected node/material that fades out. Same capped-per-frame accumulation as
/// camera transitions, so an idle gap before the flash can't fast-forward it to
/// the end. `None` when no flash is playing.
pub(crate) struct FlashProgress {
    elapsed: Duration,
    last_tick: Instant,
}

/// How long the selection-highlight flash takes to fade from full to gone.
const SELECTION_FLASH: Duration = Duration::from_millis(500);

/// Cap on how much the selection flash advances in one frame (≈30 Hz), so an idle
/// gap before a selection change doesn't skip the flash. Matches the camera
/// transition step guard.
const MAX_FLASH_STEP: Duration = Duration::from_millis(33);

/// Hermite smoothstep `3t² − 2t³` on a clamped `t ∈ [0, 1]` — an ease with zero
/// slope at both ends.
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl App {
    /// Advance the selection-highlight flash and write this frame's fade factor (1
    /// → 0 over [`SELECTION_FLASH`]) into [`review_ui::UiState::selection_fade`],
    /// where the scene callback reads it. A change to a different node/material
    /// (set by the Outliner) restarts the flash; selecting nothing ends it. The fade
    /// lives in `app` because the redraw loop does (invariant 6) — the flash keeps
    /// requesting frames via `selection_flash.is_some()` in `render`, the same way
    /// camera transitions do. Like those, time accumulates from a capped per-frame
    /// delta so an idle gap before the selection can't fast-forward the flash.
    pub(crate) fn update_selection_flash(&mut self) {
        let now = Instant::now();

        // (Re)start the flash whenever the selection changes to a different target;
        // clear it when nothing is selected.
        if self.ui.selection != self.flashed_selection {
            self.flashed_selection = self.ui.selection;
            self.selection_flash = self.ui.selection.is_active().then_some(FlashProgress {
                elapsed: Duration::ZERO,
                last_tick: now,
            });
        }

        let fade = match self.selection_flash.as_mut() {
            Some(flash) => {
                let step = now.duration_since(flash.last_tick).min(MAX_FLASH_STEP);
                flash.last_tick = now;
                flash.elapsed += step;
                if flash.elapsed >= SELECTION_FLASH {
                    // Flash done: drop it so the pacing loop stops requesting frames.
                    self.selection_flash = None;
                    0.0
                } else {
                    // Ease-out (smoothstep complement): full at the start of the
                    // flash, easing smoothly to 0 — a blink that settles rather than
                    // a linear cut.
                    let t = flash.elapsed.as_secs_f32() / SELECTION_FLASH.as_secs_f32();
                    1.0 - smoothstep(t)
                }
            }
            None => 0.0,
        };
        self.ui.selection_fade = fade;
    }
}
