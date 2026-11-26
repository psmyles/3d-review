//! The palette the Vertex Color panel's palette-index mode looks indices up in,
//! read from a file the user picks: a JSON list of colours, or an image with
//! one pixel per entry.
//!
//! JSON takes a list, or an object holding one under `colors` or `palette`.
//! Each entry is a `"#rrggbb"` string, an `[r, g, b]` list or an `{r, g, b}`
//! object; numbers are 0-255 if any is above 1, else 0-1. Colours are sRGB, as
//! a colour picker shows them, unless the object says `"linear": true`.
//!
//! An image of at most 256 pixels is read pixel by pixel, row by row (a strip,
//! or a 16 x 16 grid); a larger one by its first row, or its first column when
//! it is taller than wide.

use std::path::Path;

use review_model::color::linear_to_srgb;

use crate::{App, keys};

/// The most entries an index ÷ 255 can name.
pub(crate) const MAX_ENTRIES: usize = 256;

/// Why a palette file could not be read.
#[derive(Debug, PartialEq)]
pub(crate) enum PaletteError {
    /// Not JSON, or JSON with no list of colours in it.
    NotAPalette,
    /// Entry `n` (0-based) is not a colour.
    BadEntry(usize),
    /// No entries at all.
    Empty,
    /// More entries than an index can name.
    TooMany(usize),
    /// Not an image this build can decode.
    NotAnImage,
}

/// Read a palette from `bytes`, a JSON file when `json`, an image otherwise.
pub(crate) fn parse(bytes: &[u8], json: bool) -> Result<Vec<[f32; 3]>, PaletteError> {
    let colors = if json {
        parse_json(bytes)?
    } else {
        parse_image(bytes)?
    };
    match colors.len() {
        0 => Err(PaletteError::Empty),
        n if n > MAX_ENTRIES => Err(PaletteError::TooMany(n)),
        _ => Ok(colors),
    }
}

fn parse_json(bytes: &[u8]) -> Result<Vec<[f32; 3]>, PaletteError> {
    use serde_json::Value;
    let value: Value = serde_json::from_slice(bytes).map_err(|_| PaletteError::NotAPalette)?;
    let (list, linear) = match &value {
        Value::Array(list) => (list, false),
        Value::Object(map) => {
            let list = ["colors", "palette"]
                .iter()
                .find_map(|key| map.get(*key).and_then(Value::as_array))
                .ok_or(PaletteError::NotAPalette)?;
            let linear = map.get("linear").and_then(Value::as_bool).unwrap_or(false);
            (list, linear)
        }
        _ => return Err(PaletteError::NotAPalette),
    };
    // Each entry's three components as written, scaled below.
    let mut raw: Vec<[f64; 3]> = Vec::with_capacity(list.len());
    let mut hex = Vec::with_capacity(list.len());
    for (index, entry) in list.iter().enumerate() {
        let bad = || PaletteError::BadEntry(index);
        let components = match entry {
            Value::String(text) => {
                hex.push(index);
                parse_hex(text).ok_or_else(bad)?.map(f64::from)
            }
            Value::Array(parts) => {
                let number = |k: usize| parts.get(k).and_then(Value::as_f64).ok_or_else(bad);
                [number(0)?, number(1)?, number(2)?]
            }
            Value::Object(parts) => {
                let number = |k: &str| parts.get(k).and_then(Value::as_f64).ok_or_else(bad);
                [number("r")?, number("g")?, number("b")?]
            }
            _ => return Err(bad()),
        };
        if components.iter().any(|c| !c.is_finite() || *c < 0.0) {
            return Err(bad());
        }
        raw.push(components);
    }
    // Numbers are 0-255 if any written number is above 1 (hex strings always
    // are, and do not count towards the guess).
    let bytes_scale = raw
        .iter()
        .enumerate()
        .any(|(index, c)| !hex.contains(&index) && c.iter().any(|&v| v > 1.0));
    Ok(raw
        .iter()
        .enumerate()
        .map(|(index, c)| {
            let scale = if hex.contains(&index) || bytes_scale {
                255.0
            } else {
                1.0
            };
            c.map(|v| {
                let v = (v / scale).min(1.0) as f32;
                if linear { linear_to_srgb(v) } else { v }
            })
        })
        .collect())
}

/// `#rrggbb` (or `rrggbb`, with or without a trailing alpha byte) as bytes.
fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let digits = text.trim().trim_start_matches('#');
    if !(digits.len() == 6 || digits.len() == 8) || !digits.is_ascii() {
        return None;
    }
    let byte = |k: usize| u8::from_str_radix(&digits[k * 2..k * 2 + 2], 16).ok();
    Some([byte(0)?, byte(1)?, byte(2)?])
}

fn parse_image(bytes: &[u8]) -> Result<Vec<[f32; 3]>, PaletteError> {
    let image = image::load_from_memory(bytes)
        .map_err(|_| PaletteError::NotAnImage)?
        .to_rgb8();
    let (width, height) = image.dimensions();
    let color = |x: u32, y: u32| image.get_pixel(x, y).0.map(|v| f32::from(v) / 255.0);
    Ok(if (width as usize) * (height as usize) <= MAX_ENTRIES {
        (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| color(x, y))
            .collect()
    } else if height > width {
        (0..height).map(|y| color(0, y)).collect()
    } else {
        (0..width).map(|x| color(x, 0)).collect()
    })
}

