//! Writing a file by replacing it, so a failed write never destroys what was
//! already there.
//!
//! ## Why this exists
//!
//! Every writer in this workspace — the FBX export through ufbx_write, a saved
//! preset, the window placement — opened its destination directly. `fopen(…,
//! "wb")` truncates *before* the first byte is written, so a disk that fills up,
//! a process that is killed, or a serializer that errors halfway leaves the
//! user's previous export as a fragment. Re-exporting over a good asset is
//! exactly the moment that matters: the file being overwritten is the one the
//! user would have fallen back to.
//!
//! ## The shape
//!
//! [`write_replacing`] hands the writer a temporary sibling, and only if that
//! writer returns `Ok` does it rename the temp over the destination. A rename
//! within one directory is the closest thing either OS gives us to an atomic
//! swap, and `std::fs::rename` replaces an existing file on Windows as well as
//! Unix. On any failure the temp is removed and the destination is untouched,
//! bit for bit.
//!
//! The temp is a *sibling*, not a file in the system temp directory: a rename
//! across volumes is a copy, which is neither atomic nor free for a 200 MB FBX.
//!
//! This is deliberately not a `tempfile` dependency — the whole mechanism is
//! twenty lines of `std`, and the crate's other file work is `std` too.

use std::io;
use std::path::{Path, PathBuf};

/// Where a replacement is staged before it takes the destination's place.
///
/// The process id keeps two concurrent writers (the export worker and a preset
/// save, say) off each other's temp even when they target one path.
fn staging_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".partial-{}", std::process::id()));
    path.with_file_name(name)
}

/// Run `write` against a temporary sibling of `path`, then put its result in
/// `path`'s place.
///
/// The destination is replaced only when `write` returns `Ok` **and** the rename
/// succeeds; on any other outcome the temp is removed and `path` is left exactly
/// as it was. The writer must have closed its handle by the time it returns —
/// Windows refuses to rename a file that is still open.
pub fn write_replacing<E>(path: &Path, write: impl FnOnce(&Path) -> Result<(), E>) -> Result<(), E>
where
    E: From<io::Error>,
{
    let staging = staging_path(path);
    // A temp left behind by a killed process would otherwise be appended to or
    // refuse to open.
    let _ = std::fs::remove_file(&staging);

    match write(&staging) {
        Ok(()) => match std::fs::rename(&staging, path) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = std::fs::remove_file(&staging);
                Err(E::from(error))
            }
        },
        Err(error) => {
            let _ = std::fs::remove_file(&staging);
            Err(error)
        }
    }
}

/// [`write_replacing`] for the common case of writing bytes in one call.
pub fn write_bytes_replacing(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    write_replacing(path, |staging| std::fs::write(staging, contents.as_ref()))
}

/// Stage a replacement without committing it, for a caller writing several files
/// that must land together.
///
/// [`export_fbx`](crate::export_fbx) writes a whole LOD chain: every level is
/// staged first, and only when all of them are complete is each one committed.
/// A failure on level 3 therefore leaves levels 0–2's destinations untouched,
/// rather than a chain half from this run and half from the last.
pub struct Staged {
    staging: PathBuf,
    destination: PathBuf,
    committed: bool,
}

impl Staged {
    /// Write `destination`'s replacement to a temporary sibling. The file is
    /// removed when this value drops unless [`Self::commit`] has taken it.
    pub fn write<E>(
        destination: &Path,
        write: impl FnOnce(&Path) -> Result<(), E>,
    ) -> Result<Self, E> {
        let staging = staging_path(destination);
        let _ = std::fs::remove_file(&staging);
        match write(&staging) {
            Ok(()) => Ok(Self {
                staging,
                destination: destination.to_path_buf(),
                committed: false,
            }),
            Err(error) => {
                let _ = std::fs::remove_file(&staging);
                Err(error)
            }
        }
    }

    /// The destination this replacement is for.
    pub fn destination(&self) -> &Path {
        &self.destination
    }

