//! How this process was asked to show its window, and showing it that way.
//!
//! Nothing to do with FBX: it lives in this crate because it reads the Windows
//! `STARTUPINFOW` the shell hands a launched process and shows the window with
//! `ShowWindow` / `SetWindowPlacement`, and invariant 9 confines FFI to a named set
//! of crates, of which this is one. `app` reads the hint as a plain enum, hands the
//! window over as the raw handle winit reports, and never sees a Win32 call.

#![allow(
    unsafe_code,
    reason = "invariant 9: reads the Windows STARTUPINFO launch hint and shows the window it asks for"
)]

/// A show state the launcher explicitly asked for — e.g. a shortcut whose **Run**
/// field is set to *Maximized* or *Minimized*. Windows passes that through
/// `STARTUPINFO.wShowWindow`, but winit creates its window without consulting it.
/// There is deliberately no "normal": every shell launch carries `SW_SHOWNORMAL`,
/// which is no request at all, so the saved placement stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupShow {
    Maximized,
    Minimized,
}

/// The show state the process was asked to start in, if any. Always `None` off
/// Windows.
#[cfg(windows)]
pub fn startup_show() -> Option<StartupShow> {
    use windows_sys::Win32::System::Threading::{
        GetStartupInfoW, STARTF_USESHOWWINDOW, STARTUPINFOW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SW_FORCEMINIMIZE, SW_MINIMIZE, SW_SHOWMAXIMIZED, SW_SHOWMINIMIZED, SW_SHOWMINNOACTIVE,
    };

    // SAFETY: `GetStartupInfoW` fills the entire caller-provided `STARTUPINFOW`
    // and cannot fail; we hand it a zeroed, correctly-sized struct and only read
    // the scalar `dwFlags`/`wShowWindow` fields it sets.
    let info = unsafe {
        let mut info: STARTUPINFOW = std::mem::zeroed();
        GetStartupInfoW(&mut info);
        info
    };

    if info.dwFlags & STARTF_USESHOWWINDOW == 0 {
        return None;
    }
    // `SW_MAXIMIZE` is the same value as `SW_SHOWMAXIMIZED`.
    match i32::from(info.wShowWindow) {
        SW_SHOWMAXIMIZED => Some(StartupShow::Maximized),
        SW_SHOWMINIMIZED | SW_SHOWMINNOACTIVE | SW_MINIMIZE | SW_FORCEMINIMIZE => {
            Some(StartupShow::Minimized)
        }
        _ => None,
    }
}

/// Off Windows there is no `STARTUPINFO` launch hint.
#[cfg(not(windows))]
pub fn startup_show() -> Option<StartupShow> {
    None
}

/// Show the hidden window `hwnd` maximized, in one step, so it never appears in
/// any other state first.
///
/// The process's first show can have its command swapped for the launcher's
/// `STARTUPINFO.wShowWindow` (the `SW_SHOWNORMAL` every shell launch carries); a
/// plain `SW_SHOW` is, but an explicit `SW_SHOWMAXIMIZED` is left alone. It sends
/// `WM_SIZE` with `SIZE_MAXIMIZED` before returning, which is where winit learns
/// the window is maximized.
#[cfg(windows)]
pub fn show_window_maximized(hwnd: std::num::NonZeroIsize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_SHOWMAXIMIZED, ShowWindow};

    // SAFETY: `ShowWindow` takes a window handle by value and only fails, without
    // touching memory, when the handle is not a window.
    unsafe { ShowWindow(hwnd.get(), SW_SHOWMAXIMIZED) };
}

/// Send the hidden window `hwnd` straight to the taskbar without activating it, as
/// a *Run: Minimized* launch asks (`SW_SHOWMINNOACTIVE`), so it never appears on
/// screen first. With `restore_maximized`, restoring it gives a maximized window:
/// `WPF_RESTORETOMAXIMIZED`, which only a placement can carry.
#[cfg(windows)]
pub fn show_window_minimized(hwnd: std::num::NonZeroIsize, restore_maximized: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowPlacement, SW_SHOWMINNOACTIVE, SetWindowPlacement, WINDOWPLACEMENT,
        WPF_RESTORETOMAXIMIZED,
    };

    // SAFETY: both calls take the handle by value and fail without touching memory
    // when it is not a window; `placement` is a whole `WINDOWPLACEMENT` with its
    // `length` set, which is all they read or write.
    unsafe {
        let mut placement: WINDOWPLACEMENT = std::mem::zeroed();
        placement.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
        if GetWindowPlacement(hwnd.get(), &mut placement) == 0 {
            return;
        }
        placement.showCmd = SW_SHOWMINNOACTIVE as u32;
        placement.flags = if restore_maximized {
            WPF_RESTORETOMAXIMIZED
        } else {
            0
        };
        SetWindowPlacement(hwnd.get(), &placement);
    }
}
