//! Tiny PSDs built in memory, one per shape of document the decoder has to either
//! draw correctly or refuse cleanly.
//!
//! Every refusal here used to be memory corruption or a crash inside the vendored
//! psd_sdk: a grayscale+alpha document with a display-info resource wrote through
//! NULL, a 1-bit document over-read zero-byte planes, a corrupt RLE run overflowed
//! its buffer. The contract now is a decoded image or a `PsdError` — never a crash —
//! and each test pins one path to it.

use review_psd::{PsdError, decode_psd};

const RAW: u16 = 0;
const RLE: u16 = 1;
const GRAYSCALE: u16 = 1;
const RGB: u16 = 3;

/// One image resource block: `8BIM`, the id, an empty (even-padded) name, the data.
fn resource(id: u16, data: &[u8]) -> Vec<u8> {
    let mut out = b"8BIM".to_vec();
    out.extend_from_slice(&id.to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    out
}

struct Psd {
    channels: u16,
    width: u32,
    height: u32,
    depth: u16,
    mode: u16,
    resources: Vec<u8>,
    compression: u16,
    data: Vec<u8>,
}

impl Psd {
    fn raw(channels: u16, width: u32, height: u32, depth: u16, mode: u16, data: Vec<u8>) -> Self {
        Self {
            channels,
            width,
            height,
            depth,
            mode,
            resources: Vec::new(),
            compression: RAW,
            data,
        }
    }

    fn bytes(&self) -> Vec<u8> {
        let mut out = b"8BPS".to_vec();
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&[0; 6]);
        out.extend_from_slice(&self.channels.to_be_bytes());
        out.extend_from_slice(&self.height.to_be_bytes());
        out.extend_from_slice(&self.width.to_be_bytes());
        out.extend_from_slice(&self.depth.to_be_bytes());
        out.extend_from_slice(&self.mode.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes()); // colour mode data
        out.extend_from_slice(&(self.resources.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.resources);
        out.extend_from_slice(&0u32.to_be_bytes()); // layer and mask info
        out.extend_from_slice(&self.compression.to_be_bytes());
        out.extend_from_slice(&self.data);
        out
    }
}

/// One RLE row: a literal run of `bytes`.
fn rle_literal(bytes: &[u8]) -> Vec<u8> {
    let mut out = vec![(bytes.len() - 1) as u8];
    out.extend_from_slice(bytes);
    out
}

#[test]
fn an_rgb_document_decodes_to_its_exact_pixels() {
    // 2x1, planar: R plane, G plane, B plane.
    let psd = Psd::raw(3, 2, 1, 8, RGB, vec![10, 20, 30, 40, 50, 60]);
    let image = decode_psd(&psd.bytes()).expect("decodes");
    assert_eq!((image.width, image.height), (2, 1));
    assert_eq!(image.rgba8, vec![10, 30, 50, 255, 20, 40, 60, 255]);
}

#[test]
fn a_sixteen_bit_document_keeps_the_high_byte() {
    let psd = Psd::raw(1, 1, 1, 16, GRAYSCALE, vec![0xAB, 0xCD]);
    let image = decode_psd(&psd.bytes()).expect("decodes");
    assert_eq!(image.rgba8, vec![0xAB, 0xAB, 0xAB, 255]);
}

#[test]
fn a_float_document_is_srgb_encoded_but_its_alpha_is_not() {
    // Linear 0.5 is sRGB 188; alpha is coverage and stays linear (128).
    let half = 0.5f32.to_be_bytes();
    let mut data = Vec::new();
    data.extend_from_slice(&half);
    data.extend_from_slice(&half);
    let psd = Psd::raw(2, 1, 1, 32, GRAYSCALE, data);
    let image = decode_psd(&psd.bytes()).expect("decodes");
    assert_eq!(image.rgba8, vec![188, 188, 188, 128]);
}

#[test]
fn grayscale_with_alpha_and_a_display_info_resource_decodes() {
    // Resource 1077 (DISPLAY_INFO) is what Photoshop writes for a document with extra
    // channels. psd_sdk's resource parser assumed RGB and computed `channels - 3`
    // alpha entries, which for two channels wrapped and wrote through NULL.
    let mut psd = Psd::raw(2, 2, 1, 8, GRAYSCALE, vec![7, 9, 200, 100]);
    psd.resources = resource(1077, &[0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 100, 0]);
    let image = decode_psd(&psd.bytes()).expect("decodes");
    assert_eq!(image.rgba8, vec![7, 7, 7, 200, 9, 9, 9, 100]);
}

#[test]
fn a_one_bit_bitmap_document_is_refused() {
    // psd_sdk sizes planes as `bits / 8` bytes a sample: zero for a 1-bit document.
    let psd = Psd::raw(1, 8, 1, 1, 0, vec![0xFF]);
    assert!(matches!(
        decode_psd(&psd.bytes()),
        Err(PsdError::Unsupported(_))
    ));
}

#[test]
fn an_unhandled_bit_depth_is_refused() {
    let psd = Psd::raw(3, 1, 1, 24, RGB, vec![0; 9]);
    assert!(matches!(
        decode_psd(&psd.bytes()),
        Err(PsdError::Unsupported(_))
    ));
}

#[test]
fn a_cmyk_document_is_refused_rather_than_drawn_as_rgb() {
    let psd = Psd::raw(4, 1, 1, 8, 4, vec![0; 4]);
    assert!(matches!(
        decode_psd(&psd.bytes()),
        Err(PsdError::Unsupported(_))
    ));
}

#[test]
fn impossible_channel_counts_are_refused() {
    for channels in [0u16, 2, 57, 300] {
        let psd = Psd::raw(channels, 1, 1, 8, RGB, vec![0; channels as usize]);
        assert!(
            matches!(decode_psd(&psd.bytes()), Err(PsdError::Unsupported(_))),
            "{channels} channels"
        );
    }
}

#[test]
fn an_rle_document_decodes() {
    let row = rle_literal(&[1, 2, 3]);
    let mut data = Vec::new();
    data.extend_from_slice(&(row.len() as u16).to_be_bytes());
    data.extend_from_slice(&row);
    let mut psd = Psd::raw(1, 3, 1, 8, GRAYSCALE, data);
    psd.compression = RLE;
    let image = decode_psd(&psd.bytes()).expect("decodes");
    assert_eq!(image.rgba8, vec![1, 1, 1, 255, 2, 2, 2, 255, 3, 3, 3, 255]);
}

#[test]
fn an_rle_run_past_the_row_is_refused_not_written() {
    // A replicate run of 128 bytes into a 2-pixel plane: the unpatched decoder
    // memset 126 bytes past the end of the buffer.
    let run = vec![0x81, 0x55];
    let mut data = Vec::new();
    data.extend_from_slice(&(run.len() as u16).to_be_bytes());
    data.extend_from_slice(&run);
    let mut psd = Psd::raw(1, 2, 1, 8, GRAYSCALE, data);
    psd.compression = RLE;
    assert!(matches!(
        decode_psd(&psd.bytes()),
        Err(PsdError::DecodeFailed)
    ));
}

#[test]
fn an_rle_literal_past_its_source_is_refused() {
    // A literal of 100 bytes whose row count says it is 2 bytes long.
    let mut data = Vec::new();
    data.extend_from_slice(&2u16.to_be_bytes());
    data.extend_from_slice(&[99, 1]);
    let mut psd = Psd::raw(1, 100, 1, 8, GRAYSCALE, data);
    psd.compression = RLE;
    assert!(matches!(
        decode_psd(&psd.bytes()),
        Err(PsdError::DecodeFailed)
    ));
}

#[test]
fn rle_row_counts_larger_than_the_file_are_refused() {
    // Row counts claiming far more payload than the file holds must not size an
    // allocation.
    let mut data = Vec::new();
    for _ in 0..4 {
        data.extend_from_slice(&u16::MAX.to_be_bytes());
    }
    let mut psd = Psd::raw(1, 4, 4, 8, GRAYSCALE, data);
    psd.compression = RLE;
    assert!(decode_psd(&psd.bytes()).is_err());
}

#[test]
fn a_truncated_raw_document_is_refused() {
    let psd = Psd::raw(3, 4, 4, 8, RGB, vec![0; 10]);
    assert!(matches!(decode_psd(&psd.bytes()), Err(PsdError::Truncated)));
}

#[test]
fn a_section_length_past_the_end_of_the_file_is_refused() {
    let mut bytes = Psd::raw(1, 1, 1, 8, GRAYSCALE, vec![0]).bytes();
    // The colour-mode data length, straight after the 26-byte header.
    bytes[26..30].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(decode_psd(&bytes).is_err());
}

#[test]
fn dimensions_past_the_ceiling_are_refused_before_decoding() {
    let psd = Psd::raw(1, 65_536, 65_536, 8, GRAYSCALE, Vec::new());
    assert!(matches!(
        decode_psd(&psd.bytes()),
        Err(PsdError::TooLarge { .. })
    ));
}
