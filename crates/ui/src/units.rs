//! Shared DCC world-unit helpers: the known units and their tolerant matcher,
//! used by both the stats panel's Unit row and the bounding-box dimension labels
//! (one source of truth for both).

use review_localization::Key;

use crate::keys;

/// One DCC world unit: its size in meters, its symbol, and a length written in it.
/// Both texts are catalog messages (invariant 12); the number is formatted by the
/// caller.
#[derive(Clone, Copy)]
pub(crate) struct KnownUnit {
    pub(crate) meters: f32,
    pub(crate) symbol: Key,
    pub(crate) value: fn(String) -> String,
}

/// Known DCC world units. A value is shown in the file's authored unit when it
/// matches one of these.
pub(crate) const KNOWN_UNITS: [KnownUnit; 6] = [
    KnownUnit {
        meters: 1.0,
        symbol: keys::ui_units::METERS,
        value: meters_value,
    },
    KnownUnit {
        meters: 0.01,
        symbol: keys::ui_units::CENTIMETERS,
        value: |value| keys::ui_units::centimeters_value(value),
    },
    KnownUnit {
        meters: 0.001,
        symbol: keys::ui_units::MILLIMETERS,
        value: |value| keys::ui_units::millimeters_value(value),
    },
    KnownUnit {
        meters: 0.0254,
        symbol: keys::ui_units::INCHES,
        value: |value| keys::ui_units::inches_value(value),
    },
    KnownUnit {
        meters: 0.3048,
        symbol: keys::ui_units::FEET,
        value: |value| keys::ui_units::feet_value(value),
    },
    KnownUnit {
        meters: 0.9144,
        symbol: keys::ui_units::YARDS,
        value: |value| keys::ui_units::yards_value(value),
    },
];

/// A length in meters, for the lengths no known unit describes.
pub(crate) fn meters_value(value: String) -> String {
    keys::ui_units::meters_value(value)
}

/// Match a file's authored meters-per-unit factor against the known DCC units,
/// with a relative tolerance so float drift in the file's factor still
/// resolves. `None` for a missing (`<= 0` / non-finite) or unrecognised unit.
pub(crate) fn match_known_unit(meters_per_unit: f32) -> Option<KnownUnit> {
    if !meters_per_unit.is_finite() || meters_per_unit <= 0.0 {
        return None;
    }
    KNOWN_UNITS
        .into_iter()
        .find(|unit| (meters_per_unit - unit.meters).abs() <= unit.meters * 0.001)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drifted_factor_still_matches_its_unit() {
        // A centimeter file's factor arrives through an f32.
        let unit = match_known_unit(0.009_999_999).expect("a centimeter file");
        assert_eq!(unit.meters, 0.01);
        assert_eq!(review_localization::tr(unit.symbol), "cm");
        assert_eq!((unit.value)("12".to_owned()), "12 cm");
    }

    #[test]
    fn a_missing_or_unknown_unit_matches_nothing() {
        for factor in [0.0, -1.0, f32::NAN, f32::INFINITY, 0.5] {
            assert!(match_known_unit(factor).is_none(), "{factor}");
        }
    }
}
