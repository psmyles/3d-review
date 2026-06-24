//! Lightweight startup phase timing.
//!
//! Records the wall-clock cost of each coarse step of bringing the viewer up —
//! window creation, adapter/device acquisition, surface config, and the first
//! frame (which compiles every scene pipeline and precomputes the IBL maps) — so
//! we can see where the time before the first visible frame actually goes. The
//! finer first-frame breakdown (pipeline compile vs IBL precompute) is logged
//! from `review_render`'s `SceneResources::new`.
//!
//! Logged via `tracing::info!`; set `RUST_LOG=info` (or `review_app=info`) to
//! see it. Times are CPU-side: GPU execution of the encoded passes is async, so
//! these capture submission/compilation cost, not GPU completion.

use std::time::Instant;

use tracing::info;

/// Laps through the startup phases, logging the duration of each as it finishes
/// and a grand total at the end.
pub struct StartupTimer {
    start: Instant,
    last: Instant,
}

impl StartupTimer {
    /// Begin timing. Call at the top of `resumed`, before any window/GPU setup.
    pub fn start() -> Self {
        let now = Instant::now();
        Self {
            start: now,
            last: now,
        }
    }

    /// Log the duration of the phase that just finished and reset the lap clock.
    pub fn lap(&mut self, phase: &str) {
        let now = Instant::now();
        let dt = now.duration_since(self.last);
        self.last = now;
        info!(phase, ms = dt.as_secs_f64() * 1000.0, "startup phase");
    }

    /// Log the total time from `start` to now (resumed → first frame submitted).
    pub fn finish(self) {
        info!(
            ms = self.start.elapsed().as_secs_f64() * 1000.0,
            "startup total (resumed -> first frame submitted)"
        );
    }
}
