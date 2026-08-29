//! The crate's view of the shared Tracy helpers in `review-prof`, so call sites
//! read as `prof::zone!("…")` like every other crate's do.

pub(crate) use review_prof::zone;
