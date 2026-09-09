//! Source-image decoding for the material texture slots (Phase 3).
//!
//! Decoding uses prebuilt decoders only: the Rust `image` crate for the
//! formats it covers (PNG/TGA/TIFF/HDR/BMP/GIF/PNM), **zune** for the JPEG fast path
//! (platform intrinsics + unsafe fast paths, measured faster than `image`'s decoder),
//! and the prebuilt psd_sdk FFI crate ([`review_psd`]) for layered **PSD** source art
//! (its merged composite). This replaces the old bundled-ImageMagick `magick.exe`
//! shell-out — no external binary is shipped anymore.
//!
//! Dispatch is by magic bytes first (robust to mislabeled extensions — common in
//! game-asset exports), with the file extension as a fallback for formats that carry
//! no signature (e.g. TGA). Only the `review_psd` path is `unsafe`/FFI, and it is
//! confined to that crate (invariant 9); the decode code here stays safe.
//!
//! [`decode_image`] runs on the app thread when the user assigns a slot — never in
//! the render `prepare` callback. The decoded RGBA8 pixels live behind an `Arc`
//! ([`DecodedImage`]) so the per-frame scene render shares them by refcount, and
//! the GPU upload is deduplicated by path (the material table's path-keyed cache).

use std::io::Cursor;
use std::path::Path;

/// The seven PBR texture slots a material carries, in the order the GPU bind group
/// and the shader expect them. The numeric index is the binding offset within
/// bind group 3 (slot `i` → texture binding `i + 1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureSlot {
    BaseColor,
    Normal,
    Roughness,
    Metallic,
    Ao,
    Emissive,
    Opacity,
}

/// Number of material texture slots (kept in lockstep with [`TextureSlot::ALL`]).
pub const TEXTURE_SLOT_COUNT: usize = 7;

impl TextureSlot {
    /// Every slot, in binding order.
    pub const ALL: [TextureSlot; TEXTURE_SLOT_COUNT] = [
        TextureSlot::BaseColor,
        TextureSlot::Normal,
        TextureSlot::Roughness,
        TextureSlot::Metallic,
        TextureSlot::Ao,
        TextureSlot::Emissive,
        TextureSlot::Opacity,
    ];

    /// Index into the per-material `textures` array / shader binding order.
    pub fn index(self) -> usize {
        match self {
            TextureSlot::BaseColor => 0,
            TextureSlot::Normal => 1,
            TextureSlot::Roughness => 2,
            TextureSlot::Metallic => 3,
            TextureSlot::Ao => 4,
            TextureSlot::Emissive => 5,
            TextureSlot::Opacity => 6,
        }
    }

    /// Resolve a slot from its [`TextureSlot::index`], or `None` if out of range.
    pub fn from_index(index: usize) -> Option<TextureSlot> {
        TextureSlot::ALL.get(index).copied()
    }

    /// Human-readable slot name (Inspector label).
    pub fn label(self) -> &'static str {
        match self {
            TextureSlot::BaseColor => "Base Color",
            TextureSlot::Normal => "Normal",
            TextureSlot::Roughness => "Roughness",
            TextureSlot::Metallic => "Metallic",
            TextureSlot::Ao => "AO",
            TextureSlot::Emissive => "Emissive",
            TextureSlot::Opacity => "Opacity",
        }
    }

    /// Whether the slot's texture is color data (sampled as `Rgba8UnormSrgb`).
    /// Base color + emissive carry sRGB-authored color; the rest (normal /
    /// roughness / metallic / AO / opacity) are linear data (`Rgba8Unorm`).
    pub fn is_srgb(self) -> bool {
        matches!(self, TextureSlot::BaseColor | TextureSlot::Emissive)
    }

    /// Whether the slot reads a single channel (scalar data: roughness / metallic
    /// / AO / opacity) rather than RGB(A) color/vector data.
    pub fn is_scalar(self) -> bool {
        matches!(
            self,
            TextureSlot::Roughness | TextureSlot::Metallic | TextureSlot::Ao | TextureSlot::Opacity
        )
    }
}

/// Which channel(s) of a (possibly packed) texture feed a material property. The
/// scalar slots read one channel (R/G/B/A). The color slots (base color / emissive)
/// read either the full `Rgb` or a single channel scaled by the material color
/// value. Normal always reads RGB (no selector). There is no RGBA option — opacity
/// comes from the dedicated Opacity slot. The numeric [`ChannelSelect::shader_index`]
/// is what the shader branches/swizzles on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelSelect {
    R,
    G,
    B,
    A,
    Rgb,
}

