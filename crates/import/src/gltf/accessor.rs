//! Reading a glTF accessor's values, in any component type.
//!
//! The `gltf` crate's typed readers assume each attribute's usual type — a
//! position is read as `f32` whatever the accessor says — which misreads a
//! quantized mesh (`KHR_mesh_quantization`) and has nothing to offer a custom
//! attribute at all. This reads what the accessor declares: every value as its
//! raw 32-bit word ([`words`]), or as the float it stands for ([`floats`]),
//! with sparse substitution applied either way.

use gltf::Accessor;
use gltf::accessor::{DataType, Dimensions};
use review_model::{AttributeFormat, ComponentType};

/// The buffers' bytes, by buffer index.
pub(super) type Buffers = [Vec<u8>];

/// Why an accessor could not be read: a sentence for the import's warning.
pub(super) type Unreadable = String;

/// How the accessor stores its values, or why it is not a vertex attribute
/// this reader handles (a matrix, or a component type glTF does not define).
pub(super) fn format(accessor: &Accessor<'_>) -> Result<AttributeFormat, Unreadable> {
    let component = match accessor.data_type() {
        DataType::I8 => ComponentType::I8,
        DataType::U8 => ComponentType::U8,
        DataType::I16 => ComponentType::I16,
        DataType::U16 => ComponentType::U16,
        DataType::U32 => ComponentType::U32,
        DataType::F32 => ComponentType::F32,
    };
    let components = match accessor.dimensions() {
        Dimensions::Scalar => 1,
        Dimensions::Vec2 => 2,
        Dimensions::Vec3 => 3,
        Dimensions::Vec4 => 4,
        other => return Err(format!("accessor {} is a {other:?}", accessor.index())),
    };
    Ok(AttributeFormat {
        component,
        components,
        normalized: accessor.normalized(),
    })
}

/// Every component of every element as its raw word: an `f32`'s bits, an
/// unsigned integer as itself, a signed one sign-extended. `count * components`
/// long.
pub(super) fn words(accessor: &Accessor<'_>, buffers: &Buffers) -> Result<Vec<u32>, Unreadable> {
    let format = format(accessor)?;
    let width = usize::from(format.components);
    let count = accessor.count();
    let mut out = vec![0u32; count * width];

    if let Some(view) = accessor.view() {
        let bytes = view_bytes(&view, buffers)?;
        let element = format.size();
        let stride = view.stride().unwrap_or(element);
        let offset = accessor.offset();
        read_elements(
            bytes,
            offset,
            stride,
            count,
            format,
            &mut out,
            accessor.index(),
        )?;
    }

    if let Some(sparse) = accessor.sparse() {
        let indices = sparse.indices();
        let index_bytes = view_bytes(&indices.view(), buffers)?;
        let index_format = AttributeFormat {
            component: match indices.index_type() {
                gltf::accessor::sparse::IndexType::U8 => ComponentType::U8,
                gltf::accessor::sparse::IndexType::U16 => ComponentType::U16,
                gltf::accessor::sparse::IndexType::U32 => ComponentType::U32,
            },
            components: 1,
            normalized: false,
        };
        let sparse_count = sparse.count();
        let mut targets = vec![0u32; sparse_count];
        read_elements(
            index_bytes,
            indices.offset(),
            index_format.size(),
            sparse_count,
            index_format,
            &mut targets,
            accessor.index(),
        )?;
        let values = sparse.values();
        let value_bytes = view_bytes(&values.view(), buffers)?;
        let mut replaced = vec![0u32; sparse_count * width];
        read_elements(
            value_bytes,
            values.offset(),
            format.size(),
            sparse_count,
            format,
            &mut replaced,
            accessor.index(),
        )?;
        for (slot, &target) in targets.iter().enumerate() {
            let target = target as usize;
            if target >= count {
                return Err(format!(
                    "accessor {}'s sparse index {target} is past its {count} elements",
                    accessor.index()
                ));
            }
            out[target * width..(target + 1) * width]
                .copy_from_slice(&replaced[slot * width..(slot + 1) * width]);
        }
    }
    Ok(out)
}

/// Every component as the float it stands for: a normalized integer as its
/// fraction of the range (signed ones clamped at -1, as the glTF spec has it),
/// any other integer as its value.
pub(super) fn floats(accessor: &Accessor<'_>, buffers: &Buffers) -> Result<Vec<f32>, Unreadable> {
    let format = format(accessor)?;
    let words = words(accessor, buffers)?;
    Ok(words
        .into_iter()
        .map(|word| to_float(word, format))
        .collect())
}

