//! Source-image decoding for the material texture slots.
//!
//! Every format goes through **fire-decode**, the decoder 3D Review shares with fire and bite
//! (fire-decode plan, step 4): its `texture()` setup, which reads a file as stored (no EXIF
//! rotation, no colour conversion, never shrunk) within 30000 px and 1 GiB. It routes by magic
//! bytes first and the extension second, so a PNG saved as `.psd` still reaches the PNG decoder,
//! and it brings PSD (through psd_sdk), HEIC / HEIF / AVIF (through libheif), DDS, OpenEXR,
//! Radiance HDR, JPEG XL, WebP and the rest. Its two native bridges are the sanctioned FFI sites
//! outside this workspace (invariant 9); the decode code here stays safe.
//!
//! The renderer samples one layout, RGBA8. fire-decode hands an integer source over as RGBA8
//! (16-bit rounded to nearest) and a float source (OpenEXR, Radiance HDR, a 32-bit PSD) as float,
//! because only the slot it lands in can say how to narrow it: a colour slot clamps it and
//! sRGB-encodes it, a data slot clamps it with no curve. A float source therefore carries both
//! narrowings, and each upload takes the one its slot needs ([`DecodedImage::rgba_for`]).
//!
//! [`decode_image`] runs on the texture manager's decode workers (a file imported into the scene
//! texture pool, or a reload after it changes on disk), or on a model's own scoped pool, never on
//! the main thread and never inside a frame's render. The decoded pixels live behind an `Arc`
//! ([`DecodedImage`]) so the per-frame scene render shares them by refcount, and the GPU upload is
//! deduplicated by path (the material table's path-keyed cache).
//!
//! **Mip chains** (D7) are fire-decode's builder (its `mips` feature, FD11), one per way an image
//! is uploaded ([`Upload`]), built once and kept with the image ([`DecodedImage::mips`]). The
//! workers build the ones they know an image needs: a model's textures, those of the slots it
//! binds; a reload, those its last decode had. Any other is built on first use. Each upload's
//! cache (material slot, Tex viewport, UV view) then takes the chain rather than building its own.

pub mod imported;

use std::path::Path;
use std::sync::OnceLock;

use fire_decode::PixelFormat;
use fire_decode::mips::MipOptions;

/// The seven PBR texture slots a material carries, in the order `review.glsl`
/// declares them. The numeric index is the offset from the first material texture
/// binding (slot `i` → texture binding `5 + i`, `base_color_tex` through
/// `opacity_tex`).
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
/// scene render and the GPU upload reference the same buffer without copying.
///
/// The pixels are always RGBA8 (the renderer / Tex viewer sample one layout); the
/// `source_*` fields preserve the *original* file's channel count, per-channel bit
/// depth and format so the Tex viewport's stats panel can report them faithfully
/// (invariant 5) — they describe the file on disk, not this decoded buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// Tightly-packed RGBA8 rows, `width * height * 4` bytes: the colour reading. An integer
    /// source's samples as stored (16-bit rounded to nearest); a float source's clamped to 0..1
    /// with its colour sRGB-encoded, so a slot sampled as sRGB gets the linear values back.
    pub rgba: Vec<u8>,
    /// A float source's data reading: clamped to 0..1 with no curve, for the slots sampled as
    /// linear data. `None` for an integer source, whose bytes are the same either way.
    pub rgba_linear: Option<Vec<u8>>,
    /// Channel count of the source file as the decode reads it (1 grey, 2
    /// grey+alpha, 3 RGB, 4 RGBA): what the file's pixels carry, not how they are
    /// stored. A CMYK JPEG is colour without transparency, so it is 3; a PSD's
    /// extra channels beyond its transparency are not drawn, so they do not count.
    /// `2 | 4` is what tells a consumer the image has an alpha channel.
    pub source_channels: u8,
    /// Bits per channel of the source file (e.g. 8, 16), before the RGBA8 decode.
    pub source_bit_depth: u8,
    /// The source file's format as the decoder found it from its content, not its name: `PNG`,
    /// `JPEG`, `OpenEXR`, or a DDS file's pixel format (`DDS BC7 sRGB`).
    pub source_format: &'static str,
    /// The mip chains this image has been uploaded with ([`DecodedImage::mips`]). A new image
    /// starts with none: `MipChains::default()`.
    pub mips: MipChains,
}

