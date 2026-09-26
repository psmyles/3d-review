//! File > Open Recent: the models opened lately, most recent first.
//!
//! The list itself is plain data on [`review_ui::UiState::recent_files`], which the
//! menu draws from; this module is the only thing that changes it. A load that
//! succeeds moves its file to the front, a load that fails because the file has
//! gone drops it, and **Clear Recent Files** empties it. Each change is written
//! to the settings file at once ([`crate::settings`] owns the file), so the
//! history survives a session that never exits cleanly.

use std::path::{Path, PathBuf};

use crate::App;
use crate::paths::same_file;

/// How many files the menu lists. Older entries fall off the end.
pub(crate) const MAX_RECENT_FILES: usize = 10;

/// Move `path` to the front of `list`, dropping any earlier entry for the same
/// file and anything past [`MAX_RECENT_FILES`].
fn push_front(list: &mut Vec<PathBuf>, path: PathBuf) {
    list.retain(|entry| !same_file(entry, &path));
    list.insert(0, path);
    list.truncate(MAX_RECENT_FILES);
}

impl App {
    /// Record a model that has just loaded. The path is made absolute first: one
    /// given relative on the command line would otherwise reopen against whatever
    /// the working directory is next time.
    pub(crate) fn remember_recent_file(&mut self, path: &Path) {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        push_front(&mut self.ui.recent_files, path);
        self.persist_settings();
    }

    /// Drop `path` from the list after a failed load, but only when the file is
    /// no longer there. A file that exists and failed to parse, or was locked for
    /// a moment, stays: the user may well want to try it again.
    pub(crate) fn forget_missing_recent_file(&mut self, path: &Path) {
        if path.exists() {
            return;
        }
        let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        let before = self.ui.recent_files.len();
        self.ui
            .recent_files
            .retain(|entry| !same_file(entry, &absolute));
        if self.ui.recent_files.len() != before {
            self.persist_settings();
        }
    }

    /// File > Open Recent > (a file). Goes through the same funnel as every other
    /// way of opening a model.
    pub(crate) fn open_recent_file(&mut self, index: usize) {
        if let Some(path) = self.ui.recent_files.get(index).cloned() {
            self.open_model_from_path(&path);
        }
    }

    /// File > Open Recent > Clear Recent Files.
    pub(crate) fn clear_recent_files(&mut self) {
        self.ui.recent_files.clear();
        self.persist_settings();
        self.redraw.requested = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reopening a file already in the list moves it to the front rather than
    /// listing it twice.
    #[test]
    fn reopening_a_file_moves_it_to_the_front() {
        let mut list = vec![PathBuf::from("/a.fbx"), PathBuf::from("/b.fbx")];
        push_front(&mut list, PathBuf::from("/b.fbx"));
        assert_eq!(list, [PathBuf::from("/b.fbx"), PathBuf::from("/a.fbx")]);
    }

    /// The list never grows past its cap; the oldest entry is the one to go.
    #[test]
    fn the_oldest_entry_falls_off_the_end() {
        let mut list = Vec::new();
        for i in 0..=MAX_RECENT_FILES {
            push_front(&mut list, PathBuf::from(format!("/{i}.fbx")));
        }
        assert_eq!(list.len(), MAX_RECENT_FILES);
        assert_eq!(list[0], PathBuf::from(format!("/{MAX_RECENT_FILES}.fbx")));
        assert!(!list.contains(&PathBuf::from("/0.fbx")));
    }

    /// On Windows a path that differs only in case is the same file.
    #[cfg(windows)]
    #[test]
    fn a_path_in_another_case_is_the_same_entry() {
        let mut list = vec![PathBuf::from(r"C:\Models\Barrel.fbx")];
        push_front(&mut list, PathBuf::from(r"c:\models\barrel.FBX"));
        assert_eq!(list, [PathBuf::from(r"c:\models\barrel.FBX")]);
    }
}
