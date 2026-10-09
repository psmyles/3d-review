//! The macOS menu bar (`docs/ARCHITECTURE.md`, Platform decisions D15).
//!
//! A Mac app without a menu bar reads as broken and, more concretely, cannot be
//! quit: ⌘Q belongs to the menu, not the window, so without one there is no way out
//! but Force Quit. This builds the menu bar the viewer's own toolbar menu implies,
//! with each entry where a Mac puts it rather than where the toolbar does: About and
//! the Remember Settings preference go in the application menu, the toolbar's Exit
//! *is* Quit (ours rather than AppKit's, so it can ask about unsaved comments), and
//! Help is AppKit's Help menu (which is what gives it the search
//! field). Every item that is *ours* routes back through the caller's callback as a
//! [`MenuCommand`], into the handler the toolbar's entry already reaches.
//!
//! The rest are `muda` **predefined** items, which is deliberate: those map onto
//! AppKit's own responder-chain selectors (`terminate:`, `hide:`,
//! `performMiniaturize:` …), so they behave exactly as macOS users expect, need no
//! routing of ours, and cannot desynchronise from app state because they never touch
//! it. What *can* desynchronise — the two check marks and the Open Recent list — is
//! brought back in line by [`Installed::sync`].
//!
//! **Accelerators here intercept keys before winit ever sees them.** That is why only
//! Open, New, Save and Save As carry one (and Quit, which `shortcuts.rs` leaves
//! alone), and why each carries exactly the chord `shortcuts.rs` binds: a menu accelerator that disagreed with the keyboard handler would silently
//! shadow it, and the shortcut would look broken with no way to tell why. The other
//! file-command chords (⌘Z / ⌘⇧Z / ⌘Y) get no menu item precisely so they keep
//! reaching winit — there is no Edit menu to put them in that would not also have to
//! mirror the undo stack's state. Documentation deliberately carries no F1 either:
//! F1 opens the page for whatever the pointer is over, which a menu cannot know.

use std::path::{Path, PathBuf};

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};

use crate::{MenuCommand, MenuState};

/// The menu's typed message keys, generated from the `menu-` half of the English
/// catalog by this crate's build script (invariant 12).
mod keys {
    include!(concat!(env!("OUT_DIR"), "/keys.rs"));
}

// Menu-item ids. Only the items the viewer performs itself need one; the predefined
// items are AppKit's.
const ID_ABOUT: &str = "review.about";
const ID_REMEMBER_SETTINGS: &str = "review.remember-settings";
const ID_OPEN: &str = "review.open";
const ID_NEW: &str = "review.new";
const ID_SAVE: &str = "review.save";
const ID_SAVE_AS: &str = "review.save-as";
const ID_QUIT: &str = "review.quit";
const ID_CLEAR_RECENT: &str = "review.clear-recent";
const ID_VIEW_LOG: &str = "review.view-log";
const ID_TRACY_PROFILER: &str = "review.tracy-profiler";
const ID_DOCUMENTATION: &str = "review.documentation";
const ID_CHECK_FOR_UPDATES: &str = "review.check-for-updates";
const ID_REPORT_ISSUE: &str = "review.report-issue";
const ID_CREDITS: &str = "review.credits";
/// An Open Recent entry's id is this plus its index in the list.
const ID_RECENT_PREFIX: &str = "review.recent.";

/// The installed menu bar, and the items whose state follows the app's.
pub(crate) struct Installed {
    _menu: Menu,
    remember_settings: CheckMenuItem,
    tracy_profiler: CheckMenuItem,
    open_recent: Submenu,
    save: MenuItem,
    save_as: MenuItem,
    /// The list Open Recent was last built from; `None` until the first sync, so
    /// that one always builds it.
    recent_shown: Option<Vec<PathBuf>>,
}

impl Installed {
    /// See [`crate::MenuBar::sync`].
    pub(crate) fn sync(&mut self, state: MenuState<'_>) {
        // Compared against the mark itself rather than a cached value: AppKit
        // flips a check item's mark the moment it is clicked, before the command
        // has reached `app`, so only the item knows what it is showing.
        sync_check(&self.remember_settings, state.remember_settings);
        sync_check(&self.tracy_profiler, state.tracy_profiler);
        if self.save.is_enabled() != state.can_save {
            self.save.set_enabled(state.can_save);
            self.save_as.set_enabled(state.can_save);
        }
        if self.recent_shown.as_deref() != Some(state.recent_files) {
            self.rebuild_open_recent(state.recent_files);
            self.recent_shown = Some(state.recent_files.to_vec());
        }
    }

