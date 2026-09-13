//! The per-user settings file, beside `window.cfg` in the same config directory.
//!
//! Read-only for now: it holds the interface language, and nothing in the viewer
//! writes it yet. The format is the same hand-rolled `key=value` one
//! [`crate::window_state`] uses — no serde, no schema, and a line it does not
//! recognise is skipped rather than failing the read, because a settings file
//! that has picked up a key from a newer build must not stop this one starting.
//!
//! A Preferences window is what will eventually write it; the format is chosen so
//! that when it does, an older build still reads what it understands.

use std::path::PathBuf;

/// The file's name inside `<config dir>/<app name>/`.
const SETTINGS_FILE: &str = "settings.cfg";

/// The interface language, if the user has chosen one.
///
/// A BCP 47 tag (`de`, `pt-BR`). It is negotiated against the embedded catalogs
/// rather than matched exactly, so a tag with no catalog of its own lands on the
/// nearest one and finally on English.
pub(crate) fn locale() -> Option<String> {
    read_value("locale")
}

fn settings_file() -> Option<PathBuf> {
    let mut path = dirs::config_dir()?;
    path.push(crate::APP_NAME);
    path.push(SETTINGS_FILE);
    Some(path)
}

fn read_value(key: &str) -> Option<String> {
    let text = std::fs::read_to_string(settings_file()?).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((name, value)) = line.split_once('=')
            && name.trim() == key
        {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    /// The parse must survive whatever a hand-edited file, or a newer build,
    /// leaves in it: comments, blank lines, keys this build does not know, and a
    /// key present but empty.
    #[test]
    fn a_value_is_read_past_comments_and_unknown_keys() {
        let text = "# language\n\nunknown = 3\nlocale = pt-BR\nempty =\n";
        let value = text.lines().find_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (name, value) = line.split_once('=')?;
            (name.trim() == "locale").then(|| value.trim().to_owned())
        });
        assert_eq!(value.as_deref(), Some("pt-BR"));
    }
}
