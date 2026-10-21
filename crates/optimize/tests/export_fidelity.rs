//! Export round trips over scenes written here as ASCII FBX, each built to pin
//! exactly one thing the exporter has to give back: a blend shape's normal
//! deltas in a centimetre file, an authored tangent basis under a non-uniform or
//! mirroring node, each mesh's own UV and colour sets, a material that declares
//! no properties, display layers over a LOD chain, nested layered textures and
//! embedded image content.
//!
//! Every test writes its scene, imports it with its capture, runs a stack,
//! exports with the capture, and reads the written file back through the
//! vendored ufbx reader — and, where a re-import would normalize the question
//! away, reads the written ASCII itself.

#![cfg(all(has_meshopt, has_ufbxw))]

use std::path::{Path, PathBuf};

use glam::{Vec3, Vec4};
use review_model::extras::TextureKind;
use review_model::{ModelData, SourceExtras};
use review_optimize::{
    ExportOptions, FbxFormat, LodLevel, LodPackaging, LodParams, OpKind, OptStack,
    SimplifyAlgorithm, SimplifySettings, export_fbx,
};

mod common;

use common::{load_full, reload_full, run_with_extras, temp_dir};

const S: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// A whole ASCII FBX file. `unit_scale_factor` is FBX's centimetres per unit.
fn scene(unit_scale_factor: f64, objects: &str, connections: &str) -> String {
    format!(
        r#"; FBX 7.4.0 project file
FBXHeaderExtension:  {{
	FBXHeaderVersion: 1003
	FBXVersion: 7400
}}
GlobalSettings:  {{
	Version: 1000
	Properties70:  {{
		P: "UpAxis", "int", "Integer", "",1
		P: "UpAxisSign", "int", "Integer", "",1
		P: "FrontAxis", "int", "Integer", "",2
		P: "FrontAxisSign", "int", "Integer", "",1
		P: "CoordAxis", "int", "Integer", "",0
		P: "CoordAxisSign", "int", "Integer", "",1
		P: "UnitScaleFactor", "double", "Number", "",{unit_scale_factor}
		P: "OriginalUnitScaleFactor", "double", "Number", "",{unit_scale_factor}
	}}
}}
Objects:  {{
{objects}
}}
Connections:  {{
{connections}
}}
"#
    )
}

/// Four values per polygon vertex of a quad, flattened.
fn repeat4(values: &[f64]) -> String {
    std::iter::repeat_n(
        values
            .iter()
            .map(f64::to_string)
            .collect::<Vec<_>>()
            .join(","),
        4,
    )
    .collect::<Vec<_>>()
    .join(",")
}

/// One extra layer element of a quad: `kind` its FBX name (`LayerElementUV`),
/// `values_name` its array (`UV`), `index` its layer index.
fn layer(
    kind: &str,
    index: u32,
    name: &str,
    values_name: &str,
    values: &str,
    count: usize,
) -> String {
    format!(
        r#"		{kind}: {index} {{
			Version: 101
			Name: "{name}"
			MappingInformationType: "ByPolygonVertex"
			ReferenceInformationType: "Direct"
			{values_name}: *{count} {{
				a: {values}
			}}
		}}
"#
    )
}

/// A unit quad in the XY plane with normals and its `layers`, plus a `Layer`
/// block per entry of `bindings` (`(layer index, element kinds)`).
fn quad(id: u64, layers: &str, bindings: &[(u32, &[&str])]) -> String {
    let mut blocks = String::new();
    for (index, kinds) in bindings {
        blocks.push_str(&format!("\t\tLayer: {index} {{\n\t\t\tVersion: 100\n"));
        for kind in *kinds {
            blocks.push_str(&format!(
                "\t\t\tLayerElement:  {{\n\t\t\t\tType: \"{kind}\"\n\t\t\t\tTypedIndex: {index}\n\t\t\t}}\n"
            ));
        }
        blocks.push_str("\t\t}\n");
    }
    format!(
        r#"	Geometry: {id}, "Geometry::Quad{id}", "Mesh" {{
		Vertices: *12 {{
			a: 0,0,0,1,0,0,1,1,0,0,1,0
		}}
		PolygonVertexIndex: *4 {{
			a: 0,1,2,-4
		}}
		LayerElementNormal: 0 {{
			Version: 101
			Name: ""
			MappingInformationType: "ByPolygonVertex"
			ReferenceInformationType: "Direct"
			Normals: *12 {{
				a: 0,0,1,0,0,1,0,0,1,0,0,1
			}}
		}}
{layers}{blocks}	}}
"#
    )
}