/// How a texture is uploaded: which of an image's readings, and how its mips average. Each is one
/// mip chain ([`DecodedImage::mips`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Upload {
    /// Sampled as sRGB colour (`Rgba8Srgb`): a colour slot, and the UV view. The colour reading,
    /// its mips averaged in linear light, as a GPU's `GenerateMips` does for an sRGB format.
    Colour,
    /// Sampled as data (`Rgba8`): a data slot. The data reading, its mips averaged as stored.
    Data,
    /// Shown as stored (`Rgba8`): the Tex viewport, where the texel on screen is the texel in the
    /// file. The colour reading, its mips averaged as stored. An integer source has one reading,
    /// so this is its [`Upload::Data`] chain.
    Shown,
}

impl Upload {
    /// What a material slot uploads: colour when it samples sRGB, data otherwise.
    pub fn for_slot(slot: TextureSlot) -> Upload {
        if slot.is_srgb() {
            Upload::Colour
        } else {
            Upload::Data
        }
    }

    /// Whether the upload samples as sRGB, which is also whether its mips average in linear light.
    pub fn srgb(self) -> bool {
        matches!(self, Upload::Colour)
    }
}

/// A [`DecodedImage`]'s mip chains, levels `1..` each, built once per [`Upload`] and kept for the
/// image's life: each upload cache that shows the image takes the chain rather than building its
/// own, and one evicted (the Tex viewport's, on leaving it) uploads again without a rebuild.
/// Derived from the pixels, so it takes no part in the image's equality.
#[derive(Debug, Clone, Default)]
pub struct MipChains([OnceLock<Vec<Vec<u8>>>; 3]);

impl PartialEq for MipChains {
    fn eq(&self, _: &MipChains) -> bool {
        true
    }
}

impl Eq for MipChains {}

impl DecodedImage {
    /// The RGBA8 bytes a texture sampled as `srgb` colour (or not, as data) uploads: the two
    /// differ only for a float source.
    pub fn rgba_for(&self, srgb: bool) -> &[u8] {
        match (&self.rgba_linear, srgb) {
            (Some(linear), false) => linear,
            _ => &self.rgba,
        }
    }

    /// The RGBA8 bytes `upload` uploads as level 0.
    pub fn pixels_for(&self, upload: Upload) -> &[u8] {
        match upload {
            Upload::Data => self.rgba_for(false),
            Upload::Colour | Upload::Shown => &self.rgba,
        }
    }

    /// Levels `1..` of `upload`'s mip chain (largest first, `rgba8_mipped_or_white`'s layout),
    /// built on first use and kept. The decode workers build the ones they know of
    /// ([`DecodedImage::prepare_mips`]); any other is built on the thread that asks, the render
    /// thread at its first upload.
    pub fn mips(&self, upload: Upload) -> &[Vec<u8>] {
        self.mips.0[self.chain_of(upload)].get_or_init(|| {
            fire_decode::mips::chain(
                self.pixels_for(upload),
                self.width,
                self.height,
                PixelFormat::Rgba8Unorm,
                &mip_options(upload.srgb()),
            )
        })
    }

    /// Build `uploads`' chains now, on the calling thread: a decode worker, for the uploads it
    /// knows the image will get.
    pub fn prepare_mips(&self, uploads: impl IntoIterator<Item = Upload>) {
        for upload in uploads {
            self.mips(upload);
        }
    }

    /// The uploads whose chains are built, for a reload to build them again on its worker.
    pub fn prepared_uploads(&self) -> Vec<Upload> {
        [Upload::Colour, Upload::Data, Upload::Shown]
            .into_iter()
            .filter(|&upload| self.mips.0[self.chain_of(upload)].get().is_some())
            .collect()
    }

