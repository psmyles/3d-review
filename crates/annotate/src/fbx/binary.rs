//! The binary FBX container, read and rewritten without understanding it.
//!
//! A binary FBX is a header, a tree of node records, a null record ending the
//! top level, and a footer. Each record states its name, its raw property bytes
//! and — the part that makes editing hard — the **absolute file offset** at which
//! it ends. Inserting one property therefore moves every record after it, and
//! every enclosing record's end, so an edit cannot be a splice: the tree is parsed
//! and written back whole.
//!
//! What makes that safe is how little is interpreted. A record's property bytes
//! are carried as an opaque slice — compressed geometry arrays are never inflated,
//! floats never round-tripped through a decoder — and only the records an edit
//! touches are rebuilt. Writing the parsed tree back with no edit reproduces the
//! input byte for byte, which the fixture tests pin on every file in
//! `assets/test_models`.
//!
//! Two layout details are carried through rather than assumed:
//!
//! * whether a record's nested list ends in a null record (writers differ on
//!   records with no children), kept per record as [`Node::terminated`];
//! * the footer's alignment padding. Its length depends on where the footer
//!   lands, so an edit that changes the file's size re-pads it — preserving the
//!   writer's own convention (the Autodesk SDK writes four more zero bytes than
//!   Blender does) rather than imposing one. See [`Footer`].

use std::borrow::Cow;

use super::error::{FbxError, FbxResult};

/// The binary header's magic: `"Kaydara FBX Binary  "`, a NUL, then `0x1A 0x00`.
pub(crate) const MAGIC: &[u8] = b"Kaydara FBX Binary  \x00\x1a\x00";
/// Magic + the `u32` version.
const HEADER_LEN: usize = MAGIC.len() + 4;
/// The first version whose record headers are 64-bit.
const WIDE_VERSION: u32 = 7500;
/// The oldest binary version whose objects carry 64-bit ids (FBX 7.x). FBX 6.x
/// names objects by string alone, which this module does not address.
const OLDEST_SUPPORTED: u32 = 7000;

/// The footer's fixed layout after its alignment padding: the version, 120 zero
/// bytes, and a 16-byte magic.
const FOOTER_ID_LEN: usize = 16;
const FOOTER_FIXED_LEN: usize = 4 + 120 + 16;

/// Record-header width, by version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Layout {
    wide: bool,
}

/// A record header as read.
struct RecordHeader {
    end: u64,
    num_props: u64,
    props_len: u64,
    name_len: u8,
}

impl Layout {
    fn for_version(version: u32) -> Self {
        Self {
            wide: version >= WIDE_VERSION,
        }
    }

    /// Byte length of a record header — and of a null record, which is one of
    /// these filled with zeros.
    fn header_len(self) -> usize {
        if self.wide { 25 } else { 13 }
    }

    fn read_header(self, data: &[u8], pos: usize) -> FbxResult<RecordHeader> {
        let bytes = data
            .get(pos..pos + self.header_len())
            .ok_or(FbxError::Truncated { offset: pos })?;
        let (end, num_props, props_len, name_len) = if self.wide {
            (
                u64::from_le_bytes(bytes[0..8].try_into().unwrap_or_default()),
                u64::from_le_bytes(bytes[8..16].try_into().unwrap_or_default()),
                u64::from_le_bytes(bytes[16..24].try_into().unwrap_or_default()),
                bytes[24],
            )
        } else {
            (
                u64::from(u32::from_le_bytes(
                    bytes[0..4].try_into().unwrap_or_default(),
                )),
                u64::from(u32::from_le_bytes(
                    bytes[4..8].try_into().unwrap_or_default(),
                )),
                u64::from(u32::from_le_bytes(
                    bytes[8..12].try_into().unwrap_or_default(),
                )),
                bytes[12],
            )
        };
        Ok(RecordHeader {
            end,
            num_props,
            props_len,
            name_len,
        })
    }

