//! The textures an imported model names for its own materials: where each
//! one's pixels are, which slot it feeds, and how it binds once decoded.
//!
//! The source names a texture per *map* ([`MaterialMap`], resolved by the
//! importer across shading models), and the capture carries each texture's
//! file names as authored plus any bytes the file embeds
//! ([`SourceExtras::textures`]). Turning that into slot bindings is three
//! steps, split by what each needs:
//!
//! * [`plan`] — which slot each map feeds, and where each texture's pixels
//!   are: embedded bytes, or the first of the authored paths that exists on
//!   disk. It touches the file system (existence checks) and nothing else.
//! * [`decode`] — every planned image, in parallel, optionally halved to fit a
//!   size (a thumbnail never draws a 4K map at 4K).
//! * [`TextureUse::binding`] — the slot binding, decided once the pixels are
//!   in, because two of the choices depend on them: whether a Bump-slot image
//!   is a normal map, and which channel an opacity map carries.
//!
//! Two rules a DCC applies that the file does not spell out, and that binding
//! follows: a texture connected to a color *replaces* the color (so the factor
//! it is multiplied by becomes white, or the emission strength alone — FBX
//! stores whatever value the color had before the connection, often a 0.5
//! grey, and multiplying by it showed every textured asset half as bright as
//! its DCC did), and a glossiness map is roughness inverted (the material
//! switches to the smoothness workflow rather than the map being re-encoded).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use glam::Vec3;
use review_model::{MaterialImportDefaults, MaterialMap, SourceExtras};

use super::{
    ChannelSelect, DecodedImage, TextureSlot, decode_bytes, decode_image, suggested_channel,
};
use crate::material::{AlphaMode, RoughnessWorkflow};

/// Every texture a model's materials name, deduplicated by image, and the
/// names that could not be found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportedTextures {
    /// One entry per distinct image, in the order the materials first use them.
    pub textures: Vec<ImportedTexture>,
    /// The authored file name of every texture that was named but is neither
    /// embedded nor anywhere the authored paths point, deduplicated and in
    /// first-use order.
    pub missing: Vec<String>,
}

impl ImportedTextures {
    pub fn is_empty(&self) -> bool {
        self.textures.is_empty() && self.missing.is_empty()
    }
}

/// One image the model's materials use, and every use of it.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedTexture {
    /// What a binding records as the image's path (`TextureBinding::path`):
    /// the file itself, or, for embedded media, its name under the model's own
    /// path — a path no file can have, since the model is a file — so the
    /// Inspector names it and nothing mistakes it for something on disk.
    pub key: PathBuf,
    pub source: TextureSource,
    pub uses: Vec<TextureUse>,
}

impl ImportedTexture {
    /// Whether any use reads the image as color, which is what decides how a
    /// downscale averages it.
    fn is_color(&self) -> bool {
        self.uses.iter().any(|texture_use| {
            matches!(
                texture_use.map,
                MaterialMap::BaseColor | MaterialMap::Emission
            )
        })
    }
}

/// Where an imported texture's pixels are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextureSource {
    /// A file on disk: the first authored candidate that exists.
    File(PathBuf),
    /// Bytes the model file embeds, on the texture at this index of
    /// [`SourceExtras::textures`] or on its video.
    Embedded { texture: u32 },
}

impl TextureSource {
    pub fn is_embedded(&self) -> bool {
        matches!(self, TextureSource::Embedded { .. })
    }
}

/// One material map reading an image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextureUse {
    /// Index into the model's materials, and so into the renderer's table.
    pub material: usize,
    pub map: MaterialMap,
    /// The material's emission strength, which an emission map is multiplied
    /// by in place of the color it replaces.
    emission_factor: f32,
}

/// How one [`TextureUse`] binds, once its image is decoded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImportedBinding {
    pub material: usize,
    pub slot: TextureSlot,
    pub channel: ChannelSelect,
    /// Set for a glossiness map, which is roughness inverted.
    pub workflow: Option<RoughnessWorkflow>,
    /// Set for an opacity map.
    pub alpha_mode: Option<AlphaMode>,
    /// The color factor the texture is multiplied by, which replaces the
    /// material's own for a color map (see the module docs).
    pub base_color: Option<Vec3>,
    pub emissive: Option<Vec3>,
}

