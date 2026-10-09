//! Every comment in one file, grouped by the object each is stored on.

use std::path::Path;

use crate::codec::{self, CodecError, PROPERTY, Payload};
use crate::fbx::{self, FbxResult, Scan};
use crate::thread::Thread;

/// The comments stored on one object.
#[derive(Debug, Clone, PartialEq)]
pub struct HostComments {
    /// Index into [`Scan::models`].
    pub model: usize,
    pub payload: Payload,
    /// Whether the property carries the FBX hidden flag.
    pub hidden: bool,
}

/// A payload, or part of one, that could not be read, and the object it was on.
#[derive(Debug, Clone, PartialEq)]
pub struct Warning {
    /// Index into [`Scan::models`].
    pub model: usize,
    pub error: CodecError,
}

/// Every comment in one file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FileComments {
    pub hosts: Vec<HostComments>,
    pub warnings: Vec<Warning>,
}

impl FileComments {
    /// Decode the payloads a [`fbx::scan`] for [`PROPERTY`] found.
    pub fn from_scan(scan: &Scan) -> Self {
        let mut comments = Self::default();
        for stored in &scan.strings {
            match codec::decode(&stored.value) {
                Ok((payload, warning)) => {
                    if let Some(error) = warning {
                        comments.warnings.push(Warning {
                            model: stored.model,
                            error,
                        });
                    }
                    comments.hosts.push(HostComments {
                        model: stored.model,
                        payload,
                        hidden: stored.hidden,
                    });
                }
                Err(error) => comments.warnings.push(Warning {
                    model: stored.model,
                    error,
                }),
            }
        }
        comments
    }

    /// Every decoded thread with the object it is stored on, in file order —
    /// the order threads are numbered in.
    pub fn threads(&self) -> impl Iterator<Item = (usize, &Thread)> {
        self.hosts.iter().flat_map(|host| {
            host.payload
                .threads
                .iter()
                .map(move |thread| (host.model, thread))
        })
    }

    pub fn thread_count(&self) -> usize {
        self.hosts
            .iter()
            .map(|host| host.payload.threads.len())
            .sum()
    }
}

/// Scan `path` and decode its comments. A binary file is streamed, so this costs
/// a small fraction of reading the file.
pub fn read_file(path: &Path) -> FbxResult<(Scan, FileComments)> {
    let scan = fbx::scan_file(path, PROPERTY)?;
    let comments = FileComments::from_scan(&scan);
    Ok((scan, comments))
}

/// The object file-level notes are stored on: the first object, in file order,
/// that hangs from the scene root.
pub fn file_note_host(scan: &Scan) -> Option<usize> {
    scan.models.iter().position(|model| model.parent.is_none())
}

/// The names from the scene root down to object `model`, inclusive.
pub fn object_path(scan: &Scan, model: usize) -> Vec<&str> {
    let mut path = Vec::new();
    let mut current = scan.models.get(model);
    // Bounded by the object count, so a malformed parent cycle still ends.
    for _ in 0..scan.models.len() {
        let Some(object) = current else { break };
        path.push(object.name.as_str());
        current = object
            .parent
            .and_then(|parent| scan.models.iter().find(|model| model.id == parent));
    }
    path.reverse();
    path
}
