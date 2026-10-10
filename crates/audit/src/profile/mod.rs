//! Audit profiles: which rules run, how severe each is, and their thresholds.
//!
//! A profile is a base engine ([`Engine`]) plus one [`RuleConfig`] per rule.
//! Every rule's parameters are declared once in [`param_specs`] — key, kind,
//! limits — and that one table drives the defaults, the clamp a hand-edited file
//! goes through on load ([`AuditProfile::sanitize`]), and the widgets the
//! Inspector's profile editor draws.

mod builtin;
pub mod envelope;

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

use crate::finding::Axis;
use crate::rule::{RuleId, Severity};

pub use builtin::default_config;

/// The engine a profile targets. Picks the built-in defaults and the wording of
/// the "why it matters" text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    Unity,
    Unreal,
    Generic,
}

impl Engine {
    pub const ALL: [Engine; 3] = [Engine::Unity, Engine::Unreal, Engine::Generic];

    pub fn as_str(self) -> &'static str {
        match self {
            Engine::Unity => "unity",
            Engine::Unreal => "unreal",
            Engine::Generic => "generic",
        }
    }
}

/// One parameter value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    Flag(bool),
    Number(f64),
    Axis(Axis),
    Patterns(Vec<String>),
}

impl ParamValue {
    pub fn as_number(&self) -> Option<f64> {
        match self {
            ParamValue::Number(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_patterns(&self) -> Option<&[String]> {
        match self {
            ParamValue::Patterns(patterns) => Some(patterns),
            _ => None,
        }
    }

    pub fn as_axis(&self) -> Option<Axis> {
        match self {
            ParamValue::Axis(axis) => Some(*axis),
            _ => None,
        }
    }
}

/// What a parameter measures, which picks its widget and its display unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamKind {
    /// A whole number in `min..=max`.
    Count { min: f64, max: f64 },
    /// A length in meters.
    Meters { min: f64, max: f64 },
    /// A unitless value (a ratio, a tolerance, a factor, a screen size).
    Number { min: f64, max: f64 },
    /// Pixels (a resolution or a padding).
    Pixels { min: f64, max: f64 },
    /// Square pixels.
    SquarePixels { min: f64, max: f64 },
    /// Pixels per meter.
    PxPerMeter { min: f64, max: f64 },
    /// A scene unit, as meters per unit (m = 1, cm = 0.01).
    Unit,
    /// An up axis.
    Axis,
    /// A list of glob patterns.
    Patterns,
}

impl ParamKind {
    /// The numeric range, for the kinds that have one.
    pub fn range(self) -> Option<(f64, f64)> {
        match self {
            ParamKind::Count { min, max }
            | ParamKind::Meters { min, max }
            | ParamKind::Number { min, max }
            | ParamKind::Pixels { min, max }
            | ParamKind::SquarePixels { min, max }
            | ParamKind::PxPerMeter { min, max } => Some((min, max)),
            ParamKind::Unit => Some((1.0e-6, 1.0e6)),
            ParamKind::Axis | ParamKind::Patterns => None,
        }
    }
}

/// The declaration of one rule parameter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamSpec {
    /// The key it is stored under, and the catalog suffix of its label.
    pub key: &'static str,
    pub kind: ParamKind,
}

/// The parameters `rule` takes, in editor order.
pub fn param_specs(rule: RuleId) -> &'static [ParamSpec] {
    const fn spec(key: &'static str, kind: ParamKind) -> ParamSpec {
        ParamSpec { key, kind }
    }
    const MAX_COUNT: f64 = 1.0e9;
    const RESOLUTION: ParamKind = ParamKind::Pixels {
        min: 16.0,
        max: 4096.0,
    };
    const CHANNEL: ParamKind = ParamKind::Count { min: 0.0, max: 7.0 };
    match rule {
        RuleId::DuplicateVertices => {
            &const {
                [spec(
                    "tolerance",
                    ParamKind::Meters {
                        min: 0.0,
                        max: 0.01,
                    },
                )]
            }
        }
        RuleId::HardEdges => {
            &const { [spec("max_ratio", ParamKind::Number { min: 0.0, max: 1.0 })] }
        }
        RuleId::TriangleBudget => {
            &const {
                [
                    spec(
                        "max_per_object",
                        ParamKind::Count {
                            min: 1.0,
                            max: MAX_COUNT,
                        },
                    ),
                    spec(
                        "max_total",
                        ParamKind::Count {
                            min: 1.0,
                            max: MAX_COUNT,
                        },
                    ),
                ]
            }
        }
        RuleId::DrawCallBudget | RuleId::MaterialsPerMesh => {
            &const {
                [spec(
                    "max",
                    ParamKind::Count {
                        min: 1.0,
                        max: 1.0e5,
                    },
                )]
            }
        }
        RuleId::PivotOffset => {
            &const {
                [spec(
                    "max_distance",
                    ParamKind::Meters {
                        min: 0.0,
                        max: 1.0e4,
                    },
                )]
            }
        }
        RuleId::Unit => &const { [spec("target", ParamKind::Unit)] },
        RuleId::UpAxis => &const { [spec("target", ParamKind::Axis)] },
        RuleId::UvOverlap => &const { [spec("resolution", RESOLUTION)] },
        RuleId::LightmapMissing | RuleId::LightmapOutOfRange => {
            &const { [spec("channel", CHANNEL)] }
        }
        RuleId::LightmapOverlap => {
            &const { [spec("channel", CHANNEL), spec("resolution", RESOLUTION)] }
        }
        RuleId::LightmapPadding => {
            &const {
                [
                    spec("resolution", RESOLUTION),
                    spec(
                        "min_padding",
                        ParamKind::Pixels {
                            min: 0.0,
                            max: 64.0,
                        },
                    ),
                ]
            }
        }
        RuleId::Influences => {
            &const {
                [spec(
                    "max",
                    ParamKind::Count {
                        min: 1.0,
                        max: 64.0,
                    },
                )]
            }
        }
        RuleId::UnnormalizedWeights => {
            &const { [spec("tolerance", ParamKind::Number { min: 0.0, max: 1.0 })] }
        }
        RuleId::BonesPerMesh => {
            &const {
                [spec(
                    "max",
                    ParamKind::Count {
                        min: 1.0,
                        max: 4096.0,
                    },
                )]
            }
        }
        RuleId::TexelDensity => {
            &const {
                [
                    spec(
                        "texture_size",
                        ParamKind::Pixels {
                            min: 16.0,
                            max: 16384.0,
                        },
                    ),
                    spec(
                        "target",
                        ParamKind::PxPerMeter {
                            min: 1.0,
                            max: 1.0e5,
                        },
                    ),
                    spec(
                        "tolerance",
                        ParamKind::Number {
                            min: 1.0,
                            max: 16.0,
                        },
                    ),
                ]
            }
        }
        RuleId::TriangleLod => {
            &const {
                [
                    spec(
                        "min_pixel_area",
                        ParamKind::SquarePixels {
                            min: 0.25,
                            max: 1024.0,
                        },
                    ),
                    spec(
                        "screen_height",
                        ParamKind::Pixels {
                            min: 120.0,
                            max: 8640.0,
                        },
                    ),
                    spec("min_screen_size", ParamKind::Number { min: 0.0, max: 4.0 }),
                ]
            }
        }
        RuleId::NamePattern => {
            &const {
                [
                    spec("mesh", ParamKind::Patterns),
                    spec("skinned", ParamKind::Patterns),
                    spec("bone", ParamKind::Patterns),
                    spec("empty", ParamKind::Patterns),
                    spec("light", ParamKind::Patterns),
                    spec("camera", ParamKind::Patterns),
                ]
            }
        }
        RuleId::CollisionPrefix => &const { [spec("prefixes", ParamKind::Patterns)] },
        RuleId::AssetPrefix => {
            &const {
                [
                    spec("static", ParamKind::Patterns),
                    spec("skinned", ParamKind::Patterns),
                ]
            }
        }
        RuleId::EmptyNodes => &const { [spec("ignore", ParamKind::Patterns)] },
        _ => &const { [] },
    }
}