    /// Where `upload`'s chain is kept. An integer source's two readings are one, so its shown and
    /// data chains are too.
    fn chain_of(&self, upload: Upload) -> usize {
        match upload {
            Upload::Colour => 0,
            Upload::Data => 1,
            Upload::Shown if self.rgba_linear.is_none() => 1,
            Upload::Shown => 2,
        }
    }

    /// This image halved until neither side exceeds `max_side` — the mip level a
    /// GPU would sample for it drawn that small — averaging RGB in linear light
    /// when it is `srgb` color, exactly as the mip chain does (fire-decode's
    /// `mips::fit`). Unchanged when it already fits. A float source's data reading
    /// is halved as data. The `source_*` fields still describe the file.
    pub fn fitted_within(self, max_side: u32, srgb: bool) -> DecodedImage {
        let (width, height) = (self.width, self.height);
        let fit = |pixels: &[u8], srgb: bool| {
            fire_decode::mips::fit(
                pixels,
                width,
                height,
                PixelFormat::Rgba8Unorm,
                max_side,
                &mip_options(srgb),
            )
        };
        match fit(&self.rgba, srgb) {
            Some((rgba, width, height)) => {
                let rgba_linear = self
                    .rgba_linear
                    .as_deref()
                    .and_then(|linear| fit(linear, false))
                    .map(|(linear, ..)| linear);
                DecodedImage {
                    width,
                    height,
                    rgba,
                    rgba_linear,
                    mips: MipChains::default(),
                    ..self
                }
            }
            None => self,
        }
    }
}

/// The options every chain here is built with: `srgb` as the upload samples, at most the levels
/// sokol holds, threads from fire-decode's budget.
pub(crate) fn mip_options(srgb: bool) -> MipOptions {
    let mut options = MipOptions::new(srgb);
    options.max_levels = crate::rhi::MAX_MIP_LEVELS;
    options
}

/// The formats a texture can be read from, lower-case, as the import dialog and drag-and-drop
/// name them: everything this build of fire-decode decodes (FD9). A dropped file not on it opens
/// as a model.
pub fn texture_extensions() -> &'static [&'static str] {
    fire_decode::supported_extensions()
}

/// Whether the calling thread is inside a texture decode, or one of its helper threads: for the
/// viewer's panic hook, which aborts on every other panic ([`fire_decode::in_decode`]).
pub fn in_decode() -> bool {
    fire_decode::in_decode()
}

/// How many textures to decode at once: half the cores, at least one (bite's `default_jobs`
/// rule; fire-decode plan, FD8). Each decode's own passes draw on fire-decode's process-wide
/// thread budget, which divides the cores between the decodes running; this bounds how many run,
/// and so how many files and pixel buffers are held at once.
pub fn decode_workers() -> usize {
    (std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get) / 2).max(1)
}

/// Whether `ext` (no leading dot, any case) is in [`texture_extensions`].
pub fn is_texture_extension(ext: &str) -> bool {
    fire_decode::is_supported_extension(ext)
}

/// Decode `path` into RGBA8 pixels through fire-decode's `texture()` setup. Reads the file once;
/// the extension is only the hint for a format with no signature (TGA). Returns a human-readable
/// error string on failure (the caller warns + falls back to the slot's neutral 1×1 texture).
pub fn decode_image(path: &Path) -> Result<DecodedImage, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_default();
    decode_bytes(&bytes, &extension, path)
}

/// [`decode_image`] over bytes already read, with `extension` (lower case) as the
/// hint for a signature-less format; `path` only names the file in an error.
fn decode_bytes(bytes: &[u8], extension: &str, path: &Path) -> Result<DecodedImage, String> {
    let ext = (!extension.is_empty()).then_some(extension);
    let image = fire_decode::decode(bytes, ext, &fire_decode::DecodeOptions::texture())
        .map_err(|error| format!("decode {}: {error}", path.display()))?;
    Ok(from_fire_decode(image))
}