/// The first UV set of a quad, named `name`.
fn uv_layer(index: u32, name: &str) -> String {
    layer("LayerElementUV", index, name, "UV", "0,0,1,0,1,1,0,1", 8)
}

/// A mesh node with `properties` lines in its `Properties70`.
fn model(id: u64, name: &str, properties: &str) -> String {
    format!(
        r#"	Model: {id}, "Model::{name}", "Mesh" {{
		Version: 232
		Properties70:  {{
{properties}		}}
	}}
"#
    )
}

fn scaling(scale: [f64; 3]) -> String {
    format!(
        "\t\t\tP: \"Lcl Scaling\", \"Lcl Scaling\", \"\", \"A\",{},{},{}\n",
        scale[0], scale[1], scale[2]
    )
}

/// `C: "OO"` lines for `(child, parent)` pairs.
fn links(pairs: &[(u64, u64)]) -> String {
    pairs
        .iter()
        .map(|(child, parent)| format!("\tC: \"OO\",{child},{parent}\n"))
        .collect()
}

/// Write `text` to `dir` and import it with its capture, or `None` when this
/// build cannot import (the common skip rule).
fn import(dir: &Path, name: &str, text: &str) -> Option<(ModelData, SourceExtras)> {
    let path = dir.join(format!("{name}.fbx"));
    std::fs::write(&path, text).expect("write the scene");
    load_full(&path)
}

fn passthrough() -> OptStack {
    let mut stack = OptStack::default();
    stack.push_op(OpKind::FilterTriangles);
    stack
}

/// A stack that only reorders, so it keeps every authored tangent: a filter
/// counts as a geometry change and the run re-synthesizes the basis after one.
fn reorder_only() -> OptStack {
    let mut stack = OptStack::default();
    stack.push_op(OpKind::VertexCache);
    stack
}

/// Export `model` after `stack`, with its capture, to `path`.
fn export(
    model: &ModelData,
    extras: &SourceExtras,
    stack: &OptStack,
    path: &Path,
    options: &ExportOptions,
) {
    let result = run_with_extras(model, extras, stack);
    export_fbx(&result.lods, model, Some(extras), path, options).expect("export succeeds");
}

fn ascii() -> ExportOptions {
    ExportOptions {
        format: FbxFormat::Ascii,
        ..ExportOptions::default()
    }
}

/// The numbers of the first `a:` line after the first line naming `array`
/// (`Normals: *`), at or after the first line containing `after`.
fn ascii_array(text: &str, after: &str, array: &str) -> Vec<f64> {
    let mut lines = text.lines().map(str::trim);
    lines
        .by_ref()
        .find(|line| line.contains(after))
        .unwrap_or_else(|| panic!("the file has no {after:?}"));
    lines
        .by_ref()
        .find(|line| line.starts_with(array))
        .unwrap_or_else(|| panic!("no {array:?} after {after:?}"));
    let values = lines.next().expect("an array line");
    values
        .trim_start_matches("a:")
        .split(',')
        .map(|value| value.trim().parse().expect("a number"))
        .collect()
}

fn close(a: &[f64], b: &[f64], tolerance: f64) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tolerance)
}