impl TextureUse {
    /// The binding this use makes with `image` (read from `key`), or `None`
    /// when the image turns out not to be what the map needs — a Bump-slot
    /// height map, which read as normals would light the surface wrongly.
    pub fn binding(&self, key: &Path, image: &DecodedImage) -> Option<ImportedBinding> {
        let mut binding = ImportedBinding {
            material: self.material,
            slot: TextureSlot::BaseColor,
            channel: ChannelSelect::Rgb,
            workflow: None,
            alpha_mode: None,
            base_color: None,
            emissive: None,
        };
        match self.map {
            MaterialMap::BaseColor => binding.base_color = Some(Vec3::ONE),
            MaterialMap::Normal => binding.slot = TextureSlot::Normal,
            MaterialMap::Bump => {
                if !looks_like_normal_map(image) {
                    return None;
                }
                binding.slot = TextureSlot::Normal;
            }
            MaterialMap::Roughness => {
                binding.slot = TextureSlot::Roughness;
                binding.channel = suggested_channel(key, TextureSlot::Roughness);
            }
            MaterialMap::Glossiness => {
                binding.slot = TextureSlot::Roughness;
                binding.channel = suggested_channel(key, TextureSlot::Roughness);
                binding.workflow = Some(RoughnessWorkflow::Smoothness);
            }
            MaterialMap::Metalness => {
                binding.slot = TextureSlot::Metallic;
                binding.channel = suggested_channel(key, TextureSlot::Metallic);
            }
            MaterialMap::AmbientOcclusion => {
                binding.slot = TextureSlot::Ao;
                binding.channel = suggested_channel(key, TextureSlot::Ao);
            }
            MaterialMap::Emission => {
                binding.slot = TextureSlot::Emissive;
                binding.emissive = Some(Vec3::splat(self.emission_factor.max(0.0)));
            }
            MaterialMap::Opacity => {
                binding.slot = TextureSlot::Opacity;
                // The diffuse texture with its alpha connected again is the
                // common case; a separate mask with no alpha carries it in red.
                binding.channel = if matches!(image.source_channels, 2 | 4) {
                    ChannelSelect::A
                } else {
                    ChannelSelect::R
                };
                // A cutout, not a blend: a source's opacity map is foliage,
                // hair cards and fences far more often than glass, and a cutout
                // stays depth-correct where an unsorted blend does not. The
                // Inspector switches it.
                binding.alpha_mode = Some(AlphaMode::Clip);
            }
        }
        Some(binding)
    }
}

/// Which slot each material map feeds, and where each named texture's pixels
/// are. `model_path` is the model file's own path, when there is one: the
/// directory relative paths are resolved against and the name embedded media
/// is keyed under. Without it (a model read from bytes) only the authored
/// absolute paths can be tried.
pub fn plan(
    materials: &[MaterialImportDefaults],
    extras: &SourceExtras,
    model_path: Option<&Path>,
) -> ImportedTextures {
    let mut planned = ImportedTextures::default();
    // By texture index, the entry of `planned.textures` it went to, or `None`
    // once it is known to be missing.
    let mut resolved: Vec<Option<Option<usize>>> = vec![None; extras.textures.len()];

    for (material_index, material) in materials.iter().enumerate() {
        for map in bound_maps(material) {
            // A texture index the capture does not have is a map with nothing
            // behind it: the two tables come from one parse, so it cannot
            // happen short of a malformed capture, and then there is nothing
            // to find.
            let Some(texture) = material
                .map(map)
                .filter(|&texture| (texture as usize) < extras.textures.len())
            else {
                continue;
            };
            let entry = *resolved[texture as usize].get_or_insert_with(|| {
                let found = locate(extras, texture, model_path);
                match found {
                    Some((key, source)) => {
                        // Two texture elements over one file are one image.
                        let existing = planned.textures.iter().position(|t| t.key == key);
                        Some(existing.unwrap_or_else(|| {
                            planned.textures.push(ImportedTexture {
                                key,
                                source,
                                uses: Vec::new(),
                            });
                            planned.textures.len() - 1
                        }))
                    }
                    // A texture with no file name at all (a procedural map, an
                    // empty file node) had nothing to find, so it is not
                    // reported as missing.
                    None if authored_strings(extras, texture).is_empty() => None,
                    None => {
                        let name = authored_name(extras, texture);
                        if !planned.missing.contains(&name) {
                            planned.missing.push(name);
                        }
                        None
                    }
                }
            });
            if let Some(entry) = entry {
                planned.textures[entry].uses.push(TextureUse {
                    material: material_index,
                    map,
                    emission_factor: material.emission_factor,
                });
            }
        }
    }
    planned
}

