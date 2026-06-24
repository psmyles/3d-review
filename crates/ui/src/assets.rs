//! Embedded assets: toolbar/gizmo icons and bundled UI fonts.
//!
//! Icons and fonts are baked into the binary via `include_bytes!`. Icons are
//! decoded lazily on first use and cached as egui textures keyed by their id.

use std::sync::Arc;

use image::ImageError;

pub(crate) struct AppIcon {
    pub(crate) id: &'static str,
    pub(crate) png_bytes: &'static [u8],
}

/// "Show Wireframe" — the independent wireframe-overlay toggle.
pub(crate) const ICON_SHADING_WIRE: AppIcon = AppIcon {
    id: "icon_shading_wire",
    png_bytes: include_bytes!("../../../assets/icons/icon_shading_wire.png"),
};
/// "Wireframe Only" — the wireframe-only shading mode.
pub(crate) const ICON_SHADING_WIRE_ONLY: AppIcon = AppIcon {
    id: "icon_shading_wire_only",
    png_bytes: include_bytes!("../../../assets/icons/icon_shading_wire_only.png"),
};
pub(crate) const ICON_SHADING_UNLIT: AppIcon = AppIcon {
    id: "icon_shading_unlit",
    png_bytes: include_bytes!("../../../assets/icons/icon_shading_unlit.png"),
};
/// "Shaded" — the lit shading mode.
pub(crate) const ICON_SHADING_SHADED: AppIcon = AppIcon {
    id: "icon_shading_shaded",
    png_bytes: include_bytes!("../../../assets/icons/icon_shading_shaded.png"),
};
/// "Backface Rendering" — independent toggle: when on, back faces are drawn
/// (double-sided); when off (default) they are culled.
pub(crate) const ICON_BACKFACE: AppIcon = AppIcon {
    id: "icon_backface",
    png_bytes: include_bytes!("../../../assets/icons/icon_backface.png"),
};
/// "Source Material" — show the model's imported material (default).
pub(crate) const ICON_SHADING_TEXTURE: AppIcon = AppIcon {
    id: "icon_shading_texture",
    png_bytes: include_bytes!("../../../assets/icons/icon_shading_texture.png"),
};
pub(crate) const ICON_UV: AppIcon = AppIcon {
    id: "icon_uv",
    png_bytes: include_bytes!("../../../assets/icons/icon_uv.png"),
};
/// "UV Wire" — the wire-only UV shading mode.
pub(crate) const ICON_UV_WIRE: AppIcon = AppIcon {
    id: "icon_uv_wire",
    png_bytes: include_bytes!("../../../assets/icons/icon_uv_wire.png"),
};
/// "UV Shaded" — UV islands filled with one solid shaded color.
pub(crate) const ICON_UV_SHADED: AppIcon = AppIcon {
    id: "icon_uv_shaded",
    png_bytes: include_bytes!("../../../assets/icons/icon_uv_shaded.png"),
};
/// "UV Islands" — each UV island filled with a unique color.
pub(crate) const ICON_UV_ISLANDS: AppIcon = AppIcon {
    id: "icon_uv_colored",
    png_bytes: include_bytes!("../../../assets/icons/icon_uv_colored.png"),
};
pub(crate) const ICON_NORMALS_FACE: AppIcon = AppIcon {
    id: "icon_normals_face",
    png_bytes: include_bytes!("../../../assets/icons/icon_normals_face.png"),
};
pub(crate) const ICON_NORMALS_VERTEX: AppIcon = AppIcon {
    id: "icon_normals_vertex",
    png_bytes: include_bytes!("../../../assets/icons/icon_normals_vertex.png"),
};
pub(crate) const ICON_VERTEX_COLORS: AppIcon = AppIcon {
    id: "icon_vertex_colors",
    png_bytes: include_bytes!("../../../assets/icons/icon_vertex_colors.png"),
};
pub(crate) const ICON_VIEW_ORTHO: AppIcon = AppIcon {
    id: "icon_view_ortho",
    png_bytes: include_bytes!("../../../assets/icons/icon_view_ortho.png"),
};
pub(crate) const ICON_VIEW_PERSPECTIVE: AppIcon = AppIcon {
    id: "icon_view_perspective",
    png_bytes: include_bytes!("../../../assets/icons/icon_view_perspective.png"),
};
pub(crate) const ICON_AXIS_GIZMO: AppIcon = AppIcon {
    id: "icon_empty_axis",
    png_bytes: include_bytes!("../../../assets/icons/icon_empty_axis.png"),
};
pub(crate) const ICON_RESET: AppIcon = AppIcon {
    id: "icon_reset",
    png_bytes: include_bytes!("../../../assets/icons/icon_reset.png"),
};
pub(crate) const ICON_GRID: AppIcon = AppIcon {
    id: "icon_grid",
    png_bytes: include_bytes!("../../../assets/icons/icon_grid.png"),
};
pub(crate) const ICON_BBOX: AppIcon = AppIcon {
    id: "icon_bbox",
    png_bytes: include_bytes!("../../../assets/icons/icon_bbox.png"),
};
pub(crate) const ICON_INFO: AppIcon = AppIcon {
    id: "icon_info",
    png_bytes: include_bytes!("../../../assets/icons/icon_info.png"),
};
/// "Anti aliasing" — opens the MSAA / FXAA options panel (status bar, right).
pub(crate) const ICON_ANTI_ALIASING: AppIcon = AppIcon {
    id: "icon_anti_aliasing",
    png_bytes: include_bytes!("../../../assets/icons/icon_anti_aliasing.png"),
};
/// "Image-based lighting" — toggles IBL; right-click opens the Environment
/// options panel (status bar, right, next to Anti aliasing).
pub(crate) const ICON_IBL: AppIcon = AppIcon {
    id: "icon_ibl",
    png_bytes: include_bytes!("../../../assets/icons/icon_ibl.png"),
};
/// "Bloom" — toggles HDR bloom; right-click opens the Bloom options panel (status
/// bar, right, grouped with IBL + Anti aliasing).
pub(crate) const ICON_BLOOM: AppIcon = AppIcon {
    id: "icon_bloom",
    png_bytes: include_bytes!("../../../assets/icons/icon_bloom.png"),
};
/// "Ambient occlusion" — toggles GTAO; right-click opens the Ambient Occlusion
/// options panel (status bar, right, grouped with IBL + Bloom + Anti aliasing).
pub(crate) const ICON_AO: AppIcon = AppIcon {
    id: "icon_ao",
    png_bytes: include_bytes!("../../../assets/icons/icon_ao.png"),
};
/// "Tonemapper" — toggles tone mapping; right-click opens the Tonemapper options
/// panel (status bar, right, grouped with IBL + Bloom + AO + Anti aliasing).
pub(crate) const ICON_TONEMAPPER: AppIcon = AppIcon {
    id: "icon_tonemapper",
    png_bytes: include_bytes!("../../../assets/icons/icon_tonemapper.png"),
};
/// "Outliner" — toggles the scene-graph / materials Outliner window (Phase 2).
pub(crate) const ICON_OUTLINER: AppIcon = AppIcon {
    id: "icon_outliner",
    png_bytes: include_bytes!("../../../assets/icons/icon_outliner.png"),
};
/// "Inspector" — toggles the material / node Inspector window (Phase 2).
pub(crate) const ICON_INSPECTOR: AppIcon = AppIcon {
    id: "icon_inspector",
    png_bytes: include_bytes!("../../../assets/icons/icon_inspector.png"),
};

