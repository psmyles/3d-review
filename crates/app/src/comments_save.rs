//! Saving review comments back into the FBX, and not losing them by accident.
//!
//! A save patches the opened file's bytes with `review-annotate` — every object's
//! `ReviewComments` property set to its threads, or removed where none are left —
//! and writes the result through a sibling temp and a rename, so a failed write
//! leaves the file as it was. It runs on a worker: the file is the user's whole
//! asset, read and written in full.
//!
//! Three things guard the comments and the file:
//!
//! * **Changed on disk.** The file's size and modification time are recorded when
//!   its comments are read; a save that finds them different asks before writing
//!   over what an artist exported in the meantime.
//! * **Unsaved changes.** Opening another file, starting over or quitting with
//!   unsaved comments asks to save, discard or stay; the action that asked is
//!   carried through the question ([`AfterSave`]) and resumed when it is answered.
//! * **What isn't understood stays.** Threads that did not decode, and payloads
//!   from a newer format version, are written back exactly as they were read.

use std::collections::BTreeMap;
use std::path::PathBuf;

use review_annotate::codec::{PROPERTY, encode};
use review_annotate::fbx::{self, Edit};
use review_ui::CommentEntry;

use crate::comments::{CommentSource, Fingerprint};
use crate::dialog::Dialog;
use crate::events::UserEvent;
use crate::loading::file_label;
use crate::{App, keys, prof};

/// What to do once a save (or the decision not to) has happened — the action
/// that was about to drop the comments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AfterSave {
    Nothing,
    /// Quit the viewer.
    Exit,
    /// Open this model instead.
    Open(PathBuf),
    /// Go back to the empty start state.
    New,
}

/// A finished save, posted from the save worker.
#[derive(Debug)]
pub(crate) struct CommentsSaved {
    result: Result<Saved, String>,
    target: PathBuf,
    then: AfterSave,
}

#[derive(Debug)]
struct Saved {
    fingerprint: Fingerprint,
    /// The threads as written, which become what "unsaved changes" compares to.
    threads: Vec<CommentEntry>,
    /// Whether this was a Save As, after which saves go to the new file.
    save_as: bool,
}

/// The property edits that write `threads` into the file `source` describes:
/// every object that has threads, or had them, gets its property set — or
/// removed when none are left. Objects whose payload this version can't rewrite
/// are left alone, and threads that did not decode are written back with their
/// object's.
pub(crate) fn comment_edits(threads: &[CommentEntry], source: &CommentSource) -> Vec<Edit> {
    let mut by_object: BTreeMap<i64, Vec<review_annotate::thread::Thread>> = BTreeMap::new();
    for id in &source.had_comments {
        by_object.entry(*id).or_default();
    }
    for entry in threads {
        by_object
            .entry(entry.object)
            .or_default()
            .push(entry.thread.clone());
    }
    by_object
        .into_iter()
        .filter(|(id, _)| !source.frozen.contains(id))
        .map(|(id, threads)| {
            let opaque = source.opaque.get(&id).map_or(&[][..], Vec::as_slice);
            Edit {
                model: id,
                value: (!threads.is_empty() || !opaque.is_empty())
                    .then(|| encode(&threads, opaque)),
                hidden: false,
            }
        })
        .collect()
}

impl App {
    /// Save the comments back into the file they came from (the save shortcut),
    /// then do `then`. Asks first if the file changed on disk since it was read.
    pub(crate) fn save_comments(&mut self, then: AfterSave) {
        let Some(source) = self.comment_source.as_ref() else {
            self.resume(then);
            return;
        };
        let changed = Fingerprint::of(&source.path).ok() != Some(source.fingerprint);
        if changed {
            let file = file_label(&source.path);
            self.ask(Dialog::ConfirmOverwrite { file, then });
            return;
        }
        let target = source.path.clone();
        self.write_comments(target, then, false);
    }

    /// Save into the file the comments came from without checking it first — the
    /// user just said to.
    pub(crate) fn save_comments_unchecked(&mut self, then: AfterSave) {
        if let Some(target) = self
            .comment_source
            .as_ref()
            .map(|source| source.path.clone())
        {
            self.write_comments(target, then, false);
        }
    }

    /// Ask where to write the model with its comments (the save-as shortcut).
    pub(crate) fn save_comments_as(&mut self) {
        let Some(source) = self.comment_source.as_ref() else {
            return;
        };
        let name = source
            .path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        self.ask(Dialog::SaveCommentsAs { name });
    }