impl App {
    /// Ask for a palette file, when the Vertex Color panel's Load button asked.
    pub(crate) fn ask_for_palette_if_requested(&mut self) {
        if std::mem::take(&mut self.ui.vertex_colors.palette_requested) {
            self.ask(crate::dialog::Dialog::LoadPalette);
        }
    }

    /// Read the remembered palette file again at launch, saying nothing unless
    /// it cannot be read.
    pub(crate) fn restore_palette(&mut self) {
        if let Some(path) = self.ui.vertex_colors.palette_file.clone() {
            self.read_palette(&path, false);
        }
    }

    /// Read the palette in `path` into the Vertex Color panel, or say why not
    /// and leave the one in use.
    pub(crate) fn load_palette(&mut self, path: &Path) {
        self.read_palette(path, true);
    }

    fn read_palette(&mut self, path: &Path, announce: bool) {
        let label = crate::loading::file_label(path);
        let json = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
        let parsed = std::fs::read(path)
            .map_err(|_| None)
            .and_then(|bytes| parse(&bytes, json).map_err(Some));
        match parsed {
            Ok(colors) => {
                log::info!("palette {}: {} entries", path.display(), colors.len());
                if announce {
                    self.notifications
                        .success(keys::app_notifications::palette_loaded(
                            colors.len() as f64,
                            label,
                        ));
                }
                self.ui.vertex_colors.palette = colors;
                self.ui.vertex_colors.palette_file = Some(path.to_path_buf());
            }
            Err(error) => {
                log::warn!("palette {} not read: {error:?}", path.display());
                let why = match error {
                    None => review_localization::tr(keys::app_notifications::PALETTE_WHY_FILE)
                        .into_owned(),
                    Some(PaletteError::NotAPalette) => {
                        review_localization::tr(keys::app_notifications::PALETTE_WHY_JSON)
                            .into_owned()
                    }
                    Some(PaletteError::BadEntry(index)) => {
                        keys::app_notifications::palette_why_entry(index as f64)
                    }
                    Some(PaletteError::Empty) => {
                        review_localization::tr(keys::app_notifications::PALETTE_WHY_EMPTY)
                            .into_owned()
                    }
                    Some(PaletteError::TooMany(count)) => {
                        keys::app_notifications::palette_why_too_many(count as f64)
                    }
                    Some(PaletteError::NotAnImage) => {
                        review_localization::tr(keys::app_notifications::PALETTE_WHY_IMAGE)
                            .into_owned()
                    }
                };
                self.notifications
                    .error(keys::app_notifications::palette_not_read(label, why));
            }
        }
        self.redraw.requested = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_lists_hex_bytes_and_unit_numbers() {
        let hex = parse(br##"["#ff0000", "00ff00"]"##, true).unwrap();
        assert_eq!(hex, [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        let bytes = parse(br#"{"colors": [[255, 128, 0], [0, 0, 1]]}"#, true).unwrap();
        assert!((bytes[0][1] - 128.0 / 255.0).abs() < 1e-6);
        assert!((bytes[1][2] - 1.0 / 255.0).abs() < 1e-6);
        let unit = parse(br#"{"palette": [{"r": 0.5, "g": 1, "b": 0}]}"#, true).unwrap();
        assert_eq!(unit, [[0.5, 1.0, 0.0]]);
        let linear = parse(
            br#"{"linear": true, "colors": [[0.2140, 0.2140, 0.2140]]}"#,
            true,
        )
        .unwrap();
        assert!((linear[0][0] - 0.5).abs() < 1e-3, "{linear:?}");
    }

    #[test]
    fn what_is_not_a_palette_says_why() {
        assert_eq!(parse(b"{", true), Err(PaletteError::NotAPalette));
        assert_eq!(parse(br#"{"x": []}"#, true), Err(PaletteError::NotAPalette));
        assert_eq!(
            parse(br##"["#ff0000", "red"]"##, true),
            Err(PaletteError::BadEntry(1))
        );
        assert_eq!(parse(b"[]", true), Err(PaletteError::Empty));
        let many = format!("[{}]", vec!["\"#000000\""; 257].join(","));
        assert_eq!(
            parse(many.as_bytes(), true),
            Err(PaletteError::TooMany(257))
        );
        assert_eq!(parse(b"not an image", false), Err(PaletteError::NotAnImage));
    }

    #[test]
    fn an_image_is_read_pixel_by_pixel_or_by_its_first_row() {
        let encode = |image: image::RgbImage| {
            let mut bytes = Vec::new();
            image
                .write_to(
                    &mut std::io::Cursor::new(&mut bytes),
                    image::ImageFormat::Png,
                )
                .unwrap();
            bytes
        };
        // A 16 x 16 grid: every pixel, row by row.
        let grid = image::RgbImage::from_fn(16, 16, |x, y| image::Rgb([x as u8, y as u8, 0]));
        let colors = parse(&encode(grid), false).unwrap();
        assert_eq!(colors.len(), 256);
        assert!(
            (colors[17][0] - 1.0 / 255.0).abs() < 1e-6
                && (colors[17][1] - 1.0 / 255.0).abs() < 1e-6
        );
        // A wide strip scaled up: its first row.
        let strip = image::RgbImage::from_fn(64, 8, |x, _| image::Rgb([x as u8, 0, 0]));
        assert_eq!(parse(&encode(strip), false).unwrap().len(), 64);
    }
}