    /// Open Recent: one entry per recent model, most recent first, named by its
    /// file name, then the command that empties it — the toolbar's list, item for
    /// item. Greyed out rather than hidden while there is no history, so the File
    /// menu keeps its shape.
    fn rebuild_open_recent(&self, recent_files: &[PathBuf]) {
        while self.open_recent.remove_at(0).is_some() {}
        for (index, path) in recent_files.iter().enumerate() {
            let item = MenuItem::with_id(
                format!("{ID_RECENT_PREFIX}{index}"),
                file_name(path),
                true,
                None,
            );
            let _ = self.open_recent.append(&item);
        }
        let _ = self.open_recent.append_items(&[
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id(
                ID_CLEAR_RECENT,
                review_localization::tr(keys::menu::CLEAR_RECENT_FILES),
                true,
                None,
            ),
        ]);
        self.open_recent.set_enabled(!recent_files.is_empty());
    }
}

/// See [`crate::install_menu_bar`].
pub(crate) fn install(
    product: &str,
    on_command: impl Fn(MenuCommand) + Send + Sync + 'static,
) -> Option<Installed> {
    // Both start unticked; the first `sync` sets them from the app's state.
    let remember_settings = CheckMenuItem::with_id(
        ID_REMEMBER_SETTINGS,
        review_localization::tr(keys::menu::REMEMBER_SETTINGS),
        true,
        false,
        None,
    );
    let tracy_profiler = CheckMenuItem::with_id(
        ID_TRACY_PROFILER,
        review_localization::tr(keys::menu::TRACY_PROFILER),
        true,
        false,
        None,
    );
    // Filled by the first `sync`.
    let open_recent = Submenu::new(review_localization::tr(keys::menu::OPEN_RECENT), false);
    // Enabled by `sync` once a file that can carry comments is loaded.
    let save = MenuItem::with_id(
        ID_SAVE,
        review_localization::tr(keys::menu::SAVE),
        false,
        accelerator(Code::KeyS),
    );
    let save_as = MenuItem::with_id(
        ID_SAVE_AS,
        review_localization::tr(keys::menu::SAVE_AS),
        false,
        Some(Accelerator::new(
            Some(Modifiers::META | Modifiers::SHIFT),
            Code::KeyS,
        )),
    );

    // About opens the viewer's own About box rather than AppKit's standard panel,
    // so there is one About whichever menu it is reached from — and it is the one
    // that also says what the viewer is drawing with.
    let app_menu = Submenu::with_items(
        product,
        true,
        &[
            &item(ID_ABOUT, &keys::menu::about(product)),
            &PredefinedMenuItem::separator(),
            &remember_settings,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(Some(&review_localization::tr(keys::menu::SERVICES))),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(Some(&keys::menu::hide(product))),
            &PredefinedMenuItem::hide_others(Some(&review_localization::tr(
                keys::menu::HIDE_OTHERS,
            ))),
            &PredefinedMenuItem::show_all(Some(&review_localization::tr(keys::menu::SHOW_ALL))),
            &PredefinedMenuItem::separator(),
            // Ours, not AppKit's: `terminate:` would end the process without
            // asking about unsaved comments.
            &MenuItem::with_id(
                ID_QUIT,
                keys::menu::quit(product),
                true,
                accelerator(Code::KeyQ),
            ),
        ],
    )
    .ok()?;

    let file_menu = Submenu::with_items(
        review_localization::tr(keys::menu::FILE).as_ref(),
        true,
        &[
            &MenuItem::with_id(
                ID_NEW,
                review_localization::tr(keys::menu::NEW),
                true,
                accelerator(Code::KeyN),
            ),
            &MenuItem::with_id(
                ID_OPEN,
                review_localization::tr(keys::menu::OPEN),
                true,
                accelerator(Code::KeyO),
            ),
            &open_recent,
            &PredefinedMenuItem::separator(),
            &save,
            &save_as,
        ],
    )
    .ok()?;

    let debug_menu = Submenu::with_items(
        review_localization::tr(keys::menu::DEBUG).as_ref(),
        true,
        &[
            &item(ID_VIEW_LOG, &review_localization::tr(keys::menu::VIEW_LOG)),
            &tracy_profiler,
        ],
    )
    .ok()?;

    // No Full Screen item of ours: the viewer has no full-screen state of its own
    // for AppKit's `toggleFullScreen:` to get out of step with, and adding one here
    // would be inventing a feature rather than porting one. Minimize and Zoom are
    // both pure AppKit, as is everything AppKit itself adds to a registered Window
    // menu (Fill, Center, the tiling items, the window list).
    let window_menu = Submenu::with_items(
        review_localization::tr(keys::menu::WINDOW).as_ref(),
        true,
        &[
            &PredefinedMenuItem::minimize(Some(&review_localization::tr(keys::menu::MINIMIZE))),
            &PredefinedMenuItem::maximize(Some(&review_localization::tr(keys::menu::ZOOM))),
        ],
    )
    .ok()?;

    let help_menu = Submenu::with_items(
        review_localization::tr(keys::menu::HELP).as_ref(),
        true,
        &[
            &item(
                ID_DOCUMENTATION,
                &review_localization::tr(keys::menu::DOCUMENTATION),
            ),
            &PredefinedMenuItem::separator(),
            &item(
                ID_CHECK_FOR_UPDATES,
                &review_localization::tr(keys::menu::CHECK_FOR_UPDATES),
            ),
            &item(
                ID_REPORT_ISSUE,
                &review_localization::tr(keys::menu::REPORT_ISSUE),
            ),
            &item(ID_CREDITS, &review_localization::tr(keys::menu::CREDITS)),
        ],
    )
    .ok()?;

    let menu =
        Menu::with_items(&[&app_menu, &file_menu, &debug_menu, &window_menu, &help_menu]).ok()?;
    menu.init_for_nsapp();
    // Registered with AppKit, which is what makes the Window menu list the open
    // window and gives the Help menu its search field.
    window_menu.set_as_windows_menu_for_nsapp();
    help_menu.set_as_help_menu_for_nsapp();

    // muda delivers on its own channel; the callback hands each event to the event
    // loop so it is processed on the main thread with everything else, in order,
    // rather than racing the viewer's state.
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Some(command) = command_for(&event.id) {
            on_command(command);
        }
    }));

    Some(Installed {
        _menu: menu,
        remember_settings,
        tracy_profiler,
        open_recent,
        save,
        save_as,
        recent_shown: None,
    })
}

