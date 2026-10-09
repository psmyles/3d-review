//! The comments of a file as JSON for scripts, or as a Markdown report for people.

use serde::Serialize;

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

/// One thread with its number (its position among all of the file's threads,
/// from 1) and the object it is stored on.
pub struct Numbered<'a> {
    pub number: usize,
    pub model: usize,
    pub thread: &'a Thread,
}

/// The threads `filter` admits, numbered across the whole file — so a thread's
/// number doesn't change with the filter.
pub fn numbered<'a>(comments: &'a FileComments, filter: StatusFilter) -> Vec<Numbered<'a>> {
    comments
        .threads()
        .enumerate()
        .filter(|(_, (_, thread))| filter.admits(thread.status))
        .map(|(index, (model, thread))| Numbered {
            number: index + 1,
            model,
            thread,
        })
        .collect()
}

#[derive(Serialize)]
struct JsonObject<'a> {
    id: i64,
    name: &'a str,
    class: &'a str,
    path: Vec<&'a str>,
}

#[derive(Serialize)]
struct JsonThread<'a> {
    number: usize,
    object: JsonObject<'a>,
    #[serde(flatten)]
    thread: &'a Thread,
}

#[derive(Serialize)]
struct JsonWarning<'a> {
    object: &'a str,
    message: String,
}

#[derive(Serialize)]
struct JsonReport<'a> {
    file: &'a str,
    format: String,
    threads: Vec<JsonThread<'a>>,
    warnings: Vec<JsonWarning<'a>>,
}

fn format_label(format: Format) -> String {
    match format {
        Format::Binary { version } => format!("binary {version}"),
        Format::Ascii => "ascii".to_owned(),
    }
}

/// The listing as pretty-printed JSON. Each thread is the stored thread with
/// `number` and the `object` it is on added.
pub fn json(file: &str, scan: &Scan, comments: &FileComments, filter: StatusFilter) -> String {
    let threads = numbered(comments, filter)
        .into_iter()
        .map(|entry| {
            let object = &scan.models[entry.model];
            JsonThread {
                number: entry.number,
                object: JsonObject {
                    id: object.id,
                    name: &object.name,
                    class: &object.class,
                    path: object_path(scan, entry.model),
                },
                thread: entry.thread,
            }
        })
        .collect();
    let report = JsonReport {
        file,
        format: format_label(scan.format),
        threads,
        warnings: warnings(scan, comments),
    };
    serde_json::to_string_pretty(&report).unwrap_or_default()
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

/// The listing as a Markdown report, grouped by object and then by status.
pub fn markdown(file: &str, scan: &Scan, comments: &FileComments, filter: StatusFilter) -> String {
    use std::fmt::Write;

    let entries = numbered(comments, filter);
    let open = entries
        .iter()
        .filter(|e| e.thread.status == Status::Open)
        .count();
    let resolved = entries.len() - open;
    let mut out = String::new();
    let _ = writeln!(out, "# Review comments: {file}\n");
    let _ = writeln!(out, "{open} open, {resolved} resolved\n");

    let mut objects: Vec<usize> = entries.iter().map(|entry| entry.model).collect();
    objects.dedup();
    for model in objects {
        let object = &scan.models[model];
        let path = object_path(scan, model);
        if path.len() > 1 {
            let _ = writeln!(out, "## {} ({})\n", object.name, path.join(" / "));
        } else {
            let _ = writeln!(out, "## {}\n", object.name);
        }
        for status in [Status::Open, Status::Resolved] {
            for entry in entries
                .iter()
                .filter(|entry| entry.model == model && entry.thread.status == status)
            {
                thread_markdown(&mut out, entry);
            }
        }
    }
    for warning in warnings(scan, comments) {
        let _ = writeln!(out, "> Warning ({}): {}\n", warning.object, warning.message);
    }
    out
}

fn thread_markdown(out: &mut String, entry: &Numbered<'_>) {
    use std::fmt::Write;

    let thread = entry.thread;
    let status = match thread.status {
        Status::Open => "Open",
        Status::Resolved => "Resolved",
    };
    let number = format!("#{}", entry.number);
    let time = thread
        .messages
        .first()
        .map_or("", |message| message.time.as_str());
    let heading: Vec<&str> = [number.as_str(), status, thread.author(), time]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect();
    let _ = writeln!(out, "### {}\n", heading.join(" · "));
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
    for reply in thread.messages.iter().skip(1) {
        let _ = writeln!(
            out,
            "- **{}** ({}): {}",
            reply.author,
            reply.time,
            reply.text.replace('\n', " ")
        );
    }
    if thread.messages.len() > 1 {
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

    /// Numbers count every thread in the file, so a filter never renumbers.
    #[test]
    fn numbers_survive_a_filter() {
        let (_, comments) = file();
        let open = numbered(&comments, StatusFilter::Open);
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].number, 2);
    }

    #[test]
    fn the_markdown_groups_by_object() {
        let (scan, comments) = file();
        let text = markdown("x.fbx", &scan, &comments, StatusFilter::All);
        assert!(text.contains("1 open, 1 resolved"));
        assert!(text.contains("## Hand (Body / Hand)"));
        assert!(text.contains("### #2 · Open · Ana · 2026-10-08T12:00:00Z"));
        assert!(text.contains("> Thumb clips"));
    }

    #[test]
    fn the_json_carries_the_object_and_the_thread() {
        let (scan, comments) = file();
        let value: serde_json::Value =
            serde_json::from_str(&json("x.fbx", &scan, &comments, StatusFilter::All))
                .expect("json");
        let second = &value["threads"][1];
        assert_eq!(second["number"], 2);
        assert_eq!(
            second["object"]["path"],
            serde_json::json!(["Body", "Hand"])
        );
        assert_eq!(second["status"], "open");
        assert_eq!(second["messages"][0]["text"], "Thumb clips");
    }
}