    /// Patch the source file's bytes with the comments and write them to
    /// `target`, on a worker.
    pub(crate) fn write_comments(&mut self, target: PathBuf, then: AfterSave, save_as: bool) {
        let Some(source) = self.comment_source.clone() else {
            return;
        };
        let Some(proxy) = self.textures.proxy.clone() else {
            return;
        };
        let threads = self.ui.comments.threads.clone();
        let edits = comment_edits(&threads, &source);
        let worker_target = target.clone();
        let spawned = std::thread::Builder::new()
            .name("comments-save".into())
            .spawn(move || {
                prof::thread_name("comments-save");
                let _z = prof::zone!("Save Review Comments");
                let result = (|| {
                    let bytes = std::fs::read(&source.path).map_err(|error| error.to_string())?;
                    let patched =
                        fbx::patch(&bytes, PROPERTY, &edits).map_err(|error| error.to_string())?;
                    review_optimize::write_bytes_replacing(&worker_target, &patched)
                        .map_err(|error| error.to_string())?;
                    let fingerprint =
                        Fingerprint::of(&worker_target).map_err(|error| error.to_string())?;
                    Ok(Saved {
                        fingerprint,
                        threads,
                        save_as,
                    })
                })();
                let _ = proxy.send_event(UserEvent::CommentsSaved(Box::new(CommentsSaved {
                    result,
                    target: worker_target,
                    then,
                })));
            });
        if let Err(error) = spawned {
            log::error!("could not start the comment save thread: {error}");
            self.notifications
                .error(keys::app_notifications::comments_save_failed(
                    file_label(&target),
                    error.to_string(),
                ));
        }
    }

    /// A save finished: record what is now in the file, say so, and carry on
    /// with whatever was waiting on it.
    pub(crate) fn handle_comments_saved(&mut self, message: CommentsSaved) {
        let label = file_label(&message.target);
        match message.result {
            Ok(saved) => {
                if let Some(source) = self.comment_source.as_mut() {
                    if saved.save_as {
                        source.path = message.target.clone();
                    }
                    if source.path == message.target {
                        source.fingerprint = saved.fingerprint;
                    }
                    // What the file holds now is what a later save starts from.
                    source.had_comments = saved.threads.iter().map(|entry| entry.object).collect();
                    source.had_comments.extend(source.opaque.keys());
                    source.had_comments.extend(source.frozen.iter());
                }
                self.ui.comments.mark_saved_as(saved.threads);
                if saved.save_as {
                    self.remember_recent_file(&message.target);
                }
                self.title_shows_unsaved = None;
                log::info!("saved review comments to {}", message.target.display());
                self.notifications
                    .success(keys::app_notifications::comments_saved(label));
                self.resume(message.then);
            }
            Err(error) => {
                log::error!(
                    "saving review comments to {} failed: {error}",
                    message.target.display()
                );
                self.notifications
                    .error(keys::app_notifications::comments_save_failed(label, error));
            }
        }
        self.request_redraw();
    }

    /// Before doing something that drops the comments: `true` when it may go
    /// ahead now; otherwise the user is asked, and `then` happens once they have
    /// answered (or not at all, if they stay).
    pub(crate) fn may_drop_comments(&mut self, then: AfterSave) -> bool {
        if !self.ui.comments.is_dirty() || self.comment_source.is_none() {
            return true;
        }
        let file = self
            .comment_source
            .as_ref()
            .map(|source| file_label(&source.path))
            .unwrap_or_default();
        self.ask(Dialog::UnsavedComments { file, then });
        false
    }

    /// Carry out the action a save or an answered question was holding.
    pub(crate) fn resume(&mut self, then: AfterSave) {
        match then {
            AfterSave::Nothing => {}
            AfterSave::Exit => self.exit_requested = true,
            AfterSave::Open(path) => self.open_model_from_path(&path),
            AfterSave::New => self.reset_to_start_state(),
        }
    }

    /// After each pass of the chrome: the title's unsaved mark, the remembered
    /// author, and a pin move abandoned by leaving the Comment tool.
    pub(crate) fn sync_comment_chrome(&mut self) {
        self.sync_title();
        if self.ui.comments.author != self.persisted_author {
            self.persisted_author = self.ui.comments.author.clone();
            self.persist_settings();
        }
        if self.ui.tool != review_ui::ViewportTool::Comment {
            self.ui.comments.repin = None;
        }
    }

