//! The unit the file declares, and the factor that gets the geometry there.
//!
//! FBX *declares* its unit rather than fixing one, while import normalizes every
//! file to meters — so the writer has to both scale the coordinates into the
//! source's unit and declare it. Get one without the other and the geometry
//! reads back 100x off, which no error reports.

/// FBX's `UnitScaleFactor` — centimeters per scene unit — for each unit a DCC
/// authors in. Mirrors `review-ui`'s `KNOWN_UNITS` table (its meters-per-unit
/// figures × 100), which is the same set import reads back off a file.
pub(crate) const KNOWN_UNIT_SCALES_CM: [f64; 6] = [100.0, 1.0, 0.1, 2.54, 30.48, 91.44];

/// `UnitScaleFactor` for a source that declared no unit (the demo cube, a file
/// missing the metadata): meters, which is what import normalizes to and so
/// what the geometry already is.
pub(crate) const DEFAULT_UNIT_SCALE_CM: f64 = 100.0;

/// The unit one export is written in — the *source file's* own, so a centimeter
/// asset comes back out of the tool as centimeters instead of being silently
/// re-authored in meters.
///
/// Import normalizes every file to meters, so the geometry reaching the writer
/// is metric whatever the file declared. FBX does not fix a unit, it *declares*
/// one, so honouring the source means two figures that have to agree: the
/// declared factor, and coordinates actually expressed in that unit.
#[derive(Debug, Clone, Copy)]
pub(crate) struct UnitScale {
    /// Exported units per meter — `100.0` for a centimeter file.
    ///
    /// Applied **once, at the top of the hierarchy**: `place_node` builds the
    /// chain against a root frame of `1/per_meter` rather than the identity, so
    /// the factor lands in the topmost emitted node's scale and the whole
    /// subtree rides it. Uniform scaling acts on an affine transform by
    /// conjugation, so scaling the root by `s` scales the assembled scene by
    /// `s` exactly — no scale node injected, and rotations and normals
    /// untouched.
    ///
    /// Applying it per-*local* value instead would double-count: import parks
    /// the unit normalization in the node transforms themselves, so a node's
    /// own inverse — which is what `build_mesh` puts the geometry through under
    /// [`HierarchyMode::Rebuild`] — already hands back the source file's unit.
    /// Only flat geometry, which stays in world meters, is scaled directly.
    ///
    /// With the source's authored properties in hand the question does not
    /// arise: every node's local transform is written as the file authored it,
    /// in the file's own unit, and the geometry comes back to that unit through
    /// the node's inverse exactly as above.
    pub(crate) per_meter: f32,
    /// Centimeters per exported unit, written as FBX's `UnitScaleFactor`.
    pub(crate) unit_scale_cm: f64,
}

impl UnitScale {
    /// `source_unit_meters` is [`review_model::ModelStats::source_unit_meters`]:
    /// meters per source unit as the file declared it, `0.0` for none.
    pub(crate) fn from_source(source_unit_meters: f32) -> Self {
        let declared = f64::from(source_unit_meters) * 100.0;
        let unit_scale_cm = if declared.is_finite() && declared > 0.0 {
            // Snap to the unit the DCC meant. The factor reaches us through an
            // f32, so a centimeter file's `0.01` becomes `0.99999998` here, and
            // writing that would leave the file declaring a unit no importer
            // names and every coordinate scaled by its reciprocal.
            KNOWN_UNIT_SCALES_CM
                .into_iter()
                .find(|known| (declared - known).abs() <= known * 0.001)
                .unwrap_or(declared)
        } else {
            DEFAULT_UNIT_SCALE_CM
        };
        Self {
            // Derived from the snapped factor rather than from the source figure
            // a second time: the declared unit and the coordinates must be exact
            // reciprocals, or the file says one thing and carries another.
            per_meter: (100.0 / unit_scale_cm) as f32,
            unit_scale_cm,
        }
    }
}
