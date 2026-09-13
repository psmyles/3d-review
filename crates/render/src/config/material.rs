//! What the mesh is shaded with, and the debug materials that replace it.

/// How the 2D UV viewport draws the model's UV layout. Mutually exclusive (the
/// toolbar's UV-shading group is a radio selection); the UV edges are always
/// drawn, the fill underneath them changes per mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UvShadingMode {
    /// UV edges over the reference grid, with no fill underneath (the default
    /// wire-only layout view).
    #[default]
    Wire,
    /// The UV islands filled with one solid shaded color, edges drawn on top.
    Shaded,
    /// Each UV island filled with its own unique color, edges drawn on top.
    Islands,
}

/// Which built-in checker texture the UV-checker view samples.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CheckerTexture {
    #[default]
    Greyscale,
    Color,
}

impl CheckerTexture {
    /// Short menu label.
    pub fn label(self) -> &'static str {
        match self {
            CheckerTexture::Greyscale => "Greyscale",
            CheckerTexture::Color => "Color",
        }
    }
}

/// How the vertex-color view interprets the mesh's vertex-color attribute.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VertexColorMode {
    /// Show the RGB channels only (alpha forced opaque).
    #[default]
    Rgb,
    /// Show the alpha channel as a 0..1 greyscale value.
    Alpha,
    /// Show RGB as color and alpha as surface opacity.
    RgbAlpha,
}

impl VertexColorMode {
    /// Short menu label.
    pub fn label(self) -> &'static str {
        match self {
            VertexColorMode::Rgb => "RGB channel",
            VertexColorMode::Alpha => "Alpha channel",
            VertexColorMode::RgbAlpha => "RGB+A channel",
        }
    }
}

/// Which material the filled faces display. The choices are mutually exclusive
/// (the toolbar's material group is a radio selection). [`Source`] is the
/// model's imported material; the others replace it for inspection and apply
/// in every filled-face mode (unlit / shaded).
///
/// [`Source`]: ActiveMaterial::Source
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ActiveMaterial {
    /// The material as imported from the model.
    #[default]
    Source,
    /// A built-in UV checker pattern sampled through the model's UVs.
    UvChecker,
    /// The mesh's per-vertex color attribute.
    VertexColors,
    /// A heat map of how strongly the Outliner's selected bones influence each
    /// vertex — blue (none) through green to red (full). Flat and unlit, and it
    /// bypasses tone mapping, so the displayed color *is* the weight. Only
    /// offered for a model that carries skin weights.
    SkinWeights,
    /// A single material/geometry buffer shown flat for data inspection — base
    /// color, the world/geometric normal, roughness, metallic, AO, emission, … —
    /// selected by [`SceneDebugOptions::buffer_view`]. Bypasses lighting + tone
    /// mapping so the displayed pixel is the value itself.
    Buffers,
}

/// Which single material/geometry buffer the [`ActiveMaterial::Buffers`] view
/// displays. A data-inspection visualization: each variant renders one shading
/// input (or a raw authored map) flat to the screen, bypassing lighting + tone
/// mapping so the shown pixel *is* the value. The two "color" buffers (base color
/// / emission) are sRGB-encoded for display; the rest are written raw (a 0.5
/// scalar reads as mid-grey, a normal's `xyz` is remapped to `0..1` RGB).
///
/// Both a *final computed* normal (with the normal map applied) and the *raw
/// authored* normal map are offered, plus the geometric normal + tangent basis, so
/// a misbehaving normal map can be pinned down (handedness, green-channel
/// convention, missing tangents). The shader reads [`BufferView::shader_index`]
/// from `projection_params.z`; keep the two in lockstep (invariant 11).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BufferView {
    /// Final albedo: base-color factor × base-color map (channel-routed), sRGB-encoded.
    #[default]
    BaseColor,
    /// Final world-space shading normal (with the normal map applied), remapped to RGB.
    WorldNormal,
    /// The raw tangent-space normal-map texels, as authored (linear data, shown directly).
    NormalMap,
    /// The geometric (interpolated vertex) world normal, *without* the normal map.
    GeometricNormal,
    /// The vertex tangent (`xyz` remapped to RGB), tinted by its handedness sign.
    Tangent,
    /// Final roughness scalar (factor × map, channel-routed), as greyscale.
    Roughness,
    /// Final metallic scalar (factor × map, channel-routed), as greyscale.
    Metallic,
    /// Final ambient-occlusion scalar (map, channel-routed), as greyscale.
    AmbientOcclusion,
    /// Final emissive radiance (factor × map), sRGB-encoded.
    Emission,
    /// Final opacity (base-color alpha × opacity map), as greyscale.
    Opacity,
    /// UV0 coordinates shown as R = U, G = V.
    Uv,
}

