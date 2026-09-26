//! Offline IBL bake tool (`cargo run -p review-render --features bake --bin bake_ibl`).
//!
//! Regenerates every `assets/ibl_baked/T_IBL_*.bin` from the source HDRs in
//! `assets/textures/` on a headless GPU device. Needs a real GPU; it is never part of
//! a normal build, and the shipped viewer embeds the committed results rather than
//! ever running this (`mac-port-plan.md` D19).
//!
//! It **overwrites committed assets in place**, so `review_render`'s bake validates
//! each payload before writing it: a pass that rendered nothing would otherwise
//! destroy known-good maps that took a GPU run to make.
//!
//! `packaging/generate-ibl-bake.ps1` is the wrapper the installer build uses, and it
//! is freshness-gated — it only invokes this when a baked `.bin` is missing or older
//! than something that determines its bytes.

/// Everything the bake logs, to stderr. The viewer's logger lives in `app`, which
/// this tool does not link; without one, sokol's validation messages - the whole
/// point of reproducing a bad bake in debug - would go nowhere.
struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        eprintln!("{}: {}", record.level(), record.args());
    }

    fn flush(&self) {}
}

static LOGGER: StderrLogger = StderrLogger;

fn main() {
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    if let Err(error) = review_render::bake_ibl_assets() {
        eprintln!("bake_ibl failed: {error}");
        std::process::exit(1);
    }
}
