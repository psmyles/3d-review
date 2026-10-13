//! Parsing a request's files: every file of it at once, each on its own thread,
//! reported as one loading card, and posted back as one [`FilesParsed`].
//!
//! The files of a request are parsed in parallel because a request is decided
//! *as a whole* — which file is the rig, which attach to it — so nothing can be
//! shown until the slowest one has been read; reading them side by side makes
//! that wait the longest parse rather than the sum of them. ufbx parses one file
//! on one thread, so the cores are otherwise idle.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use review_import::{
    CancelToken, ImportError, ImportProgress, ImportStage, StagedImport,
    load_model_staged_cancellable,
};
use review_ui::ActivityId;

use super::{LoadRequest, OpenIntent, file_label};
use crate::events::UserEvent;
use crate::{App, keys, prof};

/// The shortest gap between two progress reports reaching the event loop. The
/// loading card rewrites itself in place, so this is only about not waking the
/// event loop more often than a person can read: ufbx reports every 512 KB, which
/// on a 120 MB file is a couple of hundred callbacks in under two seconds.
const PROGRESS_MIN_INTERVAL: Duration = Duration::from_millis(100);

/// One file of a request, parsed — or why it could not be.
pub(crate) struct ParsedFile {
    pub(crate) path: PathBuf,
    pub(crate) result: Result<StagedImport, ImportError>,
}

impl std::fmt::Debug for ParsedFile {
    /// The path and the outcome, not the model: a parsed file is millions of
    /// vertices that no log line wants.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParsedFile")
            .field("path", &self.path)
            .field("result", &self.result.as_ref().map(|_| "parsed"))
            .finish()
    }
}

/// A request's files, parsed, posted from the worker thread back to the event
/// loop to be classified and shown.
#[derive(Debug)]
pub(crate) struct FilesParsed {
    /// Which request this is, so the queue knows when its parse is done.
    pub(super) request: u64,
    /// The load generation the request was made in. A result whose generation is
    /// no longer the current one — a newer open, or a Ctrl+N — is stale and
    /// dropped rather than shown.
    pub(super) generation: u64,
    pub(super) intent: OpenIntent,
    /// When the request was made, so the success notice can say how long the user
    /// waited for the model to appear.
    pub(super) started: Instant,
    /// The loading card this request opened.
    pub(super) activity: ActivityId,
    pub(super) files: Vec<ParsedFile>,
}

/// An import reporting where it has got to, posted from a worker thread.
#[derive(Debug)]
pub(crate) struct ModelLoadProgress {
    /// Matched against the current load generation exactly as [`FilesParsed`]
    /// is, so a superseded import can't retitle the card of the one that
    /// replaced it.
    pub(super) generation: u64,
    /// The finished stage line, built on the worker: `Reading... 47%`.
    pub(super) message: String,
    /// How far through that stage, when it can be known — the card's bar.
    pub(super) fraction: Option<f32>,
    /// The loading card this import opened: its line is the one rewritten, even
    /// while another job's card is the one showing.
    pub(super) activity: ActivityId,
}

/// The stage line under the loading card's title. The file is already named by
/// the title, so this is just what the import is doing, plus a percentage for the
/// one stage that has a real denominator ([`ImportStage`] says which).
pub(super) fn progress_message(progress: ImportProgress) -> String {
    let stage = review_localization::tr(stage_name(progress.stage)).into_owned();
    match progress.fraction() {
        Some(fraction) => keys::app_notifications::stage_line_percent(
            f64::from((fraction * 100.0).round() as u32),
            stage,
        ),
        None => keys::app_notifications::stage_line(stage),
    }
}

/// What the loading card calls an import stage.
///
/// `import` keeps its own `label()` — it goes down the profiling channel, where
/// a translated string would make two captures harder to compare — so the map
/// lives here, on the side that draws text (invariant 12).
fn stage_name(stage: ImportStage) -> review_localization::Key {
    match stage {
        ImportStage::Reading => keys::app_notifications::STAGE_READING,
        ImportStage::Building => keys::app_notifications::STAGE_BUILDING,
        ImportStage::Extras => keys::app_notifications::STAGE_EXTRAS,
        ImportStage::Measuring => keys::app_notifications::STAGE_MEASURING,
        ImportStage::Finishing => keys::app_notifications::STAGE_FINISHING,
    }
}

/// Times an import's stages for the log, from the progress reports the import
/// already makes. Lives on the one thread that reports; `Cell` rather than a lock
/// for the same reason the throttle is one.
pub(super) struct StageClock {
    name: String,
    current: std::cell::Cell<Option<(ImportStage, Instant)>>,
}

impl StageClock {
    pub(super) fn new(name: String) -> Self {
        Self {
            name,
            current: std::cell::Cell::new(None),
        }
    }