    /// Append a header whose end offset is a placeholder for [`Self::patch_end`].
    fn push_header(self, out: &mut Vec<u8>, num_props: u64, props_len: u64, name_len: u8) {
        if self.wide {
            out.extend_from_slice(&0u64.to_le_bytes());
            out.extend_from_slice(&num_props.to_le_bytes());
            out.extend_from_slice(&props_len.to_le_bytes());
        } else {
            // Narrow files cap every field at `u32`; a record that outgrew it
            // could not have been read from one either, so this only clamps
            // what `Node::check` has already refused.
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&(num_props as u32).to_le_bytes());
            out.extend_from_slice(&(props_len as u32).to_le_bytes());
        }
        out.push(name_len);
    }

    fn patch_end(self, out: &mut [u8], start: usize, end: usize) {
        if self.wide {
            out[start..start + 8].copy_from_slice(&(end as u64).to_le_bytes());
        } else {
            out[start..start + 4].copy_from_slice(&(end as u32).to_le_bytes());
        }
    }

    fn push_null(self, out: &mut Vec<u8>) {
        out.resize(out.len() + self.header_len(), 0);
    }
}

/// One record, borrowed from the file: its name, its property count and raw
/// property bytes, and where its nested list sits. Reading through these
/// allocates nothing but the lists the caller asks for, which is what lets the
/// comment reader skip a 40 MB geometry record by its end offset.
#[derive(Clone, Copy, Debug)]
pub(crate) struct NodeView<'a> {
    pub(crate) name: &'a [u8],
    pub(crate) num_props: u64,
    pub(crate) props: &'a [u8],
    children_start: usize,
    end: usize,
}

/// A run of sibling records, plus whether a null record closed it.
pub(crate) struct List<'a> {
    pub(crate) nodes: Vec<NodeView<'a>>,
    /// Where the closing null record ends, if there was one.
    terminator_end: Option<usize>,
}

/// A borrowed binary FBX, ready to be walked.
#[derive(Clone, Copy)]
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    layout: Layout,
    pub(crate) version: u32,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> FbxResult<Self> {
        if !data.starts_with(MAGIC) {
            return Err(FbxError::NotBinary);
        }
        let version = data
            .get(MAGIC.len()..HEADER_LEN)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_le_bytes)
            .ok_or(FbxError::Truncated {
                offset: MAGIC.len(),
            })?;
        if version < OLDEST_SUPPORTED {
            return Err(FbxError::UnsupportedVersion(version));
        }
        Ok(Self {
            data,
            layout: Layout::for_version(version),
            version,
        })
    }

    /// The top-level records, which must end in a null record.
    pub(crate) fn top_level(&self) -> FbxResult<List<'a>> {
        let list = self.list(HEADER_LEN, self.data.len(), true)?;
        if list.terminator_end.is_none() {
            return Err(FbxError::Truncated {
                offset: self.data.len(),
            });
        }
        Ok(list)
    }

    /// `node`'s children.
    pub(crate) fn children(&self, node: &NodeView<'a>) -> FbxResult<List<'a>> {
        self.list(node.children_start, node.end, false)
    }

    /// The records from `start` up to `limit`, stopping at a null record. At the
    /// top level what follows the null record is the footer; inside a record the
    /// null record has to be the last thing in it, or the record holds bytes this
    /// reader would drop.
    fn list(&self, start: usize, limit: usize, top_level: bool) -> FbxResult<List<'a>> {
        let mut nodes = Vec::new();
        let mut pos = start;
        while pos < limit {
            let header = self.layout.read_header(self.data, pos)?;
            if header.end == 0 {
                let is_null =
                    header.num_props == 0 && header.props_len == 0 && header.name_len == 0;
                if !is_null {
                    return Err(FbxError::Malformed { offset: pos });
                }
                let terminator_end = pos + self.layout.header_len();
                if !top_level && terminator_end != limit {
                    return Err(FbxError::Malformed { offset: pos });
                }
                return Ok(List {
                    nodes,
                    terminator_end: Some(terminator_end),
                });
            }
            let node = self.view(pos, &header, limit)?;
            pos = node.end;
            nodes.push(node);
        }
        Ok(List {
            nodes,
            terminator_end: None,
        })
    }

    fn view(&self, pos: usize, header: &RecordHeader, limit: usize) -> FbxResult<NodeView<'a>> {
        let malformed = FbxError::Malformed { offset: pos };
        let end = usize::try_from(header.end).map_err(|_| malformed.clone())?;
        let name_start = pos + self.layout.header_len();
        let props_start = name_start + usize::from(header.name_len);
        let props_len = usize::try_from(header.props_len).map_err(|_| malformed.clone())?;
        let children_start = props_start
            .checked_add(props_len)
            .ok_or_else(|| malformed.clone())?;
        if end <= pos || end > limit || children_start > end {
            return Err(malformed);
        }
        Ok(NodeView {
            name: &self.data[name_start..props_start],
            num_props: header.num_props,
            props: &self.data[props_start..children_start],
            children_start,
            end,
        })
    }
}

