//! The DLL's COM exports and the class factory behind them.

#![allow(
    unsafe_code,
    reason = "invariant 9: the COM server's exports, which write through the raw out-pointers COM hands them"
)]

use std::ffi::c_void;

use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_POINTER, S_FALSE,
};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::core::{BOOL, Error, GUID, HRESULT, IUnknown, Interface, Ref, Result, implement};

use crate::guard::guarded;
use crate::ids::{PREVIEW_CLSID, THUMBNAIL_CLSID};

/// Makes one instance of a class.
type Construct = fn() -> IUnknown;

#[implement(IClassFactory)]
struct ClassFactory {
    construct: Construct,
}

impl IClassFactory_Impl for ClassFactory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        riid: *const GUID,
        object: *mut *mut c_void,
    ) -> Result<()> {
        guarded("CreateInstance", || {
            if riid.is_null() || object.is_null() {
                return Err(Error::from_hresult(E_POINTER));
            }
            // SAFETY: `object` is non-null, and COM's contract for an out
            // interface pointer is that it is valid for one pointer's write.
            unsafe { *object = std::ptr::null_mut() };
            if !outer.is_null() {
                return Err(Error::from_hresult(CLASS_E_NOAGGREGATION));
            }
            let instance = (self.construct)();
            // SAFETY: both pointers are non-null and valid per COM's contract;
            // `query` writes an AddRef'd interface or null through `object`.
            unsafe { instance.query(riid, object) }.ok()
        })
    }

    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

/// COM asks for the class factory of one of our classes.
#[unsafe(no_mangle)]
extern "system" fn DllGetClassObject(
    clsid: *const GUID,
    riid: *const GUID,
    object: *mut *mut c_void,
) -> HRESULT {
    guarded("DllGetClassObject", || {
        if clsid.is_null() || riid.is_null() || object.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        // SAFETY: `object` is non-null and, per COM, valid for one write.
        unsafe { *object = std::ptr::null_mut() };
        crate::initialize_once();
        // SAFETY: `clsid` is non-null and points at a GUID for the call.
        let construct: Construct = match unsafe { *clsid } {
            THUMBNAIL_CLSID => crate::thumbnail::create,
            PREVIEW_CLSID => crate::preview::create,
            _ => return Err(Error::from_hresult(CLASS_E_CLASSNOTAVAILABLE)),
        };
        let factory: IClassFactory = ClassFactory { construct }.into();
        // SAFETY: both pointers are non-null and valid per COM's contract.
        unsafe { factory.query(riid, object) }.ok()
    })
    .into()
}

/// Never: the process keeps its GPU host, its reaper thread and its preview
/// window class, and unloading the code under any of them would be fatal.
/// Surrogates exit when Explorer is done with them, which is what frees it all.
#[unsafe(no_mangle)]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}
