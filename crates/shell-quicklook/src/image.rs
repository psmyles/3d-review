//! `ReviewQlImage`: an image as it crosses the ABI, owned (a thumbnail) or
//! borrowed (a preview frame).

#![allow(
    unsafe_code,
    reason = "invariant 9: handing image buffers across the extensions' C ABI"
)]

use review_preview::Image;

/// The header's `ReviewQlImage`.
#[repr(C)]
pub struct ReviewQlImage {
    pub width: u32,
    pub height: u32,
    pub rgba: *const u8,
    pub len: usize,
}

impl ReviewQlImage {
    pub(crate) const EMPTY: Self = Self {
        width: 0,
        height: 0,
        rgba: std::ptr::null(),
        len: 0,
    };

    /// `image`'s pixels, handed over: [`review_ql_image_free`] takes them back.
    pub(crate) fn owned(image: Image) -> Self {
        let pixels = Box::into_raw(image.rgba.into_boxed_slice());
        Self {
            width: image.width,
            height: image.height,
            rgba: pixels.cast::<u8>().cast_const(),
            len: pixels.len(),
        }
    }

    /// `image`'s pixels, lent for as long as `image` is left alone.
    pub(crate) fn borrowed(image: &Image) -> Self {
        Self {
            width: image.width,
            height: image.height,
            rgba: image.rgba.as_ptr(),
            len: image.rgba.len(),
        }
    }
}

/// Release what `review_ql_thumbnail` filled in, and empty it.
///
/// # Safety
///
/// `image` is null, or points to a `ReviewQlImage` that is empty or was filled
/// by `review_ql_thumbnail` and not freed since.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn review_ql_image_free(image: *mut ReviewQlImage) {
    // SAFETY: the caller's contract — null, or a live image of ours.
    let Some(image) = (unsafe { image.as_mut() }) else {
        return;
    };
    if !image.rgba.is_null() {
        let pixels = std::ptr::slice_from_raw_parts_mut(image.rgba.cast_mut(), image.len);
        // SAFETY: `rgba` and `len` are exactly what `owned` took from
        // `Box::into_raw`, and the contract says this image was not freed since —
        // emptying it below is what makes a second free a no-op.
        drop(unsafe { Box::from_raw(pixels) });
    }
    *image = ReviewQlImage::EMPTY;
}
