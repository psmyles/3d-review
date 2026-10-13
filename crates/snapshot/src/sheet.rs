//! Laying views out as a labelled grid: one row per subject, one column per view.

use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use image::{Rgba, RgbaImage, imageops};

const FONT: &[u8] = include_bytes!("../../../assets/fonts/JetBrainsMono.ttf");
const LABEL_HEIGHT: u32 = 26;
const LABEL_PX: f32 = 16.0;
const GAP: u32 = 4;
const SHEET_BG: Rgba<u8> = Rgba([12, 12, 12, 255]);
const LABEL_FG: [f32; 3] = [235.0, 235.0, 235.0];

/// One row: its label and its images, left to right.
pub struct Row {
    pub label: String,
    pub cells: Vec<RgbaImage>,
}

/// Compose `rows` under a header naming each column.
pub fn compose(columns: &[String], rows: &[Row]) -> RgbaImage {
    let font = FontRef::try_from_slice(FONT).ok();
    let cell = rows
        .iter()
        .flat_map(|row| row.cells.first())
        .map(|image| (image.width(), image.height()))
        .next()
        .unwrap_or((1, 1));
    let count = columns.len().max(1) as u32;
    let width = count * cell.0 + (count + 1) * GAP;
    let height = LABEL_HEIGHT + rows.len() as u32 * (LABEL_HEIGHT + cell.1 + GAP) + GAP;
    let mut sheet = RgbaImage::from_pixel(width, height, SHEET_BG);

    for (column, name) in columns.iter().enumerate() {
        let x = GAP + column as u32 * (cell.0 + GAP);
        label(&mut sheet, font.as_ref(), name, x, 4);
    }
    for (index, row) in rows.iter().enumerate() {
        let top = LABEL_HEIGHT + index as u32 * (LABEL_HEIGHT + cell.1 + GAP);
        label(&mut sheet, font.as_ref(), &row.label, GAP, top + 4);
        for (column, image) in row.cells.iter().enumerate() {
            let x = GAP + column as u32 * (cell.0 + GAP);
            imageops::replace(
                &mut sheet,
                image,
                i64::from(x),
                i64::from(top + LABEL_HEIGHT),
            );
        }
    }
    sheet
}

fn label(sheet: &mut RgbaImage, font: Option<&FontRef<'_>>, text: &str, x: u32, y: u32) {
    let Some(font) = font else {
        return;
    };
    let scale = PxScale::from(LABEL_PX);
    let scaled = font.as_scaled(scale);
    let mut caret = x as f32;
    let baseline = y as f32 + scaled.ascent();
    for ch in text.chars() {
        let glyph = scaled.scaled_glyph(ch);
        let advance = scaled.h_advance(glyph.id);
        let glyph = glyph
            .id
            .with_scale_and_position(scale, point(caret, baseline));
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, coverage| {
                let px = bounds.min.x as i64 + i64::from(gx);
                let py = bounds.min.y as i64 + i64::from(gy);
                if px < 0
                    || py < 0
                    || px >= i64::from(sheet.width())
                    || py >= i64::from(sheet.height())
                {
                    return;
                }
                let pixel = sheet.get_pixel_mut(px as u32, py as u32);
                for channel in 0..3 {
                    let under = f32::from(pixel[channel]);
                    pixel[channel] = (under + (LABEL_FG[channel] - under) * coverage) as u8;
                }
            });
        }
        caret += advance;
    }
}
