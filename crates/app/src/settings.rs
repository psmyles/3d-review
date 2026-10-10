//! The per-user settings file, beside `window.cfg` in the same config directory.
//!
//! It holds the interface language, the menu's **Remember settings** switch and,
//! while that is on, the option-window values it remembers, the **Tracy
//! Profiler** preference (a saved `--tracy`, read once at launch), and the File >
//! Open Recent list (kept whatever the switch says — it is a history, not a tool
//! setting; what goes into it is [`crate::recent`]'s business), and the name
//! review comments are signed with. The format is the
//! same hand-rolled `key=value` one [`crate::window_state`] uses — no serde, no
//! schema, and a line it does not recognise is skipped rather than failing the
//! read, because a settings file that has picked up a key from a newer build must
//! not stop this one starting.
//!
//! The viewer writes only the keys it owns — [`REMEMBER_KEY`], [`TRACY_KEY`],
//! [`RECENT_KEY`] and everything under [`OPTIONS_PREFIX`] — and carries every other
//! line through a save untouched, so
//! a hand-set `locale`, a comment, or a key only a newer build knows survives this
//! one rewriting the file. What the remembered values *are* is `review_ui`'s
//! business ([`UiState::remembered_options`]); this module only owns the file.

use std::path::{Path, PathBuf};

use review_ui::UiState;

use crate::App;
use crate::recent::MAX_RECENT_FILES;

/// The file's name inside `<config dir>/<app name>/`.
const SETTINGS_FILE: &str = "settings.cfg";

/// Whether the option-window values are carried into the next session.
const REMEMBER_KEY: &str = "remember_settings";

/// Whether the Tracy client starts at launch, as if `--tracy` had been passed.
const TRACY_KEY: &str = "tracy_profiler";

/// One recently opened model. The key repeats, one line per file, most recent
/// first — the order the menu lists them in.
const RECENT_KEY: &str = "recent_file";

/// The name review comments are signed with. Kept whatever the remember switch
/// says, like the recent files: it is who the user is, not a tool setting.
const AUTHOR_KEY: &str = "comment_author";

/// The namespace the remembered option-window values are written under, so the
/// save can tell its own lines from everyone else's.
const OPTIONS_PREFIX: &str = "options.";

/// The interface language, if the user has chosen one.
///
/// A BCP 47 tag (`de`, `pt-BR`). It is negotiated against the embedded catalogs
/// rather than matched exactly, so a tag with no catalog of its own lands on the
/// nearest one and finally on English.
pub(crate) fn locale() -> Option<String> {
    read_entries()
        .into_iter()
        .find_map(|(key, value)| (key == "locale").then_some(value))
}

/// Whether the user has asked for Tracy profiling at every launch. Read by `main`
/// before the client would start, which is the only moment the answer matters.
pub(crate) fn tracy_profiler() -> bool {
    read_entries()
        .iter()
        .any(|(key, value)| key == TRACY_KEY && value == "true")
}

/// Restore the saved preferences into `ui`: the Tracy switch and the recent
/// files always, and the option-window values if the user asked for them to be
/// remembered. A value this build cannot read is skipped, leaving its default.
pub(crate) fn restore(ui: &mut UiState) {
    apply_entries(ui, &read_entries());
}

fn apply_entries(ui: &mut UiState, entries: &[(String, String)]) {
    ui.tracy_profiler = entries
        .iter()
        .any(|(key, value)| key == TRACY_KEY && value == "true");
    ui.recent_files = entries
        .iter()
        .filter(|(key, _)| key == RECENT_KEY)
        .map(|(_, value)| PathBuf::from(value))
        .take(MAX_RECENT_FILES)
        .collect();
    ui.comments.author = entries
        .iter()
        .find_map(|(key, value)| (key == AUTHOR_KEY).then(|| value.clone()))
        .unwrap_or_default();
    ui.remember_settings = entries
        .iter()
        .any(|(key, value)| key == REMEMBER_KEY && value == "true");
    if !ui.remember_settings {
        return;
    }
    for (key, value) in entries {
        if let Some(option) = key.strip_prefix(OPTIONS_PREFIX) {
            ui.restore_remembered_option(option, value);
        }
    }
}

