//! review-log: the viewer's log.
//!
//! One [`log::Log`] implementation, installed as the process's logger by `app`,
//! fans every record out to four places:
//!
//! - the **session buffer** the Log window (Debug > View Log) reads, bounded to
//!   [`CAPACITY`] entries;
//! - **today's file** on disk (`3d-review-YYYY-MM-DD.log` in the per-user logs
//!   folder `app` names), which every session that day appends to and the first
//!   session of a later day deletes — see [`open_file`];
//! - **stderr**, for a debug build's console;
//! - **Tracy's message channel**, while a client is running, which is where
//!   `prof::msg` used to send these lines and nowhere else.
//!
//! **Our crates log from Debug up; everyone else's from Warning up.** The facade
//! is shared with the whole dependency graph, so installing a logger also hears
//! winit, egui and the file watcher. Their warnings are worth having; their
//! debug chatter is not ours to read. "Ours" is a target whose root is a
//! `review_*` crate or the application binary named to [`install`].
//!
//! **Installed before the file exists.** [`install`] is an atomic store and runs
//! on the first line of `main`; [`open_file`] does the directory work once the
//! GPU bring-up is under way. Everything logged between the two is written to
//! the file when it opens, so nothing the bring-up says is lost and the launch
//! path pays no disk I/O for it.
//!
//! **The file is written unbuffered, one line per record.** Log volume is a few
//! lines a second at worst, and a release build aborts on panic — a buffer
//! would lose exactly the lines that explain the crash.

#![forbid(unsafe_code)]

mod file;

use std::cell::Cell;
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

/// How many entries the session buffer keeps. The oldest go first; the file
/// keeps everything.
pub const CAPACITY: usize = 10_000;

/// How a line's time of day is written: local time, to the millisecond.
const TIME_FORMAT: &str = "%H:%M:%S%.3f";

/// The target a panic is logged under, so its line is tagged `[panic]`.
const PANIC_TARGET: &str = "review_panic";

/// How serious a line is. Ordered, so `Debug < Error`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// Developer detail: stage timings, cleanup. Hidden in the Log window until
    /// asked for, always written to the file.
    Debug,
    /// Something happened as it should.
    Info,
    /// Something went through, but not entirely as asked.
    Warning,
    /// Something did not happen.
    Error,
}

impl Level {
    /// Every level, least serious first.
    pub const ALL: [Level; 4] = [Level::Debug, Level::Info, Level::Warning, Level::Error];

    /// The level's name in the file. A stable English identifier, like every
    /// other `label()` in the workspace: the file is what a bug report quotes, so
    /// it reads the same whatever language the viewer is showing. The Log window
    /// takes its words from the catalog instead.
    pub fn label(self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warning => "WARN",
            Level::Error => "ERROR",
        }
    }

    /// The facade's level, or `None` for `Trace`, which nothing here records.
    fn from_log(level: log::Level) -> Option<Self> {
        match level {
            log::Level::Error => Some(Level::Error),
            log::Level::Warn => Some(Level::Warning),
            log::Level::Info => Some(Level::Info),
            log::Level::Debug => Some(Level::Debug),
            log::Level::Trace => None,
        }
    }
}

/// One logged line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Position in the session, from 0, with no gaps — which is what lets a
    /// reader ask for "everything after the last one I saw".
    pub seq: u64,
    /// Local time of day, `08:10:02.840`.
    pub time: String,
    pub level: Level,
    /// Which part of the program said it: the crate, without its `review_`
    /// prefix (`render`, `import`), `app` for the binary, or a dependency's own
    /// name (`winit`).
    pub tag: String,
    /// What it said. May run over several lines.
    pub message: String,
}

impl Entry {
    /// The entry as the file writes it: `08:10:02.840 INFO  [app] message`,
    /// with any further lines of the message indented under its first.
    fn file_line(&self) -> String {
        let mut line = format!("{} {:<5} [{}] ", self.time, self.level.label(), self.tag);
        let indent = " ".repeat(line.len());
        for (index, part) in self.message.lines().enumerate() {
            if index > 0 {
                line.push('\n');
                line.push_str(&indent);
            }
            line.push_str(part);
        }
        line.push('\n');
        line
    }
}

/// The session buffer and the open file.
struct Store {
    entries: VecDeque<Entry>,
    next_seq: u64,
    file: Option<File>,
    path: Option<PathBuf>,
}

static STORE: Mutex<Store> = Mutex::new(Store {
    entries: VecDeque::new(),
    next_seq: 0,
    file: None,
    path: None,
});

