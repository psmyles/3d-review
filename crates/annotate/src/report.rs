//! The comments of one or more files as JSON for scripts, or as a Markdown
//! report for people — all of them, or only what changed since a [`Baseline`].

use serde::Serialize;

use crate::baseline::{Baseline, Change};
use crate::codec::CodecError;
use crate::comments::{FileComments, object_path};
use crate::fbx::{Format, Scan};
use crate::thread::{Anchor, Status, Thread};

/// Which threads a listing includes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatusFilter {
    #[default]
    All,
    Open,
    Resolved,
}

impl StatusFilter {
    pub fn admits(self, status: Status) -> bool {
        match self {
            StatusFilter::All => true,
            StatusFilter::Open => status == Status::Open,
            StatusFilter::Resolved => status == Status::Resolved,
        }
    }
}

/// What a listing includes.
#[derive(Clone, Copy, Default)]
pub struct Selection<'a> {
    pub status: StatusFilter,
    /// When set, only the threads that changed since this baseline, each with
    /// what changed.
    pub since: Option<&'a Baseline>,
}

/// One thread with its number (its position among all of the file's threads,
/// from 1) and the object it is stored on.
pub struct Numbered<'a> {
    pub number: usize,
    pub model: usize,
    pub thread: &'a Thread,
    /// What changed since the selection's baseline; `None` without one.
    pub change: Option<Change>,
}

/// The threads `selection` admits, numbered across the whole file — so a
/// thread's number doesn't change with the selection.
pub fn numbered<'a>(comments: &'a FileComments, selection: Selection<'_>) -> Vec<Numbered<'a>> {
    comments
        .threads()
        .enumerate()
        .filter(|(_, (_, thread))| selection.status.admits(thread.status))
        .filter_map(|(index, (model, thread))| {
            let change = match selection.since {
                Some(baseline) => Some(baseline.change(thread)?),
                None => None,
            };
            Some(Numbered {
                number: index + 1,
                model,
                thread,
                change,
            })
        })
        .collect()
}

/// One file that was read.
pub struct FileListing<'a> {
    /// The path as the listing shows it.
    pub file: &'a str,
    pub scan: &'a Scan,
    pub comments: &'a FileComments,
}

/// One file that could not be read, and why.
#[derive(Debug, Clone, Serialize)]
pub struct Unreadable {
    pub file: String,
    pub message: String,
}

#[derive(Serialize)]
struct JsonObject<'a> {
    id: i64,
    name: &'a str,
    class: &'a str,
    path: Vec<&'a str>,
}

#[derive(Serialize)]
struct JsonChange {
    /// `new` or `updated`.
    thread: &'static str,
    /// Indices into `messages`: every one for a new thread.
    new_messages: Vec<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_status: Option<Status>,
}

impl JsonChange {
    fn of(change: &Change, thread: &Thread) -> Self {
        match change {
            Change::New => Self {
                thread: "new",
                new_messages: (0..thread.messages.len()).collect(),
                previous_status: None,
            },
            Change::Updated {
                new_messages,
                previous_status,
            } => Self {
                thread: "updated",
                new_messages: new_messages.clone(),
                previous_status: *previous_status,
            },
        }
    }
}

#[derive(Serialize)]
struct JsonThread<'a> {
    number: usize,
    object: JsonObject<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    change: Option<JsonChange>,
    #[serde(flatten)]
    thread: &'a Thread,
}

#[derive(Serialize)]
struct JsonWarning<'a> {
    object: &'a str,
    message: String,
}

#[derive(Serialize)]
struct JsonFile<'a> {
    file: &'a str,
    format: String,
    threads: Vec<JsonThread<'a>>,
    warnings: Vec<JsonWarning<'a>>,
}

#[derive(Serialize)]
struct JsonListing<'a> {
    scanned: usize,
    files: Vec<JsonFile<'a>>,
    errors: &'a [Unreadable],
}