/// The corners of the mesh on node `name`.
fn corners_of<'a>(model: &'a ModelData, name: &str) -> Vec<&'a review_model::Vertex> {
    let node = model
        .nodes
        .iter()
        .position(|node| node.name == name)
        .unwrap_or_else(|| panic!("no node {name}"));
    let mut seen = std::collections::BTreeSet::new();
    for (triangle, &owner) in model.triangles.node.iter().enumerate() {
        if owner as usize == node {
            seen.extend(
                model.indices[triangle * 3..triangle * 3 + 3]
                    .iter()
                    .copied(),
            );
        }
    }
    seen.into_iter()
        .map(|index| &model.vertices[index as usize])
        .collect()
}

/// A quad carrying a blend shape that lifts corner 2 and bends its normal by
/// `(0, 0.5, 0)`, under a node scaled by `scale`, in a file of
/// `unit_scale_factor`.
fn blend_shape_scene(unit_scale_factor: f64, scale: [f64; 3]) -> String {
    let objects = quad(
        1000,
        &uv_layer(0, "map1"),
        &[(0, &["LayerElementNormal", "LayerElementUV"])],
    ) + &model(2000, "Face", &scaling(scale))
        + r#"	Geometry: 3000, "Geometry::Smile", "Shape" {
		Version: 100
		Indexes: *1 {
			a: 2
		}
		Vertices: *3 {
			a: 0,0,10
		}
		Normals: *3 {
			a: 0,0.5,0
		}
	}
	Deformer: 4000, "Deformer::Blend", "BlendShape" {
		Version: 100
	}
	Deformer: 4001, "SubDeformer::Smile", "BlendShapeChannel" {
		Version: 100
		DeformPercent: 0
		FullWeights: *1 {
			a: 100
		}
	}
"#;
    scene(
        unit_scale_factor,
        &objects,
        &links(&[
            (1000, 2000),
            (2000, 0),
            (4000, 1000),
            (4001, 4000),
            (3000, 4001),
        ]),
    )
}

/// A blend shape's normal deltas keep the strength the file gave them through
/// import and export alike, in a centimetre file and under a non-uniform node.
/// They used to ride the unnormalized normal matrix in and its exact inverse
/// out: the file round-tripped, but the model the viewer drew in between held a
/// centimetre file's deltas at 1/10000 of their strength.
#[test]
fn blend_shape_normal_deltas_keep_their_strength() {
    let dir = temp_dir("fidelity_blend_shape");
    for (label, unit, scale) in [
        ("centimetres", 1.0, [1.0, 1.0, 1.0]),
        ("stretched", 100.0, [1.0, 2.0, 1.0]),
    ] {
        let Some((source, extras)) = import(&dir, label, &blend_shape_scene(unit, scale)) else {
            return;
        };
        let morph = source.morph.as_ref().expect("the shape imports");
        if label == "centimetres" {
            assert!(
                morph.normal[0].abs_diff_eq(Vec3::new(0.0, 0.5, 0.0), 1e-5),
                "{label}: the viewer holds the delta at its own strength: {:?}",
                morph.normal[0]
            );
        }

        let path = dir.join(format!("{label}_out.fbx"));
        export(&source, &extras, &passthrough(), &path, &ascii());
        let text = std::fs::read_to_string(&path).expect("ASCII output");
        let written = ascii_array(&text, "\"Shape\"", "Normals:");
        assert!(
            close(&written, &[0.0, 0.5, 0.0], 1e-5),
            "{label}: the file gets back the delta it was given, got {written:?}"
        );
        let (loaded, _) = reload_full(&path);
        let back = loaded.morph.as_ref().expect("the shape is written");
        assert!(
            back.normal[0].abs_diff_eq(morph.normal[0], 1e-5),
            "{label}: the re-import matches the import: {:?} vs {:?}",
            back.normal[0],
            morph.normal[0]
        );
    }
}