/// The application binary's crate name, which counts as one of ours.
static APP_CRATE: OnceLock<&'static str> = OnceLock::new();
/// What to call when a line arrives while someone is watching.
static WAKER: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
/// Whether a reader wants to hear about new lines as they arrive.
static WATCHING: AtomicBool = AtomicBool::new(false);
/// A wake has been sent and the reader has not caught up since. Coalesces a
/// burst of lines into one wake.
static WAKE_PENDING: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// This thread is inside [`record`] already. A panic raised while the store
    /// is locked runs the panic hook, which logs — and would wait forever on the
    /// lock its own thread holds.
    static IN_LOGGER: Cell<bool> = const { Cell::new(false) };
}

/// Holds [`IN_LOGGER`] for the length of one record, and lets it go however the
/// record ends.
struct ReentryGuard;

impl ReentryGuard {
    fn enter() -> Option<Self> {
        (!IN_LOGGER.with(|flag| flag.replace(true))).then_some(ReentryGuard)
    }
}

impl Drop for ReentryGuard {
    fn drop(&mut self) {
        IN_LOGGER.with(|flag| flag.set(false));
    }
}

/// The store, whether or not a panic poisoned it: a log that stops working
/// after the first panic is no log at all.
fn store() -> MutexGuard<'static, Store> {
    STORE.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Logger;

static LOGGER: Logger = Logger;

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        passes(metadata.target(), metadata.level())
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        if let Some(level) = Level::from_log(record.level()) {
            self::record(level, record.target(), record.args().to_string());
        }
    }

    fn flush(&self) {
        if let Some(file) = store().file.as_mut() {
            let _ = file.flush();
        }
    }
}

/// Install the logger as the process's `log` facade, and a panic hook that logs
/// the panic before the default hook runs. `app_crate` is the binary's crate
/// name (`env!("CARGO_CRATE_NAME")`), whose lines are tagged `app` and logged
/// from Debug up like every `review_*` crate's.
///
/// Call once, first thing in `main`. A second call, or a process that already
/// has a logger, changes nothing.
pub fn install(app_crate: &'static str) {
    let _ = APP_CRATE.set(app_crate);
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Debug);
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            record(Level::Error, PANIC_TARGET, format!("panic: {info}"));
            previous(info);
        }));
    }
}

/// Open today's file in `dir`, creating the folder if need be, and write it
/// everything logged so far under a session header. Before that, every earlier
/// day's file in `dir` is deleted, so no log outlives the day it was written on.
///
/// Returns the file's path. On failure the log carries on in memory only.
pub fn open_file(dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let now = chrono::Local::now();
    let today = now.date_naive();
    let removed = file::prune(dir, today);
    let path = dir.join(file::file_name(today));
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    let continuing = file.metadata().is_ok_and(|metadata| metadata.len() > 0);
    {
        let mut store = store();
        let mut text = file::session_header(now, std::process::id(), continuing);
        for entry in &store.entries {
            text.push_str(&entry.file_line());
        }
        file.write_all(text.as_bytes())?;
        store.file = Some(file);
        store.path = Some(path.clone());
    }
    // Only now, with the store unlocked: these log.
    for (old, outcome) in removed {
        match outcome {
            Ok(()) => log::debug!("removed an earlier day's log: {}", old.display()),
            Err(error) => log::warn!(
                "could not remove an earlier day's log {}: {error}",
                old.display()
            ),
        }
    }
    Ok(path)
}

/// The file this session is writing to, once [`open_file`] has succeeded.
pub fn file_path() -> Option<PathBuf> {
    store().path.clone()
}

/// Append every entry logged since `*next_seq` to `into`, and move `next_seq`
/// past them. Starting from 0 reads the whole session buffer. Re-arms the wake,
/// so the next line to arrive while watching sends one.
///
/// The entries are copied out rather than lent, because the reader draws them
/// and drawing can log — which must not happen with the store locked.
pub fn read_since(next_seq: &mut u64, into: &mut Vec<Entry>) {
    WAKE_PENDING.store(false, Ordering::Release);
    let store = store();
    let first = store.next_seq - store.entries.len() as u64;
    let skip = next_seq.saturating_sub(first);
    let skip = usize::try_from(skip).unwrap_or(usize::MAX);
    into.extend(store.entries.iter().skip(skip).cloned());
    *next_seq = store.next_seq;
}

/// Whether anything has been logged since `next_seq`.
pub fn has_since(next_seq: u64) -> bool {
    store().next_seq > next_seq
}