/// One record, owned enough to be edited: untouched records borrow their name and
/// property bytes from the file, and only what an edit builds is allocated.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Node<'a> {
    pub(crate) name: Cow<'a, [u8]>,
    pub(crate) num_props: u64,
    pub(crate) props: Cow<'a, [u8]>,
    pub(crate) children: Vec<Node<'a>>,
    /// Whether the nested list ends in a null record. Carried from the file for
    /// a parsed record, so a rewrite reproduces it.
    pub(crate) terminated: bool,
}

impl<'a> Node<'a> {
    /// A new record named `name` with property bytes `props` (as built by
    /// [`PropWriter`]) and no children.
    pub(crate) fn new(name: &'static [u8], num_props: u64, props: Vec<u8>) -> Self {
        Self {
            name: Cow::Borrowed(name),
            num_props,
            props: Cow::Owned(props),
            children: Vec::new(),
            terminated: false,
        }
    }

    fn from_view(reader: &Reader<'a>, view: &NodeView<'a>) -> FbxResult<Self> {
        let list = reader.children(view)?;
        let children = list
            .nodes
            .iter()
            .map(|child| Self::from_view(reader, child))
            .collect::<FbxResult<Vec<_>>>()?;
        Ok(Self {
            name: Cow::Borrowed(view.name),
            num_props: view.num_props,
            props: Cow::Borrowed(view.props),
            children,
            terminated: list.terminator_end.is_some(),
        })
    }

    /// Refuse a record the file's layout cannot hold, before anything is written.
    fn check(&self, layout: Layout) -> FbxResult<()> {
        if self.name.len() > usize::from(u8::MAX) {
            return Err(FbxError::TooLarge);
        }
        if !layout.wide
            && (self.props.len() > u32::MAX as usize || self.num_props > u64::from(u32::MAX))
        {
            return Err(FbxError::TooLarge);
        }
        self.children
            .iter()
            .try_for_each(|child| child.check(layout))
    }

    fn write(&self, out: &mut Vec<u8>, layout: Layout) {
        let start = out.len();
        layout.push_header(
            out,
            self.num_props,
            self.props.len() as u64,
            self.name.len() as u8,
        );
        out.extend_from_slice(&self.name);
        out.extend_from_slice(&self.props);
        for child in &self.children {
            child.write(out, layout);
        }
        if self.terminated {
            layout.push_null(out);
        }
        let end = out.len();
        layout.patch_end(out, start, end);
    }

    /// The first child named `name`.
    pub(crate) fn child_mut(&mut self, name: &[u8]) -> Option<&mut Node<'a>> {
        self.children.iter_mut().find(|child| *child.name == *name)
    }
}

/// The footer, split where an edit has to re-pad it.
///
/// Its layout is a 16-byte id, zero padding, the version, 120 zero bytes and a
/// 16-byte magic. The padding aligns the bytes after the id to 16 — with a writer
/// convention on top: the Autodesk SDK writes 4 more zero bytes than that, Blender
/// none. Measured on every fixture, the excess is constant per writer, so it is
/// carried as `extra` and re-applied at the new offset. A footer that doesn't
/// match the layout is kept verbatim, padding and all: no reader this module was
/// tested against validates the footer's alignment, and guessing would be worse.
struct Footer<'a> {
    id: &'a [u8],
    extra: usize,
    fixed: &'a [u8],
}

