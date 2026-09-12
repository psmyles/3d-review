//! CPU mip-chain generation (D7).
//!
//! sokol_gfx has no `GenerateMips` — there is no portable equivalent of D3D11's
//! "bind the texture as a render target and let the driver downsample it" — so the
//! chain is built here and handed to `sg_make_image` alongside the base level, in
//! one immutable upload ([`super::Texture::rgba8_mipped_or_white`]).
//!
//! Two things this has to get right, because both are visible:
//!
//! * **sRGB is averaged in linear light.** Hardware `GenerateMips` on an sRGB view
//!   decodes, averages and re-encodes; averaging the stored bytes instead makes
//!   every minified sRGB texture measurably too dark. The Tex viewport uploads raw
//!   (`srgb = false`) so its displayed texel stays the stored texel — and its mips
//!   average the stored bytes, which is what the hardware did for a UNORM view.
//! * **Alpha is never sRGB.** It is averaged as stored either way.
//!
//! The filter is a plain 2×2 box over the previous level, with the sample clamped so
//! an odd (or 1-texel) axis repeats its last row/column rather than reading past it.
//! D3D11's own non-power-of-two kernel is slightly wider than that; the difference is
//! a fraction of a code in a minified mip.

use review_model::color::linear_to_srgb;
use sokol::gfx as sg;

/// Levels in a full chain for a `width`×`height` texture: `floor(log2(max)) + 1`,
/// capped at what sokol can hold.
pub(crate) fn level_count(width: u32, height: u32) -> u32 {
    (32 - width.max(height).max(1).leading_zeros()).min(sg::MAX_MIPMAPS as u32)
}

/// Every level *below* level 0, tightly packed RGBA8, largest first.
///
/// Level 0 is deliberately not included: it is the caller's own buffer, and copying
/// it would double the peak cost of a 4K upload for nothing.
pub(crate) fn levels_below_base(rgba: &[u8], width: u32, height: u32, srgb: bool) -> Vec<Vec<u8>> {
    let levels = level_count(width, height);
    let mut chain: Vec<Vec<u8>> = Vec::with_capacity(levels.saturating_sub(1) as usize);
    let (mut src_width, mut src_height) = (width.max(1) as usize, height.max(1) as usize);
    let tables = srgb.then(Tables::new);

    for _ in 1..levels {
        let src: &[u8] = chain.last().map_or(rgba, Vec::as_slice);
        let dst_width = (src_width / 2).max(1);
        let dst_height = (src_height / 2).max(1);
        chain.push(halve(
            src,
            (src_width, src_height),
            (dst_width, dst_height),
            tables.as_ref(),
        ));
        (src_width, src_height) = (dst_width, dst_height);
    }
    chain
}

/// One 2×2 box-filter step. `tables` present means the RGB channels are sRGB-encoded
/// and are averaged in linear light.
fn halve(
    src: &[u8],
    (src_width, src_height): (usize, usize),
    (dst_width, dst_height): (usize, usize),
    tables: Option<&Tables>,
) -> Vec<u8> {
    let mut dst = vec![0u8; dst_width * dst_height * 4];
    for y in 0..dst_height {
        let rows = [
            (y * 2).min(src_height - 1) * src_width,
            (y * 2 + 1).min(src_height - 1) * src_width,
        ];
        for x in 0..dst_width {
            let columns = [(x * 2).min(src_width - 1), (x * 2 + 1).min(src_width - 1)];
            // The four source texels, as byte offsets into `src`.
            let taps = [
                (rows[0] + columns[0]) * 4,
                (rows[0] + columns[1]) * 4,
                (rows[1] + columns[0]) * 4,
                (rows[1] + columns[1]) * 4,
            ];
            let out = (y * dst_width + x) * 4;
            for channel in 0..3 {
                dst[out + channel] = match tables {
                    Some(tables) => {
                        let sum: f32 = taps
                            .iter()
                            .map(|&tap| tables.to_linear[src[tap + channel] as usize])
                            .sum();
                        tables.encode(sum * 0.25)
                    }
                    None => average(taps.map(|tap| src[tap + channel])),
                };
            }
            dst[out + 3] = average(taps.map(|tap| src[tap + 3]));
        }
    }
    dst
}

