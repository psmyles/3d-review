//! The option-window values **Remember settings** carries into the next session.
//!
//! What is remembered is exactly what the tools' option windows edit — every
//! colour, length, scope, mode and level in them, plus the IBL / AO / tone-mapping
//! / anti-aliasing switches those windows carry — and nothing a toolbar toggle
//! owns: whether the wireframe or the normals are *on* is a question about the
//! model in front of you, not a preference.
//!
//! This module only turns [`UiState`] into `key=value` text and back; `app` owns
//! the file (invariant 2). The two directions are one table, [`ENTRIES`], so a
//! value cannot be written under one name and read under another. Every read is
//! forgiving: an unknown key, an unparseable value or an out-of-range number is
//! skipped or clamped, never an error, because a settings file that has been
//! hand-edited or written by a newer build must not stop this one starting.

use egui::Color32;
use review_render::{
    BufferView, CheckerTexture, EnvironmentMap, GtaoQuality, MaterialMode, MsaaSamples,
    TonemapOperator, VertexColorMode, ViewportBackground,
};

use super::{BoundsScope, UiState, range};

/// One remembered value: its key in the file, how to write it, and how to read it
/// back. A read that returns `None` leaves the state untouched.
struct Entry {
    key: &'static str,
    save: fn(&UiState) -> String,
    load: fn(&mut UiState, &str) -> Option<()>,
}

/// An enum's stable file identifiers, one per variant.
///
/// The match is exhaustive, so a variant added upstream is a compile error here
/// rather than a value that silently never persists; the identifiers are this
/// file's own rather than the enums' `label()`s, so renaming a label on screen
/// cannot orphan what a user has saved.
macro_rules! enum_codec {
    ($encode:ident, $decode:ident, $ty:ty { $($variant:path => $id:literal),+ $(,)? }) => {
        fn $encode(value: $ty) -> String {
            match value {
                $($variant => $id,)+
            }
            .to_owned()
        }

        fn $decode(text: &str) -> Option<$ty> {
            [$($variant),+].into_iter().find(|variant| $encode(*variant) == text)
        }
    };
}

enum_codec!(checker_id, checker_from, CheckerTexture {
    CheckerTexture::Greyscale => "greyscale",
    CheckerTexture::Color => "color",
});

enum_codec!(vertex_color_id, vertex_color_from, VertexColorMode {
    VertexColorMode::Rgb => "rgb",
    VertexColorMode::Alpha => "alpha",
    VertexColorMode::RgbAlpha => "rgb_alpha",
});

enum_codec!(material_mode_id, material_mode_from, MaterialMode {
    MaterialMode::Source => "source",
    MaterialMode::Standard => "standard",
    MaterialMode::Unique => "unique",
});

enum_codec!(buffer_view_id, buffer_view_from, BufferView {
    BufferView::BaseColor => "base_color",
    BufferView::WorldNormal => "world_normal",
    BufferView::NormalMap => "normal_map",
    BufferView::GeometricNormal => "geometric_normal",
    BufferView::Tangent => "tangent",
    BufferView::Roughness => "roughness",
    BufferView::Metallic => "metallic",
    BufferView::AmbientOcclusion => "ambient_occlusion",
    BufferView::Emission => "emission",
    BufferView::Opacity => "opacity",
    BufferView::Uv => "uv",
});

enum_codec!(bounds_scope_id, bounds_scope_from, BoundsScope {
    BoundsScope::AllMeshes => "all_meshes",
    BoundsScope::OnlySelection => "only_selection",
    BoundsScope::VisibleOnly => "visible_only",
});

enum_codec!(msaa_id, msaa_from, MsaaSamples {
    MsaaSamples::Off => "off",
    MsaaSamples::X2 => "2x",
    MsaaSamples::X4 => "4x",
    MsaaSamples::X8 => "8x",
    MsaaSamples::X16 => "16x",
});

enum_codec!(background_id, background_from, ViewportBackground {
    ViewportBackground::Black => "black",
    ViewportBackground::Grey25 => "grey_25",
    ViewportBackground::Grey50 => "grey_50",
    ViewportBackground::Grey75 => "grey_75",
    ViewportBackground::White => "white",
    ViewportBackground::Gradient => "gradient",
});

enum_codec!(environment_id, environment_from, EnvironmentMap {
    EnvironmentMap::Hdr01 => "hdr_01",
    EnvironmentMap::Hdr02 => "hdr_02",
    EnvironmentMap::Hdr03 => "hdr_03",
    EnvironmentMap::Hdr04 => "hdr_04",
    EnvironmentMap::Hdr05 => "hdr_05",
    EnvironmentMap::Hdr06 => "hdr_06",
});