/// Tone-mapped equirectangular preview thumbnails for the built-in HDR
/// environments, shown beside each option in the Environment dropdown. They are
/// small (128×64) sRGB PNGs generated offline from the HDRs in `assets/textures`,
/// embedded the same way as the icons above and decoded lazily on first use via
/// [`load_icon_texture`]. One entry per [`review_render::EnvironmentMap`] variant
/// (see [`crate::panels`]'s environment row, which maps a variant to its thumb).
pub(crate) const THUMB_HDR_01: AppIcon = AppIcon {
    id: "thumb_hdr_01",
    png_bytes: include_bytes!("../../../assets/thumbnails/T_HDR_01.png"),
};
pub(crate) const THUMB_HDR_02: AppIcon = AppIcon {
    id: "thumb_hdr_02",
    png_bytes: include_bytes!("../../../assets/thumbnails/T_HDR_02.png"),
};
pub(crate) const THUMB_HDR_03: AppIcon = AppIcon {
    id: "thumb_hdr_03",
    png_bytes: include_bytes!("../../../assets/thumbnails/T_HDR_03.png"),
};
pub(crate) const THUMB_HDR_04: AppIcon = AppIcon {
    id: "thumb_hdr_04",
    png_bytes: include_bytes!("../../../assets/thumbnails/T_HDR_04.png"),
};
pub(crate) const THUMB_HDR_05: AppIcon = AppIcon {
    id: "thumb_hdr_05",
    png_bytes: include_bytes!("../../../assets/thumbnails/T_HDR_05.png"),
};
pub(crate) const THUMB_HDR_06: AppIcon = AppIcon {
    id: "thumb_hdr_06",
    png_bytes: include_bytes!("../../../assets/thumbnails/T_HDR_06.png"),
};