/// The mean of four bytes, rounded to nearest.
fn average(taps: [u8; 4]) -> u8 {
    let sum: u32 = taps.iter().map(|&value| u32::from(value)).sum();
    ((sum + 2) / 4) as u8
}

/// Entries in the linear→sRGB table. Its index is the linear value quantized to this
/// many steps, so the worst case is one code of error in the darkest few levels,
/// where sRGB's slope is steepest — under the mip's own 8-bit quantization, and the
/// alternative is a `powf` per channel per texel (a 4K chain is ~17 million of them).
const ENCODE_STEPS: usize = 4096;

/// The two sRGB conversion tables, built once per chain — 4352 `powf`s against the
/// millions a per-texel conversion would cost.
struct Tables {
    to_linear: [f32; 256],
    to_srgb: [u8; ENCODE_STEPS],
}

impl Tables {
    fn new() -> Self {
        Self {
            to_linear: std::array::from_fn(|code| srgb_to_linear(code as f32 / 255.0)),
            to_srgb: std::array::from_fn(|step| {
                let linear = step as f32 / (ENCODE_STEPS - 1) as f32;
                (linear_to_srgb(linear) * 255.0).round().clamp(0.0, 255.0) as u8
            }),
        }
    }

    /// Encode a linear value in 0..1 back to an sRGB byte.
    fn encode(&self, linear: f32) -> u8 {
        let last = (ENCODE_STEPS - 1) as f32;
        let step = (linear * last).round().clamp(0.0, last);
        self.to_srgb[step as usize]
    }
}

/// The IEC 61966-2-1 sRGB transfer function and its inverse — the same pair
/// `review.glsl`'s `srgb` block applies on the GPU.
fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_square_texture_has_a_full_chain() {
        assert_eq!(level_count(256, 256), 9);
        assert_eq!(level_count(1, 1), 1);
        // Non-square: the longest axis decides.
        assert_eq!(level_count(1024, 4), 11);
    }

    /// Every level halves (floor, never below 1) and stays tightly packed, so a
    /// truncated level shows up as a wrong length rather than as garbled rows.
    #[test]
    fn the_chain_halves_down_to_one_texel() {
        let base = vec![0u8; 8 * 4 * 4];
        let chain = levels_below_base(&base, 8, 4, false);
        let sizes: Vec<usize> = chain.iter().map(Vec::len).collect();
        assert_eq!(sizes, vec![4 * 2 * 4, 2 * 4, 4]);
    }

    /// A flat colour must survive minification unchanged — the case a wrong rounding
    /// or a wrong colour space shows up in immediately.
    #[test]
    fn a_flat_colour_is_preserved_in_both_spaces() {
        for srgb in [false, true] {
            let base: Vec<u8> = std::iter::repeat_n([64u8, 128, 200, 255], 4 * 4)
                .flatten()
                .collect();
            for level in levels_below_base(&base, 4, 4, srgb) {
                for texel in level.as_chunks::<4>().0 {
                    assert_eq!(texel, &[64, 128, 200, 255], "srgb={srgb}");
                }
            }
        }
    }

    /// Half black, half white, averaged in linear light: the result is the sRGB
    /// encoding of 0.5 (~188), not the byte average (~128). This is what the `srgb`
    /// flag is for, and getting it backwards is what makes minified sRGB textures
    /// too dark.
    #[test]
    fn srgb_averages_in_linear_light() {
        let base: Vec<u8> = [[0u8, 0, 0, 255], [255, 255, 255, 255]]
            .into_iter()
            .flatten()
            .collect();
        let linear = levels_below_base(&base, 2, 1, true);
        assert_eq!(linear[0][0], 188);
        let raw = levels_below_base(&base, 2, 1, false);
        assert_eq!(raw[0][0], 128);
        // Alpha is never sRGB-encoded, so it averages the same either way.
        assert_eq!(linear[0][3], 255);
    }
}
