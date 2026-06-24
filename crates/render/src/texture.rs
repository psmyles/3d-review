//! Source-image decoding for the material texture slots (Phase 3).
//!
//! Decoding uses prebuilt systems only (CLAUDE.md / the materials plan): the Rust
//! `image` crate for the formats it covers (PNG/JPG/TGA/TIFF/BMP/GIF), and a
//! **bundled ImageMagick `magick.exe` CLI** for the rest (PSD, multi-layer TIFF,
//! …) by streaming uncompressed PAM to stdout (`magick <in> pam:-`) — which the
//! `pnm` feature of `image` then decodes. No custom decoder, no `unsafe`, no FFI.
//!
//! [`decode_image`] runs on the app thread when the user assigns a slot — never in
//! the render `prepare` callback. The decoded RGBA8 pixels live behind an `Arc`
//! ([`DecodedImage`]) so the per-frame scene callback shares them by refcount, and
//! the GPU upload (still inside `prepare`) is deduplicated by path
//! ([`crate::material::MaterialTable`]).

use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Decode `path` into RGBA8 pixels. Dispatches on the file extension: formats the
/// `image` crate covers go straight through; the rest (PSD, …) shell out to the
/// bundled `magick.exe`, which streams uncompressed PAM the `pnm` feature decodes.
/// Returns a human-readable error string on failure (the caller warns + falls back
/// to the slot's neutral 1×1 texture).
pub fn decode_image(path: &Path) -> Result<DecodedImage, String> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_default();

    let image = if image_crate_handles(&extension) {
        image::open(path).map_err(|error| format!("decode {}: {error}", path.display()))?
    } else {
        decode_via_magick(path)?
    };

    // Capture the source color type *before* the RGBA8 flatten, so the Tex
    // viewport can report the file's real channel count + bit depth.
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

/// Whether the `image` crate (with this workspace's enabled features) decodes a
/// given extension directly. Everything else routes through `magick`.
fn image_crate_handles(extension: &str) -> bool {
    matches!(
        extension,
        "png" | "jpg" | "jpeg" | "tga" | "tif" | "tiff" | "bmp" | "gif" | "pnm" | "pam" | "ppm"
    )
}

/// Shell out to the bundled `magick.exe`, converting `path` to uncompressed PAM on
/// stdout, and decode that with the `image` crate's `pnm` reader. PAM is chosen
/// over PNG/MIFF deliberately (the materials plan): uncompressed + lossless so the
/// write is near-instant even for 4K images, yet still read by a prebuilt crate.
fn decode_via_magick(path: &Path) -> Result<image::DynamicImage, String> {
    let magick = locate_magick()
        .ok_or_else(|| "ImageMagick `magick` not found (bundle it beside the exe)".to_string())?;

    // `magick <in> pam:-` writes a PAM stream to stdout. `[0]` would pick the first
    // layer of a multi-layer file, but the bare path lets ImageMagick flatten — the
    // common case for a single-image PSD/TIFF.
    let output = Command::new(&magick)
        .arg(path)
        .arg("pam:-")
        .output()
        .map_err(|error| format!("run magick: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "magick failed for {}: {}",
            path.display(),
            stderr.trim()
        ));
    }

    image::load_from_memory_with_format(&output.stdout, image::ImageFormat::Pnm)
        .map_err(|error| format!("decode magick PAM for {}: {error}", path.display()))
}

/// Locate the bundled `magick.exe`: first next to the running executable (where
/// the installer ships it), then fall back to `magick` on `PATH`. `None` only when
/// neither resolves (the caller then warns + falls back to the slot's neutral
/// texture).
fn locate_magick() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(if cfg!(windows) {
                "magick.exe"
            } else {
                "magick"
            });
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    // Defer to PATH resolution by the OS when launching the bare command name.
    Some(PathBuf::from("magick"))
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
    if let Some(token) = stem.rsplit(['_', '-', '.']).next() {
        if let Some(channel) = packed_channel(token, slot) {
            return channel;
        }
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
        TextureSlot::Ao => [b'O', b'A'].as_slice(),
        TextureSlot::Roughness => [b'R'].as_slice(),
        TextureSlot::Metallic => [b'M'].as_slice(),
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
}