enum_codec!(gtao_quality_id, gtao_quality_from, GtaoQuality {
    GtaoQuality::Low => "low",
    GtaoQuality::Medium => "medium",
    GtaoQuality::High => "high",
});

enum_codec!(tonemap_id, tonemap_from, TonemapOperator {
    TonemapOperator::PbrNeutral => "pbr_neutral",
    TonemapOperator::Linear => "linear",
    TonemapOperator::Reinhard => "reinhard",
    TonemapOperator::Aces => "aces",
    TonemapOperator::Agx => "agx",
});

/// Store a value read back from the file, or leave the slot alone when it could
/// not be read.
fn set<T>(slot: &mut T, value: Option<T>) -> Option<()> {
    *slot = value?;
    Some(())
}

fn color_text(color: Color32) -> String {
    color.to_hex()
}

fn color_from(text: &str) -> Option<Color32> {
    Color32::from_hex(text).ok()
}

fn bool_text(value: bool) -> String {
    value.to_string()
}

fn bool_from(text: &str) -> Option<bool> {
    text.parse().ok()
}

fn f32_text(value: f32) -> String {
    value.to_string()
}

/// A finite number, clamped into the range its slider enforces — a hand-edited
/// `radius=1e9` lands on the slider's end rather than past it.
fn f32_in(text: &str, min: f32, max: f32) -> Option<f32> {
    let value: f32 = text.parse().ok()?;
    value.is_finite().then(|| value.clamp(min, max))
}

fn u32_in(text: &str, min: u32, max: u32) -> Option<u32> {
    let value: u32 = text.parse().ok()?;
    Some(value.clamp(min, max))
}

