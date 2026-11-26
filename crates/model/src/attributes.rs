//! Per-vertex data the renderer does not draw: custom attributes, and the form a
//! vertex attribute was stored in.
//!
//! A glTF file may carry application attributes beside the standard ones, each
//! named with a leading underscore (`_CAD_FACE`, `_PALETTE`). They are ids and
//! labels more often than measurements, so they are carried *exactly* — the
//! source's own bits, one 32-bit word per component — and never blended: the
//! optimizer keeps vertices whose values differ apart (a seam, like a UV seam),
//! and an export writes each back in the component type it came in.

/// The component type of an attribute's values, as glTF names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComponentType {
    I8,
    U8,
    I16,
    U16,
    U32,
    F32,
}

impl ComponentType {
    /// The glTF `componentType` code.
    pub fn code(self) -> u32 {
        match self {
            ComponentType::I8 => 5120,
            ComponentType::U8 => 5121,
            ComponentType::I16 => 5122,
            ComponentType::U16 => 5123,
            ComponentType::U32 => 5125,
            ComponentType::F32 => 5126,
        }
    }

    /// The type a glTF `componentType` code names, or `None` for one glTF does
    /// not define.
    pub fn from_code(code: u32) -> Option<Self> {
        Some(match code {
            5120 => ComponentType::I8,
            5121 => ComponentType::U8,
            5122 => ComponentType::I16,
            5123 => ComponentType::U16,
            5125 => ComponentType::U32,
            5126 => ComponentType::F32,
            _ => return None,
        })
    }

    /// Bytes per component.
    pub fn size(self) -> usize {
        match self {
            ComponentType::I8 | ComponentType::U8 => 1,
            ComponentType::I16 | ComponentType::U16 => 2,
            ComponentType::U32 | ComponentType::F32 => 4,
        }
    }

    /// A stable lower-case name for reports and logs (`u16`, `f32`).
    pub fn label(self) -> &'static str {
        match self {
            ComponentType::I8 => "i8",
            ComponentType::U8 => "u8",
            ComponentType::I16 => "i16",
            ComponentType::U16 => "u16",
            ComponentType::U32 => "u32",
            ComponentType::F32 => "f32",
        }
    }
}

/// How an attribute's values were stored: their component type, how many
/// components make one value (1 to 4), and whether an integer type is
/// normalized (read as a fraction of its range).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AttributeFormat {
    pub component: ComponentType,
    pub components: u8,
    pub normalized: bool,
}

impl AttributeFormat {
    /// Four 32-bit floats: what a colour the optimizer made itself is written as
    /// when the source stored none.
    pub const RGBA_F32: Self = Self {
        component: ComponentType::F32,
        components: 4,
        normalized: false,
    };

    /// Four 16-bit normalized integers: what Blender writes a colour as, exact
    /// for any value that is a multiple of 1/65535 — so a palette index stored
    /// as `index / 255` survives.
    pub const RGBA_U16_NORMALIZED: Self = Self {
        component: ComponentType::U16,
        components: 4,
        normalized: true,
    };

    /// Bytes per vertex.
    pub fn size(self) -> usize {
        self.component.size() * usize::from(self.components)
    }

    /// `u16x4n`-style shorthand for reports.
    pub fn label(self) -> String {
        let normalized = if self.normalized { "n" } else { "" };
        format!("{}x{}{normalized}", self.component.label(), self.components)
    }
}

/// One custom vertex attribute, parallel to `ModelData::vertices`.
///
/// `values` holds `format.components` words per vertex, each the component's
/// own value: an `f32`'s bits, an unsigned integer as itself, a signed one
/// sign-extended to 32 bits. Nothing is converted, so nothing rounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomAttribute {
    /// The name as the file wrote it, leading underscore included.
    pub name: String,
    pub format: AttributeFormat,
    pub values: Vec<u32>,
}

impl CustomAttribute {
    /// Components per vertex.
    pub fn width(&self) -> usize {
        usize::from(self.format.components)
    }

    /// Vertex `vertex`'s words.
    pub fn value(&self, vertex: usize) -> &[u32] {
        let width = self.width();
        &self.values[vertex * width..(vertex + 1) * width]
    }

    /// How many vertices it covers.
    pub fn vertex_count(&self) -> usize {
        self.values.len() / self.width().max(1)
    }
}

