//! A review comment thread, as stored: the types the JSON payload decodes into.
//!
//! The wire format is documented in `docs/review-comments-format.md`, which is
//! the contract other tools read against. Every struct keeps the fields it does
//! not know (`extra`), so a file written by a newer viewer loses nothing when an
//! older one edits a different thread in it.

use std::hash::{BuildHasher, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// One conversation about one thing in the file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Thread {
    /// Unique within the file; random 64-bit hex. Optional on the wire, so a tool
    /// writing a comment by hand needn't invent one: the viewer assigns one to a
    /// thread that arrives without it.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub status: Status,
    /// `object` for a comment about the object it is stored on, `file` for a
    /// note about the whole file (stored on the root-most object).
    #[serde(default)]
    pub scope: Scope,
    /// Where in the scene the comment points, if anywhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<AnchorValue>,
    /// The animation frame or range it is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<FrameRange>,
    /// The camera the reviewer was looking through.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<SavedView>,
    /// The first message opens the thread; the rest are replies, oldest first.
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    #[default]
    Open,
    Resolved,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    #[default]
    Object,
    File,
}

/// An anchor this version understands, or one it doesn't (kept verbatim, so a
/// newer anchor kind survives an older viewer's save).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnchorValue {
    Known(Anchor),
    Unknown(Value),
}

impl AnchorValue {
    pub fn known(&self) -> Option<&Anchor> {
        match self {
            AnchorValue::Known(anchor) => Some(anchor),
            AnchorValue::Unknown(_) => None,
        }
    }
}

/// Where a comment points.
///
/// Positions are in the scene's own axes, in meters — the space the viewer
/// shows the model in, whatever unit the file declares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Anchor {
    /// A point on the surface of the host object's mesh, which follows it
    /// through animation and skinning.
    Surface {
        /// Polygon index within the host object's mesh, in the file's order.
        face: u32,
        /// Which triangle of that polygon's fan triangulation.
        tri: u32,
        /// Barycentric weights on that triangle's three corners.
        bary: [f32; 3],
        /// The same point in the host object's local space, used when the mesh
        /// has been edited since and `face` no longer means the same polygon.
        local: [f32; 3],
        /// The host mesh's polygon and vertex counts when the pin was placed —
        /// how a reader tells whether `face` still means the same polygon.
        topo: Topology,
    },
    /// A fixed point in the scene.
    World { pos: [f32; 3] },
    /// A point on a UV layout.
    Uv {
        /// The UV set's name.
        set: String,
        uv: [f32; 2],
    },
}

/// A mesh's size, as the source file authored it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Topology {
    pub polys: u64,
    pub verts: u64,
}

/// An animation frame (`start == end`) or an inclusive range of frames of the
/// clip named `clip`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameRange {
    pub clip: String,
    pub start: u32,
    pub end: u32,
}

/// An orbit camera: the point it turns around, its angles in radians and its
/// distance in meters, its vertical field of view in radians, and whether the
/// projection was orthographic.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SavedView {
    pub target: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub fov: f32,
    #[serde(default)]
    pub ortho: bool,
}

/// One message of a thread.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    #[serde(default)]
    pub author: String,
    /// When it was written, RFC 3339 in UTC (`2026-10-08T12:00:00Z`). Empty when
    /// unknown.
    #[serde(default)]
    pub time: String,
    #[serde(default)]
    pub text: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Thread {
    /// The opening message's text, which is what a list shows.
    pub fn title(&self) -> &str {
        self.messages
            .first()
            .map_or("", |message| message.text.as_str())
    }

    /// The opening message's author.
    pub fn author(&self) -> &str {
        self.messages
            .first()
            .map_or("", |message| message.author.as_str())
    }
}

/// A fresh thread id: 16 random hex digits, from the standard library's
/// per-process random hash keys mixed with the clock.
pub fn new_thread_id() -> String {
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos()),
    );
    format!("{:016x}", hasher.finish())
}

/// The current time as a message's `time`: RFC 3339 in UTC, to the second.
pub fn utc_now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    utc_from_unix(seconds)
}

/// `seconds` since the Unix epoch as RFC 3339 in UTC — the civil-from-days
/// conversion (Howard Hinnant's), so no date library is needed for one format.
fn utc_from_unix(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rem = seconds % 86_400;
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_times_format_as_utc() {
        assert_eq!(utc_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_from_unix(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(utc_from_unix(1_791_460_800), "2026-10-08T12:00:00Z");
    }

    #[test]
    fn thread_ids_are_sixteen_hex_digits_and_differ() {
        let (a, b) = (new_thread_id(), new_thread_id());
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
