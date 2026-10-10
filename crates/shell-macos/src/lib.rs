//! The macOS shell leaves: Launch Services opens (D14) and the menu bar (D15).
//!
//! Its own crate rather than a module of `review-app` for the same reason
//! `review-psd` and `render`'s `rhi/backend/` are separate: the open hook needs
//! `unsafe` — adding a method to winit's application delegate is `class_addMethod`
//! and nothing else — and invariant 9 keeps `crates/app` at
//! `#![forbid(unsafe_code)]`. Confining it here means the shell's one `unsafe` site
//! is named and bounded, exactly like the GPU's and the importers'.
//!
//! Nothing here knows what a model, a camera or a frame is. `app` hands over a
//! callback and gets back paths and a [`MenuCommand`], and hands the menu bar the
//! few plain values it shows ([`MenuState`]); that is the whole interface, and it
//! is why this crate depends on neither `winit` nor any crate of ours.
//!
//! Every item is a no-op stub off macOS, so `app` calls them unconditionally and
//! carries no `cfg` of its own.

#[cfg(target_os = "macos")]
mod menubar;
#[cfg(target_os = "macos")]
mod openfiles;

use std::path::PathBuf;

/// A menu item `app` performs itself, as opposed to the predefined ones AppKit
/// handles through its own responder chain.
///
/// Named for the *command*, not the menu item: these are the entries of the
/// toolbar's own menu (`review-ui`'s `MenuIntent`, plus the few it acts on in
/// place), and the menu bar is a second door onto them rather than a second
/// implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCommand {
    /// App → About: the viewer's own About box.
    About,
    /// App → Remember Settings.
    ToggleRememberSettings,
    /// File → Open… (⌘O)
    Open,
    /// File → Open Recent → the model at this index of the list last handed to
    /// [`MenuBar::sync`].
    OpenRecent(usize),
    /// File → Open Recent → Clear Recent Files.
    ClearRecentFiles,
    /// File → New (⌘N): back to the empty start state.
    New,
    /// Debug → View Log.
    ViewLog,
    /// Debug → Tracy Profiler.
    ToggleTracyProfiler,
    /// Help → Documentation: the manual's contents page.
    Documentation,
    /// Help → Check for Updates.
    CheckForUpdates,
    /// Help → Report an Issue.
    ReportIssue,
    /// Help → Credits.
    Credits,
}

/// The app state the menu bar shows: its two check items and the Open Recent list.
///
/// Borrowed plain values, handed over by [`MenuBar::sync`], so this crate still
/// knows nothing of where they live.
#[derive(Debug, Clone, Copy)]
pub struct MenuState<'a> {
    pub remember_settings: bool,
    pub tracy_profiler: bool,
    pub recent_files: &'a [PathBuf],
}

/// The installed menu bar. Opaque on purpose — `app` keeps it alive and hands it
/// the state to show, and dropping it takes the menu bar down with it.
///
/// Holding muda's items inside rather than handing them out is what keeps muda
/// from becoming a dependency of `app`.
pub struct MenuBar {
    #[cfg(target_os = "macos")]
    inner: menubar::Installed,
}

impl MenuBar {
    /// Bring the check marks and the Open Recent list in line with `state`.
    ///
    /// Cheap when nothing changed — two reads of a check mark and a list compare —
    /// so `app` calls it after every batch of events rather than tracking which
    /// ones could have changed it.
    pub fn sync(&mut self, state: MenuState<'_>) {
        #[cfg(target_os = "macos")]
        self.inner.sync(state);
        #[cfg(not(target_os = "macos"))]
        let _ = state;
    }
}

/// Build and install the application menu bar, routing the viewer's own items to
/// `on_command`. `product` is the name the application menu and its items carry.
///
/// Must run on the main thread, once the event loop is *running* — winit's
/// `StartCause::Init`. Earlier is too early: winit's `applicationDidFinishLaunching:`
/// installs its own default menu, over whatever was there. `None` means the menu could not be built, which leaves the app running
/// without one rather than failing to start.
///
/// Off macOS this is `None` and nothing is built: Windows has no application menu
/// bar and never wanted one — the toolbar's menu is the only one there.
#[cfg(target_os = "macos")]
pub fn install_menu_bar(
    product: &str,
    on_command: impl Fn(MenuCommand) + Send + Sync + 'static,
) -> Option<MenuBar> {
    menubar::install(product, on_command).map(|inner| MenuBar { inner })
}

/// The non-macOS stub. See the macOS arm.
#[cfg(not(target_os = "macos"))]
pub fn install_menu_bar(
    product: &str,
    on_command: impl Fn(MenuCommand) + Send + Sync + 'static,
) -> Option<MenuBar> {
    let _ = (product, on_command);
    None
}

/// Teach winit's application delegate to answer `application:openURLs:`, so a
/// Finder double-click, an `open(1)` and a drop on the Dock icon all reach
/// `on_open` (D14).
///
/// Returns `false` if the hook could not be installed, which leaves the app working
/// exactly as it did before — openable from the command line, deaf to Finder.
///
/// Call after the event loop is built (the delegate has to exist) and before it
/// runs. Opens that arrive *before* the first window — a launch-by-open delivers the
/// event between `applicationWillFinishLaunching:` and `applicationDidFinishLaunching:`
/// — are held until [`take_pending_opens`] drains them.
///
/// Off macOS this is `false` and installs nothing: a double-clicked file arrives as
/// `argv[1]` there, which `main` already reads.
#[cfg(target_os = "macos")]
pub fn install_open_handler(on_open: impl Fn(PathBuf) + 'static) -> bool {
    openfiles::install(on_open)
}

/// The non-macOS stub. See the macOS arm.
#[cfg(not(target_os = "macos"))]
pub fn install_open_handler(on_open: impl Fn(PathBuf) + 'static) -> bool {
    let _ = on_open;
    false
}

/// The first window now exists: take the opens that arrived before it did, and route
/// everything after this straight to the callback.
///
/// Called once, from where the window is created, so a launch-by-open comes up
/// *showing* the file rather than coming up empty and loading it a frame later.
#[cfg(target_os = "macos")]
pub fn take_pending_opens() -> Vec<PathBuf> {
    openfiles::take_pending()
}

/// The non-macOS stub. See the macOS arm.
#[cfg(not(target_os = "macos"))]
pub fn take_pending_opens() -> Vec<PathBuf> {
    Vec::new()
}