/// One rule's settings in a profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleConfig {
    pub enabled: bool,
    pub severity: Severity,
    #[serde(default)]
    pub params: BTreeMap<String, ParamValue>,
}

impl RuleConfig {
    pub fn number(&self, key: &str) -> Option<f64> {
        self.params.get(key).and_then(ParamValue::as_number)
    }

    pub fn patterns(&self, key: &str) -> &[String] {
        self.params
            .get(key)
            .and_then(ParamValue::as_patterns)
            .unwrap_or(&[])
    }

    pub fn axis(&self, key: &str) -> Option<Axis> {
        self.params.get(key).and_then(ParamValue::as_axis)
    }

    /// A hash of everything that changes what the rule *finds* — whether it
    /// runs and its parameters — but not its severity, which only relabels the
    /// result.
    pub fn finding_key(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.enabled.hash(&mut hasher);
        hash_params(&self.params, &mut hasher);
        hasher.finish()
    }
}

fn hash_params(params: &BTreeMap<String, ParamValue>, hasher: &mut impl Hasher) {
    for (key, value) in params {
        key.hash(hasher);
        match value {
            ParamValue::Flag(flag) => flag.hash(hasher),
            ParamValue::Number(number) => number.to_bits().hash(hasher),
            ParamValue::Axis(axis) => axis.hash(hasher),
            ParamValue::Patterns(patterns) => patterns.hash(hasher),
        }
    }
}

/// A complete audit profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditProfile {
    /// The name shown in the Profile row; free text.
    pub name: String,
    /// The engine the profile targets, and the built-in it was derived from.
    pub base: Engine,
    pub rules: BTreeMap<RuleId, RuleConfig>,
}

