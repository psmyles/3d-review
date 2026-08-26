//! Shared DCC world-unit helpers: the known `(meters-per-unit, label)` table
//! and its tolerant matcher, used by both the stats panel's Unit row and the
//! bounding-box dimension labels (one source of truth for both).

/// Known DCC world units (meters per unit) and their short labels. A value is
/// shown in the file's authored unit when it matches one of these.
const KNOWN_UNITS: [(f32, &str); 6] = [
    (1.0, "m"),
    (0.01, "cm"),
    (0.001, "mm"),
    (0.0254, "in"),
    (0.3048, "ft"),
    (0.9144, "yd"),
];

/// Match a file's authored meters-per-unit factor against the known DCC units,
/// with a relative tolerance so float drift in the file's factor still
/// resolves. `None` for a missing (`<= 0` / non-finite) or unrecognised unit.
pub(crate) fn match_known_unit(meters_per_unit: f32) -> Option<(f32, &'static str)> {
    if !meters_per_unit.is_finite() || meters_per_unit <= 0.0 {
        return None;
    }
    KNOWN_UNITS
        .into_iter()
        .find(|(factor, _)| (meters_per_unit - factor).abs() <= factor * 0.001)
}
