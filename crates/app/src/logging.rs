//! The viewer's log, from `app`'s side: where the day's file goes, the lines a
//! session opens with, and handing the Log window its lines.
//!
//! The logger itself is `review_log`'s. `main` installs it on its first line;
//! [`open`] gives it a file once the GPU bring-up is running; and each frame the
//! Log window is open, [`App::feed_log_window`] copies it whatever has been
//! logged since the last one (invariant 2: the window holds a plain copy, and
//! `app` is what fills it).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use winit::event_loop::EventLoopProxy;

use crate::events::UserEvent;
use crate::{APP_NAME, App};

/// Where the day's log is kept. Logs describe this machine rather than the
/// user, so they stay out of the roaming profile the settings live in:
///
/// - Windows: `%LOCALAPPDATA%\3D Review\logs`
/// - macOS: `~/Library/Logs/3D Review`, where Console.app looks for them
///
/// `cfg!` rather than `#[cfg]`, so both arms are compiled on both platforms.
fn logs_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        dirs::home_dir().map(|home| home.join("Library").join("Logs").join(APP_NAME))
    } else {
        dirs::data_local_dir().map(|local| local.join(APP_NAME).join("logs"))
    }
}

/// Open today's file (removing any earlier day's), wake the event loop through
/// `proxy` whenever a line arrives for an open Log window, and write the lines
/// every session begins with. Returns the file, for the Log window's footer.
pub(crate) fn open(proxy: EventLoopProxy<UserEvent>) -> Option<PathBuf> {
    // Behind a lock only so the closure is `Sync`, which the logger needs to
    // call it from whichever thread logged.
    let proxy = Mutex::new(proxy);
    review_log::set_waker(move || {
        if let Ok(proxy) = proxy.lock() {
            // A closed event loop means the app is exiting; nobody is reading.
            let _ = proxy.send_event(UserEvent::LogUpdated);
        }
    });

    let file = logs_dir().and_then(|dir| match review_log::open_file(&dir) {
        Ok(path) => Some(path),
        Err(error) => {
            log::warn!("could not open a log file in {}: {error}", dir.display());
            None
        }
    });
    log_session_start(file.as_deref());
    file
}

/// What this build is and where it keeps things — the first thing anyone
/// reading a log sent with a bug report needs to know. The graphics adapter
/// joins it once the GPU is up (`init_shell`).
fn log_session_start(file: Option<&Path>) {
    log::info!(
        "{APP_NAME} {} | {}/{}",
        env!("REVIEW_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    match file {
        Some(file) => log::info!("log file: {}", file.display()),
        None => log::info!("log file: none, this session logs to memory only"),
    }
    if let Some(settings) = crate::settings::settings_file() {
        log::info!("settings file: {}", settings.display());
    }
}

impl App {
    /// Before the egui pass: copy the Log window whatever has been logged since
    /// the last frame it was open. A closed window is handed nothing, and
    /// catches up in one go when it opens.
    pub(crate) fn feed_log_window(&mut self) {
        if self.ui.log.open {
            review_log::read_since(&mut self.ui.log.next_seq, &mut self.ui.log.entries);
            self.ui.log.trim();
        }
    }

    /// After the egui pass, which may have opened or closed the window: have
    /// the logger wake the loop for new lines only while the window is up, and
    /// draw once more now if it is behind — the window that opened this frame,
    /// or a line this frame logged.
    pub(crate) fn watch_log_window(&mut self) {
        review_log::set_watching(self.ui.log.open);
        if self.ui.log.open && review_log::has_since(self.ui.log.next_seq) {
            self.redraw.requested = true;
        }
    }
}