/// The maps of `material` that bind, in slot order: a normal map ahead of a
/// Bump-slot one, and a roughness map ahead of a glossiness one, so each slot
/// is offered at most one image.
fn bound_maps(material: &MaterialImportDefaults) -> impl Iterator<Item = MaterialMap> + '_ {
    let normal = if material.map(MaterialMap::Normal).is_some() {
        MaterialMap::Normal
    } else {
        MaterialMap::Bump
    };
    let roughness = if material.map(MaterialMap::Roughness).is_some() {
        MaterialMap::Roughness
    } else {
        MaterialMap::Glossiness
    };
    [
        MaterialMap::BaseColor,
        normal,
        roughness,
        MaterialMap::Metalness,
        MaterialMap::AmbientOcclusion,
        MaterialMap::Emission,
        MaterialMap::Opacity,
    ]
    .into_iter()
    .filter(|map| material.map(*map).is_some())
}

/// Where texture `index`'s pixels are, and the key a binding records for them.
fn locate(
    extras: &SourceExtras,
    index: u32,
    model_path: Option<&Path>,
) -> Option<(PathBuf, TextureSource)> {
    if embedded_bytes(extras, index).is_some() {
        let name = authored_name(extras, index);
        let key = match model_path {
            Some(path) => path.join(&name),
            None => PathBuf::from(&name),
        };
        return Some((key, TextureSource::Embedded { texture: index }));
    }
    candidates(&authored_strings(extras, index), model_path)
        .into_iter()
        .find(|candidate| candidate.is_file())
        .map(|path| (path.clone(), TextureSource::File(path)))
}

/// The embedded bytes of texture `index`: its own, or its video's, which is
/// where FBX keeps them.
fn embedded_bytes(extras: &SourceExtras, index: u32) -> Option<&[u8]> {
    let texture = extras.textures.get(index as usize)?;
    if !texture.content.is_empty() {
        return Some(&texture.content);
    }
    let video = extras.videos.get(texture.video? as usize)?;
    (!video.content.is_empty()).then_some(video.content.as_slice())
}

/// The file names texture `index` (and its video) was authored with, best
/// first.
fn authored_strings(extras: &SourceExtras, index: u32) -> Vec<&str> {
    let Some(texture) = extras.textures.get(index as usize) else {
        return Vec::new();
    };
    let video = texture
        .video
        .and_then(|video| extras.videos.get(video as usize));
    [
        Some(texture.filename.as_str()),
        Some(texture.absolute_filename.as_str()),
        Some(texture.relative_filename.as_str()),
        video.map(|video| video.filename.as_str()),
        video.map(|video| video.absolute_filename.as_str()),
        video.map(|video| video.relative_filename.as_str()),
    ]
    .into_iter()
    .flatten()
    .filter(|name| !name.is_empty())
    .collect()
}

/// The name a texture is shown by: the file name of its first authored path,
/// or the texture element's own name when it has none.
fn authored_name(extras: &SourceExtras, index: u32) -> String {
    authored_strings(extras, index)
        .into_iter()
        .map(file_name)
        .find(|name| !name.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            extras
                .textures
                .get(index as usize)
                .map(|texture| texture.name.clone())
        })
        .unwrap_or_default()
}

/// Every path a texture authored as `authored` might be at, in the order to
/// try them: the paths as authored (ufbx's own resolution against the model's
/// directory first), then the file name beside the model and in the folders
/// exporters put textures in. An asset is routinely moved after it is
/// exported, so the authored absolute path is usually stale and the file name
/// is what still finds it.
fn candidates(authored: &[&str], model_path: Option<&Path>) -> Vec<PathBuf> {
    let directory = model_path.and_then(Path::parent);
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut push = |path: PathBuf| {
        if !paths.contains(&path) {
            paths.push(path);
        }
    };
    for name in authored {
        let path = native_path(name);
        if path.is_absolute() {
            push(path);
        } else if let Some(directory) = directory {
            push(directory.join(path));
        }
    }
    if let Some(directory) = directory {
        let stem = model_path
            .and_then(Path::file_stem)
            .map(|stem| format!("{}.fbm", stem.to_string_lossy()));
        for name in authored.iter().map(|name| file_name(name)) {
            if name.is_empty() {
                continue;
            }
            push(directory.join(name));
            push(directory.join("textures").join(name));
            if let Some(stem) = &stem {
                push(directory.join(stem).join(name));
            }
            if let Some(parent) = directory.parent() {
                push(parent.join("textures").join(name));
            }
        }
    }
    paths
}

