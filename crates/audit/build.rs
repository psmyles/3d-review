//! Two things from the repository root, the one place each is edited:
//! - the product's name and version, from `product.json`, for the `generator` line of the
//!   profiles it writes;
//! - the rules, from `rules-catalog.toml`: asset-rules checks the catalog (no rule may be newer
//!   than this version) and generates `RuleId` and the rest of the shared rule code; this script
//!   adds 3D Review's own facts about each rule (its category, what its offenders are, how they
//!   are drawn, which views show them, and its parameters' widgets) from the catalog's
//!   categories, `[rule.app]` and parameters. All of it goes to `$OUT_DIR/rules.rs`, which
//!   `rule.rs` includes.

use std::fmt::Write as _;
use std::path::PathBuf;

use asset_rules::catalog::{Catalog, Param, ParamKind};

const PRODUCT_JSON: &str = "../../product.json";
const CATALOG: &str = "../../rules-catalog.toml";

fn main() {
    println!("cargo:rerun-if-changed={PRODUCT_JSON}");
    println!("cargo:rerun-if-changed={CATALOG}");
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let product: serde_json::Value = std::fs::read_to_string(manifest.join(PRODUCT_JSON))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_else(|| panic!("{PRODUCT_JSON} must parse"));
    let mut version = String::new();
    for (var, key) in [
        ("REVIEW_PRODUCT", "productName"),
        ("REVIEW_VERSION", "version"),
    ] {
        let value = product
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| panic!("{PRODUCT_JSON} has no `{key}`"));
        println!("cargo:rustc-env={var}={value}");
        version = value.to_owned();
    }

    let path = manifest.join(CATALOG);
    let mut code =
        asset_rules::codegen::generate(&path, &version).unwrap_or_else(|why| panic!("{why}"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{CATALOG}: {e}"));
    let catalog = Catalog::from_toml(&text).unwrap_or_else(|e| panic!("{CATALOG} {e}"));
    code.push_str(&app_code(&catalog).unwrap_or_else(|why| panic!("{CATALOG}: {why}")));
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("rules.rs");
    std::fs::write(&out, code).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
}

/// The variants of 3D Review's own enums that the categories and `[rule.app]` may name, in the
/// catalog's spelling.
const CATEGORIES: &[&str] = &[
    "geometry",
    "transforms",
    "uv",
    "skin",
    "density",
    "naming",
    "hierarchy",
];
const ELEMENTS: &[&str] = &["triangle", "face", "edge", "vertex", "node", "scene"];
const MARKERS: &[&str] = &["fill", "edge", "dot", "object", "none"];
const VIEWS: &[&str] = &[
    "issues",
    "texel_density",
    "triangle_density",
    "overdraw",
    "quad_overdraw",
];

/// `impl RuleId` with 3D Review's facts about each rule.
fn app_code(catalog: &Catalog) -> Result<String, String> {
    let mut arms = Arms::default();
    for rule in &catalog.rules {
        let name = rule.code_name();
        let app = rule.app.as_ref();
        let field = |key: &str| app.and_then(|app| app.get(key)).and_then(|v| v.as_str());
        let one_of = |what: &str, value: Option<&str>, known: &[&str]| -> Result<String, String> {
            match value {
                Some(value) if known.contains(&value) => Ok(camel(value)),
                Some(value) => Err(format!(
                    "rule \"{}\": {what} \"{value}\" isn't one of {}",
                    rule.id,
                    known.join(", ")
                )),
                None => Err(format!("rule \"{}\" has no {what}", rule.id)),
            }
        };
        let category = one_of("category", Some(&rule.category), CATEGORIES)?;
        let element = one_of("[rule.app] element", field("element"), ELEMENTS)?;
        let marker = one_of("[rule.app] marker", field("marker"), MARKERS)?;
        let view = one_of("[rule.app] view", field("view"), VIEWS)?;
        let see_also = match field("see_also") {
            None => "None".to_owned(),
            value => format!(
                "Some(DiagnosticView::{})",
                one_of("[rule.app] see_also", value, VIEWS)?
            ),
        };
        let specs: Vec<String> = rule
            .params
            .iter()
            .map(|param| {
                let kind = param_kind(param).map_err(|why| {
                    format!("rule \"{}\" param \"{}\": {why}", rule.id, param.key)
                })?;
                Ok(format!(
                    "ParamSpec {{ key: {:?}, kind: {kind} }}",
                    param.key
                ))
            })
            .collect::<Result<_, String>>()?;
        arms.category
            .push(format!("RuleId::{name} => Category::{category},"));
        arms.element
            .push(format!("RuleId::{name} => ElementKind::{element},"));
        arms.marker
            .push(format!("RuleId::{name} => Marker::{marker},"));
        arms.view
            .push(format!("RuleId::{name} => DiagnosticView::{view},"));
        arms.see_also.push(format!("RuleId::{name} => {see_also},"));
        arms.specs
            .push(format!("RuleId::{name} => &[{}],", specs.join(", ")));
    }

    let mut out = String::new();
    let o = &mut out;
    let _ = writeln!(o);
    let _ = writeln!(
        o,
        "// 3D Review's own facts about each rule, from the catalog's categories,"
    );
    let _ = writeln!(o, "// `[rule.app]` and parameters (crates/audit/build.rs).");
    let _ = writeln!(o);
    let _ = writeln!(o, "use crate::profile::{{ParamKind, ParamSpec}};");
    let _ = writeln!(
        o,
        "use crate::rule::{{Category, DiagnosticView, ElementKind, Marker}};"
    );
    let _ = writeln!(o);
    let _ = writeln!(o, "impl RuleId {{");
    method(
        o,
        "The group the rule is listed under.",
        "category(self) -> Category",
        &arms.category,
    );
    method(
        o,
        "What the rule's offenders are.",
        "element(self) -> ElementKind",
        &arms.element,
    );
    method(
        o,
        "How the rule's offenders are drawn in the viewport.",
        "marker(self) -> Marker",
        &arms.marker,
    );
    method(
        o,
        "The view that shows the rule's problem best.",
        "related_view(self) -> DiagnosticView",
        &arms.view,
    );
    method(
        o,
        "A second view worth a look for the rule's problem, beside `related_view`.",
        "see_also(self) -> Option<DiagnosticView>",
        &arms.see_also,
    );
    method(
        o,
        "The parameters the rule takes, in editor order: key, kind and limits.",
        "param_specs(self) -> &'static [ParamSpec]",
        &arms.specs,
    );
    let _ = writeln!(o, "}}");
    Ok(out)
}

#[derive(Default)]
struct Arms {
    category: Vec<String>,
    element: Vec<String>,
    marker: Vec<String>,
    view: Vec<String>,
    see_also: Vec<String>,
    specs: Vec<String>,
}

fn method(out: &mut String, doc: &str, signature: &str, arms: &[String]) {
    let _ = writeln!(out, "    /// {doc}");
    let _ = writeln!(out, "    pub const fn {signature} {{");
    let _ = writeln!(out, "        match self {{");
    for arm in arms {
        let _ = writeln!(out, "            {arm}");
    }
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out);
}

