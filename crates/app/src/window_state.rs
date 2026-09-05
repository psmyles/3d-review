//! Persist the window's last position, size, and maximized state so the viewer
//! reopens where the user left it.
//!
//! The state file lives in the OS's per-user config directory, not next to the
//! executable: an install in `Program Files` is read-only for a non-admin user, so
//! writing config beside the exe would silently fail. Reading and writing here
//! needs no `unsafe`, so this stays in `app` rather than `import` (invariant 9
//! only funnels FFI through `import`).
//!
//! The format is a tiny `key=value` text file — deliberately dependency-free
//! (no serde) for a five-field record. A missing, unreadable, or malformed file
//! is never fatal: load returns `None` and the window falls back to OS defaults.

use std::path::PathBuf;
use std::time::Duration;

use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use crate::{App, FALLBACK_REFRESH_HZ, prof};

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
/// (`mac-port-plan.md` D17). `None` on a platform or environment `dirs` can't
/// answer for, in which case persistence is simply skipped.
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

    prof::msg(&format!("restored window placement: {placement:?}"));
    Some(placement)
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
        prof::msg(&format!(
            "failed to create window-state directory {}: {error}",
            dir.display()
        ));
        return;
    }

    let contents = format!(
        "x={}\ny={}\nwidth={}\nheight={}\nmaximized={}\n",
        placement.x, placement.y, placement.width, placement.height, placement.maximized,
    );

    if let Err(error) = std::fs::write(&path, contents) {
        prof::msg(&format!(
            "failed to write window state {}: {error}",
            path.display()
        ));
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
    let win_left = placement.x;
    let win_top = placement.y;
    let win_right = placement.x + placement.width as i32;
    let win_bottom = placement.y + placement.height as i32;

    let mut any_monitor = false;
    for monitor in event_loop.available_monitors() {
        any_monitor = true;
        let pos = monitor.position();
        let size = monitor.size();
        let mon_left = pos.x;
        let mon_top = pos.y;
        let mon_right = pos.x + size.width as i32;
        let mon_bottom = pos.y + size.height as i32;

        let overlaps = win_left < mon_right
            && win_right > mon_left
            && win_top < mon_bottom
            && win_bottom > mon_top;
        if overlaps {
            return true;
        }
    }

    // No monitors enumerated (rare/headless): don't throw the placement away.
    !any_monitor
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
    // Maximized client width tracks the work area exactly (≈100% with a bottom
    // taskbar, a bit less with a side taskbar); height loses the taskbar band.
    const MIN_WIDTH_FRACTION: f32 = 0.9;
    const MIN_HEIGHT_FRACTION: f32 = 0.85;

    for monitor in event_loop.available_monitors() {
        let pos = monitor.position();
        let size = monitor.size();
        let at_origin = placement.x <= pos.x && placement.y <= pos.y;
        let fills = placement.width as f32 >= size.width as f32 * MIN_WIDTH_FRACTION
            && placement.height as f32 >= size.height as f32 * MIN_HEIGHT_FRACTION;
        if at_origin && fills {
            return true;
        }
    }
    false
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
    use super::state_dir;

    /// `dirs::config_dir()` must resolve to the *same* directory the hand-read
    /// `APPDATA` environment variable used to give (`mac-port-plan.md` D17) — the
    /// point of the swap is a path that also exists on macOS, not a new location
    /// on Windows. Getting this wrong would silently strand every existing user's
    /// saved window placement, with no error and no way to notice but a window
    /// that stopped reopening where it was left.
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
}