impl App {
    /// Write the settings file from the current state: the switch always, and the
    /// option-window values only while it is on — so turning it off is also what
    /// clears them, and the next launch starts from the defaults.
    ///
    /// A gate run neither reads nor writes it: the two builds being compared
    /// have to render the same thing, and whatever this box last remembered is
    /// not that.
    pub(crate) fn persist_settings(&self) {
        if self.gate.is_some() {
            return;
        }
        let Some(path) = settings_file() else {
            return;
        };
        // Every line the viewer does not own is carried through the rewrite, so a
        // file that exists but cannot be read - locked by a sync client or an
        // antivirus scan, or no longer UTF-8 after a hand edit - must not be taken
        // for an empty one: that would save over the user's own lines. Only a
        // file that is not there yet starts from nothing.
        let existing = match read_settings(&path) {
            Ok(existing) => existing,
            Err(error) => {
                log::warn!(
                    "not saving settings: could not read {} to carry its other lines: {error}",
                    path.display()
                );
                return;
            }
        };
        let contents = settings_text(&existing, &self.ui);

        if let Some(dir) = path.parent()
            && let Err(error) = std::fs::create_dir_all(dir)
        {
            log::warn!(
                "failed to create settings directory {}: {error}",
                dir.display()
            );
            return;
        }
        // Replaced rather than overwritten, as `window.cfg` is: a process killed
        // mid-write would otherwise leave half a file.
        if let Err(error) = review_optimize::write_bytes_replacing(&path, &contents) {
            log::warn!("failed to save settings {}: {error}", path.display());
        }
    }
}

/// The new file: every line of `existing` the viewer does not own, in order,
/// then the switches, the recent files and (while the switch is on) the
/// remembered values.
fn settings_text(existing: &str, ui: &UiState) -> String {
    let mut text = String::new();
    for line in existing.lines() {
        let owned = parse_line(line).is_some_and(|(key, _)| {
            key == REMEMBER_KEY
                || key == TRACY_KEY
                || key == RECENT_KEY
                || key == AUTHOR_KEY
                || key.starts_with(OPTIONS_PREFIX)
        });
        if !owned {
            text.push_str(line);
            text.push('\n');
        }
    }
    text.push_str(&format!("{TRACY_KEY}={}\n", ui.tracy_profiler));
    text.push_str(&format!("{REMEMBER_KEY}={}\n", ui.remember_settings));
    // One line: a name with a line break in it could not be read back.
    let author = ui.comments.author.trim();
    if !author.is_empty() && !author.contains(['\n', '\r']) {
        text.push_str(&format!("{AUTHOR_KEY}={author}\n"));
    }
    for path in ui
        .recent_files
        .iter()
        .filter_map(|path| storable_path(path))
    {
        text.push_str(&format!("{RECENT_KEY}={path}\n"));
    }
    if ui.remember_settings {
        for (key, value) in ui.remembered_options() {
            text.push_str(&format!("{OPTIONS_PREFIX}{key}={value}\n"));
        }
    }
    text
}

/// `path` as it can be written to one `key=value` line and read back unchanged,
/// or `None` for the rare path that cannot: one that is not valid Unicode, that
/// holds a line break, or whose ends the parse would trim. Such a file simply
/// goes unremembered.
fn storable_path(path: &Path) -> Option<&str> {
    let text = path.to_str()?;
    let fits = !text.is_empty() && text.trim() == text && !text.contains(['\n', '\r']);
    fits.then_some(text)
}

pub(crate) fn settings_file() -> Option<PathBuf> {
    let mut path = dirs::config_dir()?;
    path.push(crate::APP_NAME);
    path.push(SETTINGS_FILE);
    Some(path)
}

/// The settings file's text: empty when there is no file yet, an error when
/// there is one that cannot be read.
fn read_settings(path: &Path) -> std::io::Result<String> {
    match std::fs::read_to_string(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        other => other,
    }
}

/// Every `key=value` line of the file with a non-empty value, in file order. An
/// absent file is simply empty; an unreadable one is too, with a warning, since
/// the defaults are the only thing to start from either way.
fn read_entries() -> Vec<(String, String)> {
    let Some(path) = settings_file() else {
        return Vec::new();
    };
    match read_settings(&path) {
        Ok(text) => parse_entries(&text),
        Err(error) => {
            log::warn!("could not read settings {}: {error}", path.display());
            Vec::new()
        }
    }
}