/// One raw word as the float it stands for under `format`.
pub(super) fn to_float(word: u32, format: AttributeFormat) -> f32 {
    let signed = word as i32 as f32;
    let unsigned = word as f32;
    match (format.component, format.normalized) {
        (ComponentType::F32, _) => f32::from_bits(word),
        (ComponentType::I8, true) => (signed / 127.0).max(-1.0),
        (ComponentType::U8, true) => unsigned / 255.0,
        (ComponentType::I16, true) => (signed / 32767.0).max(-1.0),
        (ComponentType::U16, true) => unsigned / 65535.0,
        (ComponentType::U32, true) => unsigned / 4_294_967_295.0,
        (ComponentType::I8 | ComponentType::I16, false) => signed,
        (ComponentType::U8 | ComponentType::U16 | ComponentType::U32, false) => unsigned,
    }
}

fn view_bytes<'b>(
    view: &gltf::buffer::View<'_>,
    buffers: &'b Buffers,
) -> Result<&'b [u8], Unreadable> {
    let buffer = buffers
        .get(view.buffer().index())
        .ok_or_else(|| format!("buffer {} was not loaded", view.buffer().index()))?;
    buffer
        .get(view.offset()..view.offset() + view.length())
        .ok_or_else(|| {
            format!(
                "buffer view {} runs past the end of buffer {} ({} bytes)",
                view.index(),
                view.buffer().index(),
                buffer.len()
            )
        })
}

/// Decode `count` elements of `format` starting at `offset`, `stride` bytes
/// apart, into `out` (`count * components` words).
fn read_elements(
    bytes: &[u8],
    offset: usize,
    stride: usize,
    count: usize,
    format: AttributeFormat,
    out: &mut [u32],
    accessor: usize,
) -> Result<(), Unreadable> {
    if count == 0 {
        return Ok(());
    }
    let width = usize::from(format.components);
    let size = format.component.size();
    let last = offset + stride * (count - 1) + format.size();
    if last > bytes.len() {
        return Err(format!(
            "accessor {accessor} reads {last} bytes of a {}-byte buffer view",
            bytes.len()
        ));
    }
    for element in 0..count {
        let start = offset + stride * element;
        for component in 0..width {
            let at = start + component * size;
            let raw = &bytes[at..at + size];
            out[element * width + component] = match format.component {
                ComponentType::I8 => raw[0] as i8 as i32 as u32,
                ComponentType::U8 => u32::from(raw[0]),
                ComponentType::I16 => i16::from_le_bytes([raw[0], raw[1]]) as i32 as u32,
                ComponentType::U16 => u32::from(u16::from_le_bytes([raw[0], raw[1]])),
                ComponentType::U32 | ComponentType::F32 => {
                    u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]])
                }
            };
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_integers_read_as_their_fraction() {
        let u16n = AttributeFormat {
            component: ComponentType::U16,
            components: 4,
            normalized: true,
        };
        assert_eq!(to_float(65535, u16n), 1.0);
        assert_eq!(to_float(0, u16n), 0.0);
        // A palette index stored as index / 255 survives the trip exactly.
        let stored = (7.0f32 / 255.0 * 65535.0).round() as u32;
        assert_eq!((to_float(stored, u16n) * 65535.0).round() as u32, stored);

        let i8n = AttributeFormat {
            component: ComponentType::I8,
            components: 1,
            normalized: true,
        };
        assert_eq!(to_float(-128i32 as u32, i8n), -1.0, "clamped, per the spec");
        assert_eq!(to_float(127, i8n), 1.0);

        let plain = AttributeFormat {
            component: ComponentType::I16,
            components: 1,
            normalized: false,
        };
        assert_eq!(to_float(-3i32 as u32, plain), -3.0);
    }

    #[test]
    fn elements_honour_offset_and_stride() {
        // Two u16 VEC2 elements, 8 bytes apart, starting 2 bytes in.
        let bytes = [0, 0, 1, 0, 2, 0, 9, 9, 9, 9, 3, 0, 4, 0, 9, 9];
        let format = AttributeFormat {
            component: ComponentType::U16,
            components: 2,
            normalized: false,
        };
        let mut out = [0u32; 4];
        read_elements(&bytes, 2, 8, 2, format, &mut out, 0).unwrap();
        assert_eq!(out, [1, 2, 3, 4]);
        assert!(read_elements(&bytes, 2, 8, 3, format, &mut [0; 6], 0).is_err());
    }
}