/// What to call when a line arrives while [`set_watching`] is on — `app`'s
/// event-loop proxy, so an open Log window redraws for a line logged on a worker
/// thread. Called outside the store lock, at most once until the next
/// [`read_since`]. Only the first call takes effect.
pub fn set_waker(waker: impl Fn() + Send + Sync + 'static) {
    let _ = WAKER.set(Box::new(waker));
}

/// Whether a reader is showing the log right now. While it is off no wake is
/// sent, so a closed Log window costs the event loop nothing.
pub fn set_watching(watching: bool) {
    WATCHING.store(watching, Ordering::Relaxed);
}

/// Record one line: into the buffer, the file, stderr and Tracy.
fn record(level: Level, target: &str, message: String) {
    let Some(_guard) = ReentryGuard::enter() else {
        return;
    };
    let tag = tag_for(target);
    let time = chrono::Local::now().format(TIME_FORMAT).to_string();
    let line = {
        let mut store = store();
        let entry = Entry {
            seq: store.next_seq,
            time,
            level,
            tag,
            message,
        };
        store.next_seq += 1;
        let line = entry.file_line();
        if let Some(file) = store.file.as_mut() {
            // Nowhere to report a failed write to but the log itself.
            let _ = file.write_all(line.as_bytes());
        }
        if store.entries.len() == CAPACITY {
            store.entries.pop_front();
        }
        store.entries.push_back(entry);
        line
    };
    // Not `eprint!`: it panics when stderr cannot be written - a closed pipe,
    // say - and a release build aborts on panic. A line the console missed is
    // still in the file.
    let _ = std::io::stderr().write_all(line.as_bytes());
    review_prof::msg(line.trim_end());
    wake();
}

/// Tell the watcher, if there is one and it has not been told already.
fn wake() {
    if WATCHING.load(Ordering::Relaxed)
        && !WAKE_PENDING.swap(true, Ordering::AcqRel)
        && let Some(waker) = WAKER.get()
    {
        waker();
    }
}

/// A target's crate: the part before the first `::`.
fn root_of(target: &str) -> &str {
    target.split("::").next().unwrap_or(target)
}

/// Whether a target is one of this workspace's crates.
fn is_ours(target: &str) -> bool {
    let root = root_of(target);
    root.starts_with("review_") || APP_CRATE.get().is_some_and(|app| *app == root)
}

/// Whether a record at `level` from `target` is kept: Debug and up from our
/// own crates, Warning and up from anyone else's.
fn passes(target: &str, level: log::Level) -> bool {
    let floor = if is_ours(target) {
        log::Level::Debug
    } else {
        log::Level::Warn
    };
    level <= floor
}

/// The tag a line from `target` is shown with.
fn tag_for(target: &str) -> String {
    let root = root_of(target);
    if APP_CRATE.get().is_some_and(|app| *app == root) {
        return "app".to_owned();
    }
    root.strip_prefix("review_").unwrap_or(root).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(message: &str) -> Entry {
        Entry {
            seq: 0,
            time: "08:10:02.840".to_owned(),
            level: Level::Info,
            tag: "app".to_owned(),
            message: message.to_owned(),
        }
    }

    #[test]
    fn a_line_reads_time_level_tag_message() {
        assert_eq!(
            entry("model loaded").file_line(),
            "08:10:02.840 INFO  [app] model loaded\n"
        );
    }

    /// A message over several lines keeps them, indented under the first, so a
    /// multi-line error does not read as several entries.
    #[test]
    fn continuation_lines_sit_under_the_message() {
        assert_eq!(
            entry("first\nsecond").file_line(),
            "08:10:02.840 INFO  [app] first\n                         second\n"
        );
    }

    #[test]
    fn our_crates_log_from_debug_and_others_from_warning() {
        assert!(passes("review_render::rhi", log::Level::Debug));
        assert!(!passes("winit::platform", log::Level::Info));
        assert!(passes("winit::platform", log::Level::Warn));
        assert!(!passes("review_render", log::Level::Trace));
    }

    #[test]
    fn a_tag_is_the_crate_without_its_prefix() {
        assert_eq!(tag_for("review_import::ffi::bridge"), "import");
        assert_eq!(tag_for("egui::context"), "egui");
        assert_eq!(tag_for(PANIC_TARGET), "panic");
    }

    #[test]
    fn levels_order_by_seriousness() {
        assert!(Level::Debug < Level::Info);
        assert!(Level::Warning < Level::Error);
        assert_eq!(Level::ALL.len(), 4);
    }
}