/// A quad with an authored tangent `(1, 1, 0) / sqrt 2` and binormal
/// `(-1, 1, 0) / sqrt 2` under a node scaled `scale`.
fn tangent_scene(scale: [f64; 3]) -> String {
    let layers = uv_layer(0, "map1")
        + &layer(
            "LayerElementTangent",
            0,
            "map1",
            "Tangents",
            &repeat4(&[S, S, 0.0]),
            12,
        )
        + &layer(
            "LayerElementBinormal",
            0,
            "map1",
            "Binormals",
            &repeat4(&[-S, S, 0.0]),
            12,
        );
    let objects = quad(
        1000,
        &layers,
        &[(
            0,
            &[
                "LayerElementNormal",
                "LayerElementUV",
                "LayerElementTangent",
                "LayerElementBinormal",
            ],
        )],
    ) + &model(2000, "Panel", &scaling(scale));
    scene(100.0, &objects, &links(&[(1000, 2000), (2000, 0)]))
}

/// An authored tangent basis comes back as it was written, under a node that
/// stretches it and under one that mirrors it. A tangent rides the positions'
/// map (import used the normal matrix, export its inverse's transpose: under
/// `(1, 2, 1)` the authored `(1, 1, 0)` came back as `(0.97, 0.24, 0)`), and a
/// mirror turns its handedness over between world and node space (the
/// binormal came back pointing the other way).
#[test]
fn an_authored_tangent_basis_survives_its_node() {
    let dir = temp_dir("fidelity_tangents");
    for (label, scale) in [
        ("stretched", [1.0, 2.0, 1.0]),
        ("mirrored", [-1.0, 1.0, 1.0]),
    ] {
        let Some((source, extras)) = import(&dir, label, &tangent_scene(scale)) else {
            return;
        };
        let path = dir.join(format!("{label}_out.fbx"));
        export(&source, &extras, &reorder_only(), &path, &ascii());
        let text = std::fs::read_to_string(&path).expect("ASCII output");
        let tangents = ascii_array(&text, "LayerElementTangent", "Tangents:");
        let binormals = ascii_array(&text, "LayerElementBinormal", "Binormals:");
        for (index, tangent) in tangents.chunks(3).enumerate() {
            assert!(
                close(tangent, &[S, S, 0.0], 1e-4),
                "{label}: tangent {index} was written as {tangent:?}"
            );
        }
        for (index, binormal) in binormals.chunks(3).enumerate() {
            assert!(
                close(binormal, &[-S, S, 0.0], 1e-4),
                "{label}: binormal {index} was written as {binormal:?}"
            );
        }

        let (loaded, _) = reload_full(&path);
        let before = corners_of(&source, "Panel");
        let after = corners_of(&loaded, "Panel");
        for vertex in after {
            assert!(
                before.iter().any(|source| {
                    source
                        .tangent
                        .truncate()
                        .abs_diff_eq(vertex.tangent.truncate(), 1e-4)
                        && source.tangent.w == vertex.tangent.w
                }),
                "{label}: the re-imported basis {:?} is the imported one",
                vertex.tangent
            );
        }
    }
}

