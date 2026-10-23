//! Putting the bundled `review-cli` on the PATH (see [`crate::install_cli`]).
//!
//! The .dmg is drag-to-install and runs nothing, so unlike Setup's Add to PATH
//! task on Windows this is the app's own job: the application menu's Install
//! Command Line Tool links [`crate::CLI_LINK`] to the CLI inside the bundle,
//! asking for an administrator password when the folder needs one. That folder is
//! on the default PATH of every macOS shell, and on Apple silicon it is not
//! Homebrew's, so a link there collides with nothing.
//!
//! The link points into the bundle rather than at a copy, so it updates with the
//! app. It also breaks if the app is moved, which [`crate::cli_install_offered`]
//! then reports at the next launch. `review-cli` reads nothing from beside its own
//! executable, so being started through a link changes nothing for it.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{CLI_LINK, CLI_NAME, CliInstallError};

/// The CLI inside the running bundle, or `None` when this is not a bundled build:
/// a bare `cargo run` binary has nothing beside it worth linking.
fn bundled_cli() -> Option<PathBuf> {
    let executable = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .ok()?;
    let directory = executable.parent()?;
    let in_bundle = directory.ends_with("Contents/MacOS")
        && directory
            .parent()
            .and_then(Path::parent)
            .and_then(Path::extension)
            .is_some_and(|extension| extension == "app");
    let cli = directory.join(CLI_NAME);
    (in_bundle && cli.is_file()).then_some(cli)
}

/// Whether a shell would find a `review-cli` command.
///
/// An app started from Finder inherits launchd's PATH rather than the shell's, so
/// the PATH it sees misses `/usr/local/bin` and Homebrew's folder; those are
/// checked by name. `is_file` follows links, so a link left dangling by a moved or
/// deleted app counts as missing.
fn on_path() -> bool {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(["/usr/local/bin", "/opt/homebrew/bin"].map(PathBuf::from))
        .any(|directory| directory.join(CLI_NAME).is_file())
}

/// See [`crate::cli_install_offered`].
pub(crate) fn offered() -> bool {
    bundled_cli().is_some() && !on_path()
}

/// See [`crate::install_cli`].
pub(crate) fn install(prompt: &str) -> Result<PathBuf, CliInstallError> {
    let cli = bundled_cli().ok_or(CliInstallError::NotBundled)?;
    // A quarantined app run from Downloads is started from a randomised read-only
    // copy, and one run from the disk image lives on a volume that goes away. A
    // link to either breaks.
    let location = cli.to_string_lossy();
    if location.contains("/AppTranslocation/") || location.starts_with("/Volumes/") {
        return Err(CliInstallError::NotInApplications);
    }
    let link = Path::new(CLI_LINK);
    // Replacing an earlier link is what reinstalling is; replacing a real file
    // would be overwriting some other program.
    if let Ok(metadata) = link.symlink_metadata()
        && !metadata.file_type().is_symlink()
    {
        return Err(CliInstallError::Occupied);
    }

    if link_directly(&cli, link).is_err() {
        link_as_administrator(&cli, prompt)?;
    }
    match std::fs::canonicalize(link) {
        Ok(target) if target == cli => Ok(cli),
        _ => Err(CliInstallError::Failed(format!(
            "{CLI_LINK} was not created"
        ))),
    }
}

fn link_directly(cli: &Path, link: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(link.parent().unwrap_or(Path::new("/")))?;
    let _ = std::fs::remove_file(link);
    std::os::unix::fs::symlink(cli, link)
}

/// The same link through `osascript`, which shows the system's own password
/// prompt. The bundle path reaches the shell through AppleScript's `quoted form
/// of`, so the only quoting done here is the AppleScript string literals'.
///
/// A separate process, so the prompt's modal loop is not ours — but it blocks
/// until answered, which is why `app` calls this from a worker thread.
fn link_as_administrator(cli: &Path, prompt: &str) -> Result<(), CliInstallError> {
    let directory = Path::new(CLI_LINK).parent().unwrap_or(Path::new("/"));
    let script = format!(
        "do shell script \"/bin/mkdir -p {directory} && /bin/ln -sfn \" & quoted form of \"{cli}\" \
         & \" {CLI_LINK}\" with prompt \"{prompt}\" with administrator privileges",
        directory = directory.display(),
        cli = applescript_string(&cli.to_string_lossy()),
        prompt = applescript_string(prompt),
    );
    let output = Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .output()
        .map_err(|error| CliInstallError::Failed(format!("could not run osascript: {error}")))?;
    if output.status.success() {
        return Ok(());
    }
    let error = String::from_utf8_lossy(&output.stderr);
    // -128 is "User canceled".
    if error.contains("(-128)") {
        Err(CliInstallError::Cancelled)
    } else {
        Err(CliInstallError::Failed(error.trim().to_owned()))
    }
}

/// `text` as the inside of an AppleScript string literal.
fn applescript_string(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applescript_strings_escape_quotes_and_backslashes() {
        assert_eq!(
            applescript_string(r#"/Apps/My "Review"\x.app"#),
            r#"/Apps/My \"Review\"\\x.app"#
        );
    }

    #[test]
    fn a_development_build_has_nothing_to_install() {
        // The test binary lives in target/, not in a bundle.
        assert!(bundled_cli().is_none());
        assert!(!offered());
        assert_eq!(install("unused"), Err(CliInstallError::NotBundled));
    }
}