    /// The import has reached `stage`. Ends (and logs) the stage before it; a
    /// repeat report within one stage changes nothing.
    pub(super) fn enter(&self, stage: ImportStage) {
        match self.current.get() {
            Some((current, _)) if current == stage => {}
            _ => {
                self.finish();
                self.current.set(Some((stage, Instant::now())));
            }
        }
    }

    /// End the stage in progress, if any, and log how long it took.
    pub(super) fn finish(&self) {
        if let Some((stage, since)) = self.current.take() {
            log::debug!(
                "{}: import stage '{}' took {:.0} ms",
                self.name,
                stage.label(),
                since.elapsed().as_secs_f64() * 1000.0
            );
        }
    }
}

/// A file's size for the log, or `?` when it cannot be read.
pub(super) fn file_size_label(path: &Path) -> String {
    std::fs::metadata(path).map_or_else(
        |_| "?".to_owned(),
        |metadata| format!("{:.1} MB", metadata.len() as f64 / (1024.0 * 1024.0)),
    )
}

/// Where each file of a request has got to, folded into the one line its loading
/// card shows: while any file is still being read, the bytes read of all of them
/// over the bytes of all of them; after that, the earliest stage any file is in.
struct BatchProgress {
    files: Vec<FileProgress>,
}

#[derive(Default)]
struct FileProgress {
    /// The file's [`ImportStage`], as its index in [`STAGES`].
    stage: AtomicU8,
    done: AtomicU64,
    total: AtomicU64,
}

const STAGES: [ImportStage; 5] = [
    ImportStage::Reading,
    ImportStage::Building,
    ImportStage::Extras,
    ImportStage::Measuring,
    ImportStage::Finishing,
];

impl BatchProgress {
    fn new(count: usize) -> Self {
        Self {
            files: (0..count).map(|_| FileProgress::default()).collect(),
        }
    }

    /// Record file `file`'s report, and return the request's as a whole.
    fn record(&self, file: usize, progress: ImportProgress) -> ImportProgress {
        if let Some(entry) = self.files.get(file) {
            let stage = STAGES
                .iter()
                .position(|&stage| stage == progress.stage)
                .unwrap_or(0);
            entry.stage.store(stage as u8, Ordering::Relaxed);
            if progress.stage == ImportStage::Reading {
                entry.done.store(progress.done, Ordering::Relaxed);
                entry.total.store(progress.total, Ordering::Relaxed);
            }
        }
        let earliest = self
            .files
            .iter()
            .map(|entry| entry.stage.load(Ordering::Relaxed) as usize)
            .min()
            .unwrap_or(0);
        if earliest > 0 {
            return ImportProgress::stage(STAGES[earliest.min(STAGES.len() - 1)]);
        }
        let sum = |field: fn(&FileProgress) -> &AtomicU64| -> u64 {
            self.files
                .iter()
                .map(|entry| field(entry).load(Ordering::Relaxed))
                .sum()
        };
        ImportProgress {
            stage: ImportStage::Reading,
            done: sum(|entry| &entry.done),
            total: sum(|entry| &entry.total),
        }
    }
}

/// Parse every one of `paths`, each on its own thread, reporting the request's
/// progress to `report`. Results come back in the order of `paths`.
pub(super) fn parse_files(
    paths: &[PathBuf],
    cancel: &CancelToken,
    report: &(dyn Fn(ImportProgress) + Sync),
) -> Vec<ParsedFile> {
    let progress = BatchProgress::new(paths.len());
    let parse = |file: usize, path: &PathBuf| {
        let clock = StageClock::new(file_label(path));
        clock.enter(ImportStage::Reading);
        let sink = |stage: ImportProgress| {
            clock.enter(stage.stage);
            report(progress.record(file, stage));
        };
        let result = {
            let _z = prof::zone!("Import Model");
            load_model_staged_cancellable(path, &sink, Some(cancel))
        };
        clock.finish();
        ParsedFile {
            path: path.clone(),
            result,
        }
    };
    if paths.len() == 1 {
        return vec![parse(0, &paths[0])];
    }
    std::thread::scope(|scope| {
        let handles: Vec<_> = paths
            .iter()
            .enumerate()
            .map(|(file, path)| {
                std::thread::Builder::new()
                    .name("model-import".into())
                    .spawn_scoped(scope, move || {
                        prof::thread_name("model-import");
                        parse(file, path)
                    })
            })
            .collect();
        handles
            .into_iter()
            .zip(paths)
            .enumerate()
            .map(|(file, (handle, path))| match handle {
                // A worker that panicked takes its file with it, not the batch.
                Ok(handle) => handle.join().unwrap_or_else(|_| ParsedFile {
                    path: path.clone(),
                    result: Err(ImportError::LoadFailed(
                        "the import thread panicked".to_owned(),
                    )),
                }),
                // No thread to be had: parse this one here instead.
                Err(_) => parse(file, path),
            })
            .collect()
    })
}

