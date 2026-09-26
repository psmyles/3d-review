//! The day's file: what it is called, the header each session opens with, and
//! the cleanup that keeps no log past the day it was written on.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDate};

/// Every log file's name begins with this, and the cleanup touches nothing
/// that does not — the folder is the user's, and anything else in it is theirs.
const PREFIX: &str = "3d-review-";
const EXTENSION: &str = ".log";
const DATE_FORMAT: &str = "%Y-%m-%d";

/// `3d-review-2026-09-26.log`.
pub(crate) fn file_name(date: NaiveDate) -> String {
    format!("{PREFIX}{}{EXTENSION}", date.format(DATE_FORMAT))
}

/// The date a log file was written on, or `None` for a file that is not one.
fn date_of(name: &str) -> Option<NaiveDate> {
    let date = name.strip_prefix(PREFIX)?.strip_suffix(EXTENSION)?;
    NaiveDate::parse_from_str(date, DATE_FORMAT).ok()
}

/// Delete every log file in `dir` written on a day other than `today`, and say
/// what happened to each. A file dated *after* today (a clock that has since
/// been put back) goes too: it is not today's, and nothing else would ever
/// remove it. A file another running session still has open is deleted when
/// that session closes it (Rust opens files shareable for deletion).
pub(crate) fn prune(dir: &Path, today: NaiveDate) -> Vec<(PathBuf, std::io::Result<()>)> {
    let Ok(listing) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    listing
        .flatten()
        .filter(|item| item.file_type().is_ok_and(|kind| kind.is_file()))
        .filter(|item| {
            item.file_name()
                .to_str()
                .and_then(date_of)
                .is_some_and(|date| date != today)
        })
        .map(|item| {
            let path = item.path();
            let outcome = std::fs::remove_file(&path);
            (path, outcome)
        })
        .collect()
}

/// The line a session opens its part of the file with, so the sessions sharing
/// one day's file can be told apart. `continuing` puts a blank line between it
/// and the session before.
pub(crate) fn session_header(now: DateTime<Local>, pid: u32, continuing: bool) -> String {
    let gap = if continuing { "\n" } else { "" };
    format!(
        "{gap}===== session started {} (process {pid}) =====\n",
        now.format("%Y-%m-%d %H:%M:%S")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, DATE_FORMAT).unwrap()
    }

    #[test]
    fn a_name_carries_its_date_both_ways() {
        let day = date("2026-09-26");
        assert_eq!(file_name(day), "3d-review-2026-09-26.log");
        assert_eq!(date_of(&file_name(day)), Some(day));
    }

    #[test]
    fn only_our_names_parse() {
        assert_eq!(date_of("3d-review-2026-09-26.txt"), None);
        assert_eq!(date_of("other-2026-09-26.log"), None);
        assert_eq!(date_of("3d-review-yesterday.log"), None);
    }

    /// Yesterday's file and a future-dated one go; today's stays, and so does
    /// everything that is not one of ours.
    #[test]
    fn pruning_keeps_today_and_everything_not_ours() {
        let dir = std::env::temp_dir().join(format!("review-log-prune-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let today = date("2026-09-26");
        let names = [
            "3d-review-2026-09-25.log",
            "3d-review-2026-09-26.log",
            "3d-review-2026-09-27.log",
            "notes.txt",
            "3d-review-crash.log",
        ];
        for name in names {
            std::fs::write(dir.join(name), "x").unwrap();
        }

        let removed = prune(&dir, today);
        assert_eq!(removed.len(), 2);
        assert!(removed.iter().all(|(_, outcome)| outcome.is_ok()));
        assert!(!dir.join("3d-review-2026-09-25.log").exists());
        assert!(!dir.join("3d-review-2026-09-27.log").exists());
        for kept in [
            "3d-review-2026-09-26.log",
            "notes.txt",
            "3d-review-crash.log",
        ] {
            assert!(dir.join(kept).exists(), "{kept} should be kept");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
