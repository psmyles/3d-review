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
//! callback and gets back paths and a two-variant [`MenuCommand`]; that is the whole
//! interface, and it is why this crate depends on neither `winit` nor any crate of
//! ours.
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
/// Deliberately tiny and named for the *command*, not the menu item: it is the
/// same pair of actions the primary-modifier chords in `shortcuts.rs` fire, and the
/// menu is a second door onto them rather than a second implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCommand {
    /// File → Open… (⌘O)
    Open,
    /// File → New (⌘N): back to the empty start state.
    New,
}

/// The product identity the About panel shows, read from `product.json` by `app`'s
/// build script so this crate needs no file access of its own.
#[derive(Debug, Clone, Copy)]
pub struct About<'a> {
    pub product: &'a str,
    pub version: &'a str,
    pub copyright: &'a str,
}

/// The installed menu bar. Opaque on purpose — `app` only has to keep it alive,
/// and dropping it takes the menu bar down with it.
///
/// Holding muda's `Menu` inside rather than handing it out is what keeps muda from
/// becoming a dependency of `app`.
pub struct MenuBar {
    #[cfg(target_os = "macos")]
    _menu: muda::Menu,
}

/// Build and install the application menu bar, routing its own two items to
/// `on_command`.
///
/// Must run on the main thread, after the event loop exists (`NSApplication` has to
/// be up). `None` means the menu could not be built, which leaves the app running
/// without one rather than failing to start.
///
/// Off macOS this is `None` and nothing is built: Windows has no application menu
/// bar and never wanted one.
#[cfg(target_os = "macos")]
pub fn install_menu_bar(
    about: About<'_>,
    on_command: impl Fn(MenuCommand) + Send + Sync + 'static,
) -> Option<MenuBar> {
    menubar::install(about, on_command).map(|menu| MenuBar { _menu: menu })
}

/// The non-macOS stub. See the macOS arm.
#[cfg(not(target_os = "macos"))]
pub fn install_menu_bar(
    about: About<'_>,
    on_command: impl Fn(MenuCommand) + Send + Sync + 'static,
) -> Option<MenuBar> {
    let _ = (about, on_command);
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