impl App {
    /// Parse `request`'s files on a worker thread and post them back as one
    /// [`FilesParsed`], showing a loading card that names the file — or counts
    /// them — while it runs.
    pub(super) fn spawn_parse(&mut self, id: u64, request: LoadRequest) {
        // The clock the success notice reports runs from here — what the user
        // waited, not what the parse cost.
        let started = Instant::now();
        let generation = self.model_load_generation();
        let cancel = CancelToken::new(Arc::clone(&self.model_load_generation), generation);
        for path in &request.paths {
            log::info!("loading {} ({})", path.display(), file_size_label(path));
        }
        let title = match request.paths.as_slice() {
            [path] => keys::app_notifications::loading(file_label(path)),
            paths => keys::app_notifications::loading_files(paths.len() as f64),
        };
        let activity = self.notifications.begin_activity(title);
        self.redraw.requested = true;
        let LoadRequest { paths, intent } = request;
        let first_label = paths
            .first()
            .map(|path| file_label(path))
            .unwrap_or_default();

        let Some(proxy) = self.textures.proxy.clone() else {
            // No proxy to post back through (never the case once `main` has built
            // the event loop) — parse in place, so the files still open.
            let files = parse_files(&paths, &cancel, &|_| {});
            self.handle_files_parsed(FilesParsed {
                request: id,
                generation,
                intent,
                started,
                activity,
                files,
            });
            return;
        };
        let spawned = std::thread::Builder::new()
            .name("model-import".into())
            .spawn(move || {
                prof::thread_name("model-import");
                // Report where the parse has got to, so a 100 MB file shows
                // movement instead of a silent "Loading...". Two filters keep the
                // card calm and the event loop idle: the rendered line must have
                // actually changed (ufbx calls back every 512 KB, which is many
                // reports per whole percent) and a beat must have passed since the
                // last one. Behind a lock, since every file's thread reports.
                let last_sent = Mutex::new((None::<Instant>, String::new()));
                let progress_proxy = proxy.clone();
                let report = |progress: ImportProgress| {
                    let message = progress_message(progress);
                    let Ok(mut last) = last_sent.lock() else {
                        return;
                    };
                    if last.1 == message
                        || last
                            .0
                            .is_some_and(|sent| sent.elapsed() < PROGRESS_MIN_INTERVAL)
                    {
                        return;
                    }
                    *last = (Some(Instant::now()), message.clone());
                    let _ = progress_proxy.send_event(UserEvent::ModelLoadProgress(
                        ModelLoadProgress {
                            generation,
                            message,
                            fraction: progress.fraction(),
                            activity,
                        },
                    ));
                };
                let files = parse_files(&paths, &cancel, &report);
                // A send failure only means the event loop has exited.
                let _ = proxy.send_event(UserEvent::FilesParsed(Box::new(FilesParsed {
                    request: id,
                    generation,
                    intent,
                    started,
                    activity,
                    files,
                })));
            });
        if let Err(error) = spawned {
            // No worker, so no message will ever close the loading card or free
            // the queue.
            log::error!("could not start the import thread: {error}");
            self.notifications.end_activity(activity);
            self.notifications
                .error(keys::app_notifications::couldnt_load(
                    first_label,
                    error.to_string(),
                ));
            self.finish_request(id);
        }
    }

    /// Rewrite the loading card's stage line from a worker's progress report. A
    /// report from a superseded load is dropped, exactly as its result would be.
    pub(crate) fn handle_model_load_progress(&mut self, message: ModelLoadProgress) {
        if message.generation != self.model_load_generation() {
            return;
        }
        self.notifications
            .update_activity(message.activity, message.message, message.fraction);
        self.request_redraw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(done: u64, total: u64) -> ImportProgress {
        ImportProgress {
            stage: ImportStage::Reading,
            done,
            total,
        }
    }

    #[test]
    fn a_batch_reads_as_the_bytes_of_all_its_files() {
        let progress = BatchProgress::new(2);
        progress.record(0, reading(50, 100));
        let both = progress.record(1, reading(0, 300));
        assert_eq!((both.done, both.total), (50, 400));
        assert_eq!(both.fraction(), Some(0.125));
    }

    #[test]
    fn a_batch_moves_on_only_when_every_file_has() {
        let progress = BatchProgress::new(2);
        progress.record(0, reading(100, 100));
        let one_building = progress.record(0, ImportProgress::stage(ImportStage::Building));
        assert_eq!(
            one_building.stage,
            ImportStage::Reading,
            "file 1 is still reading"
        );
        progress.record(1, reading(300, 300));
        let both_building = progress.record(1, ImportProgress::stage(ImportStage::Building));
        assert_eq!(both_building.stage, ImportStage::Building);
        assert_eq!(both_building.fraction(), None);
    }
}
