//! The crate's view of the shared Tracy helpers in `review-prof`, so call sites
//! read as `prof::zone!("…")` like every other crate's do. `app` is the only
//! crate that uses the whole set — the others open zones and nothing more.

pub(crate) use review_prof::{frame_mark, msg, plot, thread_name, zone};
