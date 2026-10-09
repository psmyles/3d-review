//! The review comments of the loaded file, as the chrome shows them: the threads
//! (read by `app` from the FBX and mapped onto scene nodes), which are listed,
//! which one is selected, and where `app` resolved each pin to this frame.

use glam::Vec3;
use review_annotate::thread::{
    Anchor, AnchorValue, FrameRange, Message, SavedView, Scope, Status, Thread, new_thread_id,
    utc_now,
};

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

/// An FBX object a comment can be stored on: its id and its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectRef {
    pub id: i64,
    pub name: String,
}

/// Where a comment being written points.
#[derive(Debug, Clone, PartialEq)]
pub enum DraftAnchor {
    /// A click on the mesh: a surface anchor on `node` (a [`Anchor::Surface`]),
    /// at `world` this frame. The composer can make it a fixed scene point instead.
    Surface {
        node: usize,
        anchor: Anchor,
        world: Vec3,
    },
    /// A click on empty space: the comment is about the view, not a point.
    View,
    /// An object picked in the Outliner, with no particular point on it.
    Object { node: usize },
    /// A note about the whole file.
    File,
}

/// A comment being written, before it is posted.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub anchor: DraftAnchor,
    pub text: String,
    /// For a surface click: whether the pin follows the surface (through
    /// animation and skinning) or stays at the clicked point in the scene.
    pub follow_surface: bool,
    /// The clip and frame on screen when the draft was started, if any.
    pub frames: Option<FrameRange>,
    pub include_frames: bool,
    /// The camera when the draft was started.
    pub view: SavedView,
    pub include_view: bool,
    /// Where the composer opens when the draft has no point of its own to sit by
    /// (a view-only note, an object or the file): the click, or the viewport's
    /// centre.
    pub screen: egui::Pos2,
    /// Set once the composer has taken keyboard focus.
    pub focused: bool,
}

/// Why comments can't be written right now, when they can't.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotWritable {
    /// No file is loaded (the start state).
    NoFile,
    /// The file couldn't be read for comments (an FBX 6 file, say), so nothing
    /// could be written back to it either.
    Unreadable,
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

    // ── Writing ────────────────────────────────────────────────────────────
    /// The comment being written, if any.
    pub draft: Option<Draft>,
    /// The name new messages are signed with; asked for on the first comment and
    /// remembered by `app` in the settings file.
    pub author: String,
    /// Per scene node, the FBX object it was imported from — what a comment on it
    /// is stored on. Set by `app` with the comments.
    pub node_objects: Vec<Option<ObjectRef>>,
    /// The object notes about the whole file are stored on, and its node.
    pub file_host: Option<(ObjectRef, Option<usize>)>,
    /// Why comments can't be written, when they can't.
    pub not_writable: Option<NotWritable>,
    /// The threads as last loaded or saved — what "unsaved changes" compares to.
    saved: Vec<CommentEntry>,
    dirty_key: Option<(u64, u64)>,
    dirty: bool,
    /// Bumped whenever `saved` changes.
    saved_revision: u64,
    /// While set, the Comment tool's next click moves this thread's pin instead of
    /// starting a new comment.
    pub repin: Option<usize>,
    /// The Inspector's reply box, for the selected thread.
    pub reply: String,
    /// A message being edited in the Inspector: (thread, message, text).
    pub editing: Option<(usize, usize, String)>,
    /// A thread whose Delete was clicked once and awaits the second click.
    pub confirm_delete: Option<usize>,
    /// The current camera as a saved view, refreshed by the overlay each frame —
    /// what a draft started from the chrome (rather than a viewport click)
    /// records.
    pub view_now: Option<SavedView>,
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
            draft: None,
            author: String::new(),
            node_objects: Vec::new(),
            file_host: None,
            not_writable: Some(NotWritable::NoFile),
            saved: Vec::new(),
            dirty_key: None,
            dirty: false,
            saved_revision: 0,
            repin: None,
            reply: String::new(),
            editing: None,
            confirm_delete: None,
            view_now: None,
        }
    }
}

impl CommentsState {
    /// Replace the threads with a newly loaded file's, keeping the view settings
    /// (filter, pins on or off) and dropping everything that pointed into the old
    /// list.
    pub fn load(&mut self, threads: Vec<CommentEntry>) {
        self.saved = threads.clone();
        self.saved_revision = self.saved_revision.wrapping_add(1);
        self.threads = threads;
        self.revision = self.revision.wrapping_add(1);
        self.selected = None;
        self.inspected = false;
        self.pins.clear();
        self.counts_key = None;
        self.draft = None;
        self.repin = None;
        self.reply.clear();
        self.editing = None;
        self.confirm_delete = None;
    }

