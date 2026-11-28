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
//!
//! On Wayland the placement has no position. A client there can neither read
//! where its window is nor choose where it goes, because the compositor places
//! every window, so winit answers `outer_position` with an error and ignores
//! `with_position`. The size and the maximized state are saved and restored as
//! everywhere else.

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
    /// The outer position, `(x, y)`, or `None` where the platform does not report
    /// one (Wayland; see the module docs).
    pub position: Option<(i32, i32)>,
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
/// The size is required, and so is a position with both its figures or with
/// neither (a Wayland placement), so a partial file reads as absent. So does a
/// figure that does not parse, and a zero size: a corrupt file must not open a
/// window with no area. Unknown keys and lines with no `=` are skipped rather
/// than failing the read.
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
            "x" => x = Some(value.parse().ok()?),
            "y" => y = Some(value.parse().ok()?),
            "width" => width = Some(value.parse().ok()?),
            "height" => height = Some(value.parse().ok()?),
            "maximized" => maximized = value == "true",
            _ => {}
        }
    }

    // Half a position is treated as absent, like a missing size.
    let position = match (x, y) {
        (Some(x), Some(y)) => Some((x, y)),
        (None, None) => None,
        _ => return None,
    };
    let placement = WindowPlacement {
        position,
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
    let position = placement
        .position
        .map(|(x, y)| format!("x={x}\ny={y}\n"))
        .unwrap_or_default();
    format!(
        "{position}width={}\nheight={}\nmaximized={}\n",
        placement.width, placement.height, placement.maximized,
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
    /// Snapshot the window's maximized state, and its current outer position +
    /// inner size while it isn't maximized, so the persisted placement describes a
    /// real window. The bounds are skipped while maximized (they then cover the
    /// whole monitor). When winit can't report the outer position, as on Wayland
    /// always, the size is still recorded, with the last position known (none, on
    /// Wayland). Nothing is sampled while minimized: that is neither state, and the
    /// window keeps the one restoring it gives back — so a maximized window closed
    /// from the taskbar still reopens maximized.
    pub(crate) fn record_windowed_bounds(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        if window.is_minimized() == Some(true) {
            return;
        }
        self.placement.maximized = window.is_maximized();
        if self.placement.maximized {
            return;
        }
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        let position = window
            .outer_position()
            .ok()
            .map(|position| (position.x, position.y))
            .or_else(|| {
                self.placement
                    .last_windowed_bounds
                    .and_then(|bounds| bounds.position)
            });
        self.placement.last_windowed_bounds = Some(WindowBounds {
            position,
            width: size.width,
            height: size.height,
        });
    }

    /// Persist the current window placement on exit. Uses the last
    /// recorded non-maximized bounds (so un-maximize restores correctly) together
    /// with the maximized state as last sampled outside a minimize.
    pub(crate) fn save_window_placement(&mut self) {
        // A gate run sizes the window itself, to the same figure every time; that
        // is not a placement the user chose, and it must not replace one.
        if self.gate.is_some() || self.window.is_none() {
            return;
        }
        // Refresh from the live window first in case the latest move/resize hasn't
        // been recorded yet.
        self.record_windowed_bounds();

        let Some(bounds) = self.placement.last_windowed_bounds else {
            return;
        };

        save(WindowPlacement {
            position: bounds.position,
            width: bounds.width,
            height: bounds.height,
            maximized: self.placement.maximized,
        });
    }
}

/// How the startup window first appears: the saved state, or the one the launcher
/// asked for (`review_import::startup_show`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct FirstShow {
    /// Maximized. Together with `minimized`, the state restoring it from the
    /// taskbar gives back.
    pub(crate) maximized: bool,
    /// On the taskbar without ever appearing on screen (*Run: Minimized*).
    pub(crate) minimized: bool,
}