impl<'a> Footer<'a> {
    /// Split `tail` (everything after the top-level null record, which ends at
    /// `tail_start`), or `None` when it isn't the layout above.
    fn parse(tail: &'a [u8], tail_start: usize, version: u32) -> Option<Self> {
        let pad = tail.len().checked_sub(FOOTER_ID_LEN + FOOTER_FIXED_LEN)?;
        let (id, rest) = tail.split_at(FOOTER_ID_LEN);
        let (padding, fixed) = rest.split_at(pad);
        if padding.iter().any(|&byte| byte != 0) || !fixed.starts_with(&version.to_le_bytes()) {
            return None;
        }
        let extra = pad.checked_sub(alignment(tail_start + FOOTER_ID_LEN))?;
        (extra <= 16).then_some(Self { id, extra, fixed })
    }

    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.id);
        let pad = alignment(out.len()) + self.extra;
        out.resize(out.len() + pad, 0);
        out.extend_from_slice(self.fixed);
    }
}

/// Zero bytes from `offset` to the next 16-byte boundary, a full 16 when it is
/// already on one — the rule both observed writers share.
fn alignment(offset: usize) -> usize {
    match offset % 16 {
        0 => 16,
        rem => 16 - rem,
    }
}

/// A binary FBX parsed into an editable tree.
pub(crate) struct Document<'a> {
    header: &'a [u8],
    layout: Layout,
    pub(crate) nodes: Vec<Node<'a>>,
    /// Everything after the top-level null record.
    tail: &'a [u8],
    /// Where `tail` began in the input.
    tail_start: usize,
    version: u32,
}

impl<'a> Document<'a> {
    pub(crate) fn parse(data: &'a [u8]) -> FbxResult<Self> {
        let reader = Reader::new(data)?;
        let list = reader.top_level()?;
        let nodes = list
            .nodes
            .iter()
            .map(|view| Node::from_view(&reader, view))
            .collect::<FbxResult<Vec<_>>>()?;
        let tail_start = list.terminator_end.unwrap_or(data.len());
        Ok(Self {
            header: &data[..HEADER_LEN],
            layout: reader.layout,
            nodes,
            tail: &data[tail_start..],
            tail_start,
            version: reader.version,
        })
    }

    /// The top-level record named `name`.
    pub(crate) fn top_mut(&mut self, name: &[u8]) -> Option<&mut Node<'a>> {
        self.nodes.iter_mut().find(|node| *node.name == *name)
    }

    /// The file, written back. Unedited, this is the input byte for byte.
    pub(crate) fn write(&self) -> FbxResult<Vec<u8>> {
        self.nodes
            .iter()
            .try_for_each(|node| node.check(self.layout))?;
        let mut out = Vec::with_capacity(self.tail_start + self.tail.len() + 4096);
        out.extend_from_slice(self.header);
        for node in &self.nodes {
            node.write(&mut out, self.layout);
        }
        self.layout.push_null(&mut out);
        if !self.layout.wide && out.len() > u32::MAX as usize {
            return Err(FbxError::TooLarge);
        }
        if out.len() == self.tail_start {
            out.extend_from_slice(self.tail);
        } else {
            match Footer::parse(self.tail, self.tail_start, self.version) {
                Some(footer) => footer.write(&mut out),
                None => out.extend_from_slice(self.tail),
            }
        }
        Ok(out)
    }
}

/// A decoded property value — only the kinds this crate reads. Arrays and raw
/// blobs are skipped by length, never decoded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Value<'a> {
    Int(i64),
    Float(f64),
    Str(&'a [u8]),
    /// An array, a raw blob or a bool: present, not read.
    Other,
}

impl Value<'_> {
    pub(crate) fn as_str(&self) -> Option<&[u8]> {
        match self {
            Value::Str(bytes) => Some(bytes),
            _ => None,
        }
    }

    pub(crate) fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(value) => Some(*value),
            _ => None,
        }
    }
}

