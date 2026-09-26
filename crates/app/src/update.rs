//! Help > Check for Updates: ask GitHub for the newest release, and either open
//! the releases page (a newer one exists) or say this build is the latest.
//!
//! The request is the operating system's own `curl` — part of Windows 10 1803+
//! and of every macOS — run on a worker thread, so the viewer links no HTTP or
//! TLS stack for one request a user makes by hand. A box without it (or offline,
//! or rate-limited) gets a notice saying the check could not be made, never a
//! wrong answer. The result comes back through the event loop as
//! [`UserEvent::UpdateChecked`], like every other worker's.
//!
//! Versions compare numerically on `major.minor.patch`: a release is tagged
//! `v0.4.1`, this build knows itself as `0.4.1` (`product.json`), and anything
//! after a `-` or `+` is ignored — `releases/latest` never returns a pre-release
//! anyway.

use std::process::Command;

use crate::events::UserEvent;
use crate::{App, keys};

/// This build's version, from `product.json` via `build.rs`.
const CURRENT_VERSION: &str = env!("REVIEW_VERSION");

/// The project's home, `https://github.com/<owner>/<repo>`, from `product.json`.
const HOMEPAGE: &str = env!("REVIEW_HOMEPAGE");

/// How long the request may take before the check gives up, in seconds.
const TIMEOUT_SECONDS: &str = "15";

/// What asking GitHub found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UpdateCheck {
    /// A release newer than this build, by its tag.
    Newer(String),
    /// Nothing newer (including a repository with no releases yet).
    Latest,
    /// The check could not be made; the detail is a diagnostic, not catalog text.
    Failed(String),
}

impl App {
    /// Start a check, unless one is already on its way.
    pub(crate) fn check_for_updates(&mut self) {
        if self.update_check_in_flight {
            return;
        }
        let Some(proxy) = self.textures.proxy.clone() else {
            return;
        };
        self.update_check_in_flight = true;
        self.notifications.info(
            review_localization::tr(keys::app_notifications::CHECKING_FOR_UPDATES).into_owned(),
        );
        self.redraw.requested = true;

        let spawned = std::thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let result = check(HOMEPAGE, CURRENT_VERSION);
                // A closed event loop means the app is already exiting.
                let _ = proxy.send_event(UserEvent::UpdateChecked(result));
            });
        if let Err(error) = spawned {
            self.update_check_in_flight = false;
            self.handle_update_checked(UpdateCheck::Failed(error.to_string()));
        }
    }

    /// Act on a finished check: open the releases page for a newer release, or
    /// say there is nothing newer, or that the check failed.
    pub(crate) fn handle_update_checked(&mut self, result: UpdateCheck) {
        self.update_check_in_flight = false;
        match result {
            UpdateCheck::Newer(tag) => {
                self.notifications
                    .success(keys::app_notifications::update_available(tag));
                if let Some(ctx) = self.egui_ctx.as_ref() {
                    // Queued now, handed to the browser by the next frame's
                    // platform output — which the redraw below makes happen.
                    ctx.open_url(egui::OpenUrl::new_tab(format!("{HOMEPAGE}/releases")));
                }
            }
            UpdateCheck::Latest => {
                self.notifications
                    .success(keys::app_notifications::update_latest(
                        CURRENT_VERSION.to_owned(),
                    ))
            }
            UpdateCheck::Failed(detail) => {
                log::warn!("update check failed: {detail}");
                self.notifications
                    .warning(keys::app_notifications::update_check_failed(detail));
            }
        }
        self.redraw.requested = true;
    }
}

/// Ask GitHub, and compare. Runs on the worker thread.
fn check(homepage: &str, current: &str) -> UpdateCheck {
    let Some(url) = latest_release_api_url(homepage) else {
        return UpdateCheck::Failed(format!("not a GitHub homepage: {homepage}"));
    };
    match fetch(&url, current) {
        Ok(Response { status: 404, .. }) => UpdateCheck::Latest,
        Ok(Response { status: 200, body }) => match tag_name(&body) {
            Some(tag) if is_newer(&tag, current) => UpdateCheck::Newer(tag),
            Some(_) => UpdateCheck::Latest,
            None => UpdateCheck::Failed("the reply named no release tag".into()),
        },
        Ok(Response { status, .. }) => {
            UpdateCheck::Failed(format!("GitHub answered HTTP {status}"))
        }
        Err(error) => UpdateCheck::Failed(error),
    }
}

