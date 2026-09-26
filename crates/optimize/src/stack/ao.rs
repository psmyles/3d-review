//! The vertex-color AO bake's settings.

use serde::{Deserialize, Serialize};

/// Ray count for the AO bake, as named steps rather than a raw slider — the
/// visual difference between adjacent counts is subtle, and named steps keep
/// presets comparable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AoQuality {
    Low,
    Medium,
    #[default]
    High,
    Ultra,
}

impl AoQuality {
    pub const ALL: [AoQuality; 4] = [
        AoQuality::Low,
        AoQuality::Medium,
        AoQuality::High,
        AoQuality::Ultra,
    ];

    /// Hemisphere rays cast per vertex.
    pub fn rays(self) -> usize {
        match self {
            AoQuality::Low => 32,
            AoQuality::Medium => 64,
            AoQuality::High => 128,
            AoQuality::Ultra => 512,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AoQuality::Low => "Low (32 rays)",
            AoQuality::Medium => "Medium (64 rays)",
            AoQuality::High => "High (128 rays)",
            AoQuality::Ultra => "Ultra (512 rays)",
        }
    }
}

/// Which part of the RGBA vertex color the baked AO value is written to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AoTarget {
    /// `color.a = ao`; RGB untouched. Alpha is always written linear.
    #[default]
    Alpha,
    /// `color.rgb = ao` (grayscale); alpha untouched.
    Rgb,
    /// `color.rgb *= ao`, darkening whatever colors are authored; alpha untouched.
    MultiplyRgb,
    /// That one channel `= ao`; everything else untouched.
    Red,
    Green,
    Blue,
}

impl AoTarget {
    pub const ALL: [AoTarget; 6] = [
        AoTarget::Alpha,
        AoTarget::Rgb,
        AoTarget::MultiplyRgb,
        AoTarget::Red,
        AoTarget::Green,
        AoTarget::Blue,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AoTarget::Alpha => "Alpha channel",
            AoTarget::Rgb => "RGB (grayscale)",
            AoTarget::MultiplyRgb => "Multiply into RGB",
            AoTarget::Red => "Red channel",
            AoTarget::Green => "Green channel",
            AoTarget::Blue => "Blue channel",
        }
    }

    /// Whether this target writes into the RGB components — the only ones the
    /// sRGB-encode option applies to (alpha is always linear).
    pub fn is_rgb(self) -> bool {
        !matches!(self, AoTarget::Alpha)
    }
}

/// Settings for the [`OpKind::BakeAo`](crate::stack::OpKind::BakeAo) operation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BakeAoParams {
    pub quality: AoQuality,
    /// Farthest a surface can be and still occlude, in world meters;
    /// `0.0` = unlimited (classic whole-scene occlusion).
    pub max_distance: f32,
    /// Power applied to visibility (`1.0` = physical), matching the viewport
    /// Ambient Occlusion panel's Intensity semantics.
    pub intensity: f32,
    pub target: AoTarget,
    /// sRGB-encode the written value. RGB-family targets only; alpha is
    /// always linear.
    pub srgb: bool,
}

impl Default for BakeAoParams {
    fn default() -> Self {
        Self {
            quality: AoQuality::default(),
            max_distance: 0.0,
            intensity: 1.0,
            target: AoTarget::default(),
            srgb: false,
        }
    }
}
