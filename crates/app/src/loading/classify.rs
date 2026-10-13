//! Which of the files opened together becomes the model, and which are attached
//! to it as animation — the rules `docs/animation-sources-plan.md` settles, as a
//! pure function of what each file holds and what binds to what, so they can be
//! tested without a window, a GPU or a file on disk.
//!
//! A file's role is decided by whether it *binds*, not by whether it carries a
//! mesh: a facial-animation file carries copies of the model's head meshes just
//! to host its blend-shape curves. "Attaches" is the matcher's word for it — the
//! file's clips drive something of the rig, and any mesh it carries is a copy of
//! one of the rig's own — and the classifier never looks inside a file itself.

use super::OpenIntent;

/// What classification needs to know about one opened file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FileShape {
    pub(super) has_geometry: bool,
    pub(super) triangles: usize,
    pub(super) has_clips: bool,
}

/// The rig a file would be attached to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Rig {
    /// The model on screen.
    Current,
    /// File `n` of the batch being classified.
    File(usize),
}

/// The binding questions classification asks, answered by whoever holds the
/// files (`super::session`'s matcher, over the parsed models).
pub(super) trait Matcher {
    /// Whether file `file` attaches to `rig`.
    fn attaches(&self, file: usize, rig: Rig) -> bool;
    /// Whether the model on screen — a skeleton standing in for a rig — would
    /// attach to file `rig`, which could then take its place.
    fn current_attaches_to(&self, rig: usize) -> bool;
}

/// What is on screen when the files arrive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Session {
    /// The empty start state.
    Empty,
    /// An animation file's skeleton, with no mesh, standing in as the rig.
    StandIn,
    /// A model with a mesh.
    Rig,
}

/// Why a file was not loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LeftOut {
    /// A second model: only one is shown at a time.
    SecondModel,
    /// Animation that drives nothing of the rig.
    NoMatch,
}

/// What to do with a batch of opened files, by their index in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Plan {
    /// File `rig` replaces whatever is on screen; `attach` are attached to it.
    NewSession {
        rig: usize,
        attach: Vec<usize>,
        left_out: Vec<(usize, LeftOut)>,
    },
    /// `attach` join the model on screen.
    Attach {
        attach: Vec<usize>,
        left_out: Vec<(usize, LeftOut)>,
    },
    /// File `rig` is a real rig for the skeleton standing in on screen: open it
    /// afresh together with every file of the session it takes over, so each of
    /// them binds to it from its own file rather than through the stand-in.
    TakeOver { rig: usize },
}

/// Decide what `files`, opened with `intent` into `session`, become.
pub(super) fn classify(
    intent: OpenIntent,
    session: Session,
    files: &[FileShape],
    matcher: &impl Matcher,
) -> Plan {
    if files.is_empty() {
        return Plan::Attach {
            attach: Vec::new(),
            left_out: Vec::new(),
        };
    }
    match (intent, session) {
        (OpenIntent::NewSession, _) | (_, Session::Empty) => new_session(files, matcher),
        (OpenIntent::Attach, _) => attach_to_current(files, matcher),
        (OpenIntent::Drop, session) => {
            if session == Session::StandIn
                && let Some(rig) = choose_rig(files).filter(|&rig| files[rig].has_geometry)
                && matcher.current_attaches_to(rig)
            {
                return Plan::TakeOver { rig };
            }
            // A model among the files that is no animation of the one on screen
            // means the drop is a new set of files, not an addition to this one.
            let foreign_model = (0..files.len())
                .any(|file| files[file].has_geometry && !matcher.attaches(file, Rig::Current));
            if foreign_model {
                new_session(files, matcher)
            } else {
                attach_to_current(files, matcher)
            }
        }
    }
}

/// The batch's rig: a file with a mesh and no clips if there is one — the rig's
/// own shape — else any file with a mesh, the most triangles first; and with no
/// mesh anywhere, the first file, whose skeleton stands in (the importer refuses
/// a file with neither a mesh nor a skeleton, so every file here has one).
fn choose_rig(files: &[FileShape]) -> Option<usize> {
    (0..files.len())
        .filter(|&file| files[file].has_geometry)
        .min_by_key(|&file| {
            (
                files[file].has_clips,
                std::cmp::Reverse(files[file].triangles),
                file,
            )
        })
        .or((!files.is_empty()).then_some(0))
}

fn new_session(files: &[FileShape], matcher: &impl Matcher) -> Plan {
    let Some(rig) = choose_rig(files) else {
        return Plan::Attach {
            attach: Vec::new(),
            left_out: Vec::new(),
        };
    };
    let mut attach = Vec::new();
    let mut left_out = Vec::new();
    for file in (0..files.len()).filter(|&file| file != rig) {
        if matcher.attaches(file, Rig::File(rig)) {
            attach.push(file);
        } else {
            left_out.push((file, why_left_out(files[file])));
        }
    }
    Plan::NewSession {
        rig,
        attach,
        left_out,
    }
}

fn attach_to_current(files: &[FileShape], matcher: &impl Matcher) -> Plan {
    let mut attach = Vec::new();
    let mut left_out = Vec::new();
    for (file, &shape) in files.iter().enumerate() {
        if matcher.attaches(file, Rig::Current) {
            attach.push(file);
        } else {
            left_out.push((file, why_left_out(shape)));
        }
    }
    Plan::Attach { attach, left_out }
}