/// Two meshes that disagree about their UV and colour sets: `Lit` has `map1`,
/// `lightmap` and a colour set, `Plain` only `UVChannel_1`. The geometry keeps
/// UV sets in one model-wide table matched by position, and the export used to
/// write that table for every mesh — `Plain` came back with its set renamed
/// `map1`, a phantom `lightmap` of zeros and an all-white colour set.
#[test]
fn each_mesh_writes_the_sets_it_had() {
    let lit_layers = uv_layer(0, "map1")
        + &layer(
            "LayerElementUV",
            1,
            "lightmap",
            "UV",
            "0,0,0.5,0,0.5,0.5,0,0.5",
            8,
        )
        + &layer(
            "LayerElementColor",
            0,
            "Col",
            "Colors",
            &repeat4(&[1.0, 0.0, 0.0, 1.0]),
            16,
        );
    let objects = quad(
        1000,
        &lit_layers,
        &[
            (
                0,
                &["LayerElementNormal", "LayerElementUV", "LayerElementColor"],
            ),
            (1, &["LayerElementUV"]),
        ],
    ) + &quad(
        1001,
        &uv_layer(0, "UVChannel_1"),
        &[(0, &["LayerElementNormal", "LayerElementUV"])],
    ) + &model(2000, "Lit", "")
        + &model(2001, "Plain", "");
    let text = scene(
        100.0,
        &objects,
        &links(&[(1000, 2000), (2000, 0), (1001, 2001), (2001, 0)]),
    );
    let dir = temp_dir("fidelity_sets");
    let Some((source, extras)) = import(&dir, "sets", &text) else {
        return;
    };
    let path = dir.join("sets_out.fbx");
    export(
        &source,
        &extras,
        &passthrough(),
        &path,
        &ExportOptions::default(),
    );
    let (loaded, written) = reload_full(&path);

    let part = |model: &ModelData, extras: &SourceExtras, name: &str| {
        let node = model
            .nodes
            .iter()
            .position(|node| node.name == name)
            .unwrap_or_else(|| panic!("no node {name}"));
        extras
            .mesh_of_node(node as u32)
            .unwrap_or_else(|| panic!("no mesh on {name}"))
            .clone()
    };
    for name in ["Lit", "Plain"] {
        let before = part(&source, &extras, name);
        let after = part(&loaded, &written, name);
        let names = |part: &review_model::extras::MeshExtras| {
            part.uv_sets
                .iter()
                .map(|set| set.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&after), names(&before), "{name}: UV sets");
        let colors = |part: &review_model::extras::MeshExtras| {
            part.color_sets
                .iter()
                .map(|set| set.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(colors(&after), colors(&before), "{name}: colour sets");
    }
    // `Lit`'s colours are its own.
    assert!(corners_of(&loaded, "Lit").iter().all(|vertex| {
        vertex
            .vertex_color
            .abs_diff_eq(Vec4::new(1.0, 0.0, 0.0, 1.0), 1e-5)
    }));
}

/// A material the source declared with no properties at all is written as
/// exactly that — its template's defaults — on whatever shading model it
/// declared. The export used to write the viewer's colours into it instead,
/// which for a captured material are zeros (a black Lambert), and on a custom
/// shading model, whose template has no `DiffuseColor`, failed the whole export.
#[test]
fn a_material_without_properties_is_written_as_declared() {
    let objects = quad(
        1000,
        &uv_layer(0, "map1"),
        &[(0, &["LayerElementNormal", "LayerElementUV"])],
    ) + &quad(
        1001,
        &uv_layer(0, "map1"),
        &[(0, &["LayerElementNormal", "LayerElementUV"])],
    ) + &model(2000, "Plain", "")
        + &model(2001, "Odd", "")
        + r#"	Material: 7000, "Material::Bare", "" {
		Version: 102
		ShadingModel: "lambert"
	}
	Material: 7001, "Material::Toon", "" {
		Version: 102
		ShadingModel: "toonshader"
	}
"#;
    let text = scene(
        100.0,
        &objects,
        &links(&[
            (1000, 2000),
            (2000, 0),
            (1001, 2001),
            (2001, 0),
            (7000, 2000),
            (7001, 2001),
        ]),
    );
    let dir = temp_dir("fidelity_bare_material");
    let Some((source, extras)) = import(&dir, "bare", &text) else {
        return;
    };
    assert!(
        extras
            .materials
            .iter()
            .all(|material| material.props.is_empty()),
        "the scene's materials declare no properties"
    );
    let path = dir.join("bare_out.fbx");
    export(
        &source,
        &extras,
        &passthrough(),
        &path,
        &ExportOptions::default(),
    );
    let (loaded, written) = reload_full(&path);

    for material in &source.materials {
        let back = loaded
            .materials
            .iter()
            .find(|back| back.name == material.name)
            .unwrap_or_else(|| panic!("material {} was written", material.name));
        // Written with no properties, it reads back as its template's
        // defaults (a Lambert's 0.8 grey; the source here, having no
        // `Definitions` block, read as the importer's white) — never black.
        assert!(
            back.base_color.min_element() > 0.5,
            "{}: base colour {:?} came back {:?}",
            material.name,
            material.base_color,
            back.base_color
        );
    }
    // The classic one keeps its shading model. (A custom model's *name* does not
    // survive ufbx_write — it writes the `ShadingModel` node from a field its
    // API never sets — which is a writer limitation, not this test's subject.)
    let bare = loaded
        .materials
        .iter()
        .position(|back| back.name == "Bare")
        .expect("Bare was written");
    assert_eq!(written.materials[bare].shading_model, "lambert");
}

/// A captured Phong that authored no shininess keeps the smoothness it read as.
///
/// The probe scene's material is exactly that: ufbx_write's own Phong with no
/// properties of its own, so it reads through the template's
/// `ShininessExponent` of 20 — smoothness 0.447. Exported with its capture it
/// used to come back at smoothness 0: a captured material carries no viewer
/// figures (they are zeros), and the bridge wrote them anyway whenever the
/// material declared no properties — `ShininessExponent 0` among them, which
/// reads as fully rough.
#[cfg(has_ufbxw_probe)]
#[test]
fn a_material_without_authored_shininess_keeps_its_smoothness() {
    let dir = temp_dir("fidelity_shininess");
    let probe = dir.join("probe.fbx");
    review_optimize::probe::write_patch_probe(&probe, false)
        .unwrap_or_else(|error| panic!("probe write failed: {error}"));
    let Some((source, extras)) = load_full(&probe) else {
        return;
    };
    let material = source
        .materials
        .iter()
        .position(|material| material.name == "Mat")
        .expect("the probe's material imports");
    assert!(
        extras.materials[material].props.is_empty(),
        "the probe's material authors no properties of its own"
    );
    assert!(
        (source.materials[material].smoothness - 0.447_213_6).abs() < 1e-5,
        "it reads through the template's ShininessExponent of 20: {}",
        source.materials[material].smoothness
    );

    let path = dir.join("probe_out.fbx");
    export(
        &source,
        &extras,
        &passthrough(),
        &path,
        &ExportOptions::default(),
    );
    let (loaded, _) = reload_full(&path);
    let back = loaded
        .materials
        .iter()
        .find(|back| back.name == "Mat")
        .expect("the material is written");
    assert!(
        (back.smoothness - source.materials[material].smoothness).abs() < 1e-5,
        "smoothness {} came back {}",
        source.materials[material].smoothness,
        back.smoothness
    );
    assert!(
        back.base_color
            .abs_diff_eq(source.materials[material].base_color, 1e-5),
        "base colour {:?} came back {:?}",
        source.materials[material].base_color,
        back.base_color
    );
}

/// A grid of `n x n` quads in the XY plane, `size` across, for a LOD chain to
/// simplify.
fn grid(id: u64, n: usize, size: f64) -> String {
    let mut vertices = Vec::new();
    for y in 0..=n {
        for x in 0..=n {
            vertices.push(format!(
                "{},{},0",
                size * x as f64 / n as f64,
                size * y as f64 / n as f64
            ));
        }
    }
    let mut polygons = Vec::new();
    for y in 0..n {
        for x in 0..n {
            let a = y * (n + 1) + x;
            polygons.push(format!(
                "{},{},{},{}",
                a,
                a + 1,
                a + n + 2,
                -((a + n + 1) as i64) - 1
            ));
        }
    }
    let corner_count = n * n * 4;
    let normals = std::iter::repeat_n("0,0,1", corner_count)
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"	Geometry: {id}, "Geometry::Grid", "Mesh" {{
		Vertices: *{vertex_values} {{
			a: {vertices}
		}}
		PolygonVertexIndex: *{corner_count} {{
			a: {polygons}
		}}
		LayerElementNormal: 0 {{
			Version: 101
			Name: ""
			MappingInformationType: "ByPolygonVertex"
			ReferenceInformationType: "Direct"
			Normals: *{normal_values} {{
				a: {normals}
			}}
		}}
	}}
