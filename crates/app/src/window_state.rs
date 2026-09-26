//! Persist the window's last position, size, and maximized state so the viewer
//! reopens where the user left it.
//!
//! The state file lives in the OS's per-user config directory (found through
//! `dirs`, see [`state_dir`]), not next to the executable: an install in
//! `Program Files` is read-only for a non-admin user, so writing config beside
//! the exe would silently fail. It is written on the way out (a close request,
//! the menu's Exit, and `exiting`, which the macOS Quit reaches without a close
//! request) and neither read nor written by a gate run, which opens at a fixed
//! size of its own.
//!
//! The format is a tiny `key=value` text file — deliberately dependency-free
//! (no serde) for a five-field record. A missing, unreadable, or malformed file
//! is never fatal: load returns `None` and the window falls back to OS defaults,
//! as it does for a saved placement no connected monitor can show any more
//! ([`placement_is_visible`]). The monitor refresh-rate query the redraw pacing
//! uses lives here too, beside the rest of the monitor geometry.

use std::path::PathBuf;
use std::time::Duration;

use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use crate::{App, FALLBACK_REFRESH_HZ};

/// The window placement we save and restore: outer position (including
/// decorations) plus the inner client size, and whether the window was
/// maximized. When maximized, the position/size describe the *restored* (pre-
/// maximize) bounds so un-maximizing returns to a sensible window.
#[derive(Debug, Clone, Copy)]
pub struct WindowPlacement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

/// Directory holding the state file: `<config dir>\<app name>` — `%APPDATA%\3D
/// Review` on Windows (the per-user *Roaming* profile, exactly the path the
/// hand-read `APPDATA` env var used to give) and `~/Library/Application
/// Support/3D Review` on macOS, where that variable does not exist at all
/// (`docs/ARCHITECTURE.md`, Platform decisions D17). `None` on a platform or
/// environment `dirs` can't answer for, in which case persistence is simply
/// skipped.
fn state_dir() -> Option<PathBuf> {
    let mut path = dirs::config_dir()?;
    path.push(crate::APP_NAME);
    Some(path)
}

fn state_file() -> Option<PathBuf> {
    let mut path = state_dir()?;
    path.push("window.cfg");
    Some(path)
}

/// Read the saved placement, or `None` if there is no usable saved state.
pub fn load() -> Option<WindowPlacement> {
    let path = state_file()?;
    let contents = std::fs::read_to_string(&path).ok()?;
    let placement = parse(&contents)?;
    log::debug!("restored window placement: {placement:?}");
    Some(placement)
}

/// The placement a `window.cfg` describes, or `None` when it describes none.
/// Every figure but `maximized` is required, so a partial file reads as absent,
/// and so does a zero size — a corrupt file must not open a window with no area.
/// Unknown keys and unparseable lines are skipped rather than failing the read.
fn parse(contents: &str) -> Option<WindowPlacement> {
    let mut x = None;
    let mut y = None;
    let mut width = None;
    let mut height = None;
    let mut maximized = false;

    for line in contents.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "x" => x = value.parse().ok(),
            "y" => y = value.parse().ok(),
            "width" => width = value.parse().ok(),
            "height" => height = value.parse().ok(),
            "maximized" => maximized = value == "true",
            _ => {}
        }
    }

    // Position and size are all required; a partial file is treated as absent.
    let placement = WindowPlacement {
        x: x?,
        y: y?,
        width: width?,
        height: height?,
        maximized,
    };

    // Guard against zero/degenerate sizes from a corrupt file.
    if placement.width == 0 || placement.height == 0 {
        return None;
    }
    Some(placement)
}

/// The `window.cfg` text for `placement` — what [`parse`] reads back.
fn serialize(placement: WindowPlacement) -> String {
    format!(
        "x={}\ny={}\nwidth={}\nheight={}\nmaximized={}\n",
        placement.x, placement.y, placement.width, placement.height, placement.maximized,
    )
}

