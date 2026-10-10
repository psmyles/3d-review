//! Reading and writing one user-defined string property on an FBX's `Model`
//! objects, in binary and ASCII files alike, without disturbing anything else.
//!
//! [`scan`] lists the file's `Model` objects in file order — id, name, class,
//! parent — and the value of a named string property on each that carries one.
//! [`patch`] sets or removes that property on chosen objects and returns the new
//! file. Bytes in, bytes out: no I/O happens here, so the caller decides how a
//! file is written (the viewer writes through a sibling temp + rename).
//!
//! The property is a standard FBX user property — `P: "<name>", "KString", "",
//! "U", "<text>"` in ASCII terms, flagged `UH` when hidden — which is what lets
//! other applications read the file unchanged, and lets a DCC that imports user
//! properties carry it through a re-export.

mod ascii;
mod binary;
pub mod error;
mod file;

use std::collections::HashMap;

pub use error::{FbxError, FbxResult};

use binary::{Document, Node, NodeView, PropWriter, ReadAt, Reader, Value, decode_props};

/// Which encoding a file uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Binary { version: u32 },
    Ascii,
}

/// A `Model` object: a node of the scene graph as the file names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelObject {
    /// The object's 64-bit id, unique within the file.
    pub id: i64,
    /// Its name (non-UTF-8 bytes replaced).
    pub name: String,
    /// Its class: `Mesh`, `Null`, `LimbNode`, `Camera`, ...
    pub class: String,
    /// The `Model` it is parented to, `None` for a child of the scene root.
    pub parent: Option<i64>,
}

/// A stored string property, as found on one `Model`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredString {
    /// Index into [`Scan::models`].
    pub model: usize,
    /// The text, with any ASCII-FBX entities decoded.
    pub value: String,
    /// Whether the property carries the FBX hidden flag (`H`).
    pub hidden: bool,
}

/// What [`scan`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scan {
    pub format: Format,
    /// Every `Model` object, in file order.
    pub models: Vec<ModelObject>,
    /// The named property, wherever it is set.
    pub strings: Vec<StoredString>,
}

/// One change for [`patch`]: set `model`'s property to `value`, or remove it
/// (`None`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub model: i64,
    pub value: Option<String>,
    /// Mark the property hidden (`UH`) rather than only user-defined (`U`).
    pub hidden: bool,
}

/// Detect the encoding.
pub fn format(bytes: &[u8]) -> FbxResult<Format> {
    if bytes.starts_with(binary::MAGIC) {
        Reader::new(bytes).map(|reader| Format::Binary {
            version: reader.version,
        })
    } else if looks_ascii(bytes) {
        Ok(Format::Ascii)
    } else {
        Err(FbxError::NotFbx)
    }
}

/// An ASCII FBX starts with a `;` comment or a record key — never with the
/// binary magic, and never with bytes that aren't text.
fn looks_ascii(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(64)];
    let first = head.iter().find(|byte| !byte.is_ascii_whitespace());
    matches!(first, Some(b';') | Some(b'A'..=b'Z') | Some(b'a'..=b'z'))
        && head.iter().all(|&byte| {
            byte == b'\t'
                || byte == b'\n'
                || byte == b'\r'
                || (0x20..0x7f).contains(&byte)
                || byte >= 0x80
        })
}

/// List the `Model` objects and every value of the string property `property`.
pub fn scan(bytes: &[u8], property: &str) -> FbxResult<Scan> {
    match format(bytes)? {
        Format::Binary { version } => scan_binary(bytes, property, version),
        Format::Ascii => scan_ascii(bytes, property),
    }
}

/// Apply `edits` to the string property `property`, returning the new file.
/// An empty edit list returns the input unchanged.
pub fn patch(bytes: &[u8], property: &str, edits: &[Edit]) -> FbxResult<Vec<u8>> {
    match format(bytes)? {
        Format::Binary { .. } => patch_binary(bytes, property, edits),
        Format::Ascii => {
            let document = ascii::Document::parse(bytes)?;
            let ascii_edits: Vec<ascii::AsciiEdit<'_>> = edits
                .iter()
                .map(|edit| ascii::AsciiEdit {
                    model: edit.model,
                    name: property,
                    value: edit.value.as_deref(),
                    flags: flags(edit.hidden),
                })
                .collect();
            ascii::patch(&document, &ascii_edits)
        }
    }
}

fn flags(hidden: bool) -> &'static str {
    if hidden { "UH" } else { "U" }
}