    /// Drop every thread (a new model is loading), and with them the file they
    /// would be written to.
    pub fn clear(&mut self) {
        self.load(Vec::new());
        self.node_objects.clear();
        self.file_host = None;
        self.not_writable = Some(NotWritable::NoFile);
    }

    /// Replace the threads with an undo snapshot's, keeping what was saved.
    pub fn restore(&mut self, threads: Vec<CommentEntry>) {
        self.threads = threads;
        self.revision = self.revision.wrapping_add(1);
        if self
            .selected
            .is_some_and(|index| index >= self.threads.len())
        {
            self.selected = None;
        }
        self.editing = None;
        self.confirm_delete = None;
    }

    /// The threads now match what is in the file (or are to be treated as if
    /// they did: the user chose to discard their changes).
    pub fn mark_saved(&mut self) {
        self.mark_saved_as(self.threads.clone());
    }

    /// The file now holds `threads` — what a save wrote, which edits made while
    /// it was running don't count as.
    pub fn mark_saved_as(&mut self, threads: Vec<CommentEntry>) {
        self.saved = threads;
        self.saved_revision = self.saved_revision.wrapping_add(1);
    }

    /// Whether the threads differ from what the file holds. Re-compared only when
    /// either side has changed.
    pub fn is_dirty(&mut self) -> bool {
        let key = (self.revision, self.saved_revision);
        if self.dirty_key != Some(key) {
            self.dirty = self.threads != self.saved;
            self.dirty_key = Some(key);
        }
        self.dirty
    }

    /// Whether new comments can be written, and the threads edited.
    pub fn writable(&self) -> bool {
        self.not_writable.is_none()
    }

    /// Change the threads through `edit`, marking the change for the pins, the
    /// badges, undo and the unsaved-changes check.
    pub(crate) fn edit(&mut self, edit: impl FnOnce(&mut Vec<CommentEntry>)) {
        edit(&mut self.threads);
        self.revision = self.revision.wrapping_add(1);
    }

    /// Start a comment pointing at `anchor`, seeded with the current `frames`
    /// and `view`, its composer opening by `screen` when it has no point of its
    /// own. Replaces any draft in progress.
    pub fn begin_draft(
        &mut self,
        anchor: DraftAnchor,
        frames: Option<FrameRange>,
        view: SavedView,
        screen: egui::Pos2,
    ) {
        self.draft = Some(Draft {
            anchor,
            text: String::new(),
            follow_surface: true,
            include_frames: frames.is_some(),
            frames,
            view,
            include_view: true,
            screen,
            focused: false,
        });
    }

    /// Post the draft as a new thread, signed by [`CommentsState::author`], and
    /// select it. Returns its index, or `None` when there was nothing to post or
    /// nowhere to store it.
    pub(crate) fn post_draft(&mut self) -> Option<usize> {
        let draft = self.draft.take()?;
        let text = draft.text.trim().to_owned();
        if text.is_empty() {
            self.draft = Some(draft);
            return None;
        }
        // The object it is stored on: the clicked or picked node's own, or — for a
        // note about the view or the file, or a node the file can't name — the
        // root-most object.
        let node = match &draft.anchor {
            DraftAnchor::Surface { node, .. } | DraftAnchor::Object { node } => Some(*node),
            DraftAnchor::View | DraftAnchor::File => None,
        };
        let own = node.and_then(|node| {
            self.node_objects
                .get(node)
                .cloned()
                .flatten()
                .map(|object| (object, Some(node)))
        });
        let (object, host_node) = match own {
            Some(own) => own,
            None => self.file_host.clone()?,
        };
        let stored_on_own = own_host(&object, node, &self.node_objects);
        let scope = match draft.anchor_kind() {
            DraftKind::Object => Scope::Object,
            DraftKind::Whole => Scope::File,
        };
        let anchor = match draft.anchor {
            // A surface pin indexes its host's polygons, so it only stays one on
            // the node it was clicked on; elsewhere it becomes the scene point.
            DraftAnchor::Surface { anchor, world, .. } => {
                Some(if draft.follow_surface && stored_on_own {
                    anchor
                } else {
                    Anchor::World {
                        pos: world.to_array(),
                    }
                })
            }
            DraftAnchor::View | DraftAnchor::Object { .. } | DraftAnchor::File => None,
        };
        let thread = Thread {
            id: new_thread_id(),
            status: Status::Open,
            scope,
            anchor: anchor.map(AnchorValue::Known),
            frames: draft.include_frames.then_some(draft.frames).flatten(),
            view: draft.include_view.then_some(draft.view),
            messages: vec![Message {
                author: self.author.trim().to_owned(),
                time: utc_now(),
                text,
                extra: Default::default(),
            }],
            extra: Default::default(),
        };
        let entry = CommentEntry {
            thread,
            node: host_node,
            object: object.id,
            object_name: object.name,
            read_only: false,
        };
        self.edit(|threads| threads.push(entry));
        let index = self.threads.len() - 1;
        self.selected = Some(index);
        Some(index)
    }

