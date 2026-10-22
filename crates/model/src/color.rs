//! The sRGB transfer function and the OKLab conversions, in the one place every
//! user can reach.
//!
//! `optimize` sRGB-encodes the AO it bakes into a vertex color, and `render`
//! averages an sRGB mip in linear light so the result matches what the hardware
//! produced. Those two crates share no dependency but this one, and a transfer
//! function is scalar math — no host types — so it belongs here rather than
//! being written twice with the constant spelled differently each time.
//!
//! `render`'s generated palettes (the Unique material mode, the animation
//! curves, the UV islands) pick their colors in OKLCh, OKLab's polar form, so
//! every hue comes out at the same *perceived* lightness. An HSV wheel at one
//! value does not: its blue reads far darker than its yellow.

/// Encode a linear value as sRGB.
pub fn linear_to_srgb(value: f32) -> f32 {
    if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

/// Decode an sRGB value to linear. The exact inverse of [`linear_to_srgb`].
pub fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.040_449_936 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear sRGB to OKLab `[L, a, b]` (Björn Ottosson's matrices). `L` is the
/// perceived lightness, 0 for black and 1 for white.
pub fn linear_srgb_to_oklab([r, g, b]: [f32; 3]) -> [f32; 3] {
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// OKLab `[L, a, b]` to linear sRGB, unclamped: a color outside the sRGB gamut
/// comes back with a channel below 0 or above 1. The exact inverse of
/// [`linear_srgb_to_oklab`].
pub fn oklab_to_linear_srgb([lightness, a, b]: [f32; 3]) -> [f32; 3] {
    let l = (lightness + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m = (lightness - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s = (lightness - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
}

/// An OKLCh color as linear sRGB: `lightness` and `chroma` in OKLab units,
/// `hue` in turns (it wraps, so a golden-ratio step can feed it directly).
///
/// Where the color lies outside the sRGB gamut the *chroma* gives way, never
/// the lightness or the hue: clamping the channels instead would shift both,
/// and an equal lightness is the point of picking a palette in OKLCh.
pub fn oklch_to_linear_srgb(lightness: f32, chroma: f32, hue: f32) -> [f32; 3] {
    let angle = hue.rem_euclid(1.0) * std::f32::consts::TAU;
    let at =
        |chroma: f32| oklab_to_linear_srgb([lightness, chroma * angle.cos(), chroma * angle.sin()]);
    let in_gamut = |rgb: [f32; 3]| rgb.iter().all(|&c| (-1e-5..=1.0 + 1e-5).contains(&c));
    let mut rgb = at(chroma);
    if !in_gamut(rgb) {
        // Bisect for the largest chroma the gamut holds at this lightness and hue.
        let (mut inside, mut outside) = (0.0, chroma);
        for _ in 0..24 {
            let mid = 0.5 * (inside + outside);
            if in_gamut(at(mid)) {
                inside = mid;
            } else {
                outside = mid;
            }
        }
        rgb = at(inside);
    }
    rgb.map(|c| c.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oklab_round_trips() {
        for rgb in [[0.2, 0.5, 0.9], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.7; 3]] {
            let back = oklab_to_linear_srgb(linear_srgb_to_oklab(rgb));
            for (want, got) in rgb.iter().zip(back) {
                assert!((want - got).abs() < 1e-4, "{rgb:?} -> {back:?}");
            }
        }
        // White is L 1 with no chroma.
        let [l, a, b] = linear_srgb_to_oklab([1.0; 3]);
        assert!((l - 1.0).abs() < 1e-4 && a.abs() < 1e-4 && b.abs() < 1e-4);
    }

    /// Every hue at one OKLCh lightness reads equally light, including where the
    /// requested chroma falls outside the gamut and has to give way.
    #[test]
    fn oklch_holds_lightness_across_the_wheel() {
        for (lightness, chroma) in [(0.75, 0.12), (0.8, 0.2), (0.45, 0.3)] {
            for step in 0..72 {
                let hue = step as f32 / 72.0;
                let rgb = oklch_to_linear_srgb(lightness, chroma, hue);
                assert!(rgb.iter().all(|c| (0.0..=1.0).contains(c)), "{rgb:?}");
                let [l, ..] = linear_srgb_to_oklab(rgb);
                assert!(
                    (l - lightness).abs() < 2e-3,
                    "L {lightness} C {chroma} hue {hue}: got L {l}"
                );
            }
        }
    }

    #[test]
    fn the_transfer_function_round_trips() {
        for step in 0..=20 {
            let linear = step as f32 / 20.0;
            let round_tripped = srgb_to_linear(linear_to_srgb(linear));
            assert!(
                (round_tripped - linear).abs() < 1e-5,
                "{linear} -> {round_tripped}"
            );
        }
    }

    #[test]
    fn the_two_halves_meet_at_the_knee() {
        // The linear segment and the power segment have to agree where they
        // join, or a gradient shows a step at the darkest few values.
        let knee = 0.003_130_8;
        let below = linear_to_srgb(knee - 1e-6);
        let above = linear_to_srgb(knee + 1e-6);
        assert!((above - below).abs() < 1e-4, "{below} vs {above}");
    }

    #[test]
    fn black_and_white_are_fixed_points() {
        assert_eq!(linear_to_srgb(0.0), 0.0);
        assert!((linear_to_srgb(1.0) - 1.0).abs() < 1e-6);
        assert_eq!(srgb_to_linear(0.0), 0.0);
        assert!((srgb_to_linear(1.0) - 1.0).abs() < 1e-6);
    }
}
