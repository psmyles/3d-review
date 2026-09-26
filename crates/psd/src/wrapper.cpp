// C-ABI implementation over psd_sdk (Molecular Matters, C++), compiled by `cc` in this
// crate's build.rs — `cl.exe` on Windows, `clang++` on macOS. Nothing parses this with
// bindgen, so the C++ STL is free to appear here; only `wrapper.h` stays C-style.
//
// Scope: the MERGED/composited image only — the flattened result a PSD carries when it
// was saved with "Maximize Compatibility". That is what a viewer of source art wants;
// per-layer browsing is not a goal here.

#include "wrapper.h"

#include "Psd.h"
#include "PsdMallocAllocator.h"
#include "PsdAllocator.h"
#include "PsdFile.h"
#include "PsdDocument.h"
#include "PsdParseDocument.h"
#include "PsdColorMode.h"
#include "PsdPlanarImage.h"
#include "PsdImageDataSection.h"
#include "PsdParseImageDataSection.h"

#include <cmath>
#include <cstring>
#include <vector>
#include <new>

namespace {

// psd_sdk reads exclusively through the abstract File interface (async by contract).
// We back it with an in-memory buffer and serve reads synchronously.
class MemoryFile : public psd::File {
public:
    MemoryFile(psd::Allocator* allocator, const uint8_t* data, size_t size)
        : psd::File(allocator), m_data(data), m_size(size), m_shortRead(false) {}

    // Whether any read ran past end-of-file, i.e. the document is truncated. psd_sdk has no way
    // to tell us a parse went off the end (it does not check the ReadOperation we hand back, and
    // its offsets come from the file's own headers), so the caller checks this after parsing and
    // rejects the document. See DoRead.
    bool HadShortRead(void) const { return m_shortRead; }

private:
    bool DoOpenRead(const wchar_t*) PSD_OVERRIDE { return true; }
    bool DoOpenWrite(const wchar_t*) PSD_OVERRIDE { return false; }
    bool DoClose(void) PSD_OVERRIDE { return true; }

    ReadOperation DoRead(void* buffer, uint32_t count, uint64_t position) PSD_OVERRIDE {
        if (position > m_size) {
            m_shortRead = true;
            std::memset(buffer, 0, count);
            return nullptr;
        }
        const uint64_t available = m_size - position;
        const uint32_t toCopy = (count <= available) ? count : static_cast<uint32_t>(available);
        std::memcpy(buffer, m_data + position, toCopy);
        // A truncated file cannot be served in full. The buffer psd_sdk handed us is raw malloc'd
        // memory, so the tail we did not fill would otherwise stay *uninitialized* — and psd_sdk,
        // believing the read succeeded, would sample it straight into the composite, painting
        // whatever the heap happened to hold into the image. Zero the remainder and record the
        // truncation so fire_psd_open can refuse the document outright.
        if (toCopy < count) {
            std::memset(static_cast<uint8_t*>(buffer) + toCopy, 0, count - toCopy);
            m_shortRead = true;
        }
        // Non-null sentinel: the read already completed; WaitForRead just acknowledges it.
        return reinterpret_cast<ReadOperation>(1);
    }
    bool DoWaitForRead(ReadOperation&) PSD_OVERRIDE { return true; }

    WriteOperation DoWrite(const void*, uint32_t, uint64_t) PSD_OVERRIDE { return nullptr; }
    bool DoWaitForWrite(WriteOperation&) PSD_OVERRIDE { return true; }

    uint64_t DoGetSize(void) const PSD_OVERRIDE { return m_size; }

