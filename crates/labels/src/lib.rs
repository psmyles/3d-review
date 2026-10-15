//! The catalog text for the typed values `review-audit` and `review-optimize`
//! hand back: a check's name and advice, a severity, a measured figure, an
//! operation, a run's warning, an export's note.
//!
//! Every map is an exhaustive `match` with no fall-through arm (invariant 12),
//! so a variant added to a library is a compile error here rather than English
//! on a translated screen. They live in their own crate, rather than in `ui`,
//! because two things print them: the viewer's chrome and notices, and
//! `review-cli`, which links no egui. One map per value keeps the Inspector, a
//! notice and a console line calling the same thing by the same name.

#![forbid(unsafe_code)]

/// The typed message keys, generated from the `labels-*.ftl` files by this
/// crate's build script.
mod keys {
    include!(concat!(env!("OUT_DIR"), "/keys.rs"));
}

pub mod audit;
pub mod opt;
pub mod units;
