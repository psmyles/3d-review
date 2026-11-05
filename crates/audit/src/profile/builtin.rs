//! The built-in Unity / Unreal / Generic defaults.
//!
//! These are starting points, not law: each value is the common convention for
//! that engine, and every one of them is editable per profile. They are the
//! catalog's (`rules-catalog.toml`): a rule's `[rule.default]` and parameter
//! defaults, with what `[rule.base.<engine>]` changes for an engine. A rule a
//! built-in leaves off still carries a severity, used the moment someone
//! switches it on.

use super::{Engine, RuleConfig, config};
use crate::rule::RuleId;

/// The default settings of `rule` in `engine`'s built-in profile.
pub fn default_config(engine: Engine, rule: RuleId) -> RuleConfig {
    config(rule, &rule.rule().defaults(Some(engine.as_str())))
}
