//! The reader over a small glb built here, byte by byte, so every case it
//! claims to handle is in the file: a TRS hierarchy, a mirrored node, two
//! materials, a 16-bit normalized colour, an integer custom attribute, two UV
//! sets, a primitive with no normals, a triangle strip, and extras everywhere.

use std::path::Path;

use glam::{Quat, Vec2, Vec3, Vec4};
use review_model::{ComponentType, NodeKind};
use serde_json::json;

use super::*;

/// A glb under construction: accessors appended to one binary buffer.
#[derive(Default)]
pub(crate) struct Glb {
    pub bin: Vec<u8>,
    pub views: Vec<serde_json::Value>,
    pub accessors: Vec<serde_json::Value>,
}

impl Glb {
    /// Append `bytes` as a buffer view and an accessor over it; its index.
    pub fn accessor(
        &mut self,
        bytes: &[u8],
        component: u32,
        kind: &str,
        count: usize,
        normalized: bool,
        extra: serde_json::Value,
    ) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        self.views.push(json!({
            "buffer": 0,
            "byteOffset": self.bin.len(),
            "byteLength": bytes.len(),
        }));
        self.bin.extend_from_slice(bytes);
        let mut accessor = json!({
            "bufferView": self.views.len() - 1,
            "componentType": component,
            "type": kind,
            "count": count,
        });
        if normalized {
            accessor["normalized"] = json!(true);
        }
        if let (Some(target), Some(extra)) = (accessor.as_object_mut(), extra.as_object()) {
            target.extend(extra.clone());
        }
        self.accessors.push(accessor);
        self.accessors.len() - 1
    }

    pub fn floats(&mut self, values: &[f32], kind: &str, count: usize) -> usize {
        let bytes: Vec<u8> = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let extra = if kind == "VEC3" {
            // POSITION needs its bounds.
            let mut min = [f32::MAX; 3];
            let mut max = [f32::MIN; 3];
            for point in values.chunks(3) {
                for axis in 0..3 {
                    min[axis] = min[axis].min(point[axis]);
                    max[axis] = max[axis].max(point[axis]);
                }
            }
            json!({ "min": min, "max": max })
        } else {
            json!({})
        };
        self.accessor(&bytes, 5126, kind, count, false, extra)
    }

    pub fn u16s(&mut self, values: &[u16], kind: &str, count: usize, normalized: bool) -> usize {
        let bytes: Vec<u8> = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        self.accessor(&bytes, 5123, kind, count, normalized, json!({}))
    }

    /// The file: `document` (without `accessors`, `bufferViews`, `buffers`,
    /// which are filled in) and the binary chunk.
    pub fn finish(mut self, mut document: serde_json::Value) -> Vec<u8> {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        document["accessors"] = json!(self.accessors);
        document["bufferViews"] = json!(self.views);
        document["buffers"] = json!([{ "byteLength": self.bin.len() }]);
        let mut text = serde_json::to_vec(&document).unwrap();
        while !text.len().is_multiple_of(4) {
            text.push(b' ');
        }
        let total = 12 + 8 + text.len() + 8 + self.bin.len();
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(text.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&text);
        out.extend_from_slice(&(self.bin.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&self.bin);
        out
    }
}