/// The last component of an authored path, whichever separator it was written
/// with — an FBX made on Windows carries backslashes to every other host.
fn file_name(authored: &str) -> &str {
    authored.rsplit(['/', '\\']).next().unwrap_or(authored)
}

/// An authored path in this host's separators.
fn native_path(authored: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(authored)
    } else {
        PathBuf::from(authored.replace('\\', "/"))
    }
}

/// A plan with its images decoded: what a caller with no texture pool of its
/// own (the snapshot tool, a thumbnail) binds in one go with
/// [`crate::Renderer::bind_imported_textures`].
#[derive(Debug, Clone, Default)]
pub struct LoadedTextures {
    pub planned: ImportedTextures,
    /// Parallel to `planned.textures`.
    pub images: Vec<Result<std::sync::Arc<DecodedImage>, String>>,
}

impl LoadedTextures {
    /// Every planned texture whose decode failed, with the decoder's message.
    pub fn failures(&self) -> impl Iterator<Item = (&ImportedTexture, &str)> {
        self.planned
            .textures
            .iter()
            .zip(&self.images)
            .filter_map(|(texture, image)| {
                image.as_ref().err().map(|error| (texture, error.as_str()))
            })
    }
}

/// [`plan`] and [`decode`] in one call.
pub fn load(
    materials: &[MaterialImportDefaults],
    extras: &SourceExtras,
    model_path: Option<&Path>,
    max_side: Option<u32>,
) -> LoadedTextures {
    let planned = plan(materials, extras, model_path);
    let images = decode(&planned, extras, max_side)
        .into_iter()
        .map(|image| image.map(std::sync::Arc::new))
        .collect();
    LoadedTextures { planned, images }
}

/// Decode every planned texture, in parallel, each halved to fit `max_side`
/// when it is given. The result is parallel to `planned.textures`; a failure
/// is the decoder's own message.
pub fn decode(
    planned: &ImportedTextures,
    extras: &SourceExtras,
    max_side: Option<u32>,
) -> Vec<Result<DecodedImage, String>> {
    let count = planned.textures.len();
    let mut results: Vec<Option<Result<DecodedImage, String>>> = vec![None; count];
    if count == 0 {
        return Vec::new();
    }
    let workers = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(count);
    let next = AtomicUsize::new(0);
    let decoded = std::sync::Mutex::new(&mut results);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(texture) = planned.textures.get(index) else {
                        break;
                    };
                    let result = decode_one(texture, extras, max_side);
                    if let Ok(mut decoded) = decoded.lock() {
                        decoded[index] = Some(result);
                    }
                }
            });
        }
    });
    results
        .into_iter()
        .map(|result| result.unwrap_or_else(|| Err("the decode did not run".to_owned())))
        .collect()
}

fn decode_one(
    texture: &ImportedTexture,
    extras: &SourceExtras,
    max_side: Option<u32>,
) -> Result<DecodedImage, String> {
    let image = match &texture.source {
        TextureSource::File(path) => decode_image(path)?,
        TextureSource::Embedded { texture: index } => {
            let bytes = embedded_bytes(extras, *index)
                .ok_or_else(|| format!("{} has no embedded content", texture.key.display()))?;
            let extension = texture
                .key
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_ascii_lowercase)
                .unwrap_or_default();
            decode_bytes(bytes, &extension, &texture.key)?
        }
    };
    Ok(match max_side {
        Some(max_side) => image.fitted_within(max_side, texture.is_color()),
        None => image,
    })
}

