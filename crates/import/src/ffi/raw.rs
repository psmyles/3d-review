//! Pointer-to-Rust leaf helpers — the last of this crate's `unsafe` outside the
//! bridge call itself.
//!
//! Each one turns a raw pointer the bridge wrote into something safe: a checked
//! slice, an owned `String`, an error message. A null pointer with a non-zero
//! length is an error rather than a dereference, and a zero length is empty
//! rather than a null check.

use std::ffi::CStr;
use std::os::raw::c_char;
use std::ptr::NonNull;
use std::slice;

use crate::ImportError;

use super::raw_scene::ReviewImportError;

pub(super) fn read_error_message(error: &ReviewImportError) -> String {
    // SAFETY: `error.message` is a fixed 256-byte array the bridge always writes
    // as a NUL-terminated string (it is zero-initialized at `[0; 256]` before the
    // call), so `from_ptr` reads a valid C string bounded by the array.
    unsafe {
        CStr::from_ptr(error.message.as_ptr())
            .to_str()
            .ok()
            .filter(|message| !message.is_empty())
            .unwrap_or("unknown FBX import error")
            .to_owned()
    }
}

pub(super) fn read_optional_c_string(value: *const c_char) -> Option<String> {
    if value.is_null() {
        return None;
    }

    // SAFETY: `value` is non-null (checked above) and points at a bridge-owned,
    // NUL-terminated C string that lives until `review_import_free_scene`.
    let text = unsafe { CStr::from_ptr(value) };
    // Lossy: an FBX name in some other encoding still has to reach the
    // Outliner as *something* — dropping it would leave a blank row with no
    // way to tell it from an unnamed node.
    Some(String::from_utf8_lossy(text.to_bytes()).into_owned())
}

pub(super) fn checked_slice<'a, T>(
    ptr: *const T,
    len: usize,
    field_name: &str,
) -> Result<&'a [T], ImportError> {
    if len == 0 {
        return Ok(&[]);
    }

    let Some(ptr) = NonNull::new(ptr as *mut T) else {
        return Err(ImportError::LoadFailed(format!(
            "FBX bridge returned a null pointer for non-empty {field_name}"
        )));
    };

    // SAFETY: `ptr` is non-null (just checked) and the bridge guarantees it
    // points at `len` contiguous, properly aligned `T` values that stay valid
    // for the borrow `'a` (the caller holds `&ReviewImportScene` for the whole
    // walk). `len > 0` here, so the slice is non-empty and within one allocation.
    Ok(unsafe { slice::from_raw_parts(ptr.as_ptr(), len) })
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;

    use super::*;

    #[test]
    fn checked_slice_returns_empty_for_zero_len() {
        // Len 0 is the empty case regardless of the pointer (even null).
        assert!(
            checked_slice::<u32>(std::ptr::null(), 0, "verts")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn checked_slice_reads_a_valid_pointer() {
        let data = [1u32, 2, 3, 4];
        let slice = checked_slice(data.as_ptr(), data.len(), "verts").unwrap();
        assert_eq!(slice, &data[..]);
    }

    #[test]
    fn checked_slice_rejects_null_for_nonempty() {
        let err = checked_slice::<u32>(std::ptr::null(), 3, "indices").unwrap_err();
        assert!(
            matches!(err, ImportError::LoadFailed(message) if message.contains("indices")),
            "a null pointer for non-empty data must be a LoadFailed naming the field"
        );
    }

    #[test]
    fn read_optional_c_string_is_none_for_null() {
        assert_eq!(read_optional_c_string(std::ptr::null()), None);
    }

    #[test]
    fn read_optional_c_string_reads_a_c_string() {
        let text = CString::new("mesh_01").unwrap();
        assert_eq!(
            read_optional_c_string(text.as_ptr()),
            Some("mesh_01".to_owned())
        );
    }

    #[test]
    fn read_error_message_falls_back_when_empty() {
        let error = ReviewImportError { message: [0; 256] };
        assert_eq!(read_error_message(&error), "unknown FBX import error");
    }

    #[test]
    fn read_error_message_reads_the_bridge_text() {
        let mut error = ReviewImportError { message: [0; 256] };
        for (slot, &byte) in error.message.iter_mut().zip(b"bad fbx") {
            *slot = byte as c_char;
        }
        assert_eq!(read_error_message(&error), "bad fbx");
    }
}
