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

use std::cell::Cell;
use std::io::{IsTerminal, Write};

use crate::args::BarMode;

/// One line of results.
pub(crate) fn say(text: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{text}");
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