/// fire-decode's buffer as RGBA8: as it is for an integer source, both narrowings for a float one.
fn from_fire_decode(image: fire_decode::DecodedImage) -> DecodedImage {
    let (rgba, rgba_linear) = match image.format {
        PixelFormat::Rgba8Unorm => (image.pixels, None),
        PixelFormat::Rgba16Float => {
            let floats: Vec<f32> = image
                .pixels
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| half::f16::from_bits(u16::from_ne_bytes(*b)).to_f32())
                .collect();
            narrow_float(&floats)
        }
        PixelFormat::Rgba32Float => {
            let floats: Vec<f32> = image
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_ne_bytes(*b))
                .collect();
            narrow_float(&floats)
        }
        // `texture()` narrows unorm to 8-bit itself (`Output::Rgba8OrFloat`); were a 16-bit
        // buffer ever to arrive, round it the same way rather than misread it.
        PixelFormat::Rgba16Unorm => (
            image
                .pixels
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| ((u16::from_ne_bytes(*b) as u32 * 255 + 32767) / 65535) as u8)
                .collect(),
            None,
        ),
    };
    DecodedImage {
        width: image.width,
        height: image.height,
        rgba,
        rgba_linear,
        source_channels: image.source.channels,
        source_bit_depth: image.source.bit_depth,
        source_format: image.source.name,
        mips: MipChains::default(),
    }
}