"#,
        vertex_values = (n + 1) * (n + 1) * 3,
        vertices = vertices.join(","),
        polygons = polygons.join(","),
        normal_values = corner_count * 3,
    )
}

/// A display layer names a node, and in a file holding a LOD chain that node
/// is every level of it: the suffixed copies belong to the layer as much as
/// level 0 does, as their animation already did.
#[test]
fn a_display_layer_holds_every_level_of_its_node() {
    let objects = grid(1000, 8, 1.0)
        + &model(2000, "Grid", "")
        + r#"	CollectionExclusive: 8000, "DisplayLayer::Props", "DisplayLayer" {
		Properties70:  {
			P: "Color", "ColorRGB", "Color", "",0.5,0.5,0.5
			P: "Show", "bool", "", "",1
		}
	}
"#;
    let text = scene(
        100.0,
        &objects,
        &links(&[(1000, 2000), (2000, 0), (2000, 8000)]),
    );
    let dir = temp_dir("fidelity_layers");
    let Some((source, extras)) = import(&dir, "layers", &text) else {
        return;
    };
    assert_eq!(extras.display_layers.len(), 1, "the layer imports");

    let mut stack = OptStack::default();
    stack.push_op(OpKind::SimplifyLod(LodParams {
        simplify: SimplifySettings {
            algorithm: SimplifyAlgorithm::Standard,
            ..SimplifySettings::default()
        },
        levels: vec![LodLevel {
            target_ratio: 0.5,
            target_error: 0.05,
        }],
    }));
    let path = dir.join("layers_out.fbx");
    let result = run_with_extras(&source, &extras, &stack);
    assert_eq!(result.lods.len(), 2, "a two-level chain");
    export_fbx(
        &result.lods,
        &source,
        Some(&extras),
        &path,
        &ExportOptions {
            packaging: LodPackaging::SingleFileSuffixed,
            ..ExportOptions::default()
        },
    )
    .expect("export succeeds");

    let (loaded, written) = reload_full(&path);
    let layer = written
        .display_layers
        .iter()
        .find(|layer| layer.name == "Props")
        .expect("the layer is written");
    let mut members: Vec<&str> = layer
        .nodes
        .iter()
        .map(|&node| loaded.nodes[node as usize].name.as_str())
        .collect();
    members.sort_unstable();
    assert_eq!(members, ["Grid_LOD0", "Grid_LOD1"]);
}

