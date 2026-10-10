//! The ASCII FBX container: tokenized just enough to find records, and edited by
//! splicing text, so every byte outside an edit stays exactly as it was.
//!
//! ASCII FBX is a tree of `Key: value, value, ... { children }` records with `;`
//! comments. Unlike the binary form nothing stores an offset, so an edit really
//! is a splice: the patcher replaces, removes or inserts one property line and
//! leaves the rest of the file alone.
//!
//! Strings have no backslash escapes. The Autodesk SDK — and ufbx, which the
//! viewer reads with — instead decode `&quot;`, `&cr;` and `&lf;` inside them, and
//! have no escape for `&` itself. [`escape`] therefore also rewrites `&` in the
//! JSON this crate stores as the JSON escape `\u0026`, so no stored text can ever
//! spell one of those entities by accident.

use super::error::{FbxError, FbxResult};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// `Name:` — the span excludes the colon.
    Key,
    /// `"..."` — the span excludes the quotes.
    Str,
    /// A number, `*N`, `Y`, `T`, ...
    Bare,
    Open,
    Close,
    Comma,
}

#[derive(Clone, Copy, Debug)]
struct Token {
    kind: Kind,
    start: usize,
    end: usize,
}

struct Lexer<'a> {
    data: &'a [u8],
    pos: usize,
    peeked: Option<Option<Token>>,
}

impl<'a> Lexer<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            peeked: None,
        }
    }

    fn syntax_error(&self, offset: usize) -> FbxError {
        FbxError::AsciiSyntax {
            line: line_number(self.data, offset),
        }
    }

    fn peek(&mut self) -> FbxResult<Option<Token>> {
        if self.peeked.is_none() {
            self.peeked = Some(self.lex()?);
        }
        Ok(self.peeked.flatten())
    }

    fn next(&mut self) -> FbxResult<Option<Token>> {
        let token = self.peek()?;
        self.peeked = None;
        Ok(token)
    }

    fn lex(&mut self) -> FbxResult<Option<Token>> {
        let data = self.data;
        loop {
            match data.get(self.pos) {
                None => return Ok(None),
                Some(byte) if byte.is_ascii_whitespace() => self.pos += 1,
                Some(b';') => {
                    while self.pos < data.len() && data[self.pos] != b'\n' {
                        self.pos += 1;
                    }
                }
                Some(_) => break,
            }
        }
        let start = self.pos;
        let single = |kind| Token {
            kind,
            start,
            end: start + 1,
        };
        let token = match data[start] {
            b'{' => single(Kind::Open),
            b'}' => single(Kind::Close),
            b',' => single(Kind::Comma),
            b'"' => {
                let close = data[start + 1..]
                    .iter()
                    .position(|&byte| byte == b'"')
                    .ok_or_else(|| self.syntax_error(start))?;
                self.pos = start + 1 + close + 1;
                return Ok(Some(Token {
                    kind: Kind::Str,
                    start: start + 1,
                    end: start + 1 + close,
                }));
            }
            _ => {
                let mut end = start;
                while end < data.len()
                    && !data[end].is_ascii_whitespace()
                    && !matches!(data[end], b',' | b'{' | b'}' | b'"' | b';' | b':')
                {
                    end += 1;
                }
                if end == start {
                    return Err(self.syntax_error(start));
                }
                if data.get(end) == Some(&b':') {
                    self.pos = end + 1;
                    return Ok(Some(Token {
                        kind: Kind::Key,
                        start,
                        end,
                    }));
                }
                self.pos = end;
                return Ok(Some(Token {
                    kind: Kind::Bare,
                    start,
                    end,
                }));
            }
        };
        self.pos = token.end;
        Ok(Some(token))
    }
}

/// A value of a record: a string's contents (still escaped) or a bare token.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AValue {
    is_str: bool,
    start: usize,
    end: usize,
}

/// One record: its key, its values, and the byte spans an edit needs.
#[derive(Debug)]
pub(crate) struct Record {
    key_start: usize,
    key_end: usize,
    values: Vec<AValue>,
    /// End of the last value (or of the key, with no values). Excludes the
    /// closing quote of a trailing string, which is added back by
    /// [`Record::header_end`].
    header_end: usize,
    /// The block's `{` and `}` offsets, for a record with one.
    block: Option<(usize, usize)>,
    children: Vec<Record>,
}

