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

use super::{AuditProfile, Engine, ParamValue, RuleConfig, default_config};
use crate::AuditError;
use crate::rule::{RuleId, Severity};

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
    /// Left out of a hand-written file - Asset Check's rules files are often
    /// written by hand, after the plan's `{version, generator, profile: {name,
    /// rules}}` - it is the generic profile, as `3d-review-cli`'s `--profile`
    /// is when it is left out.
    #[serde(default = "default_base")]
    base: Engine,
    #[serde(default)]
    rules: BTreeMap<String, serde_json::Value>,
}

fn default_base() -> Engine {
    Engine::Generic
}

/// One rule as read. Every field may be left out of a hand-written file:
/// a rule listed there is on unless it says `"enabled": false` - listing it is
/// asking for it - and what else it leaves out is the base engine's default.
#[derive(Deserialize)]
struct RuleIn {
    enabled: Option<bool>,
    severity: Option<Severity>,
    #[serde(default)]
    params: BTreeMap<String, ParamValue>,
    #[serde(default)]
    note: Option<String>,
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
/// parameters take the base engine's defaults, a listed rule is on unless it
/// says otherwise, a missing base is generic, and every value is clamped into
/// the range the editor offers ([`AuditProfile::sanitize`]) — the file may have
/// been edited by hand, and Asset Check's rules files usually are.
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
        if let Ok(read) = serde_json::from_value::<RuleIn>(value) {
            let default = default_config(envelope.profile.base, rule);
            rules.insert(
                rule,
                RuleConfig {
                    enabled: read.enabled.unwrap_or(true),
                    severity: read.severity.unwrap_or(default.severity),
                    params: read.params,
                    note: read.note,
                },
            );
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

    /// A hand-written rule says what it changes and no more, as the Asset Check
    /// plan writes one: listing it switches it on, and what it leaves out is
    /// the base's. It used to need every field, and one written this way fell
    /// back to its default whole - note, severity and limits gone.
    #[test]
    fn a_rule_written_by_hand_needs_only_what_it_changes() {
        let json = r#"{"version": 1, "profile": {"base": "unity", "rules": {
            "geometry.triangle_budget": {"severity": "error",
                "params": {"max_per_object": 5000}, "note": "Quest budget for props."},
            "skin.influences": {"params": {"max": 2}},
            "geometry.ngons": {"enabled": false}
        }}}"#;
        let profile = from_json(json).unwrap();
        let unity = AuditProfile::builtin(Engine::Unity);

        let budget = profile.rule(RuleId::TriangleBudget);
        assert!(budget.enabled);
        assert_eq!(budget.severity, Severity::Error);
        assert_eq!(budget.number("max_per_object"), Some(5000.0));
        assert_eq!(
            budget.number("max_total"),
            unity.rule(RuleId::TriangleBudget).number("max_total"),
            "a parameter left out is the base's"
        );
        assert_eq!(budget.note.as_deref(), Some("Quest budget for props."));

        let influences = profile.rule(RuleId::Influences);
        assert!(influences.enabled);
        assert_eq!(influences.severity, unity.rule(RuleId::Influences).severity);
        assert_eq!(influences.number("max"), Some(2.0));

        assert!(!profile.rule(RuleId::Ngons).enabled);
        assert_eq!(
            profile.rule(RuleId::Ngons).severity,
            unity.rule(RuleId::Ngons).severity
        );
    }

    /// A hand-written rules file names no base engine: it is the generic one.
    #[test]
    fn a_profile_with_no_base_is_generic() {
        let json = r#"{"version": 1, "generator": "hand-written",
            "profile": {"name": "models/prop", "rules": {}}}"#;
        let profile = from_json(json).unwrap();
        assert_eq!(profile.base, Engine::Generic);
        assert!(profile.is_builtin());
    }

    /// A note is written by hand into a rules file; the profile editor shows
    /// none of it, so it has to come through the editor's load - edit - save
    /// untouched, or a technical artist's re-save quietly deletes every note.
    #[test]
    fn a_hand_written_note_survives_a_re_save() {
        let json = r#"{"version": 1, "generator": "hand-written", "profile": {
            "name": "Props", "base": "unity", "rules": {
            "geometry.triangle_budget": {"enabled": true, "severity": "error",
                "params": {"max_per_object": 5000, "max_total": 20000},
                "note": "  Quest budget for props.  "},
            "skin.influences": {"enabled": true, "severity": "warning", "note": "   "}
        }}}"#;
        let mut profile = from_json(json).unwrap();
        let budget = profile.rule(RuleId::TriangleBudget);
        assert_eq!(budget.note.as_deref(), Some("Quest budget for props."));
        assert_eq!(profile.rule(RuleId::Influences).note, None, "a blank note");
        assert_eq!(profile.rule(RuleId::Ngons).note, None, "a rule left out");

        // What the editor does: change a value in place, then save.
        let config = profile.rules.get_mut(&RuleId::TriangleBudget).unwrap();
        config.severity = Severity::Warning;
        let saved = to_json(&profile).unwrap();
        assert!(
            saved.contains("\"note\": \"Quest budget for props.\""),
            "{saved}"
        );
        assert_eq!(
            saved.matches("\"note\"").count(),
            1,
            "no empty notes written"
        );
        assert_eq!(from_json(&saved).unwrap(), profile);
    }

    /// A note explains a rule; it changes nothing it finds.
    #[test]
    fn a_note_changes_neither_the_finding_key_nor_the_digest() {
        let plain = AuditProfile::builtin(Engine::Unity);
        let mut noted = plain.clone();
        let config = noted.rules.get_mut(&RuleId::TriangleBudget).unwrap();
        config.note = Some("Quest budget for props.".to_owned());
        assert_eq!(
            noted.rule(RuleId::TriangleBudget).finding_key(),
            plain.rule(RuleId::TriangleBudget).finding_key()
        );
        assert_eq!(noted.digest(), plain.digest());
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