/// The test scene. `parent` at (1, 2, 3) carries `part` (a quad of two
/// materials and an integer `_CAD_FACE`) and `mirror` (one strip triangle,
/// scaled -1 in X, with no normals).
pub(crate) fn scene() -> Vec<u8> {
    let mut glb = Glb::default();
    // The quad: two triangles in the XY plane, 0..1.
    let quad = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0];
    let position = glb.floats(&quad, "VEC3", 4);
    let normal = glb.floats(&[0.0, 0.0, 1.0].repeat(4), "VEC3", 4);
    let uv0 = glb.floats(&[0.0, 0.0, 1.0, 0.0, 1.0, 0.25, 0.0, 0.25], "VEC2", 4);
    let uv1 = glb.floats(&[0.5; 8], "VEC2", 4);
    // R holds a palette index over 255, exactly as Blender writes 16 bits.
    let palette = |index: u16| ((f32::from(index) / 255.0) * 65535.0).round() as u16;
    let color = glb.u16s(
        &[
            palette(3),
            0,
            0,
            65535,
            palette(3),
            0,
            0,
            65535,
            palette(9),
            0,
            0,
            65535,
            palette(9),
            0,
            0,
            65535,
        ],
        "VEC4",
        4,
        true,
    );
    let cad = glb.u16s(&[7, 7, 40000, 40000], "SCALAR", 4, false);
    let first = glb.u16s(&[0, 1, 2], "SCALAR", 3, false);
    let second = glb.u16s(&[0, 2, 3], "SCALAR", 3, false);
    // The strip: four points making two triangles, the second wound back.
    let strip = glb.floats(
        &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0],
        "VEC3",
        4,
    );

    let attributes = json!({
        "POSITION": position, "NORMAL": normal, "TEXCOORD_0": uv0, "TEXCOORD_1": uv1,
        "COLOR_0": color, "_CAD_FACE": cad,
    });
    glb.finish(json!({
        "asset": { "version": "2.0", "generator": "test", "copyright": "nobody", "extras": { "kit": 1 } },
        "scene": 0,
        "scenes": [{ "name": "Kit", "nodes": [2], "extras": { "scene": true } }],
        "nodes": [
            { "name": "part", "mesh": 0, "rotation": [0.0, 0.0, 0.70710677, 0.70710677],
              "extras": { "part": "weapon", "hp": 40.0 } },
            { "name": "mirror", "mesh": 1, "scale": [-1.0, 1.0, 1.0] },
            { "name": "parent", "translation": [1.0, 2.0, 3.0], "children": [0, 1] },
        ],
        "meshes": [
            { "name": "part_mesh", "extras": { "lod": "fine" }, "primitives": [
                { "attributes": attributes, "indices": first, "material": 1 },
                { "attributes": attributes, "indices": second, "material": 0 },
            ] },
            { "name": "strip", "primitives": [
                { "attributes": { "POSITION": strip }, "mode": 5 },
            ] },
        ],
        "materials": [
            { "name": "steel", "pbrMetallicRoughness": { "baseColorFactor": [0.5, 0.25, 0.125, 1.0],
              "metallicFactor": 1.0, "roughnessFactor": 0.25 } },
            { "name": "paint", "extras": { "palette": 3 }, "emissiveFactor": [1.0, 0.5, 0.0] },
        ],
    }))
}

fn read(bytes: &[u8]) -> (ModelData, Vec<String>) {
    load_bytes(bytes, Path::new("kit.glb"), None, &|_| {}, None).expect("the test glb reads")
}

#[test]
fn the_hierarchy_comes_parents_first_with_each_nodes_own_transform() {
    let (model, warnings) = read(&scene());
    assert!(warnings.is_empty(), "{warnings:?}");
    let names: Vec<&str> = model.nodes.iter().map(|node| node.name.as_str()).collect();
    assert_eq!(names, ["parent", "part", "mirror"]);
    assert_eq!(model.nodes[0].parent, None);
    assert_eq!(model.nodes[1].parent, Some(0));
    assert_eq!(model.nodes[2].parent, Some(0));
    assert_eq!(model.nodes[0].kind, NodeKind::Empty);
    assert_eq!(model.nodes[1].kind, NodeKind::Mesh);
    assert_eq!(model.nodes[1].mesh_part, Some(0));
    assert_eq!(model.nodes[2].mesh_part, Some(1));
    // The rotation as stored, not renormalized.
    assert_eq!(
        model.nodes[1].rest_local.rotation,
        Quat::from_array([0.0, 0.0, 0.70710677, 0.70710677])
    );
    assert_eq!(
        model.nodes[0].rest_local.translation,
        Vec3::new(1.0, 2.0, 3.0)
    );
    assert_eq!(model.stats.source_axes, Some(AXES));
    assert_eq!(model.stats.source_unit_meters, 1.0);
}

#[test]
fn geometry_is_baked_into_world_space_corner_by_corner() {
    let (model, _) = read(&scene());
    // Quad (2 triangles) + strip (2 triangles), one vertex per corner.
    assert_eq!(model.stats.triangle_count, 4);
    assert_eq!(model.vertices.len(), 12);
    // Each primitive's vertices are its own, as an engine uploads them, even
    // where two primitives share accessors: 4 + 4 for the quad, 4 for the strip.
    assert_eq!(model.stats.vertex_count, 12);
    assert_eq!(model.corner_to_logical[..6], [0, 1, 2, 4, 6, 7]);
    // `part` turns a quarter about Z under `parent`'s (1, 2, 3): its (1, 0, 0)
    // corner lands at (1, 3, 3).
    let corner = model.vertices[1].position;
    assert!(
        (corner - Vec3::new(1.0, 3.0, 3.0)).length() < 1e-6,
        "{corner}"
    );
    // Its normal (0, 0, 1) is unmoved by a turn about Z.
    assert!((model.vertices[1].normal - Vec3::Z).length() < 1e-6);
    assert_eq!(
        model.nodes[1].source_vertex_count, 8,
        "both primitives' vertices"
    );
}