/// A custom font embedded into the binary and registered with egui at startup.
struct CustomFont {
    /// Unique key egui uses to reference this font.
    name: &'static str,
    /// Raw .ttf / .otf bytes, embedded via `include_bytes!`.
    ttf_bytes: &'static [u8],
    /// Family this font becomes the primary face for.
    family: egui::FontFamily,
}

/// Fonts bundled with the app, each made the default (index 0) for its family.
/// Both are variable fonts; egui/ab_glyph rasterizes their default instance
/// (regular weight), which is what we want for UI text.
const CUSTOM_FONTS: &[CustomFont] = &[
    CustomFont {
        name: "inter",
        ttf_bytes: include_bytes!("../../../assets/fonts/InterVariable.ttf"),
        family: egui::FontFamily::Proportional,
    },
    CustomFont {
        name: "jetbrains_mono",
        ttf_bytes: include_bytes!("../../../assets/fonts/JetBrainsMono.ttf"),
        family: egui::FontFamily::Monospace,
    },
];

/// Install bundled custom fonts into the egui context. Call once at startup
/// (the `app` coordinator does this when it builds the `egui::Context`).
///
/// Fonts are embedded at compile time via `include_bytes!`, matching how icons
/// are bundled. To add a font, drop the file in `assets/fonts/` and append a
/// [`CustomFont`] to [`CUSTOM_FONTS`]. Each font is inserted at index 0 of its
/// family so it shadows egui's built-in default while keeping the built-ins as
/// fallbacks for missing glyphs.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for font in CUSTOM_FONTS {
        fonts.font_data.insert(
            font.name.to_owned(),
            Arc::new(egui::FontData::from_static(font.ttf_bytes)),
        );
        fonts
            .families
            .entry(font.family.clone())
            .or_default()
            .insert(0, font.name.to_owned());
    }
    ctx.set_fonts(fonts);
}

/// Lazily decode and cache an icon as an egui texture, returning a handle sized
/// to the source image. Cached in egui temp data keyed by the icon id, so a
/// given icon is decoded at most once.
pub(crate) fn load_icon_texture(
    ui: &mut egui::Ui,
    icon: &AppIcon,
) -> Option<egui::load::SizedTexture> {
    let texture_id = egui::Id::new(("toolbar_icon_texture", icon.id));
    if let Some(handle) = ui.data(|data| data.get_temp::<egui::TextureHandle>(texture_id)) {
        return Some(egui::load::SizedTexture::from_handle(&handle));
    }

    let color_image = decode_icon_color_image(icon).ok()?;
    let handle = ui
        .ctx()
        .load_texture(icon.id, color_image, egui::TextureOptions::LINEAR);
    let texture = egui::load::SizedTexture::from_handle(&handle);
    ui.data_mut(|data| data.insert_temp(texture_id, handle));
    Some(texture)
}

fn decode_icon_color_image(icon: &AppIcon) -> Result<egui::ColorImage, ImageError> {
    let decoded = image::load_from_memory(icon.png_bytes)?.into_rgba8();
    let size = [decoded.width() as usize, decoded.height() as usize];
    let rgba = decoded.into_raw();
    Ok(egui::ColorImage::from_rgba_unmultiplied(size, &rgba))
}