fn why_left_out(file: FileShape) -> LeftOut {
    if file.has_geometry {
        LeftOut::SecondModel
    } else {
        LeftOut::NoMatch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RIG: FileShape = FileShape {
        has_geometry: true,
        triangles: 200_000,
        has_clips: false,
    };
    const BODY: FileShape = FileShape {
        has_geometry: false,
        triangles: 0,
        has_clips: true,
    };
    /// A facial-animation file: copies of the rig's head meshes and their curves.
    const FACE: FileShape = FileShape {
        has_geometry: true,
        triangles: 16_000,
        has_clips: true,
    };
    const PROP: FileShape = FileShape {
        has_geometry: true,
        triangles: 500,
        has_clips: false,
    };

    /// Binding answers from a table: `binds` lists the (file, rig) pairs that
    /// attach, `stand_in_to` the files the model on screen would attach to.
    struct Table {
        binds: Vec<(usize, Rig)>,
        stand_in_to: Vec<usize>,
    }

    impl Matcher for Table {
        fn attaches(&self, file: usize, rig: Rig) -> bool {
            self.binds.contains(&(file, rig))
        }
        fn current_attaches_to(&self, rig: usize) -> bool {
            self.stand_in_to.contains(&rig)
        }
    }

    fn table(binds: &[(usize, Rig)]) -> Table {
        Table {
            binds: binds.to_vec(),
            stand_in_to: Vec::new(),
        }
    }

    #[test]
    fn a_rig_and_its_animations_opened_together_pick_the_rig_whatever_the_order() {
        // The facial file has a mesh too; the rig is the one with no clips.
        let files = [FACE, BODY, RIG];
        let matcher = table(&[(0, Rig::File(2)), (1, Rig::File(2))]);
        assert_eq!(
            classify(OpenIntent::NewSession, Session::Empty, &files, &matcher),
            Plan::NewSession {
                rig: 2,
                attach: vec![0, 1],
                left_out: Vec::new(),
            }
        );
    }

    #[test]
    fn with_no_mesh_anywhere_the_first_skeleton_stands_in() {
        let files = [BODY, BODY];
        let matcher = table(&[(1, Rig::File(0))]);
        assert_eq!(
            classify(OpenIntent::Drop, Session::Empty, &files, &matcher),
            Plan::NewSession {
                rig: 0,
                attach: vec![1],
                left_out: Vec::new(),
            }
        );
    }

    #[test]
    fn a_second_model_in_a_batch_is_left_out_and_says_why() {
        let files = [RIG, PROP, BODY];
        let matcher = table(&[]);
        assert_eq!(
            classify(OpenIntent::NewSession, Session::Rig, &files, &matcher),
            Plan::NewSession {
                rig: 0,
                attach: Vec::new(),
                left_out: vec![(1, LeftOut::SecondModel), (2, LeftOut::NoMatch)],
            }
        );
    }

    #[test]
    fn dropping_animations_that_bind_adds_them() {
        let files = [BODY, FACE];
        let matcher = table(&[(0, Rig::Current), (1, Rig::Current)]);
        assert_eq!(
            classify(OpenIntent::Drop, Session::Rig, &files, &matcher),
            Plan::Attach {
                attach: vec![0, 1],
                left_out: Vec::new(),
            }
        );
    }

    #[test]
    fn dropping_a_model_that_does_not_bind_replaces_the_session() {
        let files = [PROP];
        assert_eq!(
            classify(OpenIntent::Drop, Session::Rig, &files, &table(&[])),
            Plan::NewSession {
                rig: 0,
                attach: Vec::new(),
                left_out: Vec::new(),
            }
        );
    }

    #[test]
    fn dropping_an_animation_that_does_not_bind_leaves_the_session_alone() {
        let files = [BODY];
        assert_eq!(
            classify(OpenIntent::Drop, Session::Rig, &files, &table(&[])),
            Plan::Attach {
                attach: Vec::new(),
                left_out: vec![(0, LeftOut::NoMatch)],
            }
        );
    }

    #[test]
    fn add_animations_never_replaces_even_for_a_model() {
        let files = [PROP, BODY];
        let matcher = table(&[(1, Rig::Current)]);
        assert_eq!(
            classify(OpenIntent::Attach, Session::Rig, &files, &matcher),
            Plan::Attach {
                attach: vec![1],
                left_out: vec![(0, LeftOut::SecondModel)],
            }
        );
    }

    #[test]
    fn a_rig_dropped_on_its_stand_in_takes_over() {
        let files = [RIG];
        let matcher = Table {
            binds: Vec::new(),
            stand_in_to: vec![0],
        };
        assert_eq!(
            classify(OpenIntent::Drop, Session::StandIn, &files, &matcher),
            Plan::TakeOver { rig: 0 }
        );
    }

    #[test]
    fn an_unrelated_model_dropped_on_a_stand_in_replaces_it() {
        let files = [PROP];
        assert_eq!(
            classify(OpenIntent::Drop, Session::StandIn, &files, &table(&[])),
            Plan::NewSession {
                rig: 0,
                attach: Vec::new(),
                left_out: Vec::new(),
            }
        );
    }

    #[test]
    fn opening_into_an_empty_viewer_is_always_a_new_session() {
        let files = [BODY];
        assert_eq!(
            classify(OpenIntent::Attach, Session::Empty, &files, &table(&[])),
            Plan::NewSession {
                rig: 0,
                attach: Vec::new(),
                left_out: Vec::new(),
            }
        );
    }
}