/// What a glTF source carried that `ModelData` has no field of its own for,
/// verbatim, so that a glb export writes it back unchanged: the `extras` on
/// the asset, the scene, the nodes and their meshes, and every material with
/// the textures, images and samplers it uses. JSON fragments are kept as the
/// file's own text, never parsed into this crate's types.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GltfSource {
    /// `asset.copyright`, when the file declared one.
    pub copyright: Option<String>,
    /// `asset.extras`, as JSON text.
    pub asset_extras: Option<String>,
    /// The scene's name and `extras`.
    pub scene_name: Option<String>,
    pub scene_extras: Option<String>,
    /// Per node of `ModelData::nodes`, in the same order.
    pub nodes: Vec<GltfNode>,
    /// Per material of `ModelData::materials`, the material object as JSON text,
    /// its texture indices into `textures`.
    pub materials: Vec<String>,
    /// The file's `textures`, `samplers` and `images`, each as JSON text, the
    /// indices between them unchanged. An image stored in the file's binary
    /// carries its bytes; one that names a URI keeps it.
    pub textures: Vec<String>,
    pub samplers: Vec<String>,
    pub images: Vec<GltfImage>,
    /// `extensionsUsed` and `extensionsRequired`, kept for the materials that
    /// use them.
    pub extensions_used: Vec<String>,
    pub extensions_required: Vec<String>,
    /// How `COLOR_0` was stored, `None` when no primitive had one: a colour is
    /// written back in the type it came in, never narrower.
    pub color_format: Option<AttributeFormat>,
    /// Whether any primitive carried authored tangents.
    pub tangents: bool,
    /// Per source vertex (what `ModelData::corner_to_logical` names), its values
    /// as the file stored them — see [`GltfVertices`].
    pub vertices: GltfVertices,
}

/// Each glTF vertex's values as the file stored them: in its node's own space,
/// with the file's UV orientation.
///
/// Import bakes a vertex into world space and turns V the other way up, and
/// neither is exact in floating point once undone. An export writes these
/// instead for every vertex the stack left bit-for-bit as it was, so a file
/// that went through unchanged comes out with the values it went in with; only
/// a vertex an operation moved is transformed back.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GltfVertices {
    pub positions: Vec<glam::Vec3>,
    /// `None` where the primitive had no `NORMAL` (import derived a flat one).
    pub normals: Vec<Option<glam::Vec3>>,
    /// `None` where the primitive had no `TANGENT`.
    pub tangents: Vec<Option<glam::Vec4>>,
    /// Per UV set, per vertex; `None` where the primitive lacked the set.
    pub uvs: Vec<Vec<Option<glam::Vec2>>>,
}

/// One node's glTF-only data.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GltfNode {
    /// The node's `extras`, as JSON text.
    pub extras: Option<String>,
    /// The name and `extras` of the mesh the node instanced, when it had one.
    pub mesh_name: Option<String>,
    pub mesh_extras: Option<String>,
    /// The node's `matrix`, when it gave one instead of translation, rotation
    /// and scale: written back as it was, rather than as its decomposition.
    pub matrix: Option<[f32; 16]>,
}

/// One glTF image: its JSON object without `bufferView`, and the bytes that
/// view held when the image was stored in the file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GltfImage {
    pub json: String,
    pub bytes: Option<Vec<u8>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_codes_round_trip() {
        for component in [
            ComponentType::I8,
            ComponentType::U8,
            ComponentType::I16,
            ComponentType::U16,
            ComponentType::U32,
            ComponentType::F32,
        ] {
            assert_eq!(ComponentType::from_code(component.code()), Some(component));
        }
        assert_eq!(ComponentType::from_code(5124), None, "glTF has no i32");
    }

    #[test]
    fn a_value_is_its_own_words() {
        let attribute = CustomAttribute {
            name: "_PAIR".to_owned(),
            format: AttributeFormat {
                component: ComponentType::U16,
                components: 2,
                normalized: false,
            },
            values: vec![1, 2, 3, 4, 5, 6],
        };
        assert_eq!(attribute.vertex_count(), 3);
        assert_eq!(attribute.value(1), [3, 4]);
        assert_eq!(attribute.format.label(), "u16x2");
        assert_eq!(attribute.format.size(), 4);
    }
}
