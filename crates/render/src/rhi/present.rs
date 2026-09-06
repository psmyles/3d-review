//! What came of handing a finished frame to the display.
//!
//! Its own module because the backend leaf reports it and [`super::Frame::finish`]
//! passes it straight out to `app`, so it must not live in either.

/// Outcome of presenting a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentStatus {
    /// The frame presented, or was skipped because there was nothing to draw into
    /// (a zero-sized window, a refused drawable). Benign backend status codes — a
    /// hidden or fully occluded window — report this too: they are not actionable.
    Presented,
    /// The device was removed or reset (a driver crash/TDR, a GPU hang, an adapter
    /// change). Terminal: the swapchain is dead and every later frame fails too, so
    /// the only recovery is a relaunch. `reason` is the driver's own removal code.
    DeviceLost { reason: i32 },
}