/// Whether `image` is a tangent-space normal map rather than a height map: a
/// normal map is blue on average (its vectors point out of the surface) with
/// red and green centred, where a height map is grey.
fn looks_like_normal_map(image: &DecodedImage) -> bool {
    let texels = image.rgba.as_chunks::<4>().0;
    if texels.is_empty() {
        return false;
    }
    // A few thousand evenly spaced texels decide it as well as all of them.
    let step = (texels.len() / 4096).max(1);
    let (mut red, mut green, mut blue, mut count) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for texel in texels.iter().step_by(step) {
        red += f64::from(texel[0]);
        green += f64::from(texel[1]);
        blue += f64::from(texel[2]);
        count += 1.0;
    }
    let mean = |sum: f64| sum / count / 255.0;
    let centred = |value: f64| (value - 0.5).abs() < 0.15;
    mean(blue) > 0.7 && centred(mean(red)) && centred(mean(green))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: u32, height: u32, texel: [u8; 4], channels: u8) -> DecodedImage {
        DecodedImage {
            width,
            height,
            rgba: std::iter::repeat_n(texel, (width * height) as usize)
                .flatten()
                .collect(),
            source_channels: channels,
            source_bit_depth: 8,
        }
    }

    #[test]
    fn a_windows_path_names_its_file_on_every_host() {
        assert_eq!(file_name(r"C:\art\rock\rock_D.png"), "rock_D.png");
        assert_eq!(file_name("textures/rock_N.tga"), "rock_N.tga");
        assert_eq!(file_name("rock.psd"), "rock.psd");
    }

    /// An asset moved after export: its authored absolute path is stale, but
    /// the file name is found in the `textures` folder beside the model, and
    /// the stale path is still tried first.
    #[test]
    fn a_moved_asset_finds_its_texture_by_name_beside_the_model() {
        let root = std::env::temp_dir().join(format!("review_imported_{}", std::process::id()));
        let textures = root.join("textures");
        std::fs::create_dir_all(&textures).expect("make the fixture folder");
        std::fs::write(textures.join("rock_D.png"), b"not decoded here").expect("write");
        let model = root.join("rock.fbx");

        let authored = [r"C:\Users\artist\work\rock_D.png"];
        let tried = candidates(&authored, Some(&model));
        let found = tried.iter().find(|path| path.is_file()).cloned();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(found, Some(textures.join("rock_D.png")));
        if cfg!(windows) {
            assert_eq!(tried[0], PathBuf::from(authored[0]));
        }
    }

    /// Without the model's path (a model read from bytes) only an absolute
    /// authored path can be tried.
    #[test]
    fn a_model_without_a_path_tries_only_absolute_paths() {
        let tried = candidates(&["textures/rock_D.png"], None);
        assert!(tried.is_empty());
    }

    #[test]
    fn a_bump_slot_binds_only_a_normal_map() {
        let normal = image(4, 4, [128, 128, 255, 255], 3);
        let height = image(4, 4, [128, 128, 128, 255], 1);
        let bump = TextureUse {
            material: 0,
            map: MaterialMap::Bump,
            emission_factor: 1.0,
        };
        let binding = bump
            .binding(Path::new("rock_N.png"), &normal)
            .expect("a blue image is a normal map");
        assert_eq!(binding.slot, TextureSlot::Normal);
        assert!(bump.binding(Path::new("rock_H.png"), &height).is_none());
    }

    #[test]
    fn an_opacity_map_reads_alpha_when_it_has_one_and_cuts_out() {
        let opacity = TextureUse {
            material: 2,
            map: MaterialMap::Opacity,
            emission_factor: 1.0,
        };
        let with_alpha = opacity
            .binding(Path::new("leaf_D.tga"), &image(2, 2, [9, 9, 9, 0], 4))
            .expect("an opacity map always binds");
        assert_eq!(with_alpha.channel, ChannelSelect::A);
        assert_eq!(with_alpha.alpha_mode, Some(AlphaMode::Clip));
        let mask = opacity
            .binding(Path::new("leaf_mask.png"), &image(2, 2, [9, 9, 9, 255], 1))
            .expect("an opacity map always binds");
        assert_eq!(mask.channel, ChannelSelect::R);
    }

    #[test]
    fn a_color_map_replaces_the_color_it_is_connected_to() {
        let base = TextureUse {
            material: 0,
            map: MaterialMap::BaseColor,
            emission_factor: 1.0,
        };
        let pixels = image(1, 1, [200, 100, 50, 255], 3);
        let binding = base.binding(Path::new("a.png"), &pixels).expect("binds");
        assert_eq!(binding.base_color, Some(Vec3::ONE));

        let emission = TextureUse {
            material: 0,
            map: MaterialMap::Emission,
            emission_factor: 3.0,
        };
        let binding = emission
            .binding(Path::new("e.png"), &pixels)
            .expect("binds");
        assert_eq!(binding.emissive, Some(Vec3::splat(3.0)));
    }

    #[test]
    fn a_glossiness_map_switches_the_material_to_smoothness() {
        let gloss = TextureUse {
            material: 0,
            map: MaterialMap::Glossiness,
            emission_factor: 1.0,
        };
        let binding = gloss
            .binding(
                Path::new("metal_gloss.png"),
                &image(1, 1, [1, 1, 1, 255], 1),
            )
            .expect("binds");
        assert_eq!(binding.slot, TextureSlot::Roughness);
        assert_eq!(binding.workflow, Some(RoughnessWorkflow::Smoothness));
    }
}