impl BufferView {
    /// Every variant in display / cycle order, for the toolbar cycle + dropdown.
    pub const ALL: [BufferView; 11] = [
        BufferView::BaseColor,
        BufferView::WorldNormal,
        BufferView::NormalMap,
        BufferView::GeometricNormal,
        BufferView::Tangent,
        BufferView::Roughness,
        BufferView::Metallic,
        BufferView::AmbientOcclusion,
        BufferView::Emission,
        BufferView::Opacity,
        BufferView::Uv,
    ];

    /// Menu / notification label.
    pub fn label(self) -> &'static str {
        match self {
            BufferView::BaseColor => "Base Color",
            BufferView::WorldNormal => "Normal (World)",
            BufferView::NormalMap => "Normal Map (Tangent)",
            BufferView::GeometricNormal => "Geometric Normal",
            BufferView::Tangent => "Tangent",
            BufferView::Roughness => "Roughness",
            BufferView::Metallic => "Metallic",
            BufferView::AmbientOcclusion => "Ambient Occlusion",
            BufferView::Emission => "Emission",
            BufferView::Opacity => "Opacity",
            BufferView::Uv => "UV",
        }
    }

    /// The next buffer in [`BufferView::ALL`] order, wrapping after the last — for
    /// cycling by re-clicking the active Buffers button.
    pub fn next(self) -> BufferView {
        let all = BufferView::ALL;
        let index = all.iter().position(|&v| v == self).unwrap_or(0);
        all[(index + 1) % all.len()]
    }

    /// The index the scene shader's buffer-view switch reads from
    /// `projection_params.z`. Must match the `idx` arms in `review.glsl`'s
    /// `fs_main` buffer-view block (invariant 11).
    pub fn shader_index(self) -> f32 {
        match self {
            BufferView::BaseColor => 0.0,
            BufferView::WorldNormal => 1.0,
            BufferView::NormalMap => 2.0,
            BufferView::GeometricNormal => 3.0,
            BufferView::Tangent => 4.0,
            BufferView::Roughness => 5.0,
            BufferView::Metallic => 6.0,
            BufferView::AmbientOcclusion => 7.0,
            BufferView::Emission => 8.0,
            BufferView::Opacity => 9.0,
            BufferView::Uv => 10.0,
        }
    }
}

/// How the source-material faces are shaded — the "Material Mode" option behind
/// the Source Material button. [`Source`] keeps the imported (and user-edited)
/// materials; the other two replace every mesh part's material with a uniform
/// matte standard material so geometry can be read without texture/material
/// noise. The replacement is applied renderer-side as an *effective* material
/// table (the imported materials are untouched), so switching back is free.
///
/// [`Source`]: MaterialMode::Source
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MaterialMode {
    /// The material as imported from the source file plus any user edits (the
    /// default, the renderer's pre-existing behavior).
    #[default]
    Source,
    /// Every mesh part replaced by one uniform matte mid-grey material (fully
    /// rough, non-metallic, no emissive).
    Standard,
    /// Like [`Standard`], but each unique mesh part gets its own randomized hue
    /// at the same mid brightness — so the parts of the model read apart.
    ///
    /// [`Standard`]: MaterialMode::Standard
    Unique,
}

impl MaterialMode {
    /// Every variant in display order, for building the dropdown.
    pub const ALL: [MaterialMode; 3] = [
        MaterialMode::Source,
        MaterialMode::Standard,
        MaterialMode::Unique,
    ];

    /// Dropdown label.
    pub fn label(self) -> &'static str {
        match self {
            MaterialMode::Source => "Source Material",
            MaterialMode::Standard => "Standard Material",
            MaterialMode::Unique => "Unique Mesh",
        }
    }

    /// The next mode in display order, wrapping back to [`Source`] after
    /// [`Unique`] — for cycling by re-clicking the active Source Material button.
    ///
    /// [`Source`]: MaterialMode::Source
    /// [`Unique`]: MaterialMode::Unique
    pub fn next(self) -> MaterialMode {
        match self {
            MaterialMode::Source => MaterialMode::Standard,
            MaterialMode::Standard => MaterialMode::Unique,
            MaterialMode::Unique => MaterialMode::Source,
        }
    }
}