impl ChannelSelect {
    /// The channel choices a scalar slot offers (single channels only).
    pub const SCALAR: [ChannelSelect; 4] = [
        ChannelSelect::R,
        ChannelSelect::G,
        ChannelSelect::B,
        ChannelSelect::A,
    ];

    /// The channel choices a color slot (base color / emissive) offers: full RGB
    /// or any single channel scaled by the color value.
    pub const COLOR: [ChannelSelect; 5] = [
        ChannelSelect::Rgb,
        ChannelSelect::R,
        ChannelSelect::G,
        ChannelSelect::B,
        ChannelSelect::A,
    ];

    /// Index the shader branches/swizzles on: 0 R, 1 G, 2 B, 3 A for a single
    /// channel, and `4` for full `Rgb` (the color slots test `> 3.5` to take the
    /// RGB path; the scalar `select_channel` only ever sees 0..3).
    pub fn shader_index(self) -> f32 {
        match self {
            ChannelSelect::R => 0.0,
            ChannelSelect::G => 1.0,
            ChannelSelect::B => 2.0,
            ChannelSelect::A => 3.0,
            ChannelSelect::Rgb => 4.0,
        }
    }

    /// Short dropdown label.
    pub fn label(self) -> &'static str {
        match self {
            ChannelSelect::R => "R",
            ChannelSelect::G => "G",
            ChannelSelect::B => "B",
            ChannelSelect::A => "A",
            ChannelSelect::Rgb => "RGB",
        }
    }
}

/// Decoded RGBA8 pixels for one source image, shared by `Arc` so the per-frame
/// scene callback and the GPU upload reference the same buffer without copying.
///
/// The pixels are always RGBA8 (the renderer / Tex viewer sample one layout); the
/// `source_*` fields preserve the *original* file's channel count and per-channel
/// bit depth so the Tex viewport's stats panel can report them faithfully
/// (invariant 5) — they describe the file on disk, not this decoded buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// Tightly-packed RGBA8 rows, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
    /// Channel count of the source file (1 grey, 2 grey+alpha, 3 RGB, 4 RGBA).
    pub source_channels: u8,
    /// Bits per channel of the source file (e.g. 8, 16), before the RGBA8 decode.
    pub source_bit_depth: u8,
}

/// Decode `path` into RGBA8 pixels. Reads the file once, then dispatches by magic
/// bytes (with the extension as a fallback for signature-less formats like TGA):
/// PSD → the psd_sdk FFI crate ([`review_psd`]); JPEG → zune's fast path; everything
/// else → the `image` crate. Returns a human-readable error string on failure (the
/// caller warns + falls back to the slot's neutral 1×1 texture).
pub fn decode_image(path: &Path) -> Result<DecodedImage, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_default();

    // PSD: layered source art. psd_sdk reads the merged/composited image (present
    // when saved with "Maximize Compatibility"), the thing the viewer displays.
    if bytes.starts_with(b"8BPS") || extension == "psd" {
        return decode_psd(&bytes, path);
    }

    // JPEG: the fast path. If zune rejects it (e.g. a truncated/odd variant), fall
    // through to the `image` crate rather than failing outright.
    if (bytes.starts_with(&[0xFF, 0xD8, 0xFF]) || matches!(extension.as_str(), "jpg" | "jpeg"))
        && let Ok(image) = decode_jpeg_zune(&bytes)
    {
        return Ok(image);
    }

    // Everything else (PNG/TGA/TIFF/HDR/BMP/GIF/PNM): the `image` crate.
    dynamic_to_decoded(decode_via_image_crate(&bytes, &extension, path)?)
}

/// Flatten a decoded [`image::DynamicImage`] into a [`DecodedImage`], capturing the
/// source color type *before* the RGBA8 conversion so the Tex viewport can report
/// the file's real channel count + bit depth (invariant 5).
fn dynamic_to_decoded(image: image::DynamicImage) -> Result<DecodedImage, String> {
    let color = image.color();
    let source_channels = color.channel_count().max(1);
    let source_bit_depth = (color.bits_per_pixel() / source_channels as u16) as u8;

    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(DecodedImage {
        width,
        height,
        rgba: rgba.into_raw(),
        source_channels,
        source_bit_depth,
    })
}