fn format_label(format: Format) -> String {
    match format {
        Format::Binary { version } => format!("binary {version}"),
        Format::Ascii => "ascii".to_owned(),
    }
}

/// The listing as pretty-printed JSON: `scanned` (how many files were looked
/// at), the `files` with something to list — each thread being the stored
/// thread with its `number`, the `object` it is on and, against a baseline, its
/// `change` added — and the `errors` for the files that could not be read.
///
/// An unfiltered listing is what [`Baseline::from_json`] reads back.
pub fn json(files: &[FileListing<'_>], errors: &[Unreadable], selection: Selection<'_>) -> String {
    let scanned = files.len() + errors.len();
    let files = files
        .iter()
        .filter_map(|listing| {
            let threads: Vec<JsonThread> = numbered(listing.comments, selection)
                .into_iter()
                .map(|entry| json_thread(listing.scan, entry))
                .collect();
            let warnings = warnings(listing.scan, listing.comments);
            (!threads.is_empty() || !warnings.is_empty()).then(|| JsonFile {
                file: listing.file,
                format: format_label(listing.scan.format),
                threads,
                warnings,
            })
        })
        .collect();
    let report = JsonListing {
        scanned,
        files,
        errors,
    };
    serde_json::to_string_pretty(&report).unwrap_or_default()
}

fn json_thread<'a>(scan: &'a Scan, entry: Numbered<'a>) -> JsonThread<'a> {
    let object = &scan.models[entry.model];
    JsonThread {
        number: entry.number,
        object: JsonObject {
            id: object.id,
            name: &object.name,
            class: &object.class,
            path: object_path(scan, entry.model),
        },
        change: entry
            .change
            .as_ref()
            .map(|change| JsonChange::of(change, entry.thread)),
        thread: entry.thread,
    }
}

fn warnings<'a>(scan: &'a Scan, comments: &FileComments) -> Vec<JsonWarning<'a>> {
    comments
        .warnings
        .iter()
        .map(|warning| JsonWarning {
            object: scan
                .models
                .get(warning.model)
                .map_or("", |model| model.name.as_str()),
            message: match &warning.error {
                CodecError::Json(detail) => format!("unreadable comments: {detail}"),
                other => other.to_string(),
            },
        })
        .collect()
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The listing as a Markdown report. One file is reported on its own, grouped
/// by object and then by status; several get a summary first and then a section
/// each for the files with something to list.
pub fn markdown(
    files: &[FileListing<'_>],
    errors: &[Unreadable],
    selection: Selection<'_>,
) -> String {
    use std::fmt::Write;

    let mut out = String::new();
    if let ([listing], []) = (files, errors) {
        let entries = numbered(listing.comments, selection);
        file_markdown(&mut out, listing, &entries, selection, 1);
        return out;
    }

    let scanned = files.len() + errors.len();
    let sections: Vec<(&FileListing, Vec<Numbered>)> = files
        .iter()
        .map(|listing| (listing, numbered(listing.comments, selection)))
        .filter(|(listing, entries)| !entries.is_empty() || !listing.comments.warnings.is_empty())
        .collect();
    let _ = writeln!(out, "# Review comments\n");
    let scanned_text = count(scanned, "file", "files");
    let _ = match (sections.len(), selection.since.is_some()) {
        (0, true) => writeln!(out, "Nothing new since the baseline in {scanned_text}.\n"),
        (0, false) => writeln!(out, "No review comments in {scanned_text}.\n"),
        (listed, _) => writeln!(out, "{scanned_text} scanned; {listed} listed below.\n"),
    };
    for error in errors {
        let _ = writeln!(out, "> Could not read {}: {}\n", error.file, error.message);
    }
    for (listing, entries) in &sections {
        file_markdown(&mut out, listing, entries, selection, 2);
    }
    out
}

/// One file's section, its heading at `level`.
fn file_markdown(
    out: &mut String,
    listing: &FileListing<'_>,
    entries: &[Numbered<'_>],
    selection: Selection<'_>,
    level: usize,
) {
    use std::fmt::Write;

    let (scan, hashes) = (listing.scan, "#".repeat(level));
    if level == 1 {
        let _ = writeln!(out, "# Review comments: {}\n", listing.file);
    } else {
        let _ = writeln!(out, "{hashes} {}\n", listing.file);
    }
    if selection.since.is_some() {
        let new = entries
            .iter()
            .filter(|e| e.change == Some(Change::New))
            .count();
        let _ = writeln!(out, "{new} new, {} updated\n", entries.len() - new);
    } else {
        let open = entries
            .iter()
            .filter(|e| e.thread.status == Status::Open)
            .count();
        let _ = writeln!(out, "{open} open, {} resolved\n", entries.len() - open);
    }

    let mut objects: Vec<usize> = entries.iter().map(|entry| entry.model).collect();
    objects.dedup();
    for model in objects {
        let object = &scan.models[model];
        let path = object_path(scan, model);
        if path.len() > 1 {
            let _ = writeln!(out, "{hashes}# {} ({})\n", object.name, path.join(" / "));
        } else {
            let _ = writeln!(out, "{hashes}# {}\n", object.name);
        }
        for status in [Status::Open, Status::Resolved] {
            for entry in entries
                .iter()
                .filter(|entry| entry.model == model && entry.thread.status == status)
            {
                thread_markdown(out, entry, level + 2);
            }
        }
    }
    for warning in warnings(scan, listing.comments) {
        let _ = writeln!(out, "> Warning ({}): {}\n", warning.object, warning.message);
    }
}

fn status_label(status: Status) -> &'static str {
    match status {
        Status::Open => "Open",
        Status::Resolved => "Resolved",
    }
}

fn thread_markdown(out: &mut String, entry: &Numbered<'_>, level: usize) {
    use std::fmt::Write;

    let thread = entry.thread;
    let number = format!("#{}", entry.number);
    let status = match &entry.change {
        Some(Change::Updated {
            previous_status: Some(previous),
            ..
        }) => format!(
            "{} (was {})",
            status_label(thread.status),
            status_label(*previous)
        ),
        _ => status_label(thread.status).to_owned(),
    };
    let new = if entry.change == Some(Change::New) {
        "New"
    } else {
        ""
    };
    let time = thread
        .messages
        .first()
        .map_or("", |message| message.time.as_str());
    let heading: Vec<&str> = [number.as_str(), new, &status, thread.author(), time]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect();
    let _ = writeln!(out, "{} {}\n", "#".repeat(level), heading.join(" · "));
    for line in thread.title().lines() {
        let _ = writeln!(out, "> {line}");
    }
    out.push('\n');
    let mut facts = Vec::new();
    if let Some(frames) = &thread.frames {
        facts.push(if frames.start == frames.end {
            format!("{} frame {}", frames.clip, frames.start)
        } else {
            format!("{} frames {}-{}", frames.clip, frames.start, frames.end)
        });
    }
    if let Some(anchor) = thread.anchor.as_ref().and_then(|anchor| anchor.known()) {
        facts.push(
            match anchor {
                Anchor::Surface { .. } => "pinned to the surface",
                Anchor::World { .. } => "pinned in the scene",
                Anchor::Uv { .. } => "pinned on the UV layout",
            }
            .to_owned(),
        );
    }
    if thread.view.is_some() {
        facts.push("camera view saved".to_owned());
    }
    if !facts.is_empty() {
        let _ = writeln!(out, "{}\n", facts.join(" · "));
    }
    // Against a baseline, an updated thread shows only the replies it lacked.
    let shown = |index: &usize| match &entry.change {
        Some(Change::Updated { new_messages, .. }) => new_messages.contains(index),
        _ => true,
    };
    let replies = thread.messages.len().saturating_sub(1);
    let earlier = (1..thread.messages.len())
        .filter(|index| !shown(index))
        .count();
    if earlier > 0 {
        let _ = writeln!(
            out,
            "- *{}*",
            count(earlier, "earlier reply", "earlier replies")
        );
    }
    for (_, reply) in thread
        .messages
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(index, _)| shown(index))
    {
        let _ = writeln!(
            out,
            "- **{}** ({}): {}",
            reply.author,
            reply.time,
            reply.text.replace('\n', " ")
        );
    }
    if replies > 0 {
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::encode;
    use crate::fbx::{ModelObject, StoredString};
    use crate::thread::{Message, Scope};

    fn thread(id: &str, status: Status, text: &str) -> Thread {
        Thread {
            id: id.to_owned(),
            status,
            scope: Scope::Object,
            anchor: None,
            frames: None,
            view: None,
            messages: vec![Message {
                author: "Ana".to_owned(),
                time: "2026-10-08T12:00:00Z".to_owned(),
                text: text.to_owned(),
                extra: Default::default(),
            }],
            extra: Default::default(),
        }
    }

    fn file() -> (Scan, FileComments) {
        let scan = Scan {
            format: Format::Binary { version: 7700 },
            models: vec![
                ModelObject {
                    id: 1,
                    name: "Body".to_owned(),
                    class: "Mesh".to_owned(),
                    parent: None,
                },
                ModelObject {
                    id: 2,
                    name: "Hand".to_owned(),
                    class: "Mesh".to_owned(),
                    parent: Some(1),
                },
            ],
            strings: vec![
                StoredString {
                    model: 0,
                    value: encode(&[thread("a", Status::Resolved, "Fixed seam")], &[]),
                    hidden: false,
                },
                StoredString {
                    model: 1,
                    value: encode(&[thread("b", Status::Open, "Thumb clips")], &[]),
                    hidden: false,
                },
            ],
        };
        let comments = FileComments::from_scan(&scan);
        (scan, comments)
    }

    fn only(status: StatusFilter) -> Selection<'static> {
        Selection {
            status,
            since: None,
        }
    }

    fn listing<'a>(file: &'a str, scan: &'a Scan, comments: &'a FileComments) -> FileListing<'a> {
        FileListing {
            file,
            scan,
            comments,
        }
    }

    /// Numbers count every thread in the file, so a filter never renumbers.
    #[test]
    fn numbers_survive_a_filter() {
        let (_, comments) = file();
        let open = numbered(&comments, only(StatusFilter::Open));
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].number, 2);
    }

    #[test]
    fn the_markdown_groups_by_object() {
        let (scan, comments) = file();
        let text = markdown(
            &[listing("x.fbx", &scan, &comments)],
            &[],
            Selection::default(),
        );
        assert!(text.contains("1 open, 1 resolved"));
        assert!(text.contains("## Hand (Body / Hand)"));
        assert!(text.contains("### #2 · Open · Ana · 2026-10-08T12:00:00Z"));
        assert!(text.contains("> Thumb clips"));
    }

    #[test]
    fn the_json_carries_the_object_and_the_thread() {
        let (scan, comments) = file();
        let text = json(
            &[listing("x.fbx", &scan, &comments)],
            &[],
            Selection::default(),
        );
        let value: serde_json::Value = serde_json::from_str(&text).expect("json");
        assert_eq!(value["scanned"], 1);
        let second = &value["files"][0]["threads"][1];
        assert_eq!(second["number"], 2);
        assert_eq!(
            second["object"]["path"],
            serde_json::json!(["Body", "Hand"])
        );
        assert_eq!(second["status"], "open");
        assert_eq!(second["messages"][0]["text"], "Thumb clips");
        assert!(second.get("change").is_none());
    }

    /// Several files get a summary, a section for each file with something to
    /// list, and a line for each one that could not be read.
    #[test]
    fn several_files_are_summarised() {
        let (scan, comments) = file();
        let empty = FileComments::default();
        let errors = [Unreadable {
            file: "broken.fbx".to_owned(),
            message: "not an FBX file".to_owned(),
        }];
        let files = [
            listing("a.fbx", &scan, &comments),
            listing("b.fbx", &scan, &empty),
        ];
        let text = markdown(&files, &errors, Selection::default());
        assert!(text.contains("3 files scanned; 1 listed below."));
        assert!(text.contains("> Could not read broken.fbx: not an FBX file"));
        assert!(text.contains("## a.fbx"));
        assert!(!text.contains("b.fbx"));
        assert!(text.contains("### Hand (Body / Hand)"));
        assert!(text.contains("#### #2 · Open · Ana"));
    }

    /// The listing an update writes is what the next run compares against: the
    /// same files report nothing, and a reply or a new thread is all that shows.
    #[test]
    fn a_listing_round_trips_as_a_baseline() {
        let (scan, comments) = file();
        let files = [listing("x.fbx", &scan, &comments)];
        let saved = json(&files, &[], Selection::default());
        let baseline = Baseline::from_json(&saved).expect("baseline");
        let since = Selection {
            status: StatusFilter::All,
            since: Some(&baseline),
        };
        assert!(numbered(&comments, since).is_empty());
        let text = markdown(&files, &[], since);
        assert!(text.contains("0 new, 0 updated"));

        let mut changed = file().0;
        let mut threads = vec![thread("b", Status::Resolved, "Thumb clips")];
        threads[0].messages.push(Message {
            author: "Raj".to_owned(),
            time: "2026-10-09T09:00:00Z".to_owned(),
            text: "Fixed in the rig".to_owned(),
            extra: Default::default(),
        });
        threads.push(thread("c", Status::Open, "Hand too small"));
        changed.strings[1].value = encode(&threads, &[]);
        let changed_comments = FileComments::from_scan(&changed);
        let entries = numbered(&changed_comments, since);
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0].change,
            Some(Change::Updated {
                new_messages: vec![1],
                previous_status: Some(Status::Open),
            })
        );
        assert_eq!(entries[1].change, Some(Change::New));

        let changed_files = [listing("x.fbx", &changed, &changed_comments)];
        let text = markdown(&changed_files, &[], since);
        assert!(text.contains("1 new, 1 updated"));
        assert!(text.contains("### #2 · Resolved (was Open) · Ana"));
        assert!(text.contains("- **Raj** (2026-10-09T09:00:00Z): Fixed in the rig"));
        assert!(text.contains("### #3 · New · Open · Ana"));

        let value: serde_json::Value =
            serde_json::from_str(&json(&changed_files, &[], since)).expect("json");
        let threads = &value["files"][0]["threads"];
        assert_eq!(threads[0]["change"]["thread"], "updated");
        assert_eq!(threads[0]["change"]["new_messages"], serde_json::json!([1]));
        assert_eq!(threads[0]["change"]["previous_status"], "open");
        assert_eq!(threads[1]["change"]["thread"], "new");
    }

    /// Against a baseline, an updated thread shows only its new replies.
    #[test]
    fn earlier_replies_are_folded() {
        let mut seen = thread("a", Status::Open, "Seam");
        for text in ["One", "Two"] {
            seen.messages.push(Message {
                author: "Raj".to_owned(),
                time: String::new(),
                text: text.to_owned(),
                extra: Default::default(),
            });
        }
        let baseline = Baseline::from_threads([&seen]);
        let mut current = seen.clone();
        current.messages.push(Message {
            author: "Ana".to_owned(),
            time: String::new(),
            text: "Three".to_owned(),
            extra: Default::default(),
        });
        let mut out = String::new();
        let entry = Numbered {
            number: 1,
            model: 0,
            change: baseline.change(&current),
            thread: &current,
        };
        thread_markdown(&mut out, &entry, 3);
        assert!(out.contains("- *2 earlier replies*"));
        assert!(out.contains("**Ana** (): Three"));
        assert!(!out.contains("One"));
    }
}
