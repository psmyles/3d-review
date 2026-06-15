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

pub(crate) const ICON_SHADING_WIRE: AppIcon = AppIcon {
    id: "icon_shading_wire",
    png_bytes: include_bytes!("../../../assets/icons/icon_shading_wire.png"),
};
pub(crate) const ICON_SHADING_UNLIT: AppIcon = AppIcon {
    id: "icon_shading_unlit",
    png_bytes: include_bytes!("../../../assets/icons/icon_shading_unlit.png"),
};
pub(crate) const ICON_SHADING_SOLID: AppIcon = AppIcon {
    id: "icon_shading_solid",
    png_bytes: include_bytes!("../../../assets/icons/icon_shading_solid.png"),
};
pub(crate) const ICON_SHADING_WIRE_SHADED: AppIcon = AppIcon {
    id: "icon_shading_wire_shaded",
    png_bytes: include_bytes!("../../../assets/icons/icon_shading_wire_shaded.png"),
};
pub(crate) const ICON_UV: AppIcon = AppIcon {
    id: "icon_uv",
    png_bytes: include_bytes!("../../../assets/icons/icon_uv.png"),
};
pub(crate) const ICON_NORMALS_FACE: AppIcon = AppIcon {
    id: "icon_normals_face",
    png_bytes: include_bytes!("../../../assets/icons/icon_normals_face.png"),
};
pub(crate) const ICON_NORMALS_VERTEX: AppIcon = AppIcon {
    id: "icon_normals_vertex",
    png_bytes: include_bytes!("../../../assets/icons/icon_normals_vertex.png"),
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
pub(crate) const ICON_CLOSE: AppIcon = AppIcon {
    id: "icon_close",
    png_bytes: include_bytes!("../../../assets/icons/icon_close.png"),
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
