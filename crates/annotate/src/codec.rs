//! The JSON payload one object's property holds: `{"v": 1, "threads": [...]}`.
//!
//! Decoding is lenient where leniency loses nothing — a thread that fails to
//! decode is kept as raw JSON and written back untouched, and a payload from a
//! newer format version is read as far as it can be and marked so the viewer
//! won't rewrite it — and strict nowhere a guess could corrupt a file.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::thread::Thread;

/// The name of the FBX user property comments are stored in.
pub const PROPERTY: &str = "ReviewComments";

/// The payload version this crate writes.
pub const FORMAT_VERSION: u32 = 1;

/// One object's decoded payload.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Payload {
    /// The version the payload declared.
    pub version: u32,
    pub threads: Vec<Thread>,
    /// Threads that did not decode, kept as written.
    pub opaque: Vec<Value>,
}

impl Payload {
    /// Whether this payload came from a newer format than this crate writes, in
    /// which case editing it could drop what it doesn't understand.
    pub fn is_newer(&self) -> bool {
        self.version > FORMAT_VERSION
    }
}

/// Why a payload, or part of one, could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CodecError {
    #[error("not valid JSON: {0}")]
    Json(String),
    #[error("{0} thread(s) could not be read and are kept as written")]
    OpaqueThreads(usize),
}

#[derive(Deserialize)]
struct EnvelopeIn {
    #[serde(default)]
    v: u32,
    #[serde(default)]
    threads: Vec<Value>,
}

#[derive(Serialize)]
struct EnvelopeOut<'a> {
    v: u32,
    threads: Vec<ThreadOut<'a>>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum ThreadOut<'a> {
    Thread(&'a Thread),
    Opaque(&'a Value),
}

/// Decode one property's text. A payload that isn't JSON at all is an error;
/// threads that don't decode are kept in [`Payload::opaque`] and reported
/// alongside the payload.
pub fn decode(text: &str) -> Result<(Payload, Option<CodecError>), CodecError> {
    let envelope: EnvelopeIn =
        serde_json::from_str(text).map_err(|error| CodecError::Json(error.to_string()))?;
    let mut payload = Payload {
        version: envelope.v,
        ..Payload::default()
    };
    for value in envelope.threads {
        match serde_json::from_value::<Thread>(value.clone()) {
            Ok(thread) => payload.threads.push(thread),
            Err(_) => payload.opaque.push(value),
        }
    }
    let warning =
        (!payload.opaque.is_empty()).then_some(CodecError::OpaqueThreads(payload.opaque.len()));
    Ok((payload, warning))
}

/// Encode threads (plus any kept opaque ones) as one property's text, at the
/// current format version. Compact: the text is also what an artist sees in a
/// DCC's attribute field.
pub fn encode(threads: &[Thread], opaque: &[Value]) -> String {
    let envelope = EnvelopeOut {
        v: FORMAT_VERSION,
        threads: threads
            .iter()
            .map(ThreadOut::Thread)
            .chain(opaque.iter().map(ThreadOut::Opaque))
            .collect(),
    };
    // Serializing plain data into a string cannot fail.
    serde_json::to_string(&envelope).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thread::{
        Anchor, AnchorValue, FrameRange, Message, SavedView, Scope, Status, Topology,
    };

    fn sample() -> Thread {
        Thread {
            id: "a1b2c3d4e5f60718".to_owned(),
            status: Status::Open,
            scope: Scope::Object,
            anchor: Some(AnchorValue::Known(Anchor::Surface {
                face: 12,
                tri: 1,
                bary: [0.2, 0.3, 0.5],
                local: [0.0, 1.0, 2.0],
                topo: Topology {
                    polys: 100,
                    verts: 98,
                },
            })),
            frames: Some(FrameRange {
                clip: "Run".to_owned(),
                start: 10,
                end: 24,
            }),
            view: Some(SavedView {
                target: [0.0, 1.0, 0.0],
                yaw: 0.5,
                pitch: -0.2,
                distance: 4.0,
                fov: 0.8,
                ortho: false,
            }),
            messages: vec![Message {
                author: "Ana".to_owned(),
                time: "2026-10-08T12:00:00Z".to_owned(),
                text: "Seam \"visible\" here — & here ✓".to_owned(),
                extra: Default::default(),
            }],
            extra: Default::default(),
        }
    }

    #[test]
    fn a_thread_round_trips() {
        let text = encode(&[sample()], &[]);
        let (payload, warning) = decode(&text).expect("decodes");
        assert_eq!(warning, None);
        assert_eq!(payload.version, FORMAT_VERSION);
        assert_eq!(payload.threads, [sample()]);
    }

    /// Fields a newer writer added survive, on the thread, the message and as
    /// an anchor kind this version doesn't know.
    #[test]
    fn unknown_fields_and_anchor_kinds_are_kept() {
        let text = r#"{"v":1,"threads":[{"id":"x","priority":"high","anchor":{"kind":"volume","box":[1,2]},
            "messages":[{"author":"A","time":"t","text":"hi","reactions":["+1"]}]}]}"#;
        let (payload, _) = decode(text).expect("decodes");
        let thread = &payload.threads[0];
        assert_eq!(thread.extra["priority"], "high");
        assert_eq!(thread.messages[0].extra["reactions"][0], "+1");
        assert!(matches!(thread.anchor, Some(AnchorValue::Unknown(_))));
        let again = encode(&payload.threads, &payload.opaque);
        let (reread, _) = decode(&again).expect("decodes");
        assert_eq!(reread, payload);
    }

    /// A thread that doesn't decode is kept verbatim and written back.
    #[test]
    fn a_broken_thread_is_kept_as_written() {
        let text = r#"{"v":1,"threads":[{"id":"ok","messages":[]},{"messages":"not a list"}]}"#;
        let (payload, warning) = decode(text).expect("decodes");
        assert_eq!(payload.threads.len(), 1);
        assert_eq!(warning, Some(CodecError::OpaqueThreads(1)));
        let again = encode(&payload.threads, &payload.opaque);
        assert!(again.contains(r#"{"messages":"not a list"}"#));
    }

    #[test]
    fn a_newer_payload_is_marked() {
        let (payload, _) = decode(r#"{"v":2,"threads":[]}"#).expect("decodes");
        assert!(payload.is_newer());
    }

    #[test]
    fn text_that_is_not_json_is_an_error() {
        assert!(matches!(decode("hello"), Err(CodecError::Json(_))));
    }
}
