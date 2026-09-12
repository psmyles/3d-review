//! Materials and the textures and videos they reference.
//!
//! With the capture in hand these are the materials the *source file* declared,
//! on the shader model it declared them with. Without it, a Phong material
//! carrying the viewer's PBR figures.

use std::ffi::CString;

use review_model::extras::{ShaderType, TextureKind};
use review_model::{ModelData, SourceExtras};

use super::*;

/// Emit every texture and video of the capture. Returns the capture's texture
/// index → scene texture index map (`-1` for one that could not be written).
///
/// Videos go out as they are. Textures are reordered so that every layer of a
/// layered texture precedes it — the bridge assembles a layered texture from
/// textures that already exist — which means a plain file texture always comes
/// first and a layered one after its members.
pub(crate) fn build_textures(
    scene: &mut SceneData,
    extras: &SourceExtras,
    report: &mut ExportReport,
) -> Vec<i32> {
    for video in &extras.videos {
        let props = scene.push_props(&video.props);
        scene.videos.push(VideoData {
            name: c_string_or_empty(&video.name),
            filename: c_string_or_empty(&video.absolute_filename),
            relative_filename: c_string_or_empty(&video.relative_filename),
            content: video.content.clone(),
            props,
        });
    }

    let mut map = vec![-1i32; extras.textures.len()];
    // Two passes: file textures, then layered ones whose layers are all placed.
    for (index, texture) in extras.textures.iter().enumerate() {
        if texture.kind != TextureKind::Layered {
            map[index] = push_texture(scene, texture, &map, false);
        }
    }
    let mut dropped = 0usize;
    for (index, texture) in extras.textures.iter().enumerate() {
        if texture.kind == TextureKind::Layered {
            if texture
                .layers
                .iter()
                .any(|layer| map.get(layer.texture as usize).copied().unwrap_or(-1) < 0)
            {
                // A layered texture nested inside another: not something the
                // exporter reassembles, and a texture no material can reach is
                // not worth a partial copy.
                dropped += 1;
                continue;
            }
            map[index] = push_texture(scene, texture, &map, true);
        }
    }
    if dropped > 0 {
        report.notes.push(format!(
            "{dropped} layered texture(s) nested inside another layered texture were not written."
        ));
    }
    map
}

pub(crate) fn push_texture(
    scene: &mut SceneData,
    texture: &review_model::extras::TextureExtras,
    map: &[i32],
    layered: bool,
) -> i32 {
    let props = scene.push_props(&texture.props);
    let video = texture
        .video
        .filter(|&video| (video as usize) < scene.videos.len())
        .map_or(-1, |video| video as i32);
    // Embedded bytes ride the video when there is one (a texture's content is
    // its video's), else the texture itself.
    let content = if video >= 0 {
        Vec::new()
    } else {
        texture.content.clone()
    };
    scene.textures.push(TextureData {
        name: c_string_or_empty(&texture.name),
        layered,
        filename: c_string_or_empty(&texture.absolute_filename),
        relative_filename: c_string_or_empty(&texture.relative_filename),
        content,
        video,
        props,
        layers: texture
            .layers
            .iter()
            .map(|layer| {
                (
                    map[layer.texture as usize],
                    layer.blend_mode.code() as i32,
                    layer.alpha,
                )
            })
            .collect(),
    });
    (scene.textures.len() - 1) as i32
}

/// Materials from the capture: the shader model the file declared, every
/// authored property, and the texture connections.
pub(crate) fn build_materials_from_extras(
    scene: &mut SceneData,
    source: &ModelData,
    extras: &SourceExtras,
    texture_map: &[i32],
) {
    for (index, material) in source.materials.iter().enumerate() {
        let name = c_string(&material.name, &format!("Material{index}"));
        let Some(authored) = extras.materials.get(index) else {
            scene.materials.push(viewer_material(name, material));
            continue;
        };
        let shader = match authored.shader_type {
            ShaderType::FbxLambert => SHADER_LAMBERT,
            ShaderType::FbxPhong => SHADER_PHONG,
            // A material ufbx could not classify still declared *some* model;
            // written as that custom name with its properties, so a reader that
            // knows the model recovers it. One that declared none is a Phong.
            ShaderType::Unknown if authored.shading_model.is_empty() => SHADER_PHONG,
            _ => SHADER_CUSTOM,
        };
        let props = scene.push_props(&authored.props);
        let textures = authored
            .textures
            .iter()
            .filter_map(|texture| {
                let index = *texture_map.get(texture.texture as usize)?;
                (index >= 0).then(|| (c_string_or_empty(&texture.material_prop), index))
            })
            .filter(|(prop, _)| !prop.is_empty())
            .collect();
        scene.materials.push(MaterialData {
            name,
            shader,
            shading_model: c_string_or_empty(&authored.shading_model),
            props,
            base_color: [0.0; 3],
            emissive: [0.0; 3],
            shininess_exponent: 0.0,
            reflection_factor: 0.0,
            textures,
        });
    }
}

/// Materials from what the viewer shows, for an export before the capture
/// landed: a Phong carrying the PBR figures on its classic slots.
pub(crate) fn build_materials_from_viewer(scene: &mut SceneData, source: &ModelData) {
    for (index, material) in source.materials.iter().enumerate() {
        let name = c_string(&material.name, &format!("Material{index}"));
        scene.materials.push(viewer_material(name, material));
    }
}

pub(crate) fn viewer_material(
    name: CString,
    material: &review_model::MaterialImportDefaults,
) -> MaterialData {
    // Import reads a Phong's shininess as `roughness = 1 − 0.1·√exponent`, so
    // the exponent that reads back as this smoothness is `(10·smoothness)²`.
    let shininess_exponent = f64::from(10.0 * material.smoothness).powi(2);
    MaterialData {
        name,
        shader: SHADER_PHONG,
        shading_model: CString::default(),
        props: PropRange::default(),
        base_color: [
            f64::from(material.base_color.x),
            f64::from(material.base_color.y),
            f64::from(material.base_color.z),
        ],
        emissive: [
            f64::from(material.emissive.x),
            f64::from(material.emissive.y),
            f64::from(material.emissive.z),
        ],
        shininess_exponent,
        reflection_factor: f64::from(material.metallic),
        textures: Vec::new(),
    }
}