#[test]
fn a_mirrored_node_keeps_its_front_faces_in_front() {
    let (model, _) = read(&scene());
    // The strip under X = -1: its triangles' winding is reversed so the
    // geometric normal still points the way the authored surface faces (+Z).
    for triangle in 2..4 {
        let [a, b, c] = [0, 1, 2].map(|k| model.vertices[model.indices[triangle * 3 + k] as usize]);
        let geometric = (b.position - a.position).cross(c.position - a.position);
        assert!(geometric.z > 0.0, "triangle {triangle} faces {geometric}");
        // No NORMAL: each corner gets the face's own.
        assert!((a.normal - Vec3::Z).length() < 1e-6, "{}", a.normal);
    }
}

#[test]
fn colours_keep_their_format_and_palette_indices_survive() {
    let (model, _) = read(&scene());
    let source = model.gltf.as_deref().expect("the side table");
    let format = source.color_format.expect("COLOR_0 was there");
    assert_eq!(format.component, ComponentType::U16);
    assert_eq!(format.components, 4);
    assert!(format.normalized);
    for vertex in &model.vertices[..6] {
        let index = (vertex.vertex_color.x * 255.0).round();
        assert!(index == 3.0 || index == 9.0, "{}", vertex.vertex_color);
    }
    // The strip carried none: white.
    assert_eq!(model.vertices[6].vertex_color, Vec4::ONE);
}

#[test]
fn custom_attributes_come_through_exact() {
    let (model, _) = read(&scene());
    let [cad] = model.custom_attributes.as_slice() else {
        panic!("one custom attribute: {:?}", model.custom_attributes);
    };
    assert_eq!(cad.name, "_CAD_FACE");
    assert_eq!(cad.format.component, ComponentType::U16);
    assert_eq!(cad.format.components, 1);
    assert_eq!(cad.values.len(), model.vertices.len());
    // First triangle 0,1,2 then 0,2,3; the strip has none.
    assert_eq!(cad.values, [7, 7, 40000, 7, 40000, 40000, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn uvs_turn_v_up_and_both_sets_are_kept() {
    let (model, _) = read(&scene());
    assert_eq!(model.stats.uv_set_count, 2);
    assert_eq!(model.uv_set_names, ["TEXCOORD_0", "TEXCOORD_1"]);
    assert_eq!(model.uv_channels.len(), 2, "set 0 mirrored in");
    // (1, 0.25) stored V-down is (1, 0.75) V-up.
    assert_eq!(model.vertices[2].uv, Vec2::new(1.0, 0.75));
    assert_eq!(model.uv_channels[0][2], Vec2::new(1.0, 0.75));
    assert_eq!(model.uv_channels[1][2], Vec2::new(0.5, 0.5));
    // The strip has no UVs.
    assert_eq!(model.uv_channels[1][7], Vec2::ZERO);
    // The originals are kept as stored, for the writer.
    let source = model.gltf.as_deref().unwrap();
    assert_eq!(source.vertices.uvs[0][2], Some(Vec2::new(1.0, 0.25)));
    assert_eq!(source.vertices.uvs[0][9], None, "the strip's vertices");
}

#[test]
fn materials_and_extras_are_kept() {
    let (model, _) = read(&scene());
    assert_eq!(model.materials.len(), 2);
    assert_eq!(model.materials[0].name, "steel");
    assert_eq!(model.materials[0].base_color, Vec3::new(0.5, 0.25, 0.125));
    assert_eq!(model.materials[0].smoothness, 0.75);
    assert_eq!(model.materials[0].metallic, 1.0);
    assert_eq!(model.materials[1].emissive, Vec3::new(1.0, 0.5, 0.0));
    // Primitive order: material 1 first, then 0; the strip has none.
    assert_eq!(model.triangles.material, [1, 0, u32::MAX, u32::MAX]);
    assert_eq!(model.triangles.node, [1, 1, 2, 2]);

    let source = model.gltf.as_deref().unwrap();
    assert_eq!(source.copyright.as_deref(), Some("nobody"));
    assert_eq!(source.asset_extras.as_deref(), Some(r#"{"kit":1}"#));
    assert_eq!(source.scene_name.as_deref(), Some("Kit"));
    assert_eq!(
        source.nodes[1].extras.as_deref(),
        Some(r#"{"hp":40.0,"part":"weapon"}"#)
    );
    assert_eq!(source.nodes[1].mesh_name.as_deref(), Some("part_mesh"));
    assert_eq!(
        source.nodes[1].mesh_extras.as_deref(),
        Some(r#"{"lod":"fine"}"#)
    );
    assert!(
        source.materials[1].contains(r#""palette":3"#),
        "{}",
        source.materials[1]
    );
}

#[test]
fn the_file_is_recognised_by_its_first_bytes() {
    assert!(sniff(&scene()));
    assert!(sniff(b"  {\"asset\":{}}"));
    assert!(!sniff(b"Kaydara FBX Binary  \0"));
}
