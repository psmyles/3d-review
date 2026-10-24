//! The process's one [`Host`]: brought up on first use, shared by every
//! thumbnail and preview the process draws, and dropped once it has been idle.
//!
//! Explorer keeps a surrogate alive long after the folder that needed it is
//! closed, and a preview host for as long as the pane is open on anything. A
//! device kept alive across that holds its pipelines, the IBL maps and the last
//! model's buffers — tens to hundreds of megabytes of GPU memory nobody is
//! looking at — so a reaper thread drops the host once nothing has drawn for
//! [`IDLE_RELEASE`] and no preview has a viewport open. The next request brings
//! it back up, which costs a device creation.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use review_preview::Host;
use windows::Win32::Foundation::E_UNEXPECTED;
use windows::core::{Error, Result};

/// How long the host outlives its last use.
const IDLE_RELEASE: Duration = Duration::from_secs(30);

/// How often the reaper looks.
const REAPER_PERIOD: Duration = Duration::from_secs(5);

struct Slot {
    host: Option<Host>,
    last_used: Option<Instant>,
    reaper_started: bool,
}

static SLOT: Mutex<Slot> = Mutex::new(Slot {
    host: None,
    last_used: None,
    reaper_started: false,
});

/// The slot, whatever an earlier holder did. A poisoned lock means a panic
/// while it was held, and the panic path below has already dropped the host it
/// may have left half-drawn.
fn slot() -> MutexGuard<'static, Slot> {
    SLOT.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Run `body` with the process's host, bringing it up first if need be. Every
/// GPU call the process makes is inside one of these: the lock is what
/// serializes them.
///
/// A panic inside `body` drops the host — a frame abandoned mid-pass leaves
/// nothing worth trusting on the device — and answers `E_UNEXPECTED`.
pub(crate) fn with_host<T>(body: impl FnOnce(&mut Host) -> T) -> Result<T> {
    let mut slot = slot();
    if slot.host.is_none() {
        let host = Host::new().map_err(|error| {
            log::error!("shell extension: no GPU: {error}");
            Error::from_hresult(E_UNEXPECTED)
        })?;
        log::info!("shell extension drawing on {}", host.adapter_name());
        slot.host = Some(host);
    }
    slot.last_used = Some(Instant::now());
    if !slot.reaper_started {
        slot.reaper_started = start_reaper();
    }
    let Some(host) = slot.host.as_mut() else {
        return Err(Error::from_hresult(E_UNEXPECTED));
    };
    match catch_unwind(AssertUnwindSafe(|| body(host))) {
        Ok(value) => Ok(value),
        Err(_) => {
            log::error!("shell extension: a draw panicked; the GPU is brought up again next time");
            slot.host = None;
            Err(Error::from_hresult(E_UNEXPECTED))
        }
    }
}

/// Run `body` with the host only if it is up — releasing a viewport needs no
/// device brought up to release it on.
pub(crate) fn with_existing_host(body: impl FnOnce(&mut Host)) {
    let mut slot = slot();
    if let Some(host) = slot.host.as_mut() {
        body(host);
    }
}

/// Start the thread that drops an idle host. False if it could not start, in
/// which case the next use tries again and the host simply lives on meanwhile.
fn start_reaper() -> bool {
    std::thread::Builder::new()
        .name("shell-gpu-reaper".into())
        .spawn(|| {
            loop {
                std::thread::sleep(REAPER_PERIOD);
                let mut slot = slot();
                let idle = slot
                    .last_used
                    .is_some_and(|last| last.elapsed() >= IDLE_RELEASE);
                if idle && slot.host.as_ref().is_some_and(Host::is_idle) {
                    slot.host = None;
                    log::debug!("shell extension released the GPU after {IDLE_RELEASE:?} idle");
                }
            }
        })
        .is_ok()
}