/// `https://github.com/<owner>/<repo>` to the API's latest-release endpoint.
fn latest_release_api_url(homepage: &str) -> Option<String> {
    let repo = homepage
        .trim_end_matches('/')
        .strip_prefix("https://github.com/")?;
    (repo.split('/').count() == 2)
        .then(|| format!("https://api.github.com/repos/{repo}/releases/latest"))
}

struct Response {
    status: u16,
    body: String,
}

/// One GET through the system `curl`. The status is appended after the body
/// (`-w`), rather than made an error with `-f`, so a 404 — a repository with no
/// releases — can be told apart from a network failure.
fn fetch(url: &str, current: &str) -> Result<Response, String> {
    let mut command = Command::new("curl");
    command.args([
        "--silent",
        "--show-error",
        "--location",
        "--max-time",
        TIMEOUT_SECONDS,
        "--header",
        "Accept: application/vnd.github+json",
        "--user-agent",
        &format!("3d-review/{current}"),
        "--write-out",
        "\n%{http_code}",
        url,
    ]);
    // A windows-subsystem binary launching a console program would flash a
    // console window for the length of the request.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command
        .output()
        .map_err(|error| format!("could not run curl: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.trim().to_owned());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let (body, status) = text
        .rsplit_once('\n')
        .ok_or_else(|| "curl printed no status".to_owned())?;
    let status = status
        .trim()
        .parse()
        .map_err(|_| format!("curl printed an unreadable status: {status}"))?;
    Ok(Response {
        status,
        body: body.to_owned(),
    })
}

/// The `"tag_name"` string out of a release's JSON. A scan rather than a JSON
/// parser: it is one string field, and a tag never contains a quote or escape.
fn tag_name(json: &str) -> Option<String> {
    let after_key = &json[json.find("\"tag_name\"")? + "\"tag_name\"".len()..];
    let after_colon = after_key.trim_start().strip_prefix(':')?.trim_start();
    let value = after_colon.strip_prefix('"')?;
    let tag = &value[..value.find('"')?];
    (!tag.is_empty()).then(|| tag.to_owned())
}

/// `v0.4.1`, `0.4.1` or `0.4.1-beta` to `(0, 4, 1)`; missing parts read as zero.
fn version_numbers(text: &str) -> Option<(u64, u64, u64)> {
    let text = text.trim().trim_start_matches(['v', 'V']);
    let core = text.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|part| part.parse::<u64>());
    let major = parts.next()?.ok()?;
    let minor = parts.next().unwrap_or(Ok(0)).ok()?;
    let patch = parts.next().unwrap_or(Ok(0)).ok()?;
    Some((major, minor, patch))
}

/// Whether release `tag` is newer than version `current`. A tag that does not
/// read as a version is never "newer" — opening the releases page on a tag we
/// cannot understand would be a guess.
fn is_newer(tag: &str, current: &str) -> bool {
    match (version_numbers(tag), version_numbers(current)) {
        (Some(tag), Some(current)) => tag > current,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_api_url_comes_from_the_homepage() {
        assert_eq!(
            latest_release_api_url("https://github.com/psmyles/3d-review").as_deref(),
            Some("https://api.github.com/repos/psmyles/3d-review/releases/latest"),
        );
        assert_eq!(
            latest_release_api_url("https://github.com/psmyles/3d-review/").as_deref(),
            Some("https://api.github.com/repos/psmyles/3d-review/releases/latest"),
        );
        assert_eq!(latest_release_api_url("https://example.com/x"), None);
        assert_eq!(latest_release_api_url("https://github.com/psmyles"), None);
    }

    #[test]
    fn the_tag_is_read_out_of_a_release() {
        let json = r#"{"url":"x","id":1,"tag_name" : "v0.5.0","name":"0.5.0"}"#;
        assert_eq!(tag_name(json).as_deref(), Some("v0.5.0"));
        assert_eq!(tag_name(r#"{"message":"Not Found"}"#), None);
        assert_eq!(tag_name(r#"{"tag_name":""}"#), None);
    }

    #[test]
    fn versions_compare_numerically_not_as_text() {
        assert!(is_newer("v0.4.2", "0.4.1"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("v0.4.1", "0.4.1"));
        assert!(!is_newer("v0.4.0", "0.4.1"));
        assert!(!is_newer("v0.4.1-beta", "0.4.1"));
        assert!(!is_newer("nightly", "0.4.1"));
    }

    #[test]
    fn this_build_knows_its_own_version() {
        assert!(version_numbers(CURRENT_VERSION).is_some());
        assert!(latest_release_api_url(HOMEPAGE).is_some());
    }
}
