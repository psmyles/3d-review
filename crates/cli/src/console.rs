//! Everything `review-cli` prints, and the progress bar.
//!
//! Lines go out whole: results on standard output, problems on standard error.
//! A write that fails is ignored rather than allowed to end the run - a reader
//! that stops reading (`review-cli info models | head -1`, a tool that closes
//! its end) must not cost the models still to be done, nor abort before the
//! report is written.
//!
//! The progress bar is the terminal's own: `OSC 9;4`, which Windows Terminal
//! and ConEmu show on the tab and the taskbar button, and Launcher on a step's
//! row. A terminal that does not know the sequence may print it, and one -
//! iTerm2 - turns any `OSC 9` into a notification, so `auto` sends it only
//! where it is known to be shown (the same list cargo keeps), and a tool that
//! reads it asks for it with `--progress-bar on`.
//!
//! Each model's first line (`[3/12] chair.fbx`) is yellow where the terminal is
//! known to show colour, so a long run's models are easy to tell apart. Through
//! a pipe or into a file every line stays plain.

use std::cell::Cell;
use std::io::{IsTerminal, Write};
use std::sync::OnceLock;

use crate::args::BarMode;

/// One line of results.
pub(crate) fn say(text: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{text}");
}

/// The line that starts a model's part of the run, yellow where colour shows.
pub(crate) fn heading(text: &str) {
    let mut out = std::io::stdout().lock();
    if colour() {
        let _ = writeln!(out, "{YELLOW}{text}{PLAIN}");
    } else {
        let _ = writeln!(out, "{text}");
    }
}

/// `SGR 33` and `SGR 0`: the terminal's own yellow, then back to its default.
const YELLOW: &str = "\x1b[33m";
const PLAIN: &str = "\x1b[0m";

/// Whether standard output shows colour, worked out once.
fn colour() -> bool {
    static SHOWN: OnceLock<bool> = OnceLock::new();
    *SHOWN.get_or_init(|| {
        shows_colour(std::io::stdout().is_terminal(), cfg!(windows), |name| {
            std::env::var(name).ok()
        })
    })
}

/// A terminal, `NO_COLOR` not set, and one known to show colour. Windows' own
/// console shows escape codes as text until a program switches them on, which
/// takes the `unsafe` this crate forbids, so there only the terminals that
/// show them already: Windows Terminal, ConEmu, and one that names itself in
/// `TERM` (Launcher's console, WezTerm). `TERM=dumb` asks for none anywhere.
fn shows_colour(terminal: bool, windows: bool, var: impl Fn(&str) -> Option<String>) -> bool {
    let term = var("TERM").filter(|t| !t.is_empty());
    if !terminal
        || var("NO_COLOR").is_some_and(|v| !v.is_empty())
        || term.as_deref() == Some("dumb")
    {
        return false;
    }
    !windows
        || term.is_some()
        || var("WT_SESSION").is_some()
        || var("ConEmuANSI").as_deref() == Some("ON")
}

/// One line about a problem.
pub(crate) fn warn(text: &str) {
    let mut err = std::io::stderr().lock();
    let _ = writeln!(err, "{text}");
}

/// The terminal's progress bar, for one run.
pub(crate) struct Bar {
    shown: bool,
    /// The last whole percent sent, so a run sends a hundred updates rather
    /// than one per progress report.
    last: Cell<Option<u32>>,
}

impl Bar {
    pub(crate) fn new(mode: BarMode) -> Self {
        let shown = match mode {
            BarMode::On => true,
            BarMode::Off => false,
            BarMode::Auto => std::io::stdout().is_terminal() && terminal_shows_progress(),
        };
        Self {
            shown,
            last: Cell::new(None),
        }
    }

    /// How far through the whole run, `0.0..=1.0`.
    pub(crate) fn set(&self, fraction: f32) {
        if !self.shown {
            return;
        }
        let percent = (fraction.clamp(0.0, 1.0) * 100.0).floor() as u32;
        // Forward only: the stages a fraction is pieced together from are
        // estimates, and a bar that steps back reads as a run going wrong.
        if self.last.get().is_some_and(|last| percent <= last) {
            return;
        }
        self.last.set(Some(percent));
        send(1, percent);
    }

    /// Take the bar down. Every run ends with this, whatever its outcome.
    pub(crate) fn clear(&self) {
        if self.shown {
            send(0, 0);
        }
    }
}

/// `OSC 9 ; 4 ; state ; percent ST`. State 1 shows a bar at `percent`, 0 removes
/// it.
fn send(state: u32, percent: u32) {
    let mut out = std::io::stdout().lock();
    let _ = write!(out, "\x1b]9;4;{state};{percent}\x1b\\");
    // No newline follows, and standard output is line-buffered.
    let _ = out.flush();
}

/// The terminals known to show `OSC 9;4` rather than print it.
fn terminal_shows_progress() -> bool {
    let set = |name: &str| std::env::var_os(name).is_some();
    let equals = |name: &str, value: &str| std::env::var(name).is_ok_and(|found| found == value);
    set("WT_SESSION")
        || equals("ConEmuANSI", "ON")
        || equals("TERM_PROGRAM", "WezTerm")
        || equals("TERM_PROGRAM", "ghostty")
}

#[cfg(test)]
mod tests {
    use super::shows_colour;

    fn with<'a>(vars: &'a [(&str, &str)]) -> impl Fn(&str) -> Option<String> + 'a {
        |name| {
            vars.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn colour_shows_only_where_the_terminal_shows_it() {
        // A Mac's or a Linux terminal, Launcher's console on any system.
        assert!(shows_colour(true, false, with(&[])));
        assert!(shows_colour(
            true,
            true,
            with(&[("TERM", "xterm-256color")])
        ));
        assert!(shows_colour(true, true, with(&[("WT_SESSION", "1")])));
        // Never through a pipe, into a file, or when asked for none.
        assert!(!shows_colour(
            false,
            false,
            with(&[("TERM", "xterm-256color")])
        ));
        assert!(!shows_colour(true, false, with(&[("NO_COLOR", "1")])));
        assert!(!shows_colour(true, false, with(&[("TERM", "dumb")])));
        // An empty NO_COLOR doesn't count, as the convention says.
        assert!(shows_colour(true, false, with(&[("NO_COLOR", "")])));
        // Windows' own console would print the escape codes.
        assert!(!shows_colour(true, true, with(&[])));
    }
}