/// Write the placement to the config directory. Failures are logged and
/// swallowed —
/// losing window position is not worth interrupting the user.
pub fn save(placement: WindowPlacement) {
    let Some(dir) = state_dir() else {
        return;
    };
    let Some(path) = state_file() else {
        return;
    };

    if let Err(error) = std::fs::create_dir_all(&dir) {
        log::warn!(
            "failed to create window-state directory {}: {error}",
            dir.display()
        );
        return;
    }

    let contents = serialize(placement);

    // Replaced rather than overwritten: a process killed mid-write would
    // otherwise leave a half-written config, and the next launch reads it back
    // as a window with no size.
    if let Err(error) = review_optimize::write_bytes_replacing(&path, &contents) {
        log::warn!("failed to write window state {}: {error}", path.display());
    }
}

impl App {
    /// Snapshot the window's current outer position + inner size while it isn't
    /// maximized, so the persisted placement describes a real window. Skipped
    /// while maximized (the bounds then cover the whole monitor) and when winit
    /// can't report the outer position.
    pub(crate) fn record_windowed_bounds(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        if window.is_maximized() {
            return;
        }
        let Ok(position) = window.outer_position() else {
            return;
        };
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.placement.last_windowed_bounds =
            Some(((position.x, position.y), (size.width, size.height)));
    }

    /// Persist the current window placement on exit. Uses the last
    /// recorded non-maximized bounds (so un-maximize restores correctly) together
    /// with the live maximized state.
    pub(crate) fn save_window_placement(&mut self) {
        // A gate run sizes the window itself, to the same figure every time; that
        // is not a placement the user chose, and it must not replace one.
        if self.gate.is_some() {
            return;
        }
        // Refresh from the live window first in case the latest move/resize hasn't
        // been recorded yet.
        self.record_windowed_bounds();

        let Some(window) = self.window.as_ref() else {
            return;
        };
        let Some(((x, y), (width, height))) = self.placement.last_windowed_bounds else {
            return;
        };

        save(WindowPlacement {
            x,
            y,
            width,
            height,
            maximized: window.is_maximized(),
        });
    }
}

/// Whether a saved placement still lands on a connected monitor. Guards against
/// restoring a window onto a display that has since been unplugged or had its
/// layout changed, which would otherwise reopen the window off-screen. A
/// placement counts as visible if its rectangle overlaps any available monitor;
/// when winit reports no monitors we trust the placement rather than discard it.
pub(crate) fn placement_is_visible(
    event_loop: &ActiveEventLoop,
    placement: &WindowPlacement,
) -> bool {
    let mut any_monitor = false;
    for monitor in event_loop.available_monitors() {
        any_monitor = true;
        if overlaps(placement, monitor.position(), monitor.size()) {
            return true;
        }
    }

    // No monitors enumerated (rare/headless): don't throw the placement away.
    !any_monitor
}

/// Whether `placement`'s rect overlaps the monitor at `position` of `size`.
fn overlaps(
    placement: &WindowPlacement,
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
) -> bool {
    // The placement is read from a file, so its figures are not trusted to stay
    // in range: saturate rather than overflow.
    let win_right = placement.x.saturating_add_unsigned(placement.width);
    let win_bottom = placement.y.saturating_add_unsigned(placement.height);
    let mon_right = position.x.saturating_add_unsigned(size.width);
    let mon_bottom = position.y.saturating_add_unsigned(size.height);
    placement.x < mon_right
        && win_right > position.x
        && placement.y < mon_bottom
        && win_bottom > position.y
}

/// Whether a placement's rect covers essentially a whole monitor — the signature
/// of *maximized* geometry rather than real restored bounds. A maximized window's
/// outer position sits at (or a few px past) the monitor's top-left and its client
/// spans the work area, so the saved rect starts at/left-of the monitor origin and
/// is nearly as large as the monitor. Used to reject restore bounds that are really
/// maximized geometry recorded by a pre-fix build, so un-maximize doesn't land on a
/// full-screen rect. The fraction thresholds (not pixel counts) absorb the taskbar
/// and DPI-dependent frame overhang without misflagging a normal or snapped window.
pub(crate) fn placement_fills_monitor(
    event_loop: &ActiveEventLoop,
    placement: &WindowPlacement,
) -> bool {
    event_loop
        .available_monitors()
        .any(|monitor| fills(placement, monitor.position(), monitor.size()))
}