/// A layered texture nested inside another is written whatever order the
/// capture lists the two in. It used to be placed only when the inner one came
/// first, and dropped (with a note) otherwise.
#[test]
fn a_nested_layered_texture_is_written_in_either_order() {
    let objects = quad(
        1000,
        &uv_layer(0, "map1"),
        &[(0, &["LayerElementNormal", "LayerElementUV"])],
    ) + &model(2000, "Crate", "")
        + r#"	LayeredTexture: 6000, "LayeredTexture::Outer", "" {
		LayeredTexture: 1
		BlendModes: *2 {
			a: 0,1
		}
		Alphas: *2 {
			a: 1,1
		}
	}
	LayeredTexture: 6001, "LayeredTexture::Inner", "" {
		LayeredTexture: 1
		BlendModes: *1 {
			a: 0
		}
		Alphas: *1 {
			a: 1
		}
	}
	Texture: 6002, "Texture::Base", "" {
		Type: "TextureVideoClip"
		FileName: "base.png"
		RelativeFilename: "base.png"
	}
	Texture: 6003, "Texture::Detail", "" {
		Type: "TextureVideoClip"
		FileName: "detail.png"
		RelativeFilename: "detail.png"
	}
	Material: 7000, "Material::Wood", "" {
		Version: 102
		ShadingModel: "phong"
	}
