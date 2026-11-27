//! The Unwrap operation's settings.
//!
//! Two layouts, for the two reasons a mesh needs new UVs. **Tiled** is for
//! surface textures that repeat: every island at one texel density, turned to
//! lie as the input UVs did, islands free to overlap. **Atlas** is for anything
//! baked per pixel (lightmaps, AO, a texture painted on the model): every island
//! packed into the 0-1 square without overlapping.
//!
//! Islands are cut where the settings say: where a seam attribute changes (a
//! CAD face id, say), where the input UVs are already cut, at sharp edges if
//! asked, and always between materials and along open borders.

use serde::{Deserialize, Serialize};

/// How the islands are laid out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum UvLayout {
    /// Each island at the texel density, turned and placed to match the
    /// reference UVs; islands may overlap, as tiling textures allow.
    Tiled,
    /// Every island packed into the 0-1 square, none overlapping.
    #[default]
    Atlas,
}

impl UvLayout {
    pub const ALL: [UvLayout; 2] = [UvLayout::Atlas, UvLayout::Tiled];
}

/// What, besides materials, borders and the reference UVs' own seams, cuts the
/// surface into islands.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SeamSource {
    /// Nothing else.
    #[default]
    None,
    /// Wherever this custom vertex attribute (a glTF `_` attribute, such as
    /// `_CAD_FACE`) differs between two faces.
    Attribute(String),
    /// Wherever the U of this UV set differs between two faces: an id carried
    /// the way an FBX export carries one, in a UV set of its own.
    UvSetU(u32),
}

/// How each chart is flattened.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Flattening {
    /// Angles kept (LSCM); on a surface curved both ways the area, and so the
    /// texel density, drifts across the chart.
    Conformal,
    /// Angle balanced against area (LSCM, then as-rigid-as-possible), as a
    /// minimum-stretch unwrap does: an even density over curved charts.
    #[default]
    MinimumStretch,
}

impl Flattening {
    pub const ALL: [Flattening; 2] = [Flattening::MinimumStretch, Flattening::Conformal];
}

/// The Unwrap operation's parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UnwrapParams {
    pub layout: UvLayout,
    /// The UV set written, from 0; a set past the mesh's last is added.
    pub target_set: u32,
    /// The UV set the islands are matched to and compared with, and whose own
    /// seams cut the surface; `None` for a mesh whose UVs mean nothing.
    pub reference_set: Option<u32>,
    /// What else cuts islands.
    pub seams: SeamSource,
    /// Cut wherever the reference UVs are discontinuous — which catches a
    /// periodic seam inside one region, such as a full cylinder's.
    pub split_at_reference_seams: bool,
    /// Cut along edges sharper than [`Self::crease_angle`].
    pub split_at_creases: bool,
    /// In degrees.
    pub crease_angle: f32,
    /// How each chart is flattened.
    pub flattening: Flattening,
    /// The most a chart may stretch, as the area-weighted mean over its
    /// triangles of each one's worst stretch either way against the chart's
    /// mean density. An island is kept whole when it flattens within this,
    /// and cut into as few charts as do otherwise: 1 cuts at any stretch,
    /// higher keeps charts larger, with fewer seams.
    #[serde(alias = "island_stretch_limit")]
    pub max_stretch: f32,
    /// Tiled: UV units per metre.
    pub density: f32,
    /// Tiled: turn each island to lie as its reference UVs did (rotation or
    /// mirror), centred where they were.
    pub align_to_reference: bool,
    /// Tiled: per island, keep the reference UVs (scaled to the density)
    /// instead, where they stretch less.
    pub keep_better: bool,
    /// Atlas: the texture size the padding is measured in, in texels.
    pub resolution: u32,
    /// Atlas: texels left between islands.
    pub padding: u32,
    /// Let the packing turn charts to fit them tighter (and, for tiled
    /// charts with no reference to turn them by, to lay them along U).
    pub rotate: bool,
}

impl Default for UnwrapParams {
    fn default() -> Self {
        Self {
            layout: UvLayout::Atlas,
            target_set: 0,
            reference_set: Some(0),
            seams: SeamSource::None,
            split_at_reference_seams: true,
            split_at_creases: false,
            crease_angle: 60.0,
            flattening: Flattening::MinimumStretch,
            max_stretch: 1.1,
            density: 1.0,
            align_to_reference: true,
            keep_better: true,
            resolution: 1024,
            padding: 4,
            rotate: true,
        }
    }
}

impl UnwrapParams {
    /// The settings that rebuild a CAD part's UVs for a tiling texture (the
    /// a CAD pipeline's): one island per CAD face, cut again where the
    /// CAD UVs are and where a face will not flatten within the stretch limit,
    /// each at one UV unit per metre and turned to lie as the CAD UVs did, the
    /// CAD UVs kept where they stretch less.
    pub fn cad_islands(attribute: &str) -> Self {
        Self {
            layout: UvLayout::Tiled,
            seams: SeamSource::Attribute(attribute.to_owned()),
            ..Self::default()
        }
    }
}