impl Record {
    fn key<'d>(&self, data: &'d [u8]) -> &'d [u8] {
        &data[self.key_start..self.key_end]
    }

    fn str_value<'d>(&self, data: &'d [u8], index: usize) -> Option<&'d [u8]> {
        self.values
            .get(index)
            .filter(|value| value.is_str)
            .map(|value| &data[value.start..value.end])
    }

    fn int_value(&self, data: &[u8], index: usize) -> Option<i64> {
        let value = self.values.get(index).filter(|value| !value.is_str)?;
        std::str::from_utf8(&data[value.start..value.end])
            .ok()?
            .parse()
            .ok()
    }

    fn child(&self, data: &[u8], key: &[u8]) -> Option<&Record> {
        self.children.iter().find(|child| child.key(data) == key)
    }
}

fn parse_records(lexer: &mut Lexer<'_>, nested: bool) -> FbxResult<Vec<Record>> {
    let mut records = Vec::new();
    loop {
        match lexer.peek()? {
            None if !nested => return Ok(records),
            Some(token) if token.kind == Kind::Close && nested => return Ok(records),
            Some(token) if token.kind == Kind::Key => {
                lexer.next()?;
                records.push(parse_record(lexer, token)?);
            }
            Some(token) => return Err(lexer.syntax_error(token.start)),
            None => return Err(lexer.syntax_error(lexer.data.len())),
        }
    }
}

fn parse_record(lexer: &mut Lexer<'_>, key: Token) -> FbxResult<Record> {
    let mut values = Vec::new();
    let mut header_end = key.end + 1;
    // Values are comma-separated and may run across lines (long arrays do); a
    // stray leading comma (`Content: ,"..."`) is tolerated. A record ends at the
    // next key, brace or end of file.
    while let Some(token) = lexer.peek()? {
        match token.kind {
            Kind::Comma => {
                lexer.next()?;
            }
            Kind::Str | Kind::Bare => {
                lexer.next()?;
                values.push(AValue {
                    is_str: token.kind == Kind::Str,
                    start: token.start,
                    end: token.end,
                });
                header_end = token.end + usize::from(token.kind == Kind::Str);
            }
            _ => break,
        }
    }
    let mut block = None;
    let mut children = Vec::new();
    if let Some(open) = lexer.peek()?.filter(|token| token.kind == Kind::Open) {
        lexer.next()?;
        children = parse_records(lexer, true)?;
        let close = lexer
            .next()?
            .filter(|token| token.kind == Kind::Close)
            .ok_or_else(|| lexer.syntax_error(open.start))?;
        block = Some((open.start, close.start));
    }
    Ok(Record {
        key_start: key.start,
        key_end: key.end,
        values,
        header_end,
        block,
        children,
    })
}

fn line_number(data: &[u8], offset: usize) -> usize {
    data[..offset.min(data.len())]
        .iter()
        .filter(|&&byte| byte == b'\n')
        .count()
        + 1
}

/// A parsed ASCII FBX.
pub(crate) struct Document<'a> {
    pub(crate) data: &'a [u8],
    records: Vec<Record>,
}

/// A `Model` object, found in `Objects`.
pub(crate) struct ModelRecord<'r> {
    pub(crate) id: i64,
    /// The object's name, without the `Model::` prefix (still escaped).
    pub(crate) name: &'r [u8],
    pub(crate) class: &'r [u8],
    record: &'r Record,
}

/// A `P:` property of a `Properties70` block.
pub(crate) struct PropertyRecord<'r> {
    pub(crate) flags: &'r [u8],
    /// The fifth value — a `KString`'s text, still escaped.
    pub(crate) string: Option<&'r [u8]>,
}

impl<'a> Document<'a> {
    pub(crate) fn parse(data: &'a [u8]) -> FbxResult<Self> {
        if data.starts_with(b"; FBX 6") {
            return Err(FbxError::AsciiLegacy);
        }
        let mut lexer = Lexer::new(data);
        let records = parse_records(&mut lexer, false)?;
        Ok(Self { data, records })
    }