/// A float RGBA buffer's two RGBA8 readings: colour (clamped, colour channels sRGB-encoded) and
/// data (clamped, no curve). Alpha is coverage, not light, and is never encoded. The colour
/// reading is the one 3D Review's own PSD bridge gave a 32-bit document, to the byte: the same
/// IEC 61966-2-1 curve in `f32`, clamped first (NaN to 0), rounded by adding a half.
fn narrow_float(floats: &[f32]) -> (Vec<u8>, Option<Vec<u8>>) {
    fn clamped(v: f32) -> f32 {
        if v > 0.0 { v.min(1.0) } else { 0.0 } // NaN fails `> 0.0` and lands on 0
    }
    fn srgb(v: f32) -> f32 {
        if v <= 0.0031308 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }
    }
    let byte = |v: f32| (v * 255.0 + 0.5) as u8;
    let mut colour = Vec::with_capacity(floats.len());
    let mut data = Vec::with_capacity(floats.len());
    for px in floats.as_chunks::<4>().0 {
        for (c, &v) in px.iter().enumerate() {
            let v = clamped(v);
            data.push(byte(v));
            colour.push(byte(if c < 3 { srgb(v) } else { v }));
        }
    }
    (colour, Some(data))
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
    use std::io::Cursor;

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

    /// A JPEG decodes end to end through `decode_image` into a full RGBA8 buffer with
    /// the source channel count reported (3 for RGB). JPEG is lossy, so this asserts
    /// structure (dimensions / buffer size / channels), not exact pixel values.
    #[test]
    fn a_jpeg_decodes() {
        let mut src = image::RgbImage::new(4, 2);
        for (x, y, pixel) in src.enumerate_pixels_mut() {
            *pixel = image::Rgb([(x * 40) as u8, (y * 80) as u8, 60]);
        }
        let mut buf = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(src)
            .write_to(&mut buf, image::ImageFormat::Jpeg)
            .expect("encode jpeg fixture");

        let path = std::env::temp_dir().join(format!("review_jpeg_{}.jpg", std::process::id()));
        std::fs::write(&path, buf.into_inner()).expect("write temp jpeg");
        let decoded = decode_image(&path);
        let _ = std::fs::remove_file(&path);

        let decoded = decoded.expect("the jpeg decodes");
        assert_eq!((decoded.width, decoded.height), (4, 2));
        assert_eq!(decoded.rgba.len(), 4 * 2 * 4);
        assert_eq!(decoded.source_channels, 3);
        assert_eq!(decoded.source_bit_depth, 8);
        assert_eq!(decoded.source_format, "JPEG");
        assert!(decoded.rgba_linear.is_none());
    }

    /// Encode `image` as `format` in memory.
    fn encoded(image: image::DynamicImage, format: image::ImageFormat) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        image.write_to(&mut buf, format).expect("encode fixture");
        buf.into_inner()
    }

    /// A greyscale JPEG is one channel: the channel count comes from the file,
    /// not from the decoder's output, which is RGB whatever the file stored.
    #[test]
    fn a_greyscale_jpeg_reports_one_channel() {
        let grey = image::GrayImage::from_fn(4, 4, |x, y| image::Luma([(x * 40 + y * 10) as u8]));
        let bytes = encoded(
            image::DynamicImage::ImageLuma8(grey),
            image::ImageFormat::Jpeg,
        );
        let decoded =
            decode_bytes(&bytes, "jpg", Path::new("t.jpg")).expect("a greyscale jpeg decodes");
        assert_eq!(decoded.source_channels, 1);
        assert_eq!(decoded.rgba.len(), 4 * 4 * 4);
        // The pixels are still grey, expanded to RGBA.
        assert!(
            decoded
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .all(|[r, g, b, a]| r == g && g == b && *a == 255)
        );
    }

    /// BMP and GIF are offered by the texture pickers, so they must decode: both
    /// carry a signature.
    #[test]
    fn bmp_and_gif_decode() {
        let pixels =
            image::RgbImage::from_fn(3, 2, |x, y| image::Rgb([x as u8 * 80, y as u8 * 120, 7]));
        let source = image::DynamicImage::ImageRgb8(pixels.clone());

        let bmp = encoded(source.clone(), image::ImageFormat::Bmp);
        let decoded = decode_bytes(&bmp, "bmp", Path::new("t.bmp")).expect("bmp decodes");
        assert_eq!((decoded.width, decoded.height), (3, 2));
        assert_eq!(decoded.source_channels, 3);
        // BMP is lossless: every texel comes back as stored, made opaque.
        let expected: Vec<u8> = pixels
            .pixels()
            .flat_map(|image::Rgb([r, g, b])| [*r, *g, *b, 255])
            .collect();
        assert_eq!(decoded.rgba, expected);

        let gif = encoded(source, image::ImageFormat::Gif);
        let decoded = decode_bytes(&gif, "gif", Path::new("t.gif")).expect("gif decodes");
        assert_eq!((decoded.width, decoded.height), (3, 2));
        assert_eq!(decoded.rgba.len(), 3 * 2 * 4);
    }

    /// The extension never outranks the content: a PNG saved under a `.psd` name
    /// is a PNG, where routing by name sent it to a decoder that could only refuse.
    #[test]
    fn a_png_named_psd_decodes_as_a_png() {
        let pixels =
            image::RgbaImage::from_fn(2, 2, |x, y| image::Rgba([x as u8, y as u8, 3, 128]));
        let png = encoded(
            image::DynamicImage::ImageRgba8(pixels.clone()),
            image::ImageFormat::Png,
        );
        let decoded = decode_bytes(&png, "psd", Path::new("t.psd")).expect("decodes by content");
        assert_eq!(decoded.rgba, pixels.into_raw());
        assert_eq!(decoded.source_channels, 4);
        assert_eq!(
            decoded.source_format, "PNG",
            "the Format row names the content"
        );
    }

    /// A 16-bit source rounds to nearest, where 3D Review's own PSD path truncated
    /// (`>> 8`): `0xFF00` is 254.0 of 255 and stays 254, where truncation made it
    /// 255; `0x0081` is 0.502 and rounds up to 1, where truncation made it 0.
    #[test]
    fn sixteen_bit_rounds_to_nearest() {
        let pixels = image::ImageBuffer::<image::Rgba<u16>, _>::from_raw(
            2,
            1,
            vec![0xFF00, 0x0081, 0xFFFF, 0xFFFF, 0, 0x0080, 0x8000, 0xFFFF],
        )
        .unwrap();
        let png = encoded(
            image::DynamicImage::ImageRgba16(pixels),
            image::ImageFormat::Png,
        );
        let decoded = decode_bytes(&png, "png", Path::new("t.png")).expect("decodes");
        assert_eq!(decoded.rgba, [254, 1, 255, 255, 0, 0, 128, 255]);
        assert_eq!(decoded.source_bit_depth, 16);
        assert!(decoded.rgba_linear.is_none());
    }

    /// A float source carries both readings: colour clamps and sRGB-encodes, data
    /// clamps with no curve, and alpha is never encoded. Linear 0.5 is sRGB 188.
    #[test]
    fn a_float_source_narrows_per_slot() {
        let hdr = encoded(
            image::DynamicImage::ImageRgb32F(
                image::ImageBuffer::from_raw(2, 1, vec![0.5f32, 0.0, 4.0, 0.2140, 1.0, 0.0])
                    .unwrap(),
            ),
            image::ImageFormat::Hdr,
        );
        let decoded = decode_bytes(&hdr, "hdr", Path::new("t.hdr")).expect("decodes");
        assert_eq!(decoded.source_format, "Radiance HDR");
        let colour = decoded.rgba_for(true);
        let data = decoded.rgba_for(false);
        // RGBE stores a shared exponent, so allow a code either way.
        let near = |a: &[u8], b: &[u8]| a.iter().zip(b).all(|(x, y)| x.abs_diff(*y) <= 1);
        assert!(near(&colour[..4], &[188, 0, 255, 255]), "{colour:?}");
        assert!(near(&data[..4], &[128, 0, 255, 255]), "{data:?}");
        assert!(near(&colour[4..], &[128, 255, 0, 255]), "{colour:?}");
        assert!(near(&data[4..], &[55, 255, 0, 255]), "{data:?}");
        assert_eq!(decoded.rgba_for(true), decoded.rgba.as_slice());
    }

    /// The float narrowing matches the PSD bridge 3D Review used to carry, to the byte.
    #[test]
    fn the_colour_reading_is_the_old_psd_bridges() {
        let old = |v: f32| -> u8 {
            let v = if v > 0.0 { v.min(1.0) } else { 0.0 };
            let e = if v <= 0.0031308 {
                v * 12.92
            } else {
                1.055 * v.powf(1.0 / 2.4) - 0.055
            };
            (e * 255.0 + 0.5) as u8
        };
        let samples: Vec<f32> = (0..=4096).map(|i| i as f32 / 4000.0 - 0.01).collect();
        let floats: Vec<f32> = samples.iter().flat_map(|&v| [v, v, v, 1.0]).collect();
        let (colour, _) = narrow_float(&floats);
        for (i, &v) in samples.iter().enumerate() {
            assert_eq!(colour[i * 4], old(v), "{v}");
        }
        assert_eq!(
            narrow_float(&[f32::NAN, 2.0, -1.0, 0.5]).0,
            [0, 255, 0, 128]
        );
    }

    /// The import dialog and drag-and-drop accept what fire-decode decodes: DDS,
    /// OpenEXR and HEIC too, which the old fixed list of nine left out.
    #[test]
    fn the_texture_extensions_are_fire_decodes() {
        for ext in [
            "png", "jpg", "tga", "psd", "dds", "exr", "hdr", "heic", "avif", "PNG",
        ] {
            assert!(is_texture_extension(ext), ".{ext}");
        }
        assert!(!is_texture_extension("fbx"));
        assert!(texture_extensions().contains(&"dds"));
    }

    /// The psd_sdk FFI decodes a real layered PSD's merged composite into a
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
        assert_eq!(decoded.source_format, "PSD");
        assert!(decoded.width > 0 && decoded.height > 0);
        assert_eq!(
            decoded.rgba.len() as u32,
            decoded.width * decoded.height * 4
        );
        assert!(decoded.source_bit_depth >= 8);
    }
}