/// Decode a record's property bytes into its values.
pub(crate) fn decode_props(props: &[u8], num_props: u64) -> FbxResult<Vec<Value<'_>>> {
    let malformed = FbxError::BadProperty;
    let mut values = Vec::with_capacity(num_props.min(64) as usize);
    let mut pos = 0usize;
    let take = |pos: &mut usize, len: usize| -> FbxResult<&[u8]> {
        let bytes = props.get(*pos..*pos + len).ok_or(FbxError::BadProperty)?;
        *pos += len;
        Ok(bytes)
    };
    for _ in 0..num_props {
        let code = *take(&mut pos, 1)?.first().ok_or(FbxError::BadProperty)?;
        let value = match code {
            b'Y' => Value::Int(i64::from(i16::from_le_bytes(
                take(&mut pos, 2)?
                    .try_into()
                    .map_err(|_| malformed.clone())?,
            ))),
            b'C' => {
                take(&mut pos, 1)?;
                Value::Other
            }
            b'I' => Value::Int(i64::from(i32::from_le_bytes(
                take(&mut pos, 4)?
                    .try_into()
                    .map_err(|_| malformed.clone())?,
            ))),
            b'F' => Value::Float(f64::from(f32::from_le_bytes(
                take(&mut pos, 4)?
                    .try_into()
                    .map_err(|_| malformed.clone())?,
            ))),
            b'D' => Value::Float(f64::from_le_bytes(
                take(&mut pos, 8)?
                    .try_into()
                    .map_err(|_| malformed.clone())?,
            )),
            b'L' => Value::Int(i64::from_le_bytes(
                take(&mut pos, 8)?
                    .try_into()
                    .map_err(|_| malformed.clone())?,
            )),
            b'S' | b'R' => {
                let len = u32::from_le_bytes(
                    take(&mut pos, 4)?
                        .try_into()
                        .map_err(|_| malformed.clone())?,
                );
                let bytes = take(&mut pos, len as usize)?;
                if code == b'S' {
                    Value::Str(bytes)
                } else {
                    Value::Other
                }
            }
            b'f' | b'd' | b'l' | b'i' | b'b' | b'c' => {
                // Array header: element count, encoding, then the stored byte
                // length — which is the data's length whether or not it is
                // compressed, so an array is skipped without inflating it.
                take(&mut pos, 8)?;
                let stored = u32::from_le_bytes(
                    take(&mut pos, 4)?
                        .try_into()
                        .map_err(|_| malformed.clone())?,
                );
                take(&mut pos, stored as usize)?;
                Value::Other
            }
            _ => return Err(malformed),
        };
        values.push(value);
    }
    if pos != props.len() {
        return Err(malformed);
    }
    Ok(values)
}

/// Builds a record's property bytes.
#[derive(Default)]
pub(crate) struct PropWriter {
    bytes: Vec<u8>,
    count: u64,
}

impl PropWriter {
    pub(crate) fn string(mut self, value: &[u8]) -> FbxResult<Self> {
        let len = u32::try_from(value.len()).map_err(|_| FbxError::TooLarge)?;
        self.bytes.push(b'S');
        self.bytes.extend_from_slice(&len.to_le_bytes());
        self.bytes.extend_from_slice(value);
        self.count += 1;
        Ok(self)
    }

    pub(crate) fn finish(self) -> (u64, Vec<u8>) {
        (self.count, self.bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_rounds_up_to_the_next_boundary_and_never_to_zero() {
        assert_eq!(alignment(0), 16);
        assert_eq!(alignment(1), 15);
        assert_eq!(alignment(15), 1);
        assert_eq!(alignment(16), 16);
        assert_eq!(alignment(17), 15);
    }

    #[test]
    fn a_written_string_decodes_back() {
        let (count, bytes) = PropWriter::default()
            .string(b"Name")
            .and_then(|writer| writer.string(b""))
            .map(PropWriter::finish)
            .expect("small strings fit");
        let values = decode_props(&bytes, count).expect("decodes");
        assert_eq!(values, [Value::Str(b"Name"), Value::Str(b"")]);
    }

    #[test]
    fn a_property_list_with_trailing_bytes_is_refused() {
        let (count, mut bytes) = PropWriter::default()
            .string(b"x")
            .map(PropWriter::finish)
            .expect("fits");
        bytes.push(0);
        assert!(decode_props(&bytes, count).is_err());
    }

    #[test]
    fn a_file_without_the_magic_is_not_binary() {
        assert!(matches!(
            Reader::new(b"; FBX 7.7.0 project file"),
            Err(FbxError::NotBinary)
        ));
    }
}
