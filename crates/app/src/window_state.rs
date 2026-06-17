//! Persist the window's last position, size, and maximized state so the viewer
//! reopens where the user left it.
//!
//! The state file lives under `%APPDATA%` (the per-user *Roaming* profile), not
//! next to the executable: an install in `Program Files` is read-only for a
//! non-admin user, so writing config beside the exe would silently fail. Reading
//! and writing here needs no `unsafe`, so this stays in `app` rather than
//! `import` (invariant 9 only funnels FFI through `import`).
//!
//! The format is a tiny `key=value` text file — deliberately dependency-free
//! (no serde) for a five-field record. A missing, unreadable, or malformed file
//! is never fatal: load returns `None` and the window falls back to OS defaults.

use std::path::PathBuf;

use tracing::{debug, warn};

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

/// Directory holding the state file: `%APPDATA%\3D Review`. `None` if `APPDATA`
/// isn't set (e.g. an unusual environment), in which case persistence is simply
/// skipped.
fn state_dir() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    let mut path = PathBuf::from(appdata);
    path.push("3D Review");
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

    debug!(?placement, "restored window placement");
    Some(placement)
}

/// Write the placement to `%APPDATA%`. Failures are logged and swallowed —
/// losing window position is not worth interrupting the user.
pub fn save(placement: WindowPlacement) {
    let Some(dir) = state_dir() else {
        return;
    };
    let Some(path) = state_file() else {
        return;
    };

    if let Err(error) = std::fs::create_dir_all(&dir) {
        warn!(%error, dir = %dir.display(), "failed to create window-state directory");
        return;
    }

    let contents = format!(
        "x={}\ny={}\nwidth={}\nheight={}\nmaximized={}\n",
        placement.x, placement.y, placement.width, placement.height, placement.maximized,
    );

    if let Err(error) = std::fs::write(&path, contents) {
        warn!(%error, path = %path.display(), "failed to write window state");
    }
}
