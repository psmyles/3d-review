//! The review comments of the loaded file, as the chrome shows them: the threads
//! (read by `app` from the FBX and mapped onto scene nodes), which are listed,
//! which one is selected, and where `app` resolved each pin to this frame.

use glam::Vec3;
use review_annotate::thread::{Status, Thread};

/// Which threads the Comments tab and the viewport pins show.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CommentFilter {
    /// The default: resolved threads are out of the way until asked for.
    #[default]
    Open,
    Resolved,
    All,
}

impl CommentFilter {
    /// In display order (the Comments tab's segmented control).
    pub const ALL: [CommentFilter; 3] = [
        CommentFilter::Open,
        CommentFilter::Resolved,
        CommentFilter::All,
    ];

    pub fn admits(self, status: Status) -> bool {
        match self {
            CommentFilter::Open => status == Status::Open,
            CommentFilter::Resolved => status == Status::Resolved,
            CommentFilter::All => true,
        }
    }
}

/// One thread of the loaded file.
#[derive(Debug, Clone, PartialEq)]
pub struct CommentEntry {
    pub thread: Thread,
    /// The scene node the thread is stored on; `None` when the file's object could
    /// not be matched to one (the thread is still listed).
    pub node: Option<usize>,
    /// The FBX object id it is stored on — what a save writes back to.
    pub object: i64,
    /// That object's name, shown even when `node` is `None`.
    pub object_name: String,
    /// Read from a newer payload version: shown, never rewritten.
    pub read_only: bool,
}

/// Where one pin is this frame, resolved by `app` from the thread's anchor and
/// the current pose. Plain values, app → UI (invariant 2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CommentPin {
    /// Index into [`CommentsState::threads`].
    pub thread: usize,
    /// The pinned point, in the scene's world space.
    pub world: Vec3,
    /// Whether the point sits on a surface — the occlusion test then ignores the
    /// surface it sits on.
    pub on_surface: bool,
    /// Whether the thread's frame range includes the frame on screen (a thread
    /// with no range always does). A pin outside it is drawn ghosted.
    pub in_range: bool,
}

/// The review comments of the loaded file.
#[derive(Debug, Clone)]
pub struct CommentsState {
    /// Every thread, in file order; a thread's number is its index plus one.
    pub threads: Vec<CommentEntry>,
    /// Bumped whenever `threads` changes.
    pub revision: u64,
    pub filter: CommentFilter,
    /// The selected thread.
    pub selected: Option<usize>,
    /// Whether the Inspector shows the selected thread rather than the scene
    /// selection — see [`crate::UiState::comment_inspected`].
    pub inspected: bool,
    /// Whether the viewport draws the pins.
    pub show_pins: bool,
    /// This frame's pins, set by `app` before the chrome is drawn.
    pub pins: Vec<CommentPin>,
    /// Per scene node, how many threads the filter admits on it — the Outliner's
    /// badges. Rebuilt by [`CommentsState::refresh_counts`] when its key moves.
    counts: Vec<u32>,
    counts_key: Option<(u64, CommentFilter, usize)>,
}

impl Default for CommentsState {
    fn default() -> Self {
        Self {
            threads: Vec::new(),
            revision: 0,
            filter: CommentFilter::default(),
            selected: None,
            inspected: false,
            show_pins: true,
            pins: Vec::new(),
            counts: Vec::new(),
            counts_key: None,
        }
    }
}

impl CommentsState {
    /// Replace the threads with a newly loaded file's, keeping the view settings
    /// (filter, pins on or off) and dropping everything that pointed into the old
    /// list.
    pub fn load(&mut self, threads: Vec<CommentEntry>) {
        self.threads = threads;
        self.revision = self.revision.wrapping_add(1);
        self.selected = None;
        self.inspected = false;
        self.pins.clear();
        self.counts_key = None;
    }

    /// Drop every thread (a new model is loading).
    pub fn clear(&mut self) {
        self.load(Vec::new());
    }

    /// Whether the filter lists thread `index`.
    pub fn listed(&self, index: usize) -> bool {
        self.threads
            .get(index)
            .is_some_and(|entry| self.filter.admits(entry.thread.status))
    }

    /// Rebuild the per-node counts if the threads, the filter or the model moved.
    pub(crate) fn refresh_counts(&mut self, node_count: usize) {
        let key = (self.revision, self.filter, node_count);
        if self.counts_key == Some(key) {
            return;
        }
        self.counts = vec![0; node_count];
        for entry in &self.threads {
            if let Some(slot) = entry
                .node
                .filter(|_| self.filter.admits(entry.thread.status))
                .and_then(|node| self.counts.get_mut(node))
            {
                *slot += 1;
            }
        }
        self.counts_key = Some(key);
    }

    /// How many listed threads node `node` carries (after
    /// [`CommentsState::refresh_counts`]).
    pub(crate) fn count_on(&self, node: usize) -> u32 {
        self.counts.get(node).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use review_annotate::thread::{Message, Scope};

    fn entry(node: Option<usize>, status: Status) -> CommentEntry {
        CommentEntry {
            thread: Thread {
                id: String::new(),
                status,
                scope: Scope::Object,
                anchor: None,
                frames: None,
                view: None,
                messages: vec![Message {
                    author: String::new(),
                    time: String::new(),
                    text: String::new(),
                    extra: Default::default(),
                }],
                extra: Default::default(),
            },
            node,
            object: 0,
            object_name: String::new(),
            read_only: false,
        }
    }

    #[test]
    fn counts_follow_the_filter() {
        let mut comments = CommentsState::default();
        comments.load(vec![
            entry(Some(1), Status::Open),
            entry(Some(1), Status::Resolved),
            entry(Some(2), Status::Open),
            entry(None, Status::Open),
        ]);
        comments.refresh_counts(3);
        assert_eq!(
            [
                comments.count_on(0),
                comments.count_on(1),
                comments.count_on(2)
            ],
            [0, 1, 1]
        );
        comments.filter = CommentFilter::All;
        comments.refresh_counts(3);
        assert_eq!(comments.count_on(1), 2);
    }
}