/// The [`MenuCommand`] an item id stands for.
fn command_for(id: &MenuId) -> Option<MenuCommand> {
    let id = id.as_ref();
    if let Some(index) = id.strip_prefix(ID_RECENT_PREFIX) {
        return index.parse().ok().map(MenuCommand::OpenRecent);
    }
    Some(match id {
        ID_ABOUT => MenuCommand::About,
        ID_REMEMBER_SETTINGS => MenuCommand::ToggleRememberSettings,
        ID_OPEN => MenuCommand::Open,
        ID_NEW => MenuCommand::New,
        ID_SAVE => MenuCommand::Save,
        ID_SAVE_AS => MenuCommand::SaveAs,
        ID_QUIT => MenuCommand::Quit,
        ID_CLEAR_RECENT => MenuCommand::ClearRecentFiles,
        ID_VIEW_LOG => MenuCommand::ViewLog,
        ID_TRACY_PROFILER => MenuCommand::ToggleTracyProfiler,
        ID_DOCUMENTATION => MenuCommand::Documentation,
        ID_CHECK_FOR_UPDATES => MenuCommand::CheckForUpdates,
        ID_REPORT_ISSUE => MenuCommand::ReportIssue,
        ID_CREDITS => MenuCommand::Credits,
        _ => return None,
    })
}

/// A plain, always-enabled item of ours with no accelerator.
fn item(id: &'static str, text: &str) -> MenuItem {
    MenuItem::with_id(id, text, true, None)
}

/// Set `check`'s mark to `checked` if it is not showing it already.
fn sync_check(check: &CheckMenuItem, checked: bool) {
    if check.is_checked() != checked {
        check.set_checked(checked);
    }
}

/// What Open Recent calls `path`: its file name, as the toolbar's list does.
fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// `key` chorded with the primary modifier — ⌘ here, the same key
/// `shortcuts.rs`'s `primary_held` tests for on this OS (D10).
fn accelerator(key: Code) -> Option<Accelerator> {
    Some(Accelerator::new(Some(Modifiers::META), key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_item_id_names_its_command() {
        let ids = [
            (ID_ABOUT, MenuCommand::About),
            (ID_REMEMBER_SETTINGS, MenuCommand::ToggleRememberSettings),
            (ID_OPEN, MenuCommand::Open),
            (ID_NEW, MenuCommand::New),
            (ID_CLEAR_RECENT, MenuCommand::ClearRecentFiles),
            (ID_VIEW_LOG, MenuCommand::ViewLog),
            (ID_TRACY_PROFILER, MenuCommand::ToggleTracyProfiler),
            (ID_DOCUMENTATION, MenuCommand::Documentation),
            (ID_CHECK_FOR_UPDATES, MenuCommand::CheckForUpdates),
            (ID_REPORT_ISSUE, MenuCommand::ReportIssue),
            (ID_CREDITS, MenuCommand::Credits),
        ];
        for (id, command) in ids {
            assert_eq!(command_for(&MenuId::new(id)), Some(command), "{id}");
        }
    }

    #[test]
    fn recent_ids_carry_their_index() {
        let id = MenuId::new(format!("{ID_RECENT_PREFIX}3"));
        assert_eq!(command_for(&id), Some(MenuCommand::OpenRecent(3)));
        assert_eq!(
            command_for(&MenuId::new(format!("{ID_RECENT_PREFIX}x"))),
            None
        );
        assert_eq!(command_for(&MenuId::new("unknown")), None);
    }
}