/// The widget and display unit the Aud editor gives a parameter, from its kind and unit:
/// - a whole number in pixels is `Pixels`, any other whole number `Count`;
/// - a number with suggested options is a scene `Unit`; otherwise its unit picks `Meters`,
///   `Pixels`, `SquarePixels` or `PxPerMeter`, and no unit is a plain `Number`;
/// - a choice is an `Axis` (the one choice 3D Review has), and patterns are `Patterns`.
fn param_kind(param: &Param) -> Result<String, String> {
    let range = || -> Result<String, String> {
        match (param.min, param.max) {
            (Some(min), Some(max)) => Ok(format!("{{ min: {min:?}, max: {max:?} }}")),
            _ => Err("a number needs a min and a max".to_owned()),
        }
    };
    let unit = param.unit.as_deref();
    Ok(match param.kind {
        ParamKind::Count if unit == Some("px") => format!("ParamKind::Pixels {}", range()?),
        ParamKind::Count => format!("ParamKind::Count {}", range()?),
        ParamKind::Number if !param.options.is_empty() => "ParamKind::Unit".to_owned(),
        ParamKind::Number => {
            let kind = match unit {
                None => "Number",
                Some("m") => "Meters",
                Some("px") => "Pixels",
                Some("px²") => "SquarePixels",
                Some("px/m") => "PxPerMeter",
                Some(other) => return Err(format!("unit \"{other}\" has no widget")),
            };
            format!("ParamKind::{kind} {}", range()?)
        }
        ParamKind::Choice => "ParamKind::Axis".to_owned(),
        ParamKind::Patterns => "ParamKind::Patterns".to_owned(),
        ParamKind::Flag | ParamKind::Choices => {
            return Err(format!(
                "3D Review has no {} parameter",
                param.kind.as_str()
            ));
        }
    })
}

/// `texel_density` → `TexelDensity`.
fn camel(name: &str) -> String {
    name.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect()
}