    const uint8_t* m_data;
    size_t m_size;
    bool m_shortRead;
};

// What this wrapper decodes, checked here as well as in Rust (`review-psd`'s
// `check_supported` / `checked_output_len`, whose constants these mirror). Rust is the
// validation boundary callers see; this copy is what makes the C ABI safe on its own,
// because psd_sdk sizes its planar buffers straight from the header and never checks
// any of it.
//
// * 8/16/32 bits per channel only. psd_sdk allocates `bits / 8` bytes a sample, so a
//   1-bit Bitmap document gets zero-byte planes that the sampler below would over-read.
// * Grayscale or RGB only. Anything else (Bitmap, Indexed, CMYK, Multichannel, Duotone,
//   Lab) would be drawn as if it were RGB, which is a wrong picture rather than an error.
// * 1..=56 channels (Photoshop's own limit), and at least three for RGB.
// * The RGBA8 output and the planar data psd_sdk holds are both capped, in 64-bit
//   arithmetic, so no 32-bit product inside psd_sdk can wrap.
const uint64_t MAX_PSD_DIM = 30000;                 // review-psd MAX_DIMENSION
const uint64_t MAX_PSD_OUTPUT_BYTES = 1ull << 30;   // review-psd MAX_OUTPUT_BYTES
const uint64_t MAX_PSD_PLANAR_BYTES = 2ull << 30;   // review-psd MAX_PLANAR_BYTES
const unsigned int MAX_PSD_CHANNELS = 56;

bool psd_is_supported(const psd::Document* document) {
    const uint64_t w = document->width;
    const uint64_t h = document->height;
    if (w == 0 || h == 0 || w > MAX_PSD_DIM || h > MAX_PSD_DIM) {
        return false;
    }
    const unsigned int bits = document->bitsPerChannel;
    if (bits != 8 && bits != 16 && bits != 32) {
        return false;
    }
    const unsigned int channels = document->channelCount;
    if (channels == 0 || channels > MAX_PSD_CHANNELS) {
        return false;
    }
    switch (document->colorMode) {
        case psd::colorMode::GRAYSCALE:
            break;
        case psd::colorMode::RGB:
            if (channels < 3) {
                return false;
            }
            break;
        default:
            return false;
    }
    // w, h <= 30000 and channels <= 56, so neither product can overflow 64 bits.
    return w * h * 4 <= MAX_PSD_OUTPUT_BYTES
        && w * h * channels * (bits / 8u) <= MAX_PSD_PLANAR_BYTES;
}

// Everything parsed for one document, owned behind a single opaque handle.
struct Doc {
    psd::MallocAllocator allocator;
    std::vector<uint8_t> bytes;      // owned copy; MemoryFile points into this
    MemoryFile* file = nullptr;
    psd::Document* document = nullptr;
    psd::ImageDataSection* imageData = nullptr;
};

// sRGB encode of a linear value already clamped to 0..1 (IEC 61966-2-1).
inline float linear_to_srgb(float v) {
    return v <= 0.0031308f ? v * 12.92f : 1.055f * std::pow(v, 1.0f / 2.4f) - 0.055f;
}

// Read one channel's value at planar index `i`, normalized to 8-bit, by source bit depth.
// `bits` is one of 8/16/32 (psd_is_supported). A 32-bit document stores linear floats,
// and the viewer samples the result as sRGB, so colour channels are encoded here; alpha
// is coverage, not light, and stays linear.
inline uint8_t sample_channel(const void* data, unsigned int bits, size_t i, bool colour) {
    if (bits == 8) {
        return static_cast<const uint8_t*>(data)[i];
    }
    if (bits == 16) {
        return static_cast<uint8_t>(static_cast<const uint16_t*>(data)[i] >> 8);
    }
    float v = static_cast<const float*>(data)[i];
    if (!(v > 0.0f)) v = 0.0f;   // also catches NaN
    if (v > 1.0f) v = 1.0f;
    if (colour) v = linear_to_srgb(v);
    return static_cast<uint8_t>(v * 255.0f + 0.5f);
}

} // namespace

struct fire_psd {
    Doc d;
};

