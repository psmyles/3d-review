//! A review comment thread, as stored: the types the JSON payload decodes into.
//!
//! The wire format is documented in `docs/review-comments-format.md`, which is
//! the contract other tools read against. Every struct keeps the fields it does
//! not know (`extra`), so a file written by a newer viewer loses nothing when an
//! older one edits a different thread in it.

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
