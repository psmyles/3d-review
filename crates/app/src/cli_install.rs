//! 3D Review > Install Command Line Tool (macOS): link the bundled `review-cli`
//! onto the PATH, and suggest doing so at launch while it is not there.
//!
//! The work itself is `review-shell-macos`'s (`install_cli`), and it can sit on
//! the system's administrator password prompt for as long as the user leaves it
//! up, so it runs on a worker thread like the update check and comes home as
//! [`UserEvent::CliInstalled`]. Off macOS nothing reaches this: no menu raises it,
//! and the launch check is always `false`.

use std::path::{Path, PathBuf};

use review_shell_macos::{CLI_LINK, CLI_NAME, CliInstallError};

use crate::events::UserEvent;
use crate::{APP_NAME, App, keys};

impl App {
    /// At launch: say how to get `review-cli` on the PATH if a shell cannot find it.
    /// An info card that expires on its own, since a user who never opens a terminal
    /// has nothing to fix.
    pub(crate) fn suggest_cli_install(&mut self) {
        if review_shell_macos::cli_install_offered() {
            self.notifications
                .info(keys::app_notifications::cli_not_on_path(
                    APP_NAME.to_owned(),
                    CLI_NAME.to_owned(),
                ));
            self.redraw.requested = true;
        }
    }

    /// Start an install, unless one is already waiting on its password prompt.
    pub(crate) fn install_cli(&mut self) {
        if self.cli_install_in_flight {
            return;
        }
        let Some(proxy) = self.textures.proxy.clone() else {
            return;
        };
        self.cli_install_in_flight = true;
        let folder = Path::new(CLI_LINK)
            .parent()
            .map(|folder| folder.display().to_string())
            .unwrap_or_default();
        let prompt =
            keys::app_dialogs::install_cli_prompt(CLI_NAME.to_owned(), folder, APP_NAME.to_owned());

        let spawned = std::thread::Builder::new()
            .name("cli-install".into())
            .spawn(move || {
                let result = review_shell_macos::install_cli(&prompt);
                // A closed event loop means the app is already exiting.
                let _ = proxy.send_event(UserEvent::CliInstalled(result));
            });
        if let Err(error) = spawned {
            self.handle_cli_installed(Err(CliInstallError::Failed(error.to_string())));
        }
    }

    /// Say how the install went. A dismissed password prompt says nothing: the
    /// user knows they dismissed it.
    pub(crate) fn handle_cli_installed(&mut self, result: Result<PathBuf, CliInstallError>) {
        self.cli_install_in_flight = false;
        match result {
            Ok(cli) => {
                log::info!("linked {CLI_LINK} to {}", cli.display());
                self.notifications
                    .success(keys::app_notifications::cli_installed(
                        CLI_NAME.to_owned(),
                        CLI_LINK.to_owned(),
                    ));
            }
            Err(CliInstallError::Cancelled) => log::info!("CLI install cancelled"),
            Err(CliInstallError::NotBundled) => {
                self.notifications
                    .warning(keys::app_notifications::cli_install_not_bundled(
                        APP_NAME.to_owned(),
                    ))
            }
            Err(CliInstallError::NotInApplications) => self.notifications.warning(
                keys::app_notifications::cli_install_not_in_applications(APP_NAME.to_owned()),
            ),
            Err(CliInstallError::Occupied) => {
                self.notifications
                    .warning(keys::app_notifications::cli_install_occupied(
                        APP_NAME.to_owned(),
                        CLI_LINK.to_owned(),
                    ))
            }
            Err(CliInstallError::Failed(detail)) => {
                log::warn!("CLI install failed: {detail}");
                self.notifications
                    .error(keys::app_notifications::cli_install_failed(
                        CLI_NAME.to_owned(),
                        detail,
                    ));
            }
        }
        self.redraw.requested = true;
    }
}
