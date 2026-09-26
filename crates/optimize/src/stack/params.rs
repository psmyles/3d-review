//! The parameters each operation carries.
//!
//! The two operations that simplify share one [`SimplifySettings`], flattened on
//! the wire so older presets still load.

use serde::{Deserialize, Serialize};

use crate::meshopt::options::simplify as simplify_options;

use super::*;

/// Vertex welding settings.
///
/// meshoptimizer always requires positions to match *exactly* to merge two
/// vertices; the tolerance applies to the other attributes. That is the useful
/// knob in practice: it rejoins a seam that was split only because a normal or
/// UV differs in the last few digits, without moving any geometry.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WeldParams {
    /// Maximum per-component attribute difference still considered equal.
    /// `0.0` means exact binary equality (the fast path).
    pub attribute_tolerance: f32,
    /// Include normals in the equality test. Turning this off merges across
    /// hard edges, which flattens their shading — but it is what you want when
    /// normals will be recomputed downstream.
    pub compare_normals: bool,
    /// Include UVs in the equality test. Off merges across UV seams.
    pub compare_uvs: bool,
    /// Include vertex colors in the equality test.
    pub compare_colors: bool,
}

impl Default for WeldParams {
    fn default() -> Self {
        Self {
            attribute_tolerance: 0.0,
            compare_normals: true,
            compare_uvs: true,
            compare_colors: true,
        }
    }
}

/// Which simplifier runs, and what it is asked to preserve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SimplifyAlgorithm {
    /// Topology-preserving, position error only. The safe default.
    #[default]
    Standard,
    /// Topology-preserving, and additionally penalizes attribute drift so
    /// normals / UVs / colors stay closer to the original.
    WithAttributes,
    /// Ignores topology: much faster and hits the target far more reliably, but
    /// can close holes and merge nearby shells. Best for the smallest levels.
    Sloppy,
}

impl SimplifyAlgorithm {
    pub const ALL: [SimplifyAlgorithm; 3] = [
        SimplifyAlgorithm::Standard,
        SimplifyAlgorithm::WithAttributes,
        SimplifyAlgorithm::Sloppy,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SimplifyAlgorithm::Standard => "Standard",
            SimplifyAlgorithm::WithAttributes => "Preserve Attributes",
            SimplifyAlgorithm::Sloppy => "Sloppy (fast)",
        }
    }
}

/// How much each attribute resists being distorted, used only by
/// [`SimplifyAlgorithm::WithAttributes`]. Higher weights preserve the attribute
/// harder at the cost of geometric accuracy; `0.0` ignores it entirely.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AttributeWeights {
    pub normal: f32,
    pub uv: f32,
    pub color: f32,
}

impl Default for AttributeWeights {
    fn default() -> Self {
        // meshoptimizer's own demo uses weights around 1.0 for normals and
        // noticeably lower for UVs; colors matter least for silhouette.
        Self {
            normal: 1.0,
            uv: 0.5,
            color: 0.0,
        }
    }
}

/// How the simplifier is configured, shared by the two operations that run it:
/// [`OpKind::SimplifyLod`], which fans the mesh out into a chain, and
/// [`OpKind::Reduce`], which rewrites the mesh in place. One struct rather than
/// two copies of the same three fields, so a setting added here reaches both.
///
/// Flattened into both parameter structs on the wire, so a preset's JSON keeps
/// naming these settings at the operation's top level.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SimplifySettings {
    pub algorithm: SimplifyAlgorithm,
    pub attribute_weights: AttributeWeights,
    pub flags: SimplifyFlags,
}

/// LOD chain settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LodParams {
    #[serde(flatten)]
    pub simplify: SimplifySettings,
    /// One entry per generated level, in order. Level 0 is always the
    /// unsimplified mesh and is not listed here.
    pub levels: Vec<LodLevel>,
}

impl Default for LodParams {
    fn default() -> Self {
        Self {
            simplify: SimplifySettings::default(),
            levels: vec![
                LodLevel {
                    target_ratio: 0.5,
                    target_error: 0.01,
                },
                LodLevel {
                    target_ratio: 0.25,
                    target_error: 0.02,
                },
                LodLevel {
                    target_ratio: 0.125,
                    target_error: 0.05,
                },
            ],
        }
    }
}

/// How far one simplify run is asked to go: a LOD level's target, and equally
/// [`ReduceParams`]'s single one.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LodLevel {
    /// Target triangle count as a fraction of the level-0 mesh, in `0.0..=1.0`.
    pub target_ratio: f32,
    /// Error the simplifier may not exceed, relative to the mesh extent (or in
    /// world units when [`SimplifyFlags::error_absolute`] is set). The
    /// simplifier stops short of `target_ratio` rather than exceed this.
    pub target_error: f32,
}

impl Default for LodLevel {
    fn default() -> Self {
        Self {
            target_ratio: 0.5,
            target_error: 0.01,
        }
    }
}