/// Decode with the `image` crate, guessing the format from the content's magic
/// bytes rather than the extension — so a mislabeled file (a PNG saved as `.jpg`,
/// common in game-asset exports) reaches the right decoder. Signature-less formats
/// (TGA) that content-sniffing misses fall back to the `extension` hint.
fn decode_via_image_crate(
    bytes: &[u8],
    extension: &str,
    path: &Path,
) -> Result<image::DynamicImage, String> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    if reader.format().is_none()
        && let Some(format) = image::ImageFormat::from_extension(extension)
    {
        reader.set_format(format);
    }
    // Bound the decode: a malformed header claiming absurd dimensions must fail
    // cleanly here, not balloon RAM (the default limits cap allocation but not
    // dimensions). 30000 px matches the PSD path's ceiling.
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(30_000);
    limits.max_image_height = Some(30_000);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|error| format!("decode {}: {error}", path.display()))
}

/// Decode JPEG bytes with zune's speed-first options, normalized to RGBA8. JPEG is
/// always 8-bit; the source channel count (1 grey / 3 RGB) is captured before the
/// RGBA conversion for the stats panel.
fn decode_jpeg_zune(bytes: &[u8]) -> Result<DecodedImage, String> {
    use zune_core::bytestream::ZCursor;
    use zune_core::colorspace::ColorSpace;
    use zune_core::options::DecoderOptions;
    use zune_image::image::Image;

    // Speed is a project goal: enable platform intrinsics + unsafe fast paths.
    let options = DecoderOptions::new_fast();
    let mut image = Image::read(ZCursor::new(bytes), options)
        .map_err(|error| format!("zune decode jpeg: {error}"))?;

    // Source channel count for the stats panel, before we normalize to RGBA.
    let source_channels = image.colorspace().num_components().clamp(1, 255) as u8;

    image
        .convert_color(ColorSpace::RGBA)
        .map_err(|error| format!("zune convert jpeg to rgba: {error}"))?;

    let (width, height) = image.dimensions();
    let frame = image
        .frames_ref()
        .first()
        .ok_or_else(|| "zune jpeg has no frames".to_string())?;
    let rgba = frame.flatten::<u8>();

    Ok(DecodedImage {
        width: width as u32,
        height: height as u32,
        rgba,
        source_channels,
        source_bit_depth: 8,
    })
}

/// Decode a PSD's merged/composited image via the prebuilt psd_sdk FFI crate. The
/// `unsafe`/FFI is fully contained in [`review_psd`] (invariant 9); this only maps
/// its result into a [`DecodedImage`], reporting the source channel count + bit
/// depth from the PSD header.
fn decode_psd(bytes: &[u8], path: &Path) -> Result<DecodedImage, String> {
    let psd = review_psd::decode_psd(bytes)
        .map_err(|error| format!("decode {}: {error}", path.display()))?;
    Ok(DecodedImage {
        width: psd.width,
        height: psd.height,
        rgba: psd.rgba8,
        source_channels: psd.channels.clamp(1, 255) as u8,
        source_bit_depth: psd.bits_per_channel.clamp(1, 255) as u8,
    })
}

/// Best-effort channel routing for a freshly-assigned slot, guessed from the
/// filename suffix (e.g. `rock_ORM.png` → AO reads R, Roughness G, Metallic B).
/// The Inspector always lets the user override the result. Color / normal /
/// emissive slots use RGB; an unrecognised name routes a scalar slot to R.
pub fn suggested_channel(path: &Path, slot: TextureSlot) -> ChannelSelect {
    if !slot.is_scalar() {
        // Base color / normal / emissive sample RGB directly.
        return ChannelSelect::Rgb;
    }

    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    // The packed token is the trailing `_XYZ` group (e.g. `_ORM`, `_RMA`).
    if let Some(token) = stem.rsplit(['_', '-', '.']).next()
        && let Some(channel) = packed_channel(token, slot)
    {
        return channel;
    }

    ChannelSelect::R
}