impl AuditProfile {
    /// The built-in profile for `engine`.
    pub fn builtin(engine: Engine) -> Self {
        Self {
            name: String::new(),
            base: engine,
            rules: RuleId::ALL
                .iter()
                .map(|&rule| (rule, default_config(engine, rule)))
                .collect(),
        }
    }

    /// The settings of `rule`, falling back to the base engine's default for a
    /// rule this profile does not mention.
    pub fn rule(&self, rule: RuleId) -> RuleConfig {
        self.rules
            .get(&rule)
            .cloned()
            .unwrap_or_else(|| default_config(self.base, rule))
    }

    /// Whether this profile still equals its base built-in, ignoring the name.
    pub fn is_builtin(&self) -> bool {
        self.rules == AuditProfile::builtin(self.base).rules
    }

    /// Bring a profile — typically one read from a hand-edited file — into the
    /// shape the engine and the editor expect: every rule present, every
    /// declared parameter present and of the right kind, every number inside
    /// its declared range, and nothing undeclared.
    pub fn sanitize(&mut self) {
        for &rule in RuleId::ALL {
            let defaults = default_config(self.base, rule);
            let config = self.rules.entry(rule).or_insert_with(|| defaults.clone());
            let mut params = BTreeMap::new();
            for spec in param_specs(rule) {
                let fallback = defaults.params.get(spec.key).cloned();
                let value = match (config.params.remove(spec.key), spec.kind) {
                    (Some(ParamValue::Number(number)), kind) if number.is_finite() => {
                        match kind.range() {
                            Some((min, max)) => {
                                let mut clamped = number.clamp(min, max);
                                if matches!(kind, ParamKind::Count { .. }) {
                                    clamped = clamped.round();
                                }
                                Some(ParamValue::Number(clamped))
                            }
                            None => fallback,
                        }
                    }
                    (Some(ParamValue::Axis(axis)), ParamKind::Axis) => Some(ParamValue::Axis(axis)),
                    (Some(ParamValue::Patterns(patterns)), ParamKind::Patterns) => {
                        Some(ParamValue::Patterns(
                            patterns
                                .into_iter()
                                .map(|pattern| pattern.trim().to_owned())
                                .filter(|pattern| !pattern.is_empty())
                                .collect(),
                        ))
                    }
                    _ => fallback,
                };
                if let Some(value) = value {
                    params.insert(spec.key.to_owned(), value);
                }
            }
            config.params = params;
        }
    }

    /// A digest of the whole profile, recorded in a report so it says which
    /// profile it was judged against.
    pub fn digest(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.base.hash(&mut hasher);
        for (rule, config) in &self.rules {
            rule.hash(&mut hasher);
            config.severity.hash(&mut hasher);
            config.finding_key().hash(&mut hasher);
        }
        hasher.finish()
    }
}

impl Default for AuditProfile {
    fn default() -> Self {
        Self::builtin(Engine::Generic)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_declare_every_parameter() {
        for engine in Engine::ALL {
            let profile = AuditProfile::builtin(engine);
            for &rule in RuleId::ALL {
                let config = profile.rule(rule);
                for spec in param_specs(rule) {
                    assert!(
                        config.params.contains_key(spec.key),
                        "{} {} lacks {}",
                        engine.as_str(),
                        rule.as_str(),
                        spec.key
                    );
                }
                assert_eq!(config.params.len(), param_specs(rule).len());
            }
            let mut sanitized = profile.clone();
            sanitized.sanitize();
            assert_eq!(sanitized, profile, "a built-in is already sane");
        }
    }

    #[test]
    fn sanitize_clamps_numbers_and_drops_strangers() {
        let mut profile = AuditProfile::builtin(Engine::Unity);
        let config = profile.rules.get_mut(&RuleId::Influences).unwrap();
        config
            .params
            .insert("max".into(), ParamValue::Number(1000.4));
        config.params.insert("bogus".into(), ParamValue::Flag(true));
        let config = profile.rules.get_mut(&RuleId::NamePattern).unwrap();
        config.params.insert("mesh".into(), ParamValue::Number(3.0));
        profile.rules.remove(&RuleId::Ngons);
        profile.sanitize();
        let influences = profile.rule(RuleId::Influences);
        assert_eq!(influences.number("max"), Some(64.0));
        assert!(!influences.params.contains_key("bogus"));
        // A wrongly-typed value falls back to the default.
        assert!(
            profile
                .rule(RuleId::NamePattern)
                .patterns("mesh")
                .is_empty()
        );
        assert!(profile.rules.contains_key(&RuleId::Ngons));
    }

    #[test]
    fn severity_does_not_change_the_finding_key() {
        let mut config = default_config(Engine::Unreal, RuleId::Influences);
        let key = config.finding_key();
        config.severity = Severity::Error;
        assert_eq!(config.finding_key(), key);
        config.params.insert("max".into(), ParamValue::Number(6.0));
        assert_ne!(config.finding_key(), key);
    }
}
