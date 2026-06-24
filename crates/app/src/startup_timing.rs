//! Lightweight startup phase timing — compiled in only with the `startup-trace`
//! feature. In a default (shipping) build the feature is off, so `StartupTimer`
//! is a zero-cost no-op: the `timer.lap(...)` / `finish()` call sites in `main`
//! compile away entirely and no clock is read.
//!
//! When enabled it records the wall-clock cost of each coarse step of bringing
//! the viewer up — window creation, adapter/device acquisition, surface config,
//! and the first frame (which compiles the core scene pipeline and uploads the
//! baked IBL maps) — so we can see where the time before the first visible frame
//! goes. The finer first-frame breakdown (pipeline compile vs IBL upload) is
//! logged from `review_render`'s `SceneResources::new_core` (also behind
//! `startup-trace`).
//!
//! Logged via `tracing::info!`; build with `--features startup-trace` and set
//! `RUST_LOG=info` (or `review_app=info`) to see it. Times are CPU-side: GPU
//! execution of the encoded passes is async, so these capture
//! submission/compilation cost, not GPU completion.

#[cfg(feature = "startup-trace")]
pub use real::StartupTimer;

#[cfg(not(feature = "startup-trace"))]
pub use noop::StartupTimer;

#[cfg(feature = "startup-trace")]
mod real {
    use std::time::Instant;

    use tracing::info;

    /// Laps through the startup phases, logging the duration of each as it
    /// finishes and a grand total at the end.
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
}

#[cfg(not(feature = "startup-trace"))]
mod noop {
    /// Zero-cost stand-in used when `startup-trace` is off (the default). Every
    /// method is an empty `#[inline]` no-op, so the call sites in `main` compile
    /// to nothing and no timing is taken.
    pub struct StartupTimer;

    impl StartupTimer {
        #[inline(always)]
        pub fn start() -> Self {
            Self
        }

        #[inline(always)]
        pub fn lap(&mut self, _phase: &str) {}

        #[inline(always)]
        pub fn finish(self) {}
    }
}