impl FirstShow {
    /// Whether the window is created visible, with its first state applied by
    /// winit at creation — or created hidden and normal and left to
    /// [`reveal_startup_window`].
    ///
    /// Not on Windows. There winit shows a visible window *before* it applies the
    /// size and position, at the OS's default rect, and carries out a maximize as a
    /// `ShowWindow` that animates from there — so every launch flashed the window
    /// in the wrong place, and a maximized one grew to full screen in front of the
    /// user. A minimize would be a `ShowWindow` too, and on a hidden window both
    /// would show it. So on Windows the window is sized and placed while hidden,
    /// and the reveal *is* the state change. Elsewhere the window is created as
    /// winit expects.
    pub(crate) const CREATE_VISIBLE: bool = !cfg!(windows);

    /// Whether to create the window maximized (winit's `with_maximized`): only
    /// where it is created visible, see [`Self::CREATE_VISIBLE`].
    pub(crate) fn create_maximized(self) -> bool {
        self.maximized && Self::CREATE_VISIBLE
    }
}

/// Show the startup window, created hidden on Windows, in its first state — in
/// one step, so it never appears in any other state first (see
/// [`FirstShow::CREATE_VISIBLE`]). Elsewhere the window is already up.
pub(crate) fn reveal_startup_window(window: &Window, first: FirstShow) {
    #[cfg(windows)]
    {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        // Always `Win32` for a winit window on Windows.
        if let Ok(RawWindowHandle::Win32(handle)) =
            window.window_handle().map(|handle| handle.as_raw())
        {
            if first.minimized {
                review_import::show_window_minimized(handle.hwnd, first.maximized);
                // Before winit hears of the show below. It reads `IsIconic` and has
                // nothing left to do, but now knows the window is minimized:
                // otherwise the style refresh that comes with the show strips
                // `WS_MINIMIZE`, leaving a window that cannot be restored.
                window.set_minimized(true);
            } else if first.maximized {
                review_import::show_window_maximized(handle.hwnd);
            }
        }
        // winit keeps its own record of visibility, and without this would hide the
        // window again on its next style change. For a window that is already up,
        // the `SW_SHOW` (and `SW_MAXIMIZE`) it sends change nothing.
        window.set_visible(true);
    }
    // Created visible, and maximized by winit at creation if it was to be, so all
    // that can be left is a minimize. No launcher off Windows asks for one today
    // (`startup_show` answers `None`), but one that did would get it.
    #[cfg(not(windows))]
    if first.minimized {
        window.set_minimized(true);
    }
}

