//! What a reader has already seen, and what has changed since: the comparison
//! behind `review-comments --baseline`.
//!
//! A baseline is any earlier `review-comments --json` listing. Threads are
//! matched by `id` across *every* file in it, not by the file they were in, so a
//! file that was moved or renamed since reports nothing new. Messages are
//! matched by author, time and text together — the stored `time` is the
//! writer's clock when they wrote it, not when it reached this reader, so a
//! "newer than my last check" test on it would miss a comment written before
//! the check and pushed after it.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use crate::thread::{Message, Status, Thread};

/// What changed in one thread since the baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// The baseline never saw the thread.
    New,
    /// The baseline saw the thread; it has messages since, or another status.
    Updated {
        /// Indices into the thread's `messages` of the ones the baseline lacks.
        new_messages: Vec<usize>,
        /// The status the baseline had, when it differs from the current one.
        previous_status: Option<Status>,
    },
}

#[derive(Default)]
struct Seen {
    open: bool,
    resolved: bool,
    messages: HashSet<MessageKey>,
}

#[derive(PartialEq, Eq, Hash)]
struct MessageKey {
    author: String,
    time: String,
    text: String,
}

impl MessageKey {
    fn of(message: &Message) -> Self {
        Self {
            author: message.author.clone(),
            time: message.time.clone(),
            text: message.text.clone(),
        }
    }
}

/// The threads and messages an earlier listing held.
#[derive(Default)]
pub struct Baseline {
    threads: HashMap<String, Seen>,
}

/// The parts of a `--json` listing a baseline reads. Every other field — the
/// thread's object, its number, anchors — is ignored.
#[derive(Deserialize)]
struct Listing {
    files: Vec<ListedFile>,
}

#[derive(Deserialize)]
struct ListedFile {
    #[serde(default)]
    threads: Vec<Thread>,
}

/// How a thread is recognised. A thread written by hand may have no `id` (the
/// viewer assigns one only when it saves), so such a thread is known by its
/// opening message instead; the `\0` keeps that key from ever equalling an id.
fn thread_key(thread: &Thread) -> String {
    if !thread.id.is_empty() {
        return thread.id.clone();
    }
    let first = thread.messages.first();
    let part = |field: fn(&Message) -> &str| first.map_or("", field);
    format!(
        "\0{}\0{}\0{}",
        part(|m| &m.author),
        part(|m| &m.time),
        part(|m| &m.text)
    )
}

impl Baseline {
    /// A baseline from a `review-comments --json` listing.
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        let listing: Listing = serde_json::from_str(text)?;
        Ok(Self::from_threads(
            listing.files.iter().flat_map(|file| &file.threads),
        ))
    }

    /// A baseline that has seen exactly `threads`.
    pub fn from_threads<'a>(threads: impl IntoIterator<Item = &'a Thread>) -> Self {
        let mut baseline = Self::default();
        for thread in threads {
            // A copy of a file holds the same ids, so a key can recur: what
            // either copy held has been seen.
            let seen = baseline.threads.entry(thread_key(thread)).or_default();
            match thread.status {
                Status::Open => seen.open = true,
                Status::Resolved => seen.resolved = true,
            }
            seen.messages
                .extend(thread.messages.iter().map(MessageKey::of));
        }
        baseline
    }

    /// What changed in `thread` since the baseline, or `None` when nothing did.
    pub fn change(&self, thread: &Thread) -> Option<Change> {
        let Some(seen) = self.threads.get(&thread_key(thread)) else {
            return Some(Change::New);
        };
        let new_messages: Vec<usize> = thread
            .messages
            .iter()
            .enumerate()
            .filter(|(_, message)| !seen.messages.contains(&MessageKey::of(message)))
            .map(|(index, _)| index)
            .collect();
        let previous_status = match (thread.status, seen.open, seen.resolved) {
            (Status::Open, false, true) => Some(Status::Resolved),
            (Status::Resolved, true, false) => Some(Status::Open),
            _ => None,
        };
        if new_messages.is_empty() && previous_status.is_none() {
            None
        } else {
            Some(Change::Updated {
                new_messages,
                previous_status,
            })
        }
    }

    /// How many threads the baseline holds.
    pub fn len(&self) -> usize {
        self.threads.len()
    }

    pub fn is_empty(&self) -> bool {
        self.threads.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thread::Scope;

    fn message(author: &str, time: &str, text: &str) -> Message {
        Message {
            author: author.to_owned(),
            time: time.to_owned(),
            text: text.to_owned(),
            extra: Default::default(),
        }
    }

    fn thread(id: &str, status: Status, messages: Vec<Message>) -> Thread {
        Thread {
            id: id.to_owned(),
            status,
            scope: Scope::Object,
            anchor: None,
            frames: None,
            view: None,
            messages,
            extra: Default::default(),
        }
    }

    fn opening() -> Message {
        message("Ana", "2026-10-08T12:00:00Z", "Thumb clips")
    }

    #[test]
    fn an_unseen_thread_is_new() {
        let baseline = Baseline::default();
        let current = thread("a", Status::Open, vec![opening()]);
        assert_eq!(baseline.change(&current), Some(Change::New));
    }

    #[test]
    fn an_unchanged_thread_is_not_reported() {
        let seen = thread("a", Status::Open, vec![opening()]);
        let baseline = Baseline::from_threads([&seen]);
        assert_eq!(baseline.change(&seen), None);
    }

    /// The reply is older than anything a time-based check would have kept —
    /// written before the last check, arriving after it — and is still new.
    #[test]
    fn a_reply_is_new_by_content_not_by_time() {
        let seen = thread("a", Status::Open, vec![opening()]);
        let baseline = Baseline::from_threads([&seen]);
        let current = thread(
            "a",
            Status::Resolved,
            vec![opening(), message("Raj", "2020-01-01T00:00:00Z", "Fixed")],
        );
        assert_eq!(
            baseline.change(&current),
            Some(Change::Updated {
                new_messages: vec![1],
                previous_status: Some(Status::Open),
            })
        );
    }

    #[test]
    fn a_thread_without_an_id_is_known_by_its_opening_message() {
        let seen = thread("", Status::Open, vec![opening()]);
        let baseline = Baseline::from_threads([&seen]);
        assert_eq!(baseline.change(&seen), None);
        let other = thread("", Status::Open, vec![message("Ana", "", "Other")]);
        assert_eq!(baseline.change(&other), Some(Change::New));
    }

    /// Any earlier `--json` listing is a baseline: its extra fields (`number`,
    /// `object`, `change`) are ignored, and a moved file's threads are still
    /// recognised because matching is by id.
    #[test]
    fn a_json_listing_is_a_baseline() {
        let listing = r#"{"files": [{"file": "old/path.fbx", "format": "ascii",
            "threads": [{"number": 1, "object": {"id": 1, "name": "Body"},
                "id": "a", "status": "open",
                "messages": [{"author": "Ana", "time": "2026-10-08T12:00:00Z",
                    "text": "Thumb clips"}]}],
            "warnings": []}], "errors": []}"#;
        let baseline = Baseline::from_json(listing).expect("baseline");
        assert_eq!(baseline.len(), 1);
        assert_eq!(
            baseline.change(&thread("a", Status::Open, vec![opening()])),
            None
        );
    }

    #[test]
    fn json_that_is_not_a_listing_is_refused() {
        assert!(Baseline::from_json(r#"{"threads": []}"#).is_err());
    }
}