    /// Add a reply to thread `index` from [`CommentsState::reply`].
    pub(crate) fn post_reply(&mut self, index: usize) {
        let text = self.reply.trim().to_owned();
        if text.is_empty() || self.threads.get(index).is_none_or(|entry| entry.read_only) {
            return;
        }
        let message = Message {
            author: self.author.trim().to_owned(),
            time: utc_now(),
            text,
            extra: Default::default(),
        };
        self.edit(|threads| threads[index].thread.messages.push(message));
        self.reply.clear();
    }

    /// Open or resolve thread `index`.
    pub(crate) fn set_status(&mut self, index: usize, status: Status) {
        if self
            .threads
            .get(index)
            .is_some_and(|entry| !entry.read_only && entry.thread.status != status)
        {
            self.edit(|threads| threads[index].thread.status = status);
        }
    }

    /// Delete thread `index`, fixing up the selection.
    pub(crate) fn delete(&mut self, index: usize) {
        if self.threads.get(index).is_none_or(|entry| entry.read_only) {
            return;
        }
        self.edit(|threads| {
            threads.remove(index);
        });
        self.selected = match self.selected {
            Some(selected) if selected == index => None,
            Some(selected) if selected > index => Some(selected - 1),
            other => other,
        };
        if self.selected.is_none() {
            self.inspected = false;
        }
        self.confirm_delete = None;
        self.editing = None;
    }

    /// Replace message `message` of thread `index` with `text`.
    pub(crate) fn edit_message(&mut self, index: usize, message: usize, text: String) {
        let text = text.trim().to_owned();
        let editable = self
            .threads
            .get(index)
            .is_some_and(|entry| !entry.read_only && message < entry.thread.messages.len());
        if editable && !text.is_empty() {
            self.edit(|threads| threads[index].thread.messages[message].text = text);
        }
    }

    /// Point thread `index` at `anchor` instead (a re-pin), keeping it on the node
    /// it is stored on: a surface anchor on a different node becomes the scene
    /// point it was clicked at.
    pub fn repin_thread(&mut self, index: usize, node: usize, anchor: Anchor, world: Vec3) {
        let Some(entry) = self.threads.get(index) else {
            return;
        };
        if entry.read_only {
            return;
        }
        let anchor = if entry.node == Some(node) {
            anchor
        } else {
            Anchor::World {
                pos: world.to_array(),
            }
        };
        self.edit(|threads| threads[index].thread.anchor = Some(AnchorValue::Known(anchor)));
        self.repin = None;
    }

