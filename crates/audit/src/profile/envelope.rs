//! Saving and loading a profile as JSON.
//!
//! The file is a versioned envelope rather than a bare [`AuditProfile`], the
//! same shape as an Opt preset, so a profile written by a future build is
//! *rejected with an explanation* instead of silently half-understood.
//!
//! File IO lives with the caller (`app` owns the file dialogs and the config
//! dir); this module only converts between a profile and a string.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{AuditProfile, Engine, RuleConfig};
use crate::AuditError;
use crate::rule::RuleId;

/// Envelope version this build writes and is willing to read.
pub const PROFILE_VERSION: u32 = 1;

/// Conventional file extension for a saved profile.
pub const PROFILE_EXTENSION: &str = "json";

#[derive(Serialize)]
struct EnvelopeOut<'a> {
    version: u32,
    generator: String,
    profile: &'a AuditProfile,
}

#[derive(Deserialize)]
struct EnvelopeIn {
    version: u32,
    profile: ProfileIn,
}

/// The profile as read: rules keyed by plain strings, so an id this build does
/// not know is skipped rather than failing the whole file.
#[derive(Deserialize)]
struct ProfileIn {
    #[serde(default)]
    name: String,
    base: Engine,
    #[serde(default)]
    rules: BTreeMap<String, serde_json::Value>,
}

/// Serialize `profile` as a pretty-printed profile document.
pub fn to_json(profile: &AuditProfile) -> Result<String, AuditError> {
    let envelope = EnvelopeOut {
        version: PROFILE_VERSION,
        generator: format!("3D Review audit v{}", env!("CARGO_PKG_VERSION")),
        profile,
    };
    serde_json::to_string_pretty(&envelope).map_err(|error| AuditError::Profile(error.to_string()))
}

/// Parse a profile document. Unknown rules are ignored, missing rules and
/// parameters take the base engine's defaults, and every value is clamped into
/// the range the editor offers ([`AuditProfile::sanitize`]) — the file may have
/// been edited by hand.
pub fn from_json(json: &str) -> Result<AuditProfile, AuditError> {
    let envelope: EnvelopeIn =
        serde_json::from_str(json).map_err(|error| AuditError::Profile(error.to_string()))?;
    if envelope.version > PROFILE_VERSION {
        return Err(AuditError::ProfileVersion {
            found: envelope.version,
            supported: PROFILE_VERSION,
        });
    }
    let mut rules = BTreeMap::new();
    for (name, value) in envelope.profile.rules {
        let Some(rule) = RuleId::from_wire(&name) else {
            continue;
        };
        // A rule whose settings are malformed falls back to its default rather
        // than failing the profile.
        if let Ok(config) = serde_json::from_value::<RuleConfig>(value) {
            rules.insert(rule, config);
        }
    }
    let mut profile = AuditProfile {
        name: envelope.profile.name,
        base: envelope.profile.base,
        rules,
    };
    profile.sanitize();
    Ok(profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::ParamValue;
    use crate::rule::Severity;

    #[test]
    fn a_profile_round_trips() {
        let mut profile = AuditProfile::builtin(Engine::Unreal);
        profile.name = "Studio props".to_owned();
        let config = profile.rules.get_mut(&RuleId::Influences).unwrap();
        config.severity = Severity::Error;
        config.params.insert("max".into(), ParamValue::Number(6.0));
        let json = to_json(&profile).unwrap();
        assert!(
            json.contains("\"skin.influences\""),
            "rules key on wire ids"
        );
        assert_eq!(from_json(&json).unwrap(), profile);
    }

    #[test]
    fn a_newer_version_is_rejected_with_both_numbers() {
        let json = r#"{"version": 99, "profile": {"base": "unity"}}"#;
        match from_json(json) {
            Err(AuditError::ProfileVersion { found, supported }) => {
                assert_eq!((found, supported), (99, PROFILE_VERSION));
            }
            other => panic!("expected a version error, got {other:?}"),
        }
    }

    #[test]
    fn unknown_rules_are_skipped_and_missing_ones_filled() {
        let json = r#"{"version": 1, "profile": {"name": "x", "base": "unity", "rules": {
            "geometry.from_the_future": {"enabled": true, "severity": "error"},
            "skin.influences": {"enabled": true, "severity": "error", "params": {"max": 2}}
        }}}"#;
        let profile = from_json(json).unwrap();
        assert_eq!(profile.rules.len(), RuleId::ALL.len());
        assert_eq!(profile.rule(RuleId::Influences).number("max"), Some(2.0));
        assert_eq!(
            profile.rule(RuleId::Ngons),
            AuditProfile::builtin(Engine::Unity).rule(RuleId::Ngons)
        );
    }
}
