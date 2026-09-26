//! Finder opens on macOS: `application:openURLs:` (`docs/ARCHITECTURE.md`,
//! Platform decisions D14).
//!
//! On Windows a double-clicked `.fbx` arrives as `argv[1]`, which is what `main`
//! reads. macOS does not work that way. Finder, `open(1)` and a drop on the Dock
//! icon all go through Launch Services, which delivers the file as an **Apple event**
//! to the app — a fresh launch gets no arguments at all, and an app that is *already
//! running* gets no new process. Without this hook the `.fbx` association does
//! nothing on macOS: the viewer opens empty and ignores every later open.
//!
//! ## Why the delegate method is added at runtime
//!
//! AppKit turns that Apple event into `application:openURLs:` on the
//! `NSApplicationDelegate` — but winit owns the delegate, and it implements neither
//! that method nor a way to extend it. The three ways out are: replace the delegate
//! (winit's own accessor panics if the app's delegate is not its class, so this
//! crashes on the next event), register an `NSAppleEventManager` handler ourselves
//! (`NSApplication` installs its own during `finishLaunching`, i.e. *after* anything
//! we could do from `main`, so ours would be replaced), or add the one missing
//! method to winit's delegate class. The last is the only one that leaves winit's
//! own dispatch intact, so that is what this does.
//!
//! The delegate is then re-set on `NSApplication`. `setDelegate:` caches which
//! delegate methods exist at the moment it is called, and winit calls it before we
//! get here; without the re-set AppKit would go on believing the delegate cannot
//! open files.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, Sel};
use objc2::{ffi, sel};
use objc2_app_kit::NSApplication;
use objc2_foundation::{MainThreadMarker, NSArray, NSURL};

/// Where an open goes. Main-thread only by construction: `application:openURLs:` is
/// delivered on the main thread and so is every reader below, which is why a
/// thread-local suffices and no lock is involved.
struct Hook {
    /// `None` until [`install`]. `Rc` rather than `Box` so [`deliver`] can take a
    /// handle *out* of the borrow before calling it — see the note there.
    on_open: Option<Rc<dyn Fn(PathBuf)>>,
    /// Opens that arrived before there was a window to put them in. Drained by
    /// [`take_pending`].
    pending: Vec<PathBuf>,
    /// Whether the first window exists yet — i.e. whether `pending` is still the
    /// right answer.
    live: bool,
}

thread_local! {
    static HOOK: RefCell<Hook> = const {
        RefCell::new(Hook { on_open: None, pending: Vec::new(), live: false })
    };
}

/// See [`crate::install_open_handler`].
pub(crate) fn install(on_open: impl Fn(PathBuf) + 'static) -> bool {
    HOOK.with(|hook| hook.borrow_mut().on_open = Some(Rc::new(on_open)));

    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: reading the delegate on the main thread, which is the only thread that
    // sets it.
    let Some(delegate) = (unsafe { app.delegate() }) else {
        return false;
    };

    // The class of the delegate winit actually installed, rather than one looked up
    // by name: it is winit's private type either way, but read from the live object a
    // rename upstream cannot silently turn the hook off. `ProtocolObject` is
    // `AnyObject` underneath, which is what makes the cast sound.
    //
    // SAFETY: `delegate` is live for the duration of this borrow, and the cast is
    // between two representations of the same Objective-C object pointer.
    let object: &AnyObject = unsafe { &*(Retained::as_ptr(&delegate) as *const AnyObject) };
    let class: &AnyClass = object.class();

    // `v@:@@` — returns void, takes (self, _cmd, NSApplication*, NSArray*).
    // `class_addMethod` refuses only if the class already has the method, which would
    // mean winit had grown its own `application:openURLs:` — and then winit's is the
    // one to keep, not ours.
    //
    // SAFETY: the IMP's signature matches the type encoding, and both match what
    // AppKit sends for this selector. The transmute is the required cast to the
    // type-erased `IMP`; the pointers are otherwise identical.
    let added = Bool::from_raw(unsafe {
        let imp: unsafe extern "C" fn() = std::mem::transmute(
            open_urls as extern "C" fn(&AnyObject, Sel, &AnyObject, &NSArray<NSURL>),
        );
        ffi::class_addMethod(
            class as *const AnyClass as *mut ffi::objc_class,
            sel!(application:openURLs:).as_ptr(),
            Some(imp),
            c"v@:@@".as_ptr(),
        )
    });
    if !added.as_bool() {
        return false;
    }

    // Re-set the same delegate so AppKit re-reads which methods it has.
    // `setDelegate:` snapshots that, and winit called it before this method existed.
    // It is the same object, which winit keeps alive for the life of the event loop
    // (`NSApplication` holds its delegate weakly).
    app.setDelegate(Some(&delegate));
    true
}

/// See [`crate::take_pending_opens`].
pub(crate) fn take_pending() -> Vec<PathBuf> {
    HOOK.with(|hook| {
        let mut hook = hook.borrow_mut();
        hook.live = true;
        std::mem::take(&mut hook.pending)
    })
}

/// `application:openURLs:`. Runs on the main thread, called by AppKit.
///
/// Non-`file:` URLs are dropped: the viewer registers no URL scheme, so anything else
/// arriving here is not ours to open — `path` is `None` for exactly those.
extern "C" fn open_urls(_this: &AnyObject, _cmd: Sel, _app: &AnyObject, urls: &NSArray<NSURL>) {
    for url in urls.iter() {
        // SAFETY: `path` reads an immutable property of an `NSURL` AppKit just
        // handed us.
        let Some(path) = (unsafe { url.path() }) else {
            continue;
        };
        deliver(PathBuf::from(path.to_string()));
    }
}

/// Queue an open for the first window, or hand it to the callback if that window
/// exists.
fn deliver(path: PathBuf) {
    // The callback is cloned out of the `RefCell` before it is called: it wakes the
    // event loop, and a borrow still held across that is an "already borrowed" panic
    // waiting for the first time AppKit re-enters us from inside the wake.
    let on_open = HOOK.with(|hook| {
        let mut hook = hook.borrow_mut();
        match (hook.live, &hook.on_open) {
            (true, Some(on_open)) => Some(Rc::clone(on_open)),
            _ => {
                hook.pending.push(path.clone());
                None
            }
        }
    });
    if let Some(on_open) = on_open {
        on_open(path);
    }
}
