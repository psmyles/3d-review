/*
    review_quiet.h -- a sink `cout`/`cerr` for the vendored Instant Meshes tree.

    Added by this project (see review.patch). Instant Meshes narrates every
    stage to the console; the viewer has none, and the remesh runs on a worker
    thread whose stdout would interleave with everything else. `common.h`'s two
    `using std::cout` / `using std::cerr` declarations are patched to name the
    streams below instead, so every `cout << ...` in the vendored sources
    compiles unchanged and writes nowhere.

    Deliberately `inline` rather than a bridge symbol: the vendored tree and the
    bridge are separate `cc::Build` archives, and an inline definition needs no
    link-order guarantee between them.
*/

#pragma once

#include <ostream>
#include <streambuf>

namespace review_quiet {

/// Discards everything written to it. `xsputn` is overridden so a formatted
/// insertion costs one call rather than one per character.
class NullBuf : public std::streambuf {
protected:
    int_type overflow(int_type ch) override { return ch; }
    std::streamsize xsputn(const char *, std::streamsize count) override { return count; }
};

/// The shared sink. A function-local static, so it is constructed on first use
/// (thread-safely, C++11 magic statics) and outlives every caller.
inline std::ostream &stream() {
    static NullBuf buffer;
    static std::ostream sink(&buffer);
    return sink;
}

} // namespace review_quiet