fn parse_entries(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(parse_line)
        .filter(|(_, value)| !value.is_empty())
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

/// One line's trimmed key and value, or `None` for a blank line, a comment, or a
/// line with no `=`.
fn parse_line(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    Some((key.trim(), value.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file that is not there yet is empty; one that is there but cannot be
    /// read is an error, which is what stops a save from writing over it.
    #[test]
    fn only_a_missing_file_reads_as_empty() {
        let dir = std::env::temp_dir().join(format!("review-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let missing = dir.join("absent.cfg");
        assert_eq!(read_settings(&missing).expect("a missing file is fine"), "");

        let garbled = dir.join("garbled.cfg");
        std::fs::write(&garbled, [b'l', b'o', 0xFF, 0xFE, b'\n']).expect("write");
        assert!(
            read_settings(&garbled).is_err(),
            "a file that is not UTF-8 must not read as an empty one"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The parse must survive whatever a hand-edited file, or a newer build,
    /// leaves in it: comments, blank lines, keys this build does not know, and a
    /// key present but empty.
    #[test]
    fn a_value_is_read_past_comments_and_unknown_keys() {
        let text = "# language\n\nunknown = 3\nlocale = pt-BR\nempty =\n";
        let entries = parse_entries(text);
        assert_eq!(
            entries,
            [
                ("unknown".to_owned(), "3".to_owned()),
                ("locale".to_owned(), "pt-BR".to_owned()),
            ]
        );
    }

    /// A save keeps every line the viewer does not own, and replaces the ones it
    /// does rather than appending a second copy.
    #[test]
    fn a_save_keeps_foreign_lines_and_replaces_its_own() {
        let existing = "# mine\nlocale=de\nremember_settings=true\noptions.gone=1\nfuture=x\n";
        let mut ui = UiState {
            remember_settings: true,
            ..UiState::default()
        };
        ui.gtao.radius = 2.0;

        let text = settings_text(existing, &ui);
        assert!(text.starts_with(
            "# mine\nlocale=de\nfuture=x\ntracy_profiler=false\nremember_settings=true\n"
        ));
        assert!(!text.contains("options.gone"));
        assert!(text.contains("options.ambient_occlusion.radius=2\n"));
        assert_eq!(text.matches(REMEMBER_KEY).count(), 1);

        // Written back and read again, it restores what was saved.
        let mut restored = UiState::default();
        apply_entries(&mut restored, &parse_entries(&text));
        assert!(restored.remember_settings);
        assert_eq!(restored.remembered_options(), ui.remembered_options());
    }

    /// Turning the switch off is what drops the remembered values, so the next
    /// launch starts from the defaults.
    #[test]
    fn switching_off_drops_the_remembered_values() {
        let existing = "locale=de\nremember_settings=true\noptions.tonemapper.enabled=false\n";
        let text = settings_text(existing, &UiState::default());
        assert_eq!(
            text,
            "locale=de\ntracy_profiler=false\nremember_settings=false\n"
        );
    }

    /// The Tracy switch is its own preference: it is written and read whether or
    /// not the option-window values are being remembered.
    #[test]
    fn the_tracy_switch_survives_without_remember_settings() {
        let ui = UiState {
            tracy_profiler: true,
            ..UiState::default()
        };
        let text = settings_text("tracy_profiler=false\n", &ui);
        assert_eq!(text, "tracy_profiler=true\nremember_settings=false\n");

        let mut restored = UiState::default();
        apply_entries(&mut restored, &parse_entries(&text));
        assert!(restored.tracy_profiler);
        assert!(!restored.remember_settings);
    }

    /// The recent files are kept in order whether or not the option-window
    /// values are being remembered, a save replaces the old list rather than
    /// adding to it, and a path with an `=` in it survives the `key=value` split.
    #[test]
    fn recent_files_round_trip_in_order() {
        let ui = UiState {
            recent_files: vec![
                PathBuf::from("/assets/a=b/Barrel.fbx"),
                PathBuf::from("/assets/Crate.fbx"),
            ],
            ..UiState::default()
        };
        let text = settings_text("recent_file=/old.fbx\n", &ui);
        assert!(!text.contains("old.fbx"));
        assert!(!text.contains(OPTIONS_PREFIX));

        let mut restored = UiState::default();
        apply_entries(&mut restored, &parse_entries(&text));
        assert_eq!(restored.recent_files, ui.recent_files);
    }

    /// A path the one-line format cannot carry is left out rather than written
    /// back as something else.
    #[test]
    fn an_unstorable_path_is_skipped() {
        let ui = UiState {
            recent_files: vec![
                PathBuf::from("/models/ends in a space "),
                PathBuf::from("/models/two\nlines.fbx"),
                PathBuf::from("/models/fine.fbx"),
            ],
            ..UiState::default()
        };
        let text = settings_text("", &ui);
        assert_eq!(text.matches(RECENT_KEY).count(), 1);
        assert!(text.contains("recent_file=/models/fine.fbx\n"));
    }
}
