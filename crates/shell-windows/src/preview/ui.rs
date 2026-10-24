//! The thread every preview window lives on.
//!
//! A preview host does not give a handler a thread to keep a window on.
//! `prevhost.exe` creates the handler in the *neutral* apartment, so each call
//! Explorer makes arrives on whichever RPC thread is free, and none of those
//! threads pumps messages: a window created inside `DoPreview` received the
//! messages sent to it while it was being created, and then nothing — no paint,
//! no timer, no message posted by the load worker — until the next call into the
//! handler happened to run on its thread (measured: six seconds of silence, then
//! `Unload`). So the windows get a thread of their own, running an ordinary
//! message loop, and the handler's methods hand it work in order.
//!
//! Handing work over is posting, never waiting: a job is queued and the thread
//! is woken with a thread message. A handler call that waited could deadlock
//! against Explorer, which is blocked in that very call while the window being
//! made sends messages to its parent there.

#![allow(
    unsafe_code,
    reason = "invariant 9: the preview windows' thread - its message loop and the thread message that wakes it"
)]

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock, PoisonError};

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW,
    TranslateMessage, WM_APP, WM_USER,
};

use crate::guard::guarded;

type Job = Box<dyn FnOnce() + Send>;

/// Posted to the thread itself (no window): run what is queued.
const WM_RUN_JOBS: u32 = WM_APP + 2;

static JOBS: Mutex<VecDeque<Job>> = Mutex::new(VecDeque::new());

/// The thread's id, once it is running with a message queue; `None` if it
/// could not be started.
static THREAD: OnceLock<Option<u32>> = OnceLock::new();

/// Run `job` on the window thread, after every job posted before it.
pub(super) fn post(job: impl FnOnce() + Send + 'static) {
    let Some(thread) = *THREAD.get_or_init(start) else {
        log::error!("the preview window thread is not running; dropped a job");
        return;
    };
    JOBS.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push_back(Box::new(job));
    // SAFETY: posting a thread message to a thread that owns a message queue (it
    // made one before publishing its id) and never exits.
    if let Err(error) = unsafe { PostThreadMessageW(thread, WM_RUN_JOBS, WPARAM(0), LPARAM(0)) } {
        log::error!("could not wake the preview window thread: {error}");
    }
}

/// Start the thread and wait until it has a message queue to post to.
fn start() -> Option<u32> {
    let (ready, started) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("preview-windows".into())
        .spawn(move || {
            let mut message = MSG::default();
            // SAFETY: a peek on this thread's own queue, which is what makes the
            // system create the queue; nothing is removed.
            unsafe {
                let _ = PeekMessageW(&mut message, None, WM_USER, WM_USER, PM_NOREMOVE);
            }
            // SAFETY: reads this thread's own id.
            let _ = ready.send(unsafe { GetCurrentThreadId() });
            run();
        });
    match spawned {
        Ok(_) => started.recv().ok(),
        Err(error) => {
            log::error!("could not start the preview window thread: {error}");
            None
        }
    }
}

/// The message loop. Never returns: the thread lives as long as the process,
/// and the DLL is never unloaded under it (`DllCanUnloadNow`).
fn run() {
    let mut message = MSG::default();
    loop {
        // SAFETY: the standard loop over this thread's own queue; `message` is a
        // live MSG. 0 is WM_QUIT, which nothing posts; -1 is an error.
        let got = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if got.0 <= 0 {
            log::error!("the preview window thread's message loop ended ({})", got.0);
            return;
        }
        if message.hwnd.is_invalid() && message.message == WM_RUN_JOBS {
            run_jobs();
            continue;
        }
        // SAFETY: dispatching a message this loop just retrieved.
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn run_jobs() {
    loop {
        let job = JOBS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let Some(job) = job else {
            return;
        };
        let _ = guarded("preview window job", || {
            job();
            Ok(())
        });
    }
}