    fn top(&self, key: &[u8]) -> Option<&Record> {
        self.records
            .iter()
            .find(|record| record.key(self.data) == key)
    }

    /// Every `Model` object, in file order.
    pub(crate) fn models(&self) -> FbxResult<Vec<ModelRecord<'_>>> {
        let objects = self.top(b"Objects").ok_or(FbxError::NoObjects)?;
        let mut models = Vec::new();
        for record in &objects.children {
            if record.key(self.data) != b"Model" {
                continue;
            }
            // FBX 6.x names an object by string alone, with no id to address it
            // by.
            let id = record
                .int_value(self.data, 0)
                .ok_or(FbxError::AsciiLegacy)?;
            let full = record.str_value(self.data, 1).unwrap_or_default();
            let name = full.strip_prefix(b"Model::").unwrap_or(full);
            let class = record.str_value(self.data, 2).unwrap_or_default();
            models.push(ModelRecord {
                id,
                name,
                class,
                record,
            });
        }
        Ok(models)
    }

    /// `(child, parent)` id pairs of every object-object connection.
    pub(crate) fn connections(&self) -> Vec<(i64, i64)> {
        let Some(connections) = self.top(b"Connections") else {
            return Vec::new();
        };
        connections
            .children
            .iter()
            .filter(|record| {
                record.key(self.data) == b"C" && record.str_value(self.data, 0) == Some(b"OO")
            })
            .filter_map(|record| {
                Some((
                    record.int_value(self.data, 1)?,
                    record.int_value(self.data, 2)?,
                ))
            })
            .collect()
    }

    /// `model`'s property named `name`, if it has one.
    pub(crate) fn property<'r>(
        &'r self,
        model: &ModelRecord<'r>,
        name: &[u8],
    ) -> Option<PropertyRecord<'r>> {
        let record = find_property(self.data, model.record, name)?;
        Some(PropertyRecord {
            flags: record.str_value(self.data, 3).unwrap_or_default(),
            string: record.str_value(self.data, 4),
        })
    }
}

fn find_property<'r>(data: &[u8], model: &'r Record, name: &[u8]) -> Option<&'r Record> {
    model
        .child(data, b"Properties70")?
        .children
        .iter()
        .find(|record| record.key(data) == b"P" && record.str_value(data, 0) == Some(name))
}

/// One edit to apply: set (`Some`) or remove (`None`) a user string property.
pub(crate) struct AsciiEdit<'e> {
    pub(crate) model: i64,
    pub(crate) name: &'e str,
    pub(crate) value: Option<&'e str>,
    pub(crate) flags: &'e str,
}