/// Every remembered value, grouped by the option window that edits it and in
/// that window's order. UV-set choices are left out on purpose: they index the
/// model's own UV sets and every load resets them.
const ENTRIES: &[Entry] = &[
    // Wireframe
    Entry {
        key: "wireframe.color",
        save: |s| color_text(s.wireframe.color),
        load: |s, v| set(&mut s.wireframe.color, color_from(v)),
    },
    Entry {
        key: "wireframe.width",
        save: |s| f32_text(s.wireframe.width),
        load: |s, v| {
            set(
                &mut s.wireframe.width,
                f32_in(v, range::WIREFRAME_WIDTH_MIN, range::WIREFRAME_WIDTH_MAX),
            )
        },
    },
    // Material Mode
    Entry {
        key: "material_mode.mode",
        save: |s| material_mode_id(s.debug.material_mode),
        load: |s, v| set(&mut s.debug.material_mode, material_mode_from(v)),
    },
    // Buffers
    Entry {
        key: "buffers.view",
        save: |s| buffer_view_id(s.debug.buffer_view),
        load: |s, v| set(&mut s.debug.buffer_view, buffer_view_from(v)),
    },
    // Bounding Box
    Entry {
        key: "bounding_box.color",
        save: |s| color_text(s.bounding_box.color),
        load: |s, v| set(&mut s.bounding_box.color, color_from(v)),
    },
    Entry {
        key: "bounding_box.scope",
        save: |s| bounds_scope_id(s.bounding_box.scope),
        load: |s, v| set(&mut s.bounding_box.scope, bounds_scope_from(v)),
    },
    // UV Checker
    Entry {
        key: "uv_checker.texture",
        save: |s| checker_id(s.uv_checker.texture),
        load: |s, v| set(&mut s.uv_checker.texture, checker_from(v)),
    },
    Entry {
        key: "uv_checker.tiling",
        save: |s| s.uv_checker.tiling.to_string(),
        load: |s, v| {
            set(
                &mut s.uv_checker.tiling,
                u32_in(v, range::CHECKER_TILING_MIN, range::CHECKER_TILING_MAX),
            )
        },
    },
    // Face Normals
    Entry {
        key: "face_normals.length",
        save: |s| f32_text(s.face_normals.length),
        load: |s, v| {
            set(
                &mut s.face_normals.length,
                f32_in(v, range::NORMAL_LENGTH_MIN, range::NORMAL_LENGTH_MAX),
            )
        },
    },
    Entry {
        key: "face_normals.color",
        save: |s| color_text(s.face_normals.color),
        load: |s, v| set(&mut s.face_normals.color, color_from(v)),
    },
    // Vertex Normals
    Entry {
        key: "vertex_normals.length",
        save: |s| f32_text(s.vertex_normals.length),
        load: |s, v| {
            set(
                &mut s.vertex_normals.length,
                f32_in(v, range::NORMAL_LENGTH_MIN, range::NORMAL_LENGTH_MAX),
            )
        },
    },
    Entry {
        key: "vertex_normals.color",
        save: |s| color_text(s.vertex_normals.color),
        load: |s, v| set(&mut s.vertex_normals.color, color_from(v)),
    },
    // UV Seams
    Entry {
        key: "uv_seams.color",
        save: |s| color_text(s.uv_seams.color),
        load: |s, v| set(&mut s.uv_seams.color, color_from(v)),
    },
    // Skeleton
    Entry {
        key: "skeleton.scale",
        save: |s| f32_text(s.skeleton.scale),
        load: |s, v| {
            set(
                &mut s.skeleton.scale,
                f32_in(v, range::SKELETON_SCALE_MIN, range::SKELETON_SCALE_MAX),
            )
        },
    },
    Entry {
        key: "skeleton.color",
        save: |s| color_text(s.skeleton.color),
        load: |s, v| set(&mut s.skeleton.color, color_from(v)),
    },
    // Vertex Colors
    Entry {
        key: "vertex_colors.mode",
        save: |s| vertex_color_id(s.vertex_colors.mode),
        load: |s, v| set(&mut s.vertex_colors.mode, vertex_color_from(v)),
    },
    // Anti Aliasing
    Entry {
        key: "anti_aliasing.enabled",
        save: |s| bool_text(s.anti_aliasing.enabled),
        load: |s, v| set(&mut s.anti_aliasing.enabled, bool_from(v)),
    },
    Entry {
        key: "anti_aliasing.msaa",
        save: |s| msaa_id(s.anti_aliasing.msaa),
        load: |s, v| set(&mut s.anti_aliasing.msaa, msaa_from(v)),
    },
    // Background
    Entry {
        key: "background.preset",
        save: |s| background_id(s.viewport_background),
        load: |s, v| set(&mut s.viewport_background, background_from(v)),
    },
    // Environment
    Entry {
        key: "environment.ibl_enabled",
        save: |s| bool_text(s.environment.ibl_enabled),
        load: |s, v| set(&mut s.environment.ibl_enabled, bool_from(v)),
    },
    Entry {
        key: "environment.show_background",
        save: |s| bool_text(s.environment.show_background),
        load: |s, v| set(&mut s.environment.show_background, bool_from(v)),
    },
    Entry {
        key: "environment.map",
        save: |s| environment_id(s.environment.map),
        load: |s, v| set(&mut s.environment.map, environment_from(v)),
    },
    Entry {
        key: "environment.intensity",
        save: |s| f32_text(s.environment.intensity),
        load: |s, v| {
            set(
                &mut s.environment.intensity,
                f32_in(v, range::ENV_INTENSITY_MIN, range::ENV_INTENSITY_MAX),
            )
        },
    },
    Entry {
        key: "environment.rotation",
        save: |s| f32_text(s.environment.rotation_degrees),
        load: |s, v| {
            set(
                &mut s.environment.rotation_degrees,
                f32_in(v, range::ENV_ROTATION_MIN, range::ENV_ROTATION_MAX),
            )
        },
    },
    // Ambient Occlusion
    Entry {
        key: "ambient_occlusion.enabled",
        save: |s| bool_text(s.gtao.enabled),
        load: |s, v| set(&mut s.gtao.enabled, bool_from(v)),
    },
    Entry {
        key: "ambient_occlusion.radius",
        save: |s| f32_text(s.gtao.radius),
        load: |s, v| {
            set(
                &mut s.gtao.radius,
                f32_in(v, range::AO_RADIUS_MIN, range::AO_RADIUS_MAX),
            )
        },
    },
    Entry {
        key: "ambient_occlusion.intensity",
        save: |s| f32_text(s.gtao.intensity),
        load: |s, v| {
            set(
                &mut s.gtao.intensity,
                f32_in(v, range::AO_INTENSITY_MIN, range::AO_INTENSITY_MAX),
            )
        },
    },
    Entry {
        key: "ambient_occlusion.thickness",
        save: |s| f32_text(s.gtao.thickness),
        load: |s, v| {
            set(
                &mut s.gtao.thickness,
                f32_in(v, range::AO_THICKNESS_MIN, range::AO_THICKNESS_MAX),
            )
        },
    },
    Entry {
        key: "ambient_occlusion.quality",
        save: |s| gtao_quality_id(s.gtao.quality),
        load: |s, v| set(&mut s.gtao.quality, gtao_quality_from(v)),
    },
    // Tonemapper
    Entry {
        key: "tonemapper.enabled",
        save: |s| bool_text(s.tonemap.enabled),
        load: |s, v| set(&mut s.tonemap.enabled, bool_from(v)),
    },
    Entry {
        key: "tonemapper.operator",
        save: |s| tonemap_id(s.tonemap.operator),
        load: |s, v| set(&mut s.tonemap.operator, tonemap_from(v)),
    },
];

