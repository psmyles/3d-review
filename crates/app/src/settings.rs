//! The per-user settings file, beside `window.cfg` in the same config directory.
//!
//! It holds the interface language, the menu's **Remember settings** switch and,
//! while that is on, the option-window values it remembers. The format is the
//! same hand-rolled `key=value` one [`crate::window_state`] uses — no serde, no
//! schema, and a line it does not recognise is skipped rather than failing the
//! read, because a settings file that has picked up a key from a newer build must
//! not stop this one starting.
//!
//! The viewer writes only the keys it owns — [`REMEMBER_KEY`] and everything under
//! [`OPTIONS_PREFIX`] — and carries every other line through a save untouched, so
//! a hand-set `locale`, a comment, or a key only a newer build knows survives this
//! one rewriting the file. What the remembered values *are* is `review_ui`'s
//! business ([`UiState::remembered_options`]); this module only owns the file.

use std::path::PathBuf;

use review_ui::UiState;

use crate::{App, prof};

/// The file's name inside `<config dir>/<app name>/`.
const SETTINGS_FILE: &str = "settings.cfg";

/// Whether the option-window values are carried into the next session.
const REMEMBER_KEY: &str = "remember_settings";

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

/// Restore the remembered settings into `ui`, if the user asked for them to be
/// remembered. A value this build cannot read is skipped, leaving its default.
pub(crate) fn restore(ui: &mut UiState) {
    apply_entries(ui, &read_entries());
}

fn apply_entries(ui: &mut UiState, entries: &[(String, String)]) {
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
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let contents = settings_text(&existing, &self.ui);

        if let Some(dir) = path.parent()
            && let Err(error) = std::fs::create_dir_all(dir)
        {
            prof::msg(&format!(
                "failed to create settings directory {}: {error}",
                dir.display()
            ));
            return;
        }
        // Replaced rather than overwritten, as `window.cfg` is: a process killed
        // mid-write would otherwise leave half a file.
        if let Err(error) = review_optimize::write_bytes_replacing(&path, &contents) {
            prof::msg(&format!(
                "failed to save settings {}: {error}",
                path.display()
            ));
        }
    }
}

/// The new file: every line of `existing` the viewer does not own, in order,
/// then the switch and (while it is on) the remembered values.
fn settings_text(existing: &str, ui: &UiState) -> String {
    let mut text = String::new();
    for line in existing.lines() {
        let owned = parse_line(line)
            .is_some_and(|(key, _)| key == REMEMBER_KEY || key.starts_with(OPTIONS_PREFIX));
        if !owned {
            text.push_str(line);
            text.push('\n');
        }
    }
    text.push_str(&format!("{REMEMBER_KEY}={}\n", ui.remember_settings));
    if ui.remember_settings {
        for (key, value) in ui.remembered_options() {
            text.push_str(&format!("{OPTIONS_PREFIX}{key}={value}\n"));
        }
    }
    text
}

fn settings_file() -> Option<PathBuf> {
    let mut path = dirs::config_dir()?;
    path.push(crate::APP_NAME);
    path.push(SETTINGS_FILE);
    Some(path)
}

/// Every `key=value` line of the file with a non-empty value, in file order. An
/// absent or unreadable file is simply empty.
fn read_entries() -> Vec<(String, String)> {
    let Some(path) = settings_file() else {
        return Vec::new();
    };
    let text = std::fs::read_to_string(path).unwrap_or_default();
    parse_entries(&text)
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
        assert!(text.starts_with("# mine\nlocale=de\nfuture=x\nremember_settings=true\n"));
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
        assert_eq!(text, "locale=de\nremember_settings=false\n");
    }
}