/// Settings for the in-place [`OpKind::Reduce`] operation: the same simplifier
/// configuration a LOD chain uses, against a single target.
///
/// The difference from [`LodParams`] is entirely in what [`crate::process`] does
/// with the result. A LOD operation fans out, leaving the mesh it simplified
/// from untouched as level 0; Reduce is an ordinary stack step, so its output
/// *is* the mesh every later operation sees, every LOD level starts from, and
/// the export writes in the source mesh's place.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReduceParams {
    #[serde(flatten)]
    pub simplify: SimplifySettings,
    /// Ratio and error limit, read exactly as a LOD level's are.
    #[serde(flatten)]
    pub target: LodLevel,
}

/// The `meshopt_Simplify*` option flags, as individually-labelled toggles. Kept
/// as named booleans rather than a raw bitmask so a preset stays readable and
/// survives upstream renumbering the bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SimplifyFlags {
    /// Never move vertices on an open boundary — keeps pieces that must stay
    /// aligned with neighbouring meshes from pulling away.
    pub lock_border: bool,
    /// Interpret `target_error` in world units instead of as a fraction of the
    /// mesh extent.
    pub error_absolute: bool,
    /// Let the simplifier delete disconnected parts as it goes.
    pub prune: bool,
    /// Even out triangle size and shape, at some cost to accuracy.
    pub regularize: bool,
    /// A gentler [`Self::regularize`].
    pub regularize_light: bool,
    /// Allow collapses across attribute discontinuities (UV/normal seams).
    pub permissive: bool,
    /// Keep the fold line of a double-sided sheet (two opposite-facing
    /// triangles sharing an edge) from eroding. Experimental upstream.
    pub preserve_folds: bool,
    /// Clamp the attribute error to the position error's scale. Only has an
    /// effect with [`SimplifyAlgorithm::WithAttributes`]. Experimental upstream.
    pub clamp_attribute_error: bool,
}

impl SimplifyFlags {
    /// The bitmask meshoptimizer expects.
    pub fn bits(self) -> u32 {
        let mut bits = 0;
        if self.lock_border {
            bits |= simplify_options::LOCK_BORDER;
        }
        if self.error_absolute {
            bits |= simplify_options::ERROR_ABSOLUTE;
        }
        if self.prune {
            bits |= simplify_options::PRUNE;
        }
        if self.regularize {
            bits |= simplify_options::REGULARIZE;
        }
        if self.regularize_light {
            bits |= simplify_options::REGULARIZE_LIGHT;
        }
        if self.permissive {
            bits |= simplify_options::PERMISSIVE;
        }
        if self.preserve_folds {
            bits |= simplify_options::PRESERVE_FOLDS;
        }
        if self.clamp_attribute_error {
            bits |= simplify_options::ERROR_CLAMPED;
        }
        bits
    }
}

/// A per-object deviation from the global stack.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NodeOverride {
    /// Index into `ModelData::nodes`.
    pub node: usize,
    /// The node's name as the model carried it when the preset was written —
    /// what [`super::OptStack::rebind_to_model`] matches on when the preset is
    /// loaded against another file, since an index alone says nothing about
    /// *which* object it meant. Empty at runtime and in presets written before
    /// this field existed, which is why those load by position instead.
    pub name: String,
    /// Pass this object through untouched. It still appears in every output
    /// level, at full detail — the way to keep a hero prop or a collision shell
    /// out of the optimization.
    pub exclude: bool,
    /// Replacement settings for specific operations, matched by
    /// [`OpInstance::id`]. Operations not listed here use the global settings.
    pub ops: Vec<OpInstance>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_simplify_flag_sets_exactly_its_own_bit() {
        assert_eq!(SimplifyFlags::default().bits(), 0);
        let one = |set: fn(&mut SimplifyFlags)| {
            let mut flags = SimplifyFlags::default();
            set(&mut flags);
            flags.bits()
        };
        assert_eq!(one(|f| f.lock_border = true), simplify_options::LOCK_BORDER);
        assert_eq!(
            one(|f| f.error_absolute = true),
            simplify_options::ERROR_ABSOLUTE
        );
        assert_eq!(one(|f| f.prune = true), simplify_options::PRUNE);
        assert_eq!(one(|f| f.regularize = true), simplify_options::REGULARIZE);
        assert_eq!(
            one(|f| f.regularize_light = true),
            simplify_options::REGULARIZE_LIGHT
        );
        assert_eq!(one(|f| f.permissive = true), simplify_options::PERMISSIVE);
        assert_eq!(
            one(|f| f.preserve_folds = true),
            simplify_options::PRESERVE_FOLDS
        );
        assert_eq!(
            one(|f| f.clamp_attribute_error = true),
            simplify_options::ERROR_CLAMPED
        );
    }
}
