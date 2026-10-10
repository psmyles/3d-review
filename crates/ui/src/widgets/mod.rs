//! Reusable, theme-driven UI primitives shared by the toolbar, option panels and
//! stats overlay. None of these own state — they paint and report interactions.
//!
//! One file per family: [`bar`] the chrome bands, [`card`] the floating stats
//! cards, [`buttons`] buttons and tab strips, [`form`] the panel form rows,
//! [`swatch`] the color swatches (the only ones that paint rather than delegate),
//! [`list`] the list rows, [`thumbnail`] the pooled-texture previews, and
//! [`tooltip`] the one tooltip shape every control uses.

mod bar;
mod buttons;
mod card;
mod form;
mod list;
mod swatch;
mod thumbnail;
mod tooltip;

pub(crate) use bar::*;
pub(crate) use buttons::*;
pub(crate) use card::*;
pub(crate) use form::*;
pub(crate) use list::*;
pub(crate) use swatch::*;
pub(crate) use thumbnail::*;
pub(crate) use tooltip::*;