/// Apply `edits`, returning the new file.
pub(crate) fn patch(document: &Document<'_>, edits: &[AsciiEdit<'_>]) -> FbxResult<Vec<u8>> {
    let data = document.data;
    let eol: &[u8] = if data.windows(2).any(|pair| pair == b"\r\n") {
        b"\r\n"
    } else {
        b"\n"
    };
    let models = document.models()?;
    // (start, end, replacement), applied in order.
    let mut splices: Vec<(usize, usize, Vec<u8>)> = Vec::new();
    for edit in edits {
        let model = models
            .iter()
            .find(|model| model.id == edit.model)
            .ok_or(FbxError::NoSuchObject(edit.model))?;
        let line = edit
            .value
            .map(|value| property_line(edit.name, edit.flags, value));
        let props70 = model.record.child(data, b"Properties70");
        let existing = find_property(data, model.record, edit.name.as_bytes());
        match (existing, line) {
            (Some(record), Some(line)) => {
                splices.push((record.key_start, record.header_end, line));
            }
            (Some(record), None) => {
                splices.push((
                    line_start(data, record.key_start),
                    line_end(data, record.header_end),
                    Vec::new(),
                ));
            }
            (None, None) => {}
            (None, Some(line)) => {
                match props70.and_then(|props| props.block.map(|block| (props, block))) {
                    Some((props, (_, close))) => {
                        let indent = match props.children.first() {
                            Some(first) => indentation(data, first.key_start).to_vec(),
                            None => [indentation(data, close), b"\t"].concat(),
                        };
                        let at = line_start(data, close);
                        splices.push((at, at, [indent.as_slice(), &line, eol].concat()));
                    }
                    None => {
                        // A model with no Properties70 block gets one, after its
                        // Version line the way the SDK orders them (or first).
                        let (open, _) = model.record.block.ok_or(FbxError::Malformed {
                            offset: model.record.key_start,
                        })?;
                        let indent = match model.record.children.first() {
                            Some(first) => indentation(data, first.key_start).to_vec(),
                            None => [indentation(data, model.record.key_start), b"\t"].concat(),
                        };
                        let after = model
                            .record
                            .child(data, b"Version")
                            .map_or(open + 1, |version| version.header_end);
                        let at = line_end(data, after);
                        let block = [
                            indent.as_slice(),
                            b"Properties70:  {",
                            eol,
                            &indent,
                            b"\t",
                            &line,
                            eol,
                            &indent,
                            b"}",
                            eol,
                        ]
                        .concat();
                        splices.push((at, at, block));
                    }
                }
            }
        }
    }
    splices.sort_by_key(|&(start, end, _)| (start, end));
    let mut out = Vec::with_capacity(data.len() + splices.iter().map(|s| s.2.len()).sum::<usize>());
    let mut pos = 0;
    for (start, end, replacement) in splices {
        if start < pos {
            // Two edits of one property: the earlier one wins.
            continue;
        }
        out.extend_from_slice(&data[pos..start]);
        out.extend_from_slice(&replacement);
        pos = end;
    }
    out.extend_from_slice(&data[pos..]);
    Ok(out)
}

/// `P: "name", "KString", "", "flags", "value"`, the way the SDK writes a user
/// string property.
fn property_line(name: &str, flags: &str, value: &str) -> Vec<u8> {
    format!(
        "P: \"{}\", \"KString\", \"\", \"{}\", \"{}\"",
        escape(name),
        escape(flags),
        escape(value)
    )
    .into_bytes()
}

/// Escape text for an ASCII FBX string: `"` becomes `&quot;` and `&` the JSON
/// escape `\u0026` (see the module docs — FBX has no escape for `&`, and this
/// crate only ever stores JSON, where `\u0026` *is* `&`). Raw line breaks, which a
/// JSON serializer never emits, become the FBX entities.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("&quot;"),
            '&' => out.push_str("\\u0026"),
            '\r' => out.push_str("&cr;"),
            '\n' => out.push_str("&lf;"),
            other => out.push(other),
        }
    }
    out
}

/// Decode the entities ufbx and the Autodesk SDK decode inside an ASCII string.
/// Any other `&` is literal.
pub(crate) fn unescape(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let mut pos = 0;
    while pos < raw.len() {
        let rest = &raw[pos..];
        let (replacement, skip) = if rest.starts_with(b"&quot;") {
            (b'"', 6)
        } else if rest.starts_with(b"&cr;") {
            (b'\r', 4)
        } else if rest.starts_with(b"&lf;") {
            (b'\n', 4)
        } else {
            (raw[pos], 1)
        };
        out.push(replacement);
        pos += skip;
    }
    out
}

fn line_start(data: &[u8], offset: usize) -> usize {
    data[..offset]
        .iter()
        .rposition(|&byte| byte == b'\n')
        .map_or(0, |newline| newline + 1)
}

/// Just past the newline that ends `offset`'s line (or the end of the file).
fn line_end(data: &[u8], offset: usize) -> usize {
    data[offset..]
        .iter()
        .position(|&byte| byte == b'\n')
        .map_or(data.len(), |newline| offset + newline + 1)
}

