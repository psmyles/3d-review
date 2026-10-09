//! The streaming binary scan: the same records `scan` reads, reached by seeking.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};

use super::binary::{self, ReadAt, Reader};
use super::{Collected, FbxError, FbxResult, Format, Scan};

/// A file read through a buffer, moving between positions with relative seeks so
/// the buffer survives a short hop forward — the common case, since the records
/// of a file are visited in order.
pub(crate) struct FileSource {
    reader: BufReader<File>,
    pos: u64,
}

impl FileSource {
    pub(crate) fn new(file: File) -> Self {
        Self {
            reader: BufReader::with_capacity(64 * 1024, file),
            pos: 0,
        }
    }
}

impl ReadAt for FileSource {
    fn read_exact_at(&mut self, pos: usize, buf: &mut [u8]) -> std::io::Result<()> {
        let target = pos as u64;
        if target != self.pos {
            match i64::try_from(target as i128 - self.pos as i128) {
                Ok(delta) => self.reader.seek_relative(delta)?,
                Err(_) => {
                    self.reader.seek(SeekFrom::Start(target))?;
                }
            }
        }
        self.reader.read_exact(buf)?;
        self.pos = target + buf.len() as u64;
        Ok(())
    }
}

/// Scan a binary FBX of `len` bytes from `source`.
pub(crate) fn scan_binary(
    source: &mut impl ReadAt,
    len: usize,
    version: u32,
    property: &str,
) -> FbxResult<Scan> {
    // The top level: find Objects and Connections by name, skipping the rest.
    let mut objects = None;
    let mut connections = None;
    let mut pos = binary::FIRST_RECORD;
    while let Some(record) = binary::read_record(source, pos, len, version)? {
        match record.name.as_slice() {
            b"Objects" => objects = Some((record.children_start, record.end)),
            b"Connections" => connections = Some((record.start, record.end)),
            _ => {}
        }
        pos = record.end;
    }
    let (children_start, objects_end) = objects.ok_or(FbxError::NoObjects)?;

    let mut collected = Collected::default();
    let mut pos = children_start;
    while pos < objects_end {
        let Some(record) = binary::read_record(source, pos, objects_end, version)? else {
            break;
        };
        if record.name == b"Model" {
            let bytes = read_record_bytes(source, record.start, record.end)?;
            let reader = Reader::window(&bytes, record.start, version);
            collected.model(&reader, &reader.record_at(record.start)?, property)?;
        }
        pos = record.end;
    }
    if let Some((start, end)) = connections {
        let bytes = read_record_bytes(source, start, end)?;
        let reader = Reader::window(&bytes, start, version);
        collected.connections(&reader, &reader.record_at(start)?)?;
    }
    Ok(collected.finish(Format::Binary { version }))
}

fn read_record_bytes(source: &mut impl ReadAt, start: usize, end: usize) -> FbxResult<Vec<u8>> {
    let mut bytes = vec![0u8; end - start];
    source
        .read_exact_at(start, &mut bytes)
        .map_err(|_| FbxError::Truncated { offset: start })?;
    Ok(bytes)
}