/// Parents resolved from `(child, parent)` connections, keeping only parents
/// that are themselves `Model`s (a mesh's connection to its geometry runs the
/// other way, and the scene root is id 0, which no object has).
fn parents(models: &mut [ModelObject], connections: impl IntoIterator<Item = (i64, i64)>) {
    let index: HashMap<i64, usize> = models
        .iter()
        .enumerate()
        .map(|(position, model)| (model.id, position))
        .collect();
    for (child, parent) in connections {
        if let (Some(&child), true) = (index.get(&child), index.contains_key(&parent)) {
            models[child].parent.get_or_insert(parent);
        }
    }
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

// ── Binary ───────────────────────────────────────────────────────────────────

/// A binary `Model` record's name property is `name\0\x01Model`.
fn binary_object_name(raw: &[u8]) -> &[u8] {
    raw.windows(2)
        .position(|pair| pair == b"\x00\x01")
        .map_or(raw, |split| &raw[..split])
}

fn scan_binary(bytes: &[u8], property: &str, version: u32) -> FbxResult<Scan> {
    let reader = Reader::new(bytes)?;
    let top = reader.top_level()?;
    let objects = top
        .nodes
        .iter()
        .find(|node| node.name == b"Objects")
        .ok_or(FbxError::NoObjects)?;
    let mut collected = Collected::default();
    for node in reader
        .children(objects)?
        .nodes
        .iter()
        .filter(|node| node.name == b"Model")
    {
        collected.model(&reader, node, property)?;
    }
    if let Some(node) = top.nodes.iter().find(|node| node.name == b"Connections") {
        collected.connections(&reader, node)?;
    }
    Ok(collected.finish(Format::Binary { version }))
}

/// What a binary scan gathers, record by record — shared by the in-memory scan
/// and the streaming one, which reach the same records by different means.
#[derive(Default)]
struct Collected {
    models: Vec<ModelObject>,
    strings: Vec<StoredString>,
    connections: Vec<(i64, i64)>,
}

impl Collected {
    /// A `Model` record: its identity, and the property if it carries it.
    fn model(&mut self, reader: &Reader<'_>, node: &NodeView<'_>, property: &str) -> FbxResult<()> {
        let values = decode_props(node.props, node.num_props)?;
        let id = values
            .first()
            .and_then(Value::as_int)
            .ok_or(FbxError::BadProperty)?;
        let name = values.get(1).and_then(Value::as_str).unwrap_or_default();
        let class = values.get(2).and_then(Value::as_str).unwrap_or_default();
        if let Some((value, hidden)) = binary_property(reader, node, property)? {
            self.strings.push(StoredString {
                model: self.models.len(),
                value,
                hidden,
            });
        }
        self.models.push(ModelObject {
            id,
            name: lossy(binary_object_name(name)),
            class: lossy(class),
            parent: None,
        });
        Ok(())
    }

    /// The `Connections` record's object-object links.
    fn connections(&mut self, reader: &Reader<'_>, node: &NodeView<'_>) -> FbxResult<()> {
        for connection in reader
            .children(node)?
            .nodes
            .iter()
            .filter(|node| node.name == b"C")
        {
            let values = decode_props(connection.props, connection.num_props)?;
            if values.first().and_then(Value::as_str) == Some(b"OO")
                && let (Some(child), Some(parent)) = (
                    values.get(1).and_then(Value::as_int),
                    values.get(2).and_then(Value::as_int),
                )
            {
                self.connections.push((child, parent));
            }
        }
        Ok(())
    }

    fn finish(mut self, format: Format) -> Scan {
        parents(&mut self.models, self.connections);
        Scan {
            format,
            models: self.models,
            strings: self.strings,
        }
    }
}

/// [`scan`] a file on disk. A binary file is *streamed*: the scan seeks from record
/// to record by their end offsets and reads only the `Model` and `Connections`
/// records, so a 200 MB asset costs a few hundred kilobytes of reading rather than
/// a second copy of the file in memory. An ASCII file has no offsets to skip by and
/// is read whole.
pub fn scan_file(path: &std::path::Path, property: &str) -> FbxResult<Scan> {
    let io = |_| FbxError::Io(path.display().to_string());
    let file = std::fs::File::open(path).map_err(io)?;
    let len = file.metadata().map_err(io)?.len() as usize;
    let mut source = file::FileSource::new(file);
    let mut head = vec![0u8; binary::FIRST_RECORD.min(len)];
    source.read_exact_at(0, &mut head).map_err(io)?;
    match format(&head) {
        Ok(Format::Binary { version }) => file::scan_binary(&mut source, len, version, property),
        Ok(Format::Ascii) | Err(FbxError::NotBinary) => {
            let bytes = std::fs::read(path).map_err(io)?;
            scan(&bytes, property)
        }
        Err(error) => Err(error),
    }
}

/// The string value and hidden flag of `model`'s property `name`.
fn binary_property(
    reader: &Reader<'_>,
    model: &NodeView<'_>,
    name: &str,
) -> FbxResult<Option<(String, bool)>> {
    let children = reader.children(model)?;
    let Some(props70) = children
        .nodes
        .iter()
        .find(|node| node.name == b"Properties70")
    else {
        return Ok(None);
    };
    for p in reader
        .children(props70)?
        .nodes
        .iter()
        .filter(|node| node.name == b"P")
    {
        let values = decode_props(p.props, p.num_props)?;
        if values.first().and_then(Value::as_str) != Some(name.as_bytes()) {
            continue;
        }
        let flags = values.get(3).and_then(Value::as_str).unwrap_or_default();
        let value = values.get(4).and_then(Value::as_str).unwrap_or_default();
        return Ok(Some((lossy(value), flags.contains(&b'H'))));
    }
    Ok(None)
}

fn patch_binary(bytes: &[u8], property: &str, edits: &[Edit]) -> FbxResult<Vec<u8>> {
    let mut document = Document::parse(bytes)?;
    let objects = document.top_mut(b"Objects").ok_or(FbxError::NoObjects)?;
    for edit in edits {
        let model = objects
            .children
            .iter_mut()
            .find(|node| *node.name == *b"Model" && model_id(node) == Some(edit.model))
            .ok_or(FbxError::NoSuchObject(edit.model))?;
        set_binary_property(model, property, edit)?;
    }
    document.write()
}

fn model_id(node: &Node<'_>) -> Option<i64> {
    decode_props(&node.props, node.num_props)
        .ok()?
        .first()
        .and_then(Value::as_int)
}

fn property_name(node: &Node<'_>) -> Option<Vec<u8>> {
    decode_props(&node.props, node.num_props)
        .ok()?
        .first()
        .and_then(Value::as_str)
        .map(<[u8]>::to_vec)
}

fn set_binary_property(model: &mut Node<'_>, property: &str, edit: &Edit) -> FbxResult<()> {
    if model.child_mut(b"Properties70").is_none() {
        if edit.value.is_none() {
            return Ok(());
        }
        // The SDK orders a model's records Version, Properties70, ...
        let at = model
            .children
            .iter()
            .position(|node| *node.name == *b"Version")
            .map_or(0, |version| version + 1);
        let mut block = Node::new(b"Properties70", 0, Vec::new());
        block.terminated = true;
        model.children.insert(at, block);
    }
    let Some(props70) = model.child_mut(b"Properties70") else {
        return Ok(());
    };
    let existing = props70.children.iter().position(|node| {
        *node.name == *b"P" && property_name(node).as_deref() == Some(property.as_bytes())
    });
    match (&edit.value, existing) {
        (Some(value), existing) => {
            let (count, props) = PropWriter::default()
                .string(property.as_bytes())?
                .string(b"KString")?
                .string(b"")?
                .string(flags(edit.hidden).as_bytes())?
                .string(value.as_bytes())?
                .finish();
            let mut node = Node::new(b"P", count, props);
            // Match how this writer ends a childless `P` record.
            node.terminated = props70
                .children
                .first()
                .is_some_and(|sibling| sibling.terminated);
            match existing {
                Some(index) => props70.children[index] = node,
                None => {
                    props70.children.push(node);
                    // A block that just gained its first child closes with a
                    // null record, as every non-empty nested list does.
                    props70.terminated = true;
                }
            }
        }
        (None, Some(index)) => {
            props70.children.remove(index);
        }
        (None, None) => {}
    }
    Ok(())
}

// ── ASCII ────────────────────────────────────────────────────────────────────

fn scan_ascii(bytes: &[u8], property: &str) -> FbxResult<Scan> {
    let document = ascii::Document::parse(bytes)?;
    let records = document.models()?;
    let mut models = Vec::with_capacity(records.len());
    let mut strings = Vec::new();
    for record in &records {
        if let Some(found) = document.property(record, property.as_bytes()) {
            strings.push(StoredString {
                model: models.len(),
                value: lossy(&ascii::unescape(found.string.unwrap_or_default())),
                hidden: found.flags.contains(&b'H'),
            });
        }
        models.push(ModelObject {
            id: record.id,
            name: lossy(&ascii::unescape(record.name)),
            class: lossy(record.class),
            parent: None,
        });
    }
    parents(&mut models, document.connections());
    Ok(Scan {
        format: Format::Ascii,
        models,
        strings,
    })
}