/// Whether `placement` covers the monitor at `position` of `size` the way
/// maximized geometry does — see [`placement_fills_monitor`].
fn fills(
    placement: &WindowPlacement,
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
) -> bool {
    // Maximized client width tracks the work area exactly (≈100% with a bottom
    // taskbar, a bit less with a side taskbar); height loses the taskbar band.
    const MIN_WIDTH_FRACTION: f32 = 0.9;
    const MIN_HEIGHT_FRACTION: f32 = 0.85;

    let at_origin = placement.x <= position.x && placement.y <= position.y;
    at_origin
        && placement.width as f32 >= size.width as f32 * MIN_WIDTH_FRACTION
        && placement.height as f32 >= size.height as f32 * MIN_HEIGHT_FRACTION
}

/// A comfortable centered restored window, used as the un-maximize target when the
/// saved bounds are unusable (they described maximized geometry — see
/// [`placement_fills_monitor`]). Sized to a fraction of the primary monitor and
/// centered on it, keeping `maximized` so the window still opens maximized. Falls
/// back to the original placement if no monitor can be enumerated.
pub(crate) fn default_windowed_placement(
    event_loop: &ActiveEventLoop,
    fallback: &WindowPlacement,
) -> WindowPlacement {
    /// Fraction of the monitor a default restored window occupies.
    const SIZE_FRACTION: f32 = 0.7;

    let Some(monitor) = event_loop
        .primary_monitor()
        .or_else(|| event_loop.available_monitors().next())
    else {
        return *fallback;
    };

    let pos = monitor.position();
    let size = monitor.size();
    let width = (size.width as f32 * SIZE_FRACTION) as u32;
    let height = (size.height as f32 * SIZE_FRACTION) as u32;
    WindowPlacement {
        x: pos.x + (size.width.saturating_sub(width) / 2) as i32,
        y: pos.y + (size.height.saturating_sub(height) / 2) as i32,
        width,
        height,
        maximized: fallback.maximized,
    }
}

