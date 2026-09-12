//! The sRGB transfer function, in the one place both users can reach.
//!
//! `optimize` sRGB-encodes the AO it bakes into a vertex color, and `render`
//! averages an sRGB mip in linear light so the result matches what the hardware
//! produced. Those two crates share no dependency but this one, and a transfer
//! function is scalar math — no host types — so it belongs here rather than
//! being written twice with the constant spelled differently each time.

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

#[cfg(test)]
mod tests {
    use super::*;

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
