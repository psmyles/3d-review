/*
 * C-ABI wrapper over the vendored psd_sdk C++ library (Molecular Matters), compiled
 * from `vendor/Psd/` by `cc` in this crate's build.rs alongside `wrapper.cpp`.
 *
 * IMPORTANT: `src/bindings.rs` is a COMMITTED Rust mirror of these declarations, and
 * nothing regenerates it at build time (no bindgen, no libclang). The two are kept in
 * lockstep BY HAND: change a signature or a struct field here and `bindings.rs` must
 * change with it. `fire_psd_info` is 16 bytes, and the committed bindings assert that
 * size plus every field offset — which is what turns a layout mistake into a compile
 * error rather than garbage pixels.
 *
 * The header is deliberately C-style: only <stdint.h> / <stddef.h>, never <cstdint> or
 * any C++ standard-library header. That is the shape bindgen needs (clang < 19 hard-
 * errors, STL1000, on the installed MSVC STL headers), and keeping it that way leaves
 * regenerating the bindings a one-command job. The psd_sdk C++ behind it DOES use the
 * STL, but `cl.exe` compiles that, not clang, so it is unaffected.
 *
 * The implementation lives in wrapper.cpp.
 */
#ifndef FIRE_PSD_WRAPPER_H
#define FIRE_PSD_WRAPPER_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque handle to a decoded PSD document (concrete type defined C++-side). */
typedef struct fire_psd fire_psd;

/* Basic image info surfaced to Rust: the header as the file states it, read before any
 * pixel data is sized from it. `color_mode` is psd_sdk's colorMode::Enum (1 Grayscale,
 * 3 RGB, ...). */
typedef struct fire_psd_info {
    uint32_t width;
    uint32_t height;
    uint16_t channels;
    uint16_t bits_per_channel;
    uint16_t color_mode;
    uint16_t reserved;
} fire_psd_info;

/* Status codes returned by fire_psd_decode_merged / fire_psd_read_merged_rgba8. */
#define FIRE_PSD_OK 0
#define FIRE_PSD_BAD_ARGUMENT 1
#define FIRE_PSD_NO_MERGED_IMAGE 2
#define FIRE_PSD_UNSUPPORTED 3
#define FIRE_PSD_DECODE_FAILED 4
#define FIRE_PSD_TRUNCATED 5
#define FIRE_PSD_NOT_DECODED 6
#define FIRE_PSD_BUFFER_TOO_SMALL 7

/* Open a PSD from an in-memory buffer, parsing its header and section table only.
 * Returns NULL on failure. Nothing is allocated from the header's dimensions yet. */
fire_psd* fire_psd_open(const uint8_t* bytes, size_t len);

/* Populate *out_info from the header. Returns 0 on success, non-zero on error. */
int fire_psd_info_get(const fire_psd* doc, fire_psd_info* out_info);

/* Decode the merged/composited image. Refuses (FIRE_PSD_UNSUPPORTED) any document the
 * wrapper cannot draw faithfully or whose planes would exceed its size caps, before
 * anything is allocated. Returns FIRE_PSD_OK on success. */
int fire_psd_decode_merged(fire_psd* doc);

/* Read the decoded merged image as 8-bit RGBA into out_pixels (at least
 * width*height*4 bytes). Returns FIRE_PSD_OK on success. */
int fire_psd_read_merged_rgba8(const fire_psd* doc, uint8_t* out_pixels, size_t out_len);

/* Free a document returned by fire_psd_open. */
void fire_psd_free(fire_psd* doc);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* FIRE_PSD_WRAPPER_H */