/// The active monitor's refresh interval, used to cap continuous redraw. Falls
/// back to 60 Hz when winit can't report a rate (some virtual/headless displays).
pub(crate) fn monitor_refresh_interval(window: &Window) -> Duration {
    window
        .current_monitor()
        .and_then(|monitor| monitor.refresh_rate_millihertz())
        .filter(|millihertz| *millihertz > 0)
        .map(|millihertz| Duration::from_secs_f64(1000.0 / f64::from(millihertz)))
        .unwrap_or_else(|| Duration::from_secs_f64(1.0 / FALLBACK_REFRESH_HZ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `dirs::config_dir()` must resolve to the *same* directory the hand-read
    /// `APPDATA` environment variable used to give (`docs/ARCHITECTURE.md`,
    /// Platform decisions D17) — the point of the swap is a path that also exists
    /// on macOS, not a new location on Windows. Getting this wrong would silently
    /// strand every existing user's saved window placement, with no error and no
    /// way to notice but a window that stopped reopening where it was left.
    #[cfg(windows)]
    #[test]
    fn the_config_directory_is_still_appdata_on_windows() {
        let Some(appdata) = std::env::var_os("APPDATA") else {
            // No roaming profile in this environment; nothing to compare against.
            return;
        };
        let mut expected = std::path::PathBuf::from(appdata);
        expected.push(crate::APP_NAME);
        assert_eq!(state_dir(), Some(expected));
    }

    /// The macOS twin: `~/Library/Application Support/3D Review` (D17). The same
    /// `dirs::config_dir()` call answers for both OSes, so the only thing worth
    /// pinning per host is *which* directory it resolves to — a `dirs` upgrade that
    /// moved this would strand a Mac user's saved placement exactly as silently.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_config_directory_is_application_support_on_macos() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let mut expected = std::path::PathBuf::from(home);
        expected.push("Library/Application Support");
        expected.push(crate::APP_NAME);
        assert_eq!(state_dir(), Some(expected));
    }

    fn placement(x: i32, y: i32, width: u32, height: u32) -> WindowPlacement {
        WindowPlacement {
            x,
            y,
            width,
            height,
            maximized: false,
        }
    }

    #[test]
    fn a_saved_placement_reads_back_as_itself() {
        let saved = WindowPlacement {
            maximized: true,
            ..placement(-1920, 40, 1280, 720)
        };
        let read = parse(&serialize(saved)).expect("a whole file parses");
        assert_eq!(
            (read.x, read.y, read.width, read.height, read.maximized),
            (-1920, 40, 1280, 720, true)
        );
    }

    #[test]
    fn a_file_missing_a_figure_or_with_no_area_is_no_placement() {
        assert!(parse("x=0\ny=0\nwidth=800\n").is_none(), "no height");
        assert!(
            parse("x=0\ny=0\nwidth=800\nheight=0\n").is_none(),
            "no area"
        );
        assert!(
            parse("x=0\ny=zero\nwidth=800\nheight=600\n").is_none(),
            "bad y"
        );
        assert!(parse("").is_none());
    }

    #[test]
    fn unknown_keys_and_stray_lines_are_skipped() {
        let read = parse("# comment\nx = 10\nfuture=1\ny=20\nwidth=300\nheight=200\n")
            .expect("the four figures are there");
        assert_eq!((read.x, read.y), (10, 20));
        assert!(!read.maximized, "absent reads as not maximized");
    }

    #[test]
    fn a_window_on_an_unplugged_monitor_does_not_overlap_the_one_left() {
        let monitor = (PhysicalPosition::new(0, 0), PhysicalSize::new(1920, 1080));
        assert!(overlaps(
            &placement(100, 100, 800, 600),
            monitor.0,
            monitor.1
        ));
        // Where a second monitor to the left used to be.
        assert!(!overlaps(
            &placement(-1800, 100, 800, 600),
            monitor.0,
            monitor.1
        ));
        // Touching edges are not an overlap.
        assert!(!overlaps(
            &placement(1920, 0, 800, 600),
            monitor.0,
            monitor.1
        ));
    }

    #[test]
    fn figures_at_the_edge_of_the_integer_range_saturate_rather_than_wrap() {
        let monitor = (PhysicalPosition::new(0, 0), PhysicalSize::new(1920, 1080));
        // `i32::MAX + width` would wrap negative and read as overlapping.
        assert!(!overlaps(
            &placement(i32::MAX, 0, u32::MAX, 600),
            monitor.0,
            monitor.1
        ));
        assert!(overlaps(
            &placement(i32::MIN, 0, u32::MAX, 600),
            monitor.0,
            monitor.1
        ));
    }

    #[test]
    fn maximized_geometry_is_told_from_a_real_window() {
        let origin = PhysicalPosition::new(0, 0);
        let size = PhysicalSize::new(1920, 1080);
        // A maximized window's frame hangs a few pixels past the monitor origin.
        assert!(fills(&placement(-8, -8, 1920, 1030), origin, size));
        assert!(!fills(&placement(100, 100, 1280, 720), origin, size));
        // A snapped half-width window is at the origin but not full width.
        assert!(!fills(&placement(0, 0, 960, 1040), origin, size));
    }
}

/// The window-placement tracker: the startup maximize hint plus the windowed
/// bounds sampled for session persistence, grouped out of [`App`].
#[derive(Default)]
pub(crate) struct PlacementTracker {
    /// The process was launched with a request to start maximized (e.g. a
    /// shortcut set to **Run: Maximized**). Set in `resumed` and applied at
    /// window creation, since winit doesn't honor the OS hint on its own.
    pub(crate) start_maximized: bool,
    /// The most recent *non-maximized* window placement (outer position + inner
    /// size), tracked from `Moved`/`Resized` events so it's available to persist
    /// on exit. Recorded only while the window isn't maximized, so un-maximizing
    /// a restored session returns to a real window rather than a fullscreen rect.
    pub(crate) last_windowed_bounds: Option<((i32, i32), (u32, u32))>,
    /// Set when a `Moved`/`Resized` event arrives; the windowed bounds are then
    /// sampled once in `about_to_wait`, after the event burst has settled. This
    /// deferral matters for maximize: winit dispatches `Moved` (from
    /// `WM_WINDOWPOSCHANGED`) *before* the `WM_SIZE` that sets its maximized flag,
    /// so sampling eagerly in the `Moved` handler would record the maximized
    /// geometry as if it were windowed. By `about_to_wait` the flag is set, so
    /// `record_windowed_bounds`'s `is_maximized()` guard sees the real state.
    pub(crate) bounds_dirty: bool,
}
