//! Decode the real PSD fixture in `assets/test_textures/`.
//!
//! The unit tests in `lib.rs` cover the validation arithmetic and the failure paths;
//! this one is the only check that the vendored psd_sdk actually built and decodes —
//! the property that mattered when the crate moved off a prebuilt static lib to
//! `cc`-compiled source. Skips itself when the fixture is absent, as `review-optimize`'s
//! fixture suites do — and, as there, `REVIEW_REQUIRE_FIXTURES=1` turns that skip
//! into a failure, so the run that decides whether the tree is good cannot pass by
//! finding nothing to do.

use std::path::PathBuf;

fn fixture() -> Option<PathBuf> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/test_textures/T_Sides_D.psd");
    path.exists().then_some(path)
}

#[test]
fn decodes_the_merged_composite() {
    let Some(path) = fixture() else {
        let reason = "assets/test_textures/T_Sides_D.psd is not present";
        assert!(
            !std::env::var_os("REVIEW_REQUIRE_FIXTURES").is_some_and(|value| value != "0"),
            "REVIEW_REQUIRE_FIXTURES is set, so this run may not skip: {reason}"
        );
        eprintln!("skipping: {reason}");
        return;
    };
    let bytes = std::fs::read(&path).expect("read the PSD fixture");
    let image = review_psd::decode_psd(&bytes).expect("decode the merged composite");

    assert!(image.width > 0 && image.height > 0);
    assert_eq!(
        image.rgba8.len(),
        image.width as usize * image.height as usize * 4,
        "the buffer is exactly width * height * RGBA8"
    );
    assert!(
        matches!(image.bits_per_channel, 8 | 16 | 32),
        "bits per channel came back as {}",
        image.bits_per_channel
    );
    // A decode that silently produced nothing would still satisfy the length check
    // above, since the buffer is pre-zeroed before the C++ writes into it.
    assert!(
        image.rgba8.iter().any(|&b| b != 0),
        "every byte is zero — the merged image was never written"
    );
}
