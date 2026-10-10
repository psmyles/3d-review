//! The built-in Unity / Unreal / Generic defaults.
//!
//! These are starting points, not law: each value is the common convention for
//! that engine, and every one of them is editable per profile.

use std::collections::BTreeMap;

use super::{Engine, ParamValue, RuleConfig};
use crate::finding::Axis;
use crate::rule::{RuleId, Severity};

use Severity::{Error as E, Info as I, Warning as W};

/// The default settings of `rule` in `engine`'s built-in profile.
pub fn default_config(engine: Engine, rule: RuleId) -> RuleConfig {
    // (Unity, Unreal, Generic) severities; `None` = disabled.
    let pick =
        |unity: Option<Severity>, unreal: Option<Severity>, generic: Option<Severity>| match engine
        {
            Engine::Unity => unity,
            Engine::Unreal => unreal,
            Engine::Generic => generic,
        };
    let by_engine = |unity: f64, unreal: f64, generic: f64| match engine {
        Engine::Unity => unity,
        Engine::Unreal => unreal,
        Engine::Generic => generic,
    };
    let mut params = BTreeMap::new();
    let mut number = |key: &str, value: f64| {
        params.insert(key.to_owned(), ParamValue::Number(value));
    };

    let severity = match rule {
        RuleId::DegenerateTriangles => pick(Some(W), Some(W), Some(W)),
        RuleId::NonManifoldEdges => pick(Some(W), Some(W), Some(W)),
        RuleId::IsolatedVertices => pick(Some(I), Some(I), Some(I)),
        RuleId::DuplicateVertices => {
            number("tolerance", 1.0e-5);
            pick(Some(W), Some(W), Some(I))
        }
        RuleId::Ngons => pick(Some(W), Some(W), Some(I)),
        RuleId::MissingNormals => pick(Some(I), Some(W), Some(I)),
        RuleId::MissingTangents => pick(Some(I), Some(I), None),
        RuleId::InvertedNormals => pick(Some(E), Some(E), Some(W)),
        RuleId::HardEdges => {
            number("max_ratio", 0.5);
            pick(Some(I), Some(I), Some(I))
        }
        RuleId::TriangleBudget => {
            number("max_per_object", 200_000.0);
            number("max_total", 2_000_000.0);
            pick(Some(W), Some(W), Some(I))
        }
        RuleId::DrawCallBudget => {
            number("max", 32.0);
            pick(Some(W), Some(W), Some(I))
        }
        RuleId::Scale => pick(Some(W), Some(I), Some(I)),
        RuleId::NegativeScale => pick(Some(W), Some(W), Some(W)),
        RuleId::Unfrozen => pick(Some(W), Some(I), Some(I)),
        RuleId::PivotOffset => {
            number("max_distance", 0.01);
            pick(Some(I), Some(W), Some(I))
        }
        RuleId::Unit => {
            number("target", by_engine(1.0, 0.01, 1.0));
            pick(Some(I), Some(W), None)
        }
        RuleId::UpAxis => {
            params.insert(
                "target".to_owned(),
                ParamValue::Axis(match engine {
                    Engine::Unreal => Axis::PositiveZ,
                    _ => Axis::PositiveY,
                }),
            );
            pick(Some(W), Some(I), None)
        }
        RuleId::UvMissing => pick(Some(E), Some(E), Some(W)),
        RuleId::UvOutOfRange => pick(Some(I), Some(I), Some(I)),
        RuleId::UvOverlap => {
            number("resolution", 1024.0);
            pick(Some(I), Some(I), Some(I))
        }
        RuleId::UvFlipped => pick(Some(I), Some(I), Some(I)),
        RuleId::LightmapMissing => {
            number("channel", 1.0);
            pick(Some(I), Some(I), None)
        }
        RuleId::LightmapOverlap => {
            number("channel", 1.0);
            number("resolution", by_engine(128.0, 64.0, 128.0));
            pick(Some(W), Some(E), Some(W))
        }
        RuleId::LightmapPadding => {
            number("channel", 1.0);
            number("resolution", by_engine(128.0, 64.0, 128.0));
            number("min_padding", 2.0);
            pick(Some(W), Some(W), None)
        }
        RuleId::LightmapOutOfRange => {
            number("channel", 1.0);
            pick(Some(E), Some(E), Some(W))
        }
        RuleId::Influences => {
            number("max", 4.0);
            pick(Some(W), Some(W), Some(I))
        }
        RuleId::UnnormalizedWeights => {
            number("tolerance", 0.01);
            pick(Some(I), Some(I), Some(I))
        }
        RuleId::BonesPerMesh => {
            number("max", 80.0);
            pick(Some(I), Some(I), Some(I))
        }
        RuleId::UnusedBones => pick(Some(I), Some(I), Some(I)),
        RuleId::BindPoseMismatch => pick(Some(I), Some(W), Some(I)),
        RuleId::TexelDensity => {
            number("texture_size", 1024.0);
            number("target", 1024.0);
            number("tolerance", 2.0);
            pick(Some(W), Some(W), Some(I))
        }
        RuleId::TriangleLod => {
            number("min_pixel_area", 10.0);
            number("screen_height", 1080.0);
            number("min_screen_size", 0.1);
            pick(Some(I), Some(I), Some(I))
        }
        RuleId::TriangleReduce => pick(Some(W), Some(W), Some(I)),
        RuleId::InvalidCharacters => pick(Some(I), Some(W), Some(I)),
        RuleId::NamePattern => {
            for key in ["mesh", "skinned", "bone", "empty", "light", "camera"] {
                params.insert(key.to_owned(), ParamValue::Patterns(Vec::new()));
            }
            pick(None, None, None)
        }
        RuleId::LodSuffix => pick(Some(W), Some(W), Some(I)),
        RuleId::CollisionPrefix => {
            params.insert(
                "prefixes".to_owned(),
                ParamValue::Patterns(
                    ["UCX_*", "UBX_*", "USP_*", "UCP_*"]
                        .map(str::to_owned)
                        .to_vec(),
                ),
            );
            pick(Some(I), Some(E), Some(I))
        }
        RuleId::AssetPrefix => {
            params.insert(
                "static".to_owned(),
                ParamValue::Patterns(vec!["SM_*".to_owned()]),
            );
            params.insert(
                "skinned".to_owned(),
                ParamValue::Patterns(vec!["SK_*".to_owned()]),
            );
            pick(None, Some(W), None)
        }
        RuleId::EmptyNodes => {
            params.insert(
                "ignore".to_owned(),
                ParamValue::Patterns(vec!["SOCKET_*".to_owned()]),
            );
            pick(Some(I), Some(I), Some(I))
        }
        RuleId::DuplicateNames => pick(Some(W), Some(W), Some(I)),
        RuleId::MultipleRoots => pick(Some(I), Some(I), Some(I)),
        RuleId::LightsCameras => pick(Some(W), Some(I), Some(I)),
        RuleId::MaterialsPerMesh => {
            number("max", 4.0);
            pick(Some(W), Some(W), Some(I))
        }
    };

    RuleConfig {
        enabled: severity.is_some(),
        // A rule a built-in leaves off still carries a severity, used the
        // moment someone switches it on.
        severity: severity.unwrap_or(I),
        params,
    }
}