/// Whether a saved placement still lands on a connected monitor. Guards against
/// restoring a window onto a display that has since been unplugged or had its
/// layout changed, which would otherwise reopen the window off-screen. A
/// placement counts as visible if its rectangle overlaps any available monitor;
/// when winit reports no monitors we trust the placement rather than discard it.
/// So does a placement with no position: the compositor puts that window on a
/// monitor itself.
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
    let Some((x, y)) = placement.position else {
        return true;
    };
    // The placement is read from a file, so its figures are not trusted to stay
    // in range: saturate rather than overflow.
    let win_right = x.saturating_add_unsigned(placement.width);
    let win_bottom = y.saturating_add_unsigned(placement.height);
    let mon_right = position.x.saturating_add_unsigned(size.width);
    let mon_bottom = position.y.saturating_add_unsigned(size.height);
    x < mon_right && win_right > position.x && y < mon_bottom && win_bottom > position.y
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
/// maximized geometry does — see [`placement_fills_monitor`]. Never for a
/// placement with no position: only a build from before the position became
/// optional wrote maximized geometry, and it always wrote a position.
fn fills(
    placement: &WindowPlacement,
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
) -> bool {
    // Maximized client width tracks the work area exactly (≈100% with a bottom
    // taskbar, a bit less with a side taskbar); height loses the taskbar band.
    const MIN_WIDTH_FRACTION: f32 = 0.9;
    const MIN_HEIGHT_FRACTION: f32 = 0.85;

    let Some((x, y)) = placement.position else {
        return false;
    };
    let at_origin = x <= position.x && y <= position.y;
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
        position: Some((
            pos.x + (size.width.saturating_sub(width) / 2) as i32,
            pos.y + (size.height.saturating_sub(height) / 2) as i32,
        )),
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
            position: Some((x, y)),
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
            (read.position, read.width, read.height, read.maximized),
            (Some((-1920, 40)), 1280, 720, true)
        );
    }

    /// Wayland's placement: a size and a maximized state, and no position.
    #[test]
    fn a_placement_with_no_position_reads_back_as_itself() {
        let saved = WindowPlacement {
            position: None,
            maximized: true,
            ..placement(0, 0, 1600, 900)
        };
        let text = serialize(saved);
        assert!(!text.contains("x=") && !text.contains("y="), "{text}");
        let read = parse(&text).expect("a size alone is a placement");
        assert_eq!(
            (read.position, read.width, read.height, read.maximized),
            (None, 1600, 900, true)
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
        assert!(
            parse("x=zero\ny=zero\nwidth=800\nheight=600\n").is_none(),
            "a position that does not parse is not a missing one"
        );
        assert!(
            parse("x=0\nwidth=800\nheight=600\n").is_none(),
            "half a position"
        );
        assert!(parse("").is_none());
    }

    #[test]
    fn unknown_keys_and_stray_lines_are_skipped() {
        let read = parse("# comment\nx = 10\nfuture=1\ny=20\nwidth=300\nheight=200\n")
            .expect("the four figures are there");
        assert_eq!(read.position, Some((10, 20)));
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

    /// The compositor places a window with no position, so it is on a monitor
    /// whatever its size, and is never mistaken for maximized geometry.
    #[test]
    fn a_placement_with_no_position_is_always_on_screen_and_never_maximized_geometry() {
        let origin = PhysicalPosition::new(0, 0);
        let size = PhysicalSize::new(1920, 1080);
        let unplaced = WindowPlacement {
            position: None,
            ..placement(0, 0, 1920, 1080)
        };
        assert!(overlaps(&unplaced, PhysicalPosition::new(5000, 0), size));
        assert!(!fills(&unplaced, origin, size));
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

/// A window's restored (non-maximized) bounds, as [`PlacementTracker`] samples them.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WindowBounds {
    /// The outer position, or `None` where the platform reports none (Wayland).
    pub(crate) position: Option<(i32, i32)>,
    /// The inner (client) size.
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/// The window-placement tracker: the maximized state and the windowed bounds
/// sampled for session persistence, grouped out of [`App`].
#[derive(Default)]
pub(crate) struct PlacementTracker {
    /// Whether the window is maximized, as of the last sample taken while it was
    /// not minimized — what the saved placement records. Starts as the startup
    /// window's first state (the saved one, or a **Run: Maximized** launch).
    pub(crate) maximized: bool,
    /// The most recent *non-maximized* window placement (outer position + inner
    /// size), tracked from `Moved`/`Resized` events so it's available to persist
    /// on exit. Recorded only while the window isn't maximized, so un-maximizing
    /// a restored session returns to a real window rather than a fullscreen rect.
    pub(crate) last_windowed_bounds: Option<WindowBounds>,
    /// Set when a `Moved`/`Resized` event arrives; the windowed bounds are then
    /// sampled once in `about_to_wait`, after the event burst has settled. This
    /// deferral matters for maximize: winit dispatches `Moved` (from
    /// `WM_WINDOWPOSCHANGED`) *before* the `WM_SIZE` that sets its maximized flag,
    /// so sampling eagerly in the `Moved` handler would record the maximized
    /// geometry as if it were windowed. By `about_to_wait` the flag is set, so
    /// `record_windowed_bounds`'s `is_maximized()` guard sees the real state.
    pub(crate) bounds_dirty: bool,
}