/// The whitespace that opens `offset`'s line.
fn indentation(data: &[u8], offset: usize) -> &[u8] {
    let start = line_start(data, offset);
    let width = data[start..]
        .iter()
        .take_while(|&&byte| byte == b' ' || byte == b'\t')
        .count();
    &data[start..start + width]
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = b"; FBX 7.7.0 project file\n\
Objects:  {\n\
\tModel: 42, \"Model::Body\", \"Mesh\" {\n\
\t\tVersion: 232\n\
\t\tProperties70:  {\n\
\t\t\tP: \"InheritType\", \"enum\", \"\", \"\",1\n\
\t\t}\n\
\t\tShading: Y\n\
\t}\n\
\tModel: 7, \"Model::Bare\", \"Null\" {\n\
\t\tVersion: 232\n\
\t}\n\
}\n\
Connections:  {\n\
\t;Model::Body, Model::RootNode\n\
\tC: \"OO\",42,0\n\
\tC: \"OO\",7,42\n\
}\n";

    fn set(model: i64, value: &str) -> AsciiEdit<'_> {
        AsciiEdit {
            model,
            name: "ReviewComments",
            value: Some(value),
            flags: "U",
        }
    }

    fn read(data: &[u8], model: i64) -> Option<Vec<u8>> {
        let document = Document::parse(data).expect("parses");
        let models = document.models().expect("has objects");
        let model = models.iter().find(|m| m.id == model)?;
        document
            .property(model, b"ReviewComments")
            .and_then(|property| property.string.map(unescape))
    }

    #[test]
    fn models_connections_and_names_are_read() {
        let document = Document::parse(SAMPLE).expect("parses");
        let models = document.models().expect("has objects");
        let names: Vec<_> = models
            .iter()
            .map(|model| (model.id, model.name, model.class))
            .collect();
        assert_eq!(
            names,
            [
                (42, &b"Body"[..], &b"Mesh"[..]),
                (7, &b"Bare"[..], &b"Null"[..])
            ]
        );
        assert_eq!(document.connections(), [(42, 0), (7, 42)]);
    }

    #[test]
    fn a_property_is_inserted_replaced_and_removed_in_place() {
        let document = Document::parse(SAMPLE).expect("parses");
        let added = patch(&document, &[set(42, r#"{"a":"x\"y & z"}"#)]).expect("patches");
        assert_eq!(
            read(&added, 42).as_deref(),
            Some(&br#"{"a":"x\"y \u0026 z"}"#[..])
        );
        // Only the inserted line differs.
        let text = String::from_utf8(added.clone()).expect("utf8");
        assert!(text.contains(
            "\t\t\tP: \"ReviewComments\", \"KString\", \"\", \"U\", \"{&quot;a&quot;:&quot;x\\&quot;y \\u0026 z&quot;}\"\n\t\t}"
        ));

        let document = Document::parse(&added).expect("parses");
        let replaced = patch(&document, &[set(42, "{}")]).expect("patches");
        assert_eq!(read(&replaced, 42).as_deref(), Some(&b"{}"[..]));

        let document = Document::parse(&replaced).expect("parses");
        let removed = patch(
            &document,
            &[AsciiEdit {
                model: 42,
                name: "ReviewComments",
                value: None,
                flags: "U",
            }],
        )
        .expect("patches");
        assert_eq!(removed, SAMPLE, "removing restores the original bytes");
    }

    #[test]
    fn a_model_without_properties_gets_a_block() {
        let document = Document::parse(SAMPLE).expect("parses");
        let added = patch(&document, &[set(7, "{}")]).expect("patches");
        assert_eq!(read(&added, 7).as_deref(), Some(&b"{}"[..]));
        let text = String::from_utf8(added).expect("utf8");
        assert!(text.contains(
            "\t\tVersion: 232\n\t\tProperties70:  {\n\t\t\tP: \"ReviewComments\", \"KString\", \"\", \"U\", \"{}\"\n\t\t}\n\t}"
        ));
    }

    #[test]
    fn an_unknown_object_is_refused() {
        let document = Document::parse(SAMPLE).expect("parses");
        assert_eq!(
            patch(&document, &[set(99, "{}")]).err(),
            Some(FbxError::NoSuchObject(99))
        );
    }

    #[test]
    fn escaping_round_trips_through_the_entities() {
        let text = "a\"b&c\nd";
        assert_eq!(unescape(escape(text).as_bytes()), b"a\"b\\u0026c\nd");
    }

    #[test]
    fn a_legacy_file_is_refused() {
        assert_eq!(
            Document::parse(b"; FBX 6.1.0 project file\n").err(),
            Some(FbxError::AsciiLegacy)
        );
    }
}