"#;
    let connections = links(&[
        (1000, 2000),
        (2000, 0),
        (7000, 2000),
        (6001, 6000),
        (6002, 6000),
        (6003, 6001),
    ]) + "\tC: \"OP\",6000,7000, \"DiffuseColor\"\n";
    let text = scene(100.0, &objects, &connections);
    let dir = temp_dir("fidelity_nested_layers");
    let Some((source, extras)) = import(&dir, "nested", &text) else {
        return;
    };
    let layered = |extras: &SourceExtras| {
        extras
            .textures
            .iter()
            .filter(|texture| texture.kind == TextureKind::Layered)
            .count()
    };
    assert_eq!(layered(&extras), 2, "both layered textures import");
    let index_of = |name: &str| {
        extras
            .textures
            .iter()
            .position(|texture| texture.name == name)
            .unwrap_or_else(|| panic!("no texture {name}"))
    };
    assert!(
        index_of("Outer") < index_of("Inner"),
        "the capture lists the outer texture first, the order that used to drop it"
    );

    let path = dir.join("nested_out.fbx");
    let result = run_with_extras(&source, &extras, &passthrough());
    let report = export_fbx(
        &result.lods,
        &source,
        Some(&extras),
        &path,
        &ExportOptions::default(),
    )
    .expect("export succeeds");
    assert!(
        !report.notes.iter().any(|note| matches!(
            note,
            review_optimize::ExportNote::NestedLayeredTextures { .. }
        )),
        "nothing was dropped: {:?}",
        report.notes
    );
    let (_, written) = reload_full(&path);
    assert_eq!(layered(&written), 2, "both layered textures are written");
    let outer = written
        .textures
        .iter()
        .find(|texture| texture.name == "Outer")
        .expect("the outer texture is written");
    assert!(
        outer
            .layers
            .iter()
            .any(|layer| written.textures[layer.texture as usize].name == "Inner"),
        "the outer texture still layers the inner one"
    );
}

/// Embedded image content goes out with its video and comes back there, once.
#[test]
fn embedded_content_round_trips_on_its_video() {
    let objects = quad(
        1000,
        &uv_layer(0, "map1"),
        &[(0, &["LayerElementNormal", "LayerElementUV"])],
    ) + &model(2000, "Crate", "")
        + r#"	Video: 5000, "Video::Image", "Clip" {
		Type: "Clip"
		Filename: "image.png"
		RelativeFilename: "image.png"
		Content: "aGVsbG8sIHRleHR1cmU="
	}
	Texture: 6000, "Texture::Image", "" {
		Type: "TextureVideoClip"
		FileName: "image.png"
		RelativeFilename: "image.png"
	}
	Material: 7000, "Material::Wood", "" {
		Version: 102
		ShadingModel: "phong"
	}
"#;
    let connections = links(&[(1000, 2000), (2000, 0), (7000, 2000), (5000, 6000)])
        + "\tC: \"OP\",6000,7000, \"DiffuseColor\"\n";
    let text = scene(100.0, &objects, &connections);
    let dir = temp_dir("fidelity_embedded");
    let Some((source, extras)) = import(&dir, "embedded", &text) else {
        return;
    };
    let path: PathBuf = dir.join("embedded_out.fbx");
    export(
        &source,
        &extras,
        &passthrough(),
        &path,
        &ExportOptions::default(),
    );
    let (_, written) = reload_full(&path);
    assert_eq!(written.videos.len(), 1);
    assert_eq!(written.videos[0].content, b"hello, texture");
    assert!(
        written
            .textures
            .iter()
            .all(|texture| texture.content.is_empty()),
        "the texture's content is its video's"
    );
}