impl UiState {
    /// Every remembered option-window value as a `(key, value)` pair, in a fixed
    /// order, for `app` to write.
    pub fn remembered_options(&self) -> Vec<(&'static str, String)> {
        ENTRIES
            .iter()
            .map(|entry| (entry.key, (entry.save)(self)))
            .collect()
    }

    /// Restore one remembered value read back from the settings file. Returns
    /// whether it was applied: `false` for a key this build does not remember or a
    /// value it cannot read, both of which leave the state as it was.
    pub fn restore_remembered_option(&mut self, key: &str, value: &str) -> bool {
        ENTRIES
            .iter()
            .find(|entry| entry.key == key)
            .and_then(|entry| (entry.load)(self, value.trim()))
            .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state with every remembered value moved off its default, so a round trip
    /// that dropped any one of them would show.
    fn edited_state() -> UiState {
        let mut state = UiState::default();
        state.wireframe.color = Color32::from_rgb(1, 2, 3);
        state.wireframe.width = 2.5;
        state.debug.material_mode = MaterialMode::Unique;
        state.debug.buffer_view = BufferView::Tangent;
        state.bounding_box.color = Color32::from_rgb(4, 5, 6);
        state.bounding_box.scope = BoundsScope::VisibleOnly;
        state.uv_checker.texture = CheckerTexture::Color;
        state.uv_checker.tiling = 9;
        state.face_normals.length = 0.05;
        state.face_normals.color = Color32::from_rgb(7, 8, 9);
        state.vertex_normals.length = 0.07;
        state.vertex_normals.color = Color32::from_rgb(10, 11, 12);
        state.uv_seams.color = Color32::from_rgb(13, 14, 15);
        state.skeleton.scale = 2.5;
        state.skeleton.color = Color32::from_rgb(16, 17, 18);
        state.vertex_colors.mode = VertexColorMode::RgbAlpha;
        state.anti_aliasing.enabled = false;
        state.anti_aliasing.msaa = MsaaSamples::X8;
        state.viewport_background = ViewportBackground::Gradient;
        state.environment.ibl_enabled = false;
        state.environment.show_background = true;
        state.environment.map = EnvironmentMap::Hdr05;
        state.environment.intensity = 1.75;
        state.environment.rotation_degrees = 123.5;
        state.gtao.enabled = false;
        state.gtao.radius = 2.0;
        state.gtao.intensity = 1.5;
        state.gtao.thickness = 0.25;
        state.gtao.quality = GtaoQuality::High;
        state.tonemap.enabled = false;
        state.tonemap.operator = TonemapOperator::Agx;
        state
    }

    #[test]
    fn every_remembered_value_survives_a_round_trip() {
        let saved = edited_state().remembered_options();
        let mut restored = UiState::default();
        for (key, value) in &saved {
            assert!(
                restored.restore_remembered_option(key, value),
                "`{key}={value}` was written but does not read back",
            );
        }
        assert_eq!(restored.remembered_options(), saved);
    }

    #[test]
    fn every_key_is_distinct() {
        let mut keys: Vec<&str> = ENTRIES.iter().map(|entry| entry.key).collect();
        keys.sort_unstable();
        let count = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), count, "two remembered values share a key");
    }

    /// A hand-edited or newer-build file must never fail the read: an unknown key
    /// or an unreadable value is skipped, and a number out of range is clamped to
    /// the slider's end.
    #[test]
    fn a_bad_value_is_skipped_and_an_out_of_range_one_clamped() {
        let mut state = UiState::default();
        let before = state.remembered_options();
        assert!(!state.restore_remembered_option("no_such.key", "1"));
        assert!(!state.restore_remembered_option("tonemapper.operator", "filmic"));
        assert!(!state.restore_remembered_option("wireframe.color", "#zzzzzz"));
        assert!(!state.restore_remembered_option("ambient_occlusion.radius", "NaN"));
        assert_eq!(state.remembered_options(), before);

        assert!(state.restore_remembered_option("ambient_occlusion.radius", "1e9"));
        assert_eq!(state.gtao.radius, range::AO_RADIUS_MAX);
        assert!(state.restore_remembered_option("uv_checker.tiling", "0"));
        assert_eq!(state.uv_checker.tiling, range::CHECKER_TILING_MIN);
    }
}