extern "C" {

fire_psd* fire_psd_open(const uint8_t* bytes, size_t len) {
    if (!bytes || len == 0) {
        return nullptr;
    }
    fire_psd* handle = new (std::nothrow) fire_psd();
    if (!handle) {
        return nullptr;
    }
    // A malformed PSD must never unwind a C++ exception across the FFI boundary into
    // Rust (that is UB). Catch everything and surface it as a null handle.
    try {
        Doc& d = handle->d;
        d.bytes.assign(bytes, bytes + len);
        d.file = new (std::nothrow) MemoryFile(&d.allocator, d.bytes.data(), d.bytes.size());
        if (!d.file) {
            fire_psd_free(handle);
            return nullptr;
        }
        d.file->OpenRead(L"memory");
        // The header and the section table only: nothing is sized from the header yet, so
        // the caller can read it (fire_psd_info_get) and refuse the document before any
        // pixel data is allocated.
        d.document = psd::CreateDocument(d.file, &d.allocator);
        // Section lengths come from the file and are *skipped*, not read, so one that runs
        // past the end is not a short read — it leaves the image data starting beyond the
        // file, with psd_sdk's `size - offset` length wrapped round to gigabytes.
        if (!d.document || d.file->HadShortRead()
            || d.document->imageDataSection.offset > d.bytes.size()) {
            fire_psd_free(handle);
            return nullptr;
        }
        return handle;
    } catch (...) {
        fire_psd_free(handle);
        return nullptr;
    }
}

int fire_psd_info_get(const fire_psd* doc, fire_psd_info* out_info) {
    if (!doc || !out_info || !doc->d.document) {
        return 1;
    }
    const psd::Document* document = doc->d.document;
    out_info->width = document->width;
    out_info->height = document->height;
    out_info->channels = static_cast<uint16_t>(document->channelCount);
    out_info->bits_per_channel = static_cast<uint16_t>(document->bitsPerChannel);
    out_info->color_mode = static_cast<uint16_t>(document->colorMode);
    out_info->reserved = 0;
    return 0;
}

int fire_psd_decode_merged(fire_psd* doc) {
    if (!doc || !doc->d.document) {
        return FIRE_PSD_BAD_ARGUMENT;
    }
    Doc& d = doc->d;
    if (d.imageData) {
        return FIRE_PSD_OK;
    }
    // Checked before anything is sized from the header (see psd_is_supported).
    if (!psd_is_supported(d.document)) {
        return FIRE_PSD_UNSUPPORTED;
    }
    // Merged image is only present when the PSD was saved with Maximize Compatibility.
    if (d.document->imageDataSection.length == 0) {
        return FIRE_PSD_NO_MERGED_IMAGE;
    }
    try {
        d.imageData = psd::ParseImageDataSection(d.document, d.file, &d.allocator);
    } catch (...) {
        d.imageData = nullptr;
        return FIRE_PSD_DECODE_FAILED;
    }
    // A truncated document parses "successfully" — psd_sdk follows the header's own offsets
    // and never learns it ran off the end — so the composite would be built from the zeros we
    // substituted for the missing bytes. Refuse it rather than display a half-invented image.
    if (d.file->HadShortRead()) {
        return FIRE_PSD_TRUNCATED;
    }
    if (!d.imageData || !d.imageData->images
        || d.imageData->imageCount < d.document->channelCount) {
        return FIRE_PSD_DECODE_FAILED;
    }
    for (unsigned int i = 0; i < d.imageData->imageCount; ++i) {
        if (!d.imageData->images[i].data) {
            return FIRE_PSD_DECODE_FAILED;
        }
    }
    return FIRE_PSD_OK;
}

int fire_psd_read_merged_rgba8(const fire_psd* doc, uint8_t* out_pixels, size_t out_len) {
    if (!doc || !out_pixels || !doc->d.document) {
        return FIRE_PSD_BAD_ARGUMENT;
    }
    const Doc& d = doc->d;
    // fire_psd_decode_merged succeeded, which is what vouches for every plane below.
    if (!d.imageData || d.file->HadShortRead()) {
        return FIRE_PSD_NOT_DECODED;
    }
    const psd::Document* document = d.document;
    const size_t pixels = static_cast<size_t>(document->width) * document->height;
    if (out_len / 4 < pixels) {
        return FIRE_PSD_BUFFER_TOO_SMALL;
    }

    const unsigned int bits = document->bitsPerChannel;
    const unsigned int imageCount = d.imageData->imageCount;
    const psd::PlanarImage* images = d.imageData->images;
    const bool isGray = document->colorMode == psd::colorMode::GRAYSCALE;
    // The first extra channel is the composite's transparency when the document has one
    // (plane 1 for grayscale, plane 3 for RGB).
    const unsigned int alphaPlane = isGray ? 1u : 3u;
    const bool hasAlpha = imageCount > alphaPlane;

    for (size_t i = 0; i < pixels; ++i) {
        uint8_t r, g, b;
        if (isGray) {
            r = g = b = sample_channel(images[0].data, bits, i, true);
        } else {
            r = sample_channel(images[0].data, bits, i, true);
            g = sample_channel(images[1].data, bits, i, true);
            b = sample_channel(images[2].data, bits, i, true);
        }
        const uint8_t a = hasAlpha ? sample_channel(images[alphaPlane].data, bits, i, false) : 255;
        out_pixels[i * 4 + 0] = r;
        out_pixels[i * 4 + 1] = g;
        out_pixels[i * 4 + 2] = b;
        out_pixels[i * 4 + 3] = a;
    }
    return FIRE_PSD_OK;
}

void fire_psd_free(fire_psd* doc) {
    if (!doc) {
        return;
    }
    Doc& d = doc->d;
    if (d.imageData) {
        psd::DestroyImageDataSection(d.imageData, &d.allocator);
    }
    if (d.document) {
        psd::DestroyDocument(d.document, &d.allocator);
    }
    if (d.file) {
        d.file->Close();
        delete d.file;
        d.file = nullptr;
    }
    delete doc;
}

} // extern "C"