    /// Keep the window title's unsaved-changes mark in step with the comments.
    fn sync_title(&mut self) {
        let dirty = self.ui.comments.is_dirty();
        if self.title_shows_unsaved == Some(dirty) {
            return;
        }
        self.title_shows_unsaved = Some(dirty);
        let name = self
            .comment_source
            .as_ref()
            .map(|source| file_label(&source.path))
            .or_else(|| self.title_file.clone());
        self.set_window_title_marked(name.as_deref(), dirty);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use review_annotate::codec::decode;
    use review_annotate::fbx::Format;
    use review_annotate::thread::{Message, Scope, Status, Thread};

    use super::*;

    fn entry(object: i64, text: &str) -> CommentEntry {
        CommentEntry {
            thread: Thread {
                id: text.to_owned(),
                status: Status::Open,
                scope: Scope::Object,
                anchor: None,
                frames: None,
                view: None,
                messages: vec![Message {
                    author: "a".to_owned(),
                    time: String::new(),
                    text: text.to_owned(),
                    extra: Default::default(),
                }],
                extra: Default::default(),
            },
            node: None,
            object,
            object_name: String::new(),
            read_only: false,
        }
    }

    fn source() -> CommentSource {
        CommentSource {
            path: PathBuf::new(),
            fingerprint: Fingerprint::default(),
            format: Format::Ascii,
            opaque: HashMap::from([(3, vec![serde_json::json!({"messages": "not a list"})])]),
            frozen: HashSet::from([4]),
            had_comments: HashSet::from([2, 3, 4]),
        }
    }

    /// Each object gets its own threads; an emptied one has its property removed;
    /// opaque threads go back with their object; a frozen object isn't touched.
    #[test]
    fn edits_cover_every_object_with_or_once_with_comments() {
        let edits = comment_edits(&[entry(1, "a"), entry(1, "b"), entry(3, "c")], &source());
        let by_id: HashMap<i64, &Edit> = edits.iter().map(|edit| (edit.model, edit)).collect();
        assert_eq!(by_id.len(), 3, "objects 1, 2 and 3 — never the frozen 4");

        let (one, _) = decode(by_id[&1].value.as_deref().expect("set")).expect("decodes");
        assert_eq!(one.threads.len(), 2);
        assert_eq!(by_id[&2].value, None, "emptied, so removed");
        let (three, _) = decode(by_id[&3].value.as_deref().expect("set")).expect("decodes");
        assert_eq!((three.threads.len(), three.opaque.len()), (1, 1));
        assert!(edits.iter().all(|edit| !edit.hidden), "stored visible");
    }

    /// A save round-trips: threads written into a real file come back from it
    /// exactly, on the objects they were written to, and removing them all
    /// restores the original bytes.
    #[test]
    fn saved_comments_read_back_and_removing_them_restores_the_file() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/test_models/SM_Ammo_Crate_01a.fbx");
        if !path.exists() {
            return;
        }
        let (model, extras) = review_import::load_model_full(&path).expect("imports");
        let loaded = crate::comments::read_comments(&path, &model, extras.as_ref()).expect("reads");
        assert!(loaded.entries.is_empty());
        let host = loaded
            .node_objects
            .iter()
            .flatten()
            .next()
            .expect("an object")
            .clone();
        let mut threads = vec![entry(host.id, "Seam \"here\" & there")];
        threads[0].object_name = host.name.clone();

        let original = std::fs::read(&path).expect("reads");
        let edits = comment_edits(&threads, &loaded.source);
        let patched = fbx::patch(&original, PROPERTY, &edits).expect("patches");
        let temp = std::env::temp_dir().join(format!("review-save-{}.fbx", std::process::id()));
        std::fs::write(&temp, &patched).expect("writes");
        let reread =
            crate::comments::read_comments(&temp, &model, extras.as_ref()).expect("rereads");
        assert_eq!(reread.entries.len(), 1);
        assert_eq!(reread.entries[0].thread, threads[0].thread);
        assert_eq!(reread.entries[0].object, host.id);

        // Everything deleted: the next save removes the property again.
        let edits = comment_edits(&[], &reread.source);
        let restored = fbx::patch(&patched, PROPERTY, &edits).expect("patches");
        let _ = std::fs::remove_file(&temp);
        assert!(
            restored == original,
            "removing every comment restores the original bytes"
        );
    }
}
