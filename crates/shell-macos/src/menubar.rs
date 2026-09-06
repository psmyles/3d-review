//! The macOS menu bar (`mac-port-plan.md` D15).
//!
//! A Mac app without a menu bar reads as broken and, more concretely, cannot be
//! quit: ⌘Q belongs to the menu, not the window, so without one there is no way out
//! but Force Quit. This builds the minimum that makes the viewer behave like a Mac
//! app — an application menu, a File menu and a Window menu — and routes the two
//! items that are *ours* rather than AppKit's back through the caller's callback,
//! into the same handlers the ⌘O / ⌘N chords in `shortcuts.rs` already reach.
//!
//! Everything else here is a `muda` **predefined** item, which is deliberate: those
//! map onto AppKit's own responder-chain selectors (`terminate:`, `hide:`,
//! `performMiniaturize:` …), so they behave exactly as macOS users expect, need no
//! routing of ours, and cannot desynchronise from app state because they never touch
//! it.
//!
//! **Accelerators here intercept keys before winit ever sees them.** That is why only
//! the two app items carry one, and why each carries exactly the chord
//! `shortcuts.rs` binds: a menu accelerator that disagreed with the keyboard handler
//! would silently shadow it, and the shortcut would look broken with no way to tell
//! why. The other file-command chords (⌘Z / ⌘⇧Z / ⌘Y) get no menu item precisely so
//! they keep reaching winit — there is no Edit menu to put them in that would not
//! also have to mirror the undo stack's state.

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{AboutMetadata, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};

use crate::{About, MenuCommand};

/// Menu-item ids. Only the items the viewer performs itself need one; the predefined
/// items are AppKit's.
const ID_OPEN: &str = "review.open";
const ID_NEW: &str = "review.new";

/// See [`crate::install_menu_bar`].
pub(crate) fn install(
    about: About<'_>,
    on_command: impl Fn(MenuCommand) + Send + Sync + 'static,
) -> Option<Menu> {
    let product = about.product;
    let metadata = AboutMetadata {
        name: Some(product.to_string()),
        version: Some(about.version.to_string()),
        copyright: Some(about.copyright.to_string()),
        ..Default::default()
    };

    let app_menu = Submenu::with_items(
        product,
        true,
        &[
            &PredefinedMenuItem::about(Some(&format!("About {product}")), Some(metadata)),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(Some(&format!("Hide {product}"))),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(Some(&format!("Quit {product}"))),
        ],
    )
    .ok()?;

    let file_menu = Submenu::with_items(
        "File",
        true,
        &[
            &MenuItem::with_id(ID_NEW, "New", true, accelerator(Code::KeyN)),
            &MenuItem::with_id(ID_OPEN, "Open…", true, accelerator(Code::KeyO)),
        ],
    )
    .ok()?;

    // No Full Screen item: the viewer has no full-screen state of its own for
    // AppKit's `toggleFullScreen:` to get out of step with, and adding one here
    // would be inventing a feature rather than porting one. Minimize and Zoom are
    // both pure AppKit.
    let window_menu = Submenu::with_items(
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::maximize(None),
        ],
    )
    .ok()?;

    let menu = Menu::with_items(&[&app_menu, &file_menu, &window_menu]).ok()?;
    menu.init_for_nsapp();

    // muda delivers on its own channel; the callback hands each event to the event
    // loop so it is processed on the main thread with everything else, in order,
    // rather than racing the viewer's state.
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Some(command) = command_for(&event.id) {
            on_command(command);
        }
    }));

    Some(menu)
}

/// The [`MenuCommand`] an item id stands for.
fn command_for(id: &MenuId) -> Option<MenuCommand> {
    match id.as_ref() {
        ID_OPEN => Some(MenuCommand::Open),
        ID_NEW => Some(MenuCommand::New),
        _ => None,
    }
}

/// `key` chorded with the primary modifier — ⌘ here, the same key
/// `shortcuts.rs`'s `primary_held` tests for on this OS (D10).
fn accelerator(key: Code) -> Option<Accelerator> {
    Some(Accelerator::new(Some(Modifiers::META), key))
}
