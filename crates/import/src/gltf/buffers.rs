//! Loading a glTF file's buffers: the GLB binary chunk, a `data:` URI, or a
//! file beside the `.gltf`.
//!
//! The `gltf` crate does this only behind its `import` feature, which also
//! decodes every image with the `image` crate. Nothing here needs a decoded
//! image, so the three sources are resolved by hand.

use std::path::Path;

use gltf::buffer::Source;

/// Every buffer's bytes, by index, or a sentence saying which one could not be
/// read. `directory` is where a relative URI resolves; `None` (a file given as
/// bytes) leaves only the binary chunk and `data:` URIs readable.
pub(super) fn load(
    document: &gltf::Document,
    blob: Option<Vec<u8>>,
    directory: Option<&Path>,
) -> Result<Vec<Vec<u8>>, String> {
    let mut blob = blob;
    let mut buffers = Vec::with_capacity(document.buffers().len());
    for buffer in document.buffers() {
        let mut bytes = match buffer.source() {
            Source::Bin => blob.take().ok_or_else(|| {
                format!(
                    "buffer {} names a binary chunk the file does not have",
                    buffer.index()
                )
            })?,
            Source::Uri(uri) if uri.starts_with("data:") => data_uri(uri)
                .ok_or_else(|| format!("buffer {}'s data URI is not base64", buffer.index()))?,
            Source::Uri(uri) => {
                let directory = directory.ok_or_else(|| {
                    format!(
                        "buffer {} is a separate file ({uri}), and only the .gltf was given",
                        buffer.index()
                    )
                })?;
                let path = directory.join(percent_decoded(uri));
                std::fs::read(&path).map_err(|error| {
                    format!("buffer {} ({}): {error}", buffer.index(), path.display())
                })?
            }
        };
        if bytes.len() < buffer.length() {
            return Err(format!(
                "buffer {} holds {} bytes of the {} it declares",
                buffer.index(),
                bytes.len(),
                buffer.length()
            ));
        }
        // The GLB chunk is padded to four bytes; the buffer is what it declares.
        bytes.truncate(buffer.length());
        buffers.push(bytes);
    }
    Ok(buffers)
}

/// The bytes of a `data:[<type>];base64,<payload>` URI.
pub(super) fn data_uri(uri: &str) -> Option<Vec<u8>> {
    let (header, payload) = uri.strip_prefix("data:")?.split_once(',')?;
    if !header.ends_with(";base64") {
        return None;
    }
    base64(payload)
}

/// Standard base64 (RFC 4648), padding optional, whitespace not allowed.
fn base64(text: &str) -> Option<Vec<u8>> {
    fn value(byte: u8) -> Option<u32> {
        Some(match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    }
    let text = text.trim_end_matches('=').as_bytes();
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    for chunk in text.chunks(4) {
        if chunk.len() == 1 {
            return None;
        }
        let mut bits = 0u32;
        for (at, &byte) in chunk.iter().enumerate() {
            bits |= value(byte)? << (18 - 6 * at);
        }
        let bytes = bits.to_be_bytes();
        out.extend_from_slice(&bytes[1..chunk.len()]);
    }
    Some(out)
}

/// A URI path with its `%XX` escapes decoded. An escape that is not two hex
/// digits is kept as written.
fn percent_decoded(uri: &str) -> String {
    let bytes = uri.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%'
            && let Some(hex) = bytes.get(at + 1..at + 3)
            && let Ok(text) = std::str::from_utf8(hex)
            && let Ok(byte) = u8::from_str_radix(text, 16)
        {
            out.push(byte);
            at += 3;
            continue;
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_decodes_with_and_without_padding() {
        assert_eq!(base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(base64("aGVsbG8").unwrap(), b"hello");
        assert_eq!(base64("AAEC/w==").unwrap(), [0, 1, 2, 255]);
        assert_eq!(base64("").unwrap(), b"");
        assert!(base64("a").is_none());
        assert!(base64("a b=").is_none());
    }

    #[test]
    fn a_data_uri_must_say_base64() {
        assert_eq!(
            data_uri("data:application/octet-stream;base64,AAEC").unwrap(),
            [0, 1, 2]
        );
        assert!(data_uri("data:text/plain,hello").is_none());
    }

    #[test]
    fn escapes_decode_and_odd_ones_stay() {
        assert_eq!(percent_decoded("my%20part.bin"), "my part.bin");
        assert_eq!(percent_decoded("100%.bin"), "100%.bin");
        assert_eq!(percent_decoded("%zz.bin"), "%zz.bin");
    }
}
