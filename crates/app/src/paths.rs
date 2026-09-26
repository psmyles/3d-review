//! Whether two paths name the same file — the one comparison the recent-files
//! list and the texture watcher both need, and used to spell differently.

use std::path::Path;

/// Whether `a` and `b` name the same file.
///
/// Cheapest answer first. Equal paths are the same file. On Windows a path that
/// differs only in case is too — a file opened once from the dialog and once
/// from a command line typed in another case is one file, and the watcher
/// reports the name as the file system spells it, not as it was imported.
/// Failing both, the canonical forms decide (a relative path, a `..`, a
/// symlink); a path that no longer exists cannot be canonicalized, and is
/// judged by the first two tests alone.
pub(crate) fn same_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    if cfg!(windows) && a.as_os_str().eq_ignore_ascii_case(b.as_os_str()) {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_through_a_parent_directory_is_the_same_file() {
        let dir = std::env::temp_dir();
        let file = dir.join(format!("review-same-file-{}.txt", std::process::id()));
        std::fs::write(&file, b"x").expect("write the probe file");
        let name = file.file_name().expect("the probe has a name");
        let detour = dir
            .join("..")
            .join(dir.file_name().expect("temp dir has a name"))
            .join(name);

        let same = same_file(&file, &detour);
        let _ = std::fs::remove_file(&file);
        assert!(same);
    }

    #[test]
    fn two_files_that_do_not_exist_are_the_same_only_when_equal() {
        let a = Path::new("/no/such/a.fbx");
        assert!(same_file(a, a));
        assert!(!same_file(a, Path::new("/no/such/b.fbx")));
    }

    #[cfg(windows)]
    #[test]
    fn a_path_in_another_case_is_the_same_file_on_windows() {
        assert!(same_file(
            Path::new(r"C:\Models\Barrel.fbx"),
            Path::new(r"c:\models\barrel.FBX")
        ));
    }
}