    /// Replace thread `index`'s saved view with `view`.
    pub fn set_view(&mut self, index: usize, view: SavedView) {
        if self
            .threads
            .get(index)
            .is_some_and(|entry| !entry.read_only)
        {
            self.edit(|threads| threads[index].thread.view = Some(view));
        }
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

/// Whether `object` is the object `node` itself maps to (rather than the file
/// host standing in for it).
fn own_host(object: &ObjectRef, node: Option<usize>, node_objects: &[Option<ObjectRef>]) -> bool {
    node.and_then(|node| node_objects.get(node).cloned().flatten())
        .is_some_and(|own| own == *object)
}

/// Whether a draft is about something in particular or about the whole view.
enum DraftKind {
    Object,
    Whole,
}

impl Draft {
    fn anchor_kind(&self) -> DraftKind {
        match self.anchor {
            DraftAnchor::Surface { .. } | DraftAnchor::Object { .. } => DraftKind::Object,
            DraftAnchor::View | DraftAnchor::File => DraftKind::Whole,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

#[cfg(test)]
mod writing_tests {
    use super::*;
    use review_annotate::thread::Topology;

    fn view() -> SavedView {
        SavedView {
            target: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            distance: 1.0,
            fov: 1.0,
            ortho: false,
        }
    }

    fn writable() -> CommentsState {
        let mut comments = CommentsState {
            author: "Ana".to_owned(),
            node_objects: vec![
                None,
                Some(ObjectRef {
                    id: 11,
                    name: "Body".to_owned(),
                }),
            ],
            file_host: Some((
                ObjectRef {
                    id: 10,
                    name: "Root".to_owned(),
                },
                Some(0),
            )),
            not_writable: None,
            ..CommentsState::default()
        };
        comments.load(Vec::new());
        comments
    }

    fn surface() -> Anchor {
        Anchor::Surface {
            face: 3,
            tri: 0,
            bary: [0.2, 0.3, 0.5],
            local: [1.0, 2.0, 3.0],
            topo: Topology { polys: 8, verts: 6 },
        }
    }

    #[test]
    fn a_surface_draft_posts_on_its_node_and_is_selected() {
        let mut comments = writable();
        comments.begin_draft(
            DraftAnchor::Surface {
                node: 1,
                anchor: surface(),
                world: Vec3::ONE,
            },
            None,
            view(),
            egui::Pos2::ZERO,
        );
        if let Some(draft) = comments.draft.as_mut() {
            draft.text = " Seam here ".to_owned();
        }
        let index = comments.post_draft().expect("posts");
        let entry = &comments.threads[index];
        assert_eq!((entry.object, entry.node), (11, Some(1)));
        assert_eq!(entry.thread.anchor, Some(AnchorValue::Known(surface())));
        assert_eq!(entry.thread.messages[0].text, "Seam here");
        assert_eq!(entry.thread.messages[0].author, "Ana");
        assert_eq!(entry.thread.scope, Scope::Object);
        assert!(comments.is_dirty());
        assert_eq!(comments.selected, Some(index));
    }

    #[test]
    fn a_fixed_point_draft_stores_the_world_point() {
        let mut comments = writable();
        comments.begin_draft(
            DraftAnchor::Surface {
                node: 1,
                anchor: surface(),
                world: Vec3::new(4.0, 5.0, 6.0),
            },
            None,
            view(),
            egui::Pos2::ZERO,
        );
        let draft = comments.draft.as_mut().expect("draft");
        draft.text = "x".to_owned();
        draft.follow_surface = false;
        let index = comments.post_draft().expect("posts");
        assert_eq!(
            comments.threads[index].thread.anchor,
            Some(AnchorValue::Known(Anchor::World {
                pos: [4.0, 5.0, 6.0]
            }))
        );
    }

    /// A node the file can't name stores on the root-most object, and a surface
    /// pin there becomes the scene point (its polygon indices would mean nothing
    /// on another object).
    #[test]
    fn an_unnamed_node_stores_on_the_file_host() {
        let mut comments = writable();
        comments.begin_draft(
            DraftAnchor::Surface {
                node: 0,
                anchor: surface(),
                world: Vec3::ONE,
            },
            None,
            view(),
            egui::Pos2::ZERO,
        );
        if let Some(draft) = comments.draft.as_mut() {
            draft.text = "x".to_owned();
        }
        comments.node_objects[0] = None;
        comments.file_host = Some((
            ObjectRef {
                id: 10,
                name: "Root".to_owned(),
            },
            None,
        ));
        let index = comments.post_draft().expect("posts");
        assert_eq!(comments.threads[index].object, 10);
        assert!(matches!(
            comments.threads[index].thread.anchor,
            Some(AnchorValue::Known(Anchor::World { .. }))
        ));
    }

    #[test]
    fn an_empty_draft_is_not_posted() {
        let mut comments = writable();
        comments.begin_draft(DraftAnchor::File, None, view(), egui::Pos2::ZERO);
        assert_eq!(comments.post_draft(), None);
        assert!(comments.draft.is_some(), "the draft stays open");
    }

    #[test]
    fn replies_status_and_deletion_mark_the_file_dirty_and_saving_clears_it() {
        let mut comments = writable();
        comments.begin_draft(DraftAnchor::File, None, view(), egui::Pos2::ZERO);
        if let Some(draft) = comments.draft.as_mut() {
            draft.text = "Note".to_owned();
        }
        let index = comments.post_draft().expect("posts");
        assert_eq!(comments.threads[index].thread.scope, Scope::File);
        comments.mark_saved();
        assert!(!comments.is_dirty());

        comments.reply = "Fixed".to_owned();
        comments.post_reply(index);
        assert_eq!(comments.threads[index].thread.messages.len(), 2);
        comments.set_status(index, Status::Resolved);
        assert!(comments.is_dirty());

        comments.delete(index);
        assert!(comments.threads.is_empty());
        assert_eq!(comments.selected, None);
    }
}