/// Map a packed-suffix token (`ORM`, `RMA`, `MRAO`, …) to the channel feeding
/// `slot`, or `None` when the token isn't a recognised packing. Each character is
/// a property (`O`/`A` ambient occlusion, `R` roughness, `M` metallic) and its
/// position is the channel (0 → R, 1 → G, 2 → B, 3 → A).
fn packed_channel(token: &str, slot: TextureSlot) -> Option<ChannelSelect> {
    let upper = token.to_ascii_uppercase();
    // Only treat 3–4 letter all-property tokens as packings, so a plain
    // `..._color` or `..._normal` suffix is never misread.
    if !(3..=4).contains(&upper.len())
        || !upper
            .bytes()
            .all(|b| matches!(b, b'O' | b'A' | b'R' | b'M'))
    {
        return None;
    }

    let want = match slot {
        TextureSlot::Ao => b"OA".as_slice(),
        TextureSlot::Roughness => b"R".as_slice(),
        TextureSlot::Metallic => b"M".as_slice(),
        _ => return None,
    };
    let position = upper.bytes().position(|b| want.contains(&b))?;
    Some(match position {
        0 => ChannelSelect::R,
        1 => ChannelSelect::G,
        2 => ChannelSelect::B,
        _ => ChannelSelect::A,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orm_suffix_routes_each_property_to_its_channel() {
        let path = Path::new("rock_ORM.png");
        assert_eq!(suggested_channel(path, TextureSlot::Ao), ChannelSelect::R);
        assert_eq!(
            suggested_channel(path, TextureSlot::Roughness),
            ChannelSelect::G
        );
        assert_eq!(
            suggested_channel(path, TextureSlot::Metallic),
            ChannelSelect::B
        );
    }

    #[test]
    fn rma_and_mrao_orderings_resolve() {
        assert_eq!(
            suggested_channel(Path::new("t_RMA.tga"), TextureSlot::Roughness),
            ChannelSelect::R
        );
        assert_eq!(
            suggested_channel(Path::new("t_MRAO.tga"), TextureSlot::Ao),
            ChannelSelect::B
        );
    }

    #[test]
    fn unpacked_scalar_defaults_to_r_and_color_to_rgb() {
        assert_eq!(
            suggested_channel(Path::new("wood_roughness.png"), TextureSlot::Roughness),
            ChannelSelect::R
        );
        assert_eq!(
            suggested_channel(Path::new("wood_basecolor.png"), TextureSlot::BaseColor),
            ChannelSelect::Rgb
        );
    }

    /// A JPEG routes through the zune fast path: exercise `decode_image` end-to-end
    /// on an encoded JPEG and confirm it comes back as a full RGBA8 buffer with the
    /// source channel count reported (3 for RGB). JPEG is lossy, so this asserts
    /// structure (dimensions / buffer size / channels), not exact pixel values.
    #[test]
    fn jpeg_decodes_via_zune_path() {
        let mut src = image::RgbImage::new(4, 2);
        for (x, y, pixel) in src.enumerate_pixels_mut() {
            *pixel = image::Rgb([(x * 40) as u8, (y * 80) as u8, 60]);
        }
        let mut buf = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(src)
            .write_to(&mut buf, image::ImageFormat::Jpeg)
            .expect("encode jpeg fixture");

        let path = std::env::temp_dir().join(format!("review_zune_{}.jpg", std::process::id()));
        std::fs::write(&path, buf.into_inner()).expect("write temp jpeg");
        let decoded = decode_image(&path);
        let _ = std::fs::remove_file(&path);

        let decoded = decoded.expect("zune should decode the jpeg");
        assert_eq!((decoded.width, decoded.height), (4, 2));
        assert_eq!(decoded.rgba.len(), 4 * 2 * 4);
        assert_eq!(decoded.source_channels, 3);
        assert_eq!(decoded.source_bit_depth, 8);
    }

    /// The prebuilt psd_sdk FFI decodes a real layered PSD's merged composite into a
    /// tightly-packed RGBA8 buffer. Uses the committed fixture; skips gracefully if
    /// it's absent so the suite still passes in a trimmed checkout.
    #[test]
    fn psd_fixture_decodes_to_rgba8() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/test_textures/T_Sides_D.psd");
        if !path.exists() {
            return;
        }
        let decoded = decode_image(&path).expect("psd_sdk should decode the merged composite");
        assert!(decoded.width > 0 && decoded.height > 0);
        assert_eq!(
            decoded.rgba.len() as u32,
            decoded.width * decoded.height * 4
        );
        assert!(decoded.source_bit_depth >= 8);
    }
}