    /// Put the staged file in its destination's place.
    pub fn commit(mut self) -> io::Result<PathBuf> {
        std::fs::rename(&self.staging, &self.destination)?;
        self.committed = true;
        Ok(self.destination.clone())
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if !self.committed {
            // Abandoned: the caller failed somewhere else in the set, so this
            // replacement never happens and its temp must not be left behind.
            let _ = std::fs::remove_file(&self.staging);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A per-test directory, removed and recreated so a rerun never sees the
    /// previous run's files. (`CARGO_TARGET_TMPDIR` is only set for integration
    /// tests, so a lib test uses the system temp directory.)
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("review-optimize-tests")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create the temp directory");
        dir
    }

    /// No temp may survive a successful write, or the export directory fills
    /// with `.partial-*` files nobody deletes.
    fn assert_no_staging(dir: &Path) {
        let leftovers: Vec<_> = std::fs::read_dir(dir)
            .expect("read the directory")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".partial-"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "staging files left behind: {leftovers:?}"
        );
    }

    #[test]
    fn a_successful_write_replaces_the_destination() {
        let dir = temp_dir("replace-success");
        let path = dir.join("asset.fbx");
        std::fs::write(&path, b"old").expect("seed");

        write_bytes_replacing(&path, b"new").expect("writes");

        assert_eq!(std::fs::read(&path).expect("read"), b"new");
        assert_no_staging(&dir);
    }

    #[test]
    fn a_failed_write_leaves_the_existing_file_untouched() {
        let dir = temp_dir("replace-failure");
        let path = dir.join("asset.fbx");
        std::fs::write(&path, b"the good export").expect("seed");

        let result: Result<(), io::Error> = write_replacing(&path, |staging| {
            // A writer that produces a partial file and *then* fails — the case
            // a direct write turns into a truncated destination.
            std::fs::write(staging, b"half a fi")?;
            Err(io::Error::other("disk full"))
        });

        assert!(result.is_err());
        assert_eq!(
            std::fs::read(&path).expect("read"),
            b"the good export",
            "the previous export must survive a failed replacement"
        );
        assert_no_staging(&dir);
    }

    #[test]
    fn writing_a_new_file_needs_no_existing_destination() {
        let dir = temp_dir("replace-new");
        let path = dir.join("fresh.fbx");

        write_bytes_replacing(&path, b"content").expect("writes");

        assert_eq!(std::fs::read(&path).expect("read"), b"content");
    }

    #[test]
    fn an_abandoned_staged_write_touches_nothing() {
        let dir = temp_dir("replace-staged-abandoned");
        let path = dir.join("level.fbx");
        std::fs::write(&path, b"previous").expect("seed");

        {
            let _staged: Staged =
                Staged::write(&path, |staging| std::fs::write(staging, b"candidate"))
                    .expect("stages");
            // Dropped without `commit`, as a later level's failure would.
        }

        assert_eq!(std::fs::read(&path).expect("read"), b"previous");
        assert_no_staging(&dir);
    }

    #[test]
    fn a_committed_staged_write_replaces_its_destination() {
        let dir = temp_dir("replace-staged-commit");
        let path = dir.join("level.fbx");
        std::fs::write(&path, b"previous").expect("seed");

        let staged: Staged =
            Staged::write(&path, |staging| std::fs::write(staging, b"candidate")).expect("stages");
        assert_eq!(staged.destination(), path);
        assert_eq!(staged.commit().expect("commits"), path);

        assert_eq!(std::fs::read(&path).expect("read"), b"candidate");
        assert_no_staging(&dir);
    }

    #[test]
    fn a_whole_set_is_staged_before_any_of_it_lands() {
        let dir = temp_dir("replace-set");
        let first = dir.join("asset_LOD0.fbx");
        let second = dir.join("asset_LOD1.fbx");
        std::fs::write(&first, b"old 0").expect("seed");
        std::fs::write(&second, b"old 1").expect("seed");

        // Level 1 fails: the whole set is abandoned, and level 0's destination —
        // already staged — is never touched.
        let result: Result<Vec<Staged>, io::Error> = (|| {
            let mut staged = Vec::new();
            staged.push(Staged::write(&first, |path| {
                std::fs::write(path, b"new 0")
            })?);
            staged.push(Staged::write(&second, |_| {
                Err(io::Error::other("level 1 failed"))
            })?);
            Ok(staged)
        })();

        assert!(result.is_err());
        assert_eq!(std::fs::read(&first).expect("read"), b"old 0");
        assert_eq!(std::fs::read(&second).expect("read"), b"old 1");
        assert_no_staging(&dir);
    }
}
