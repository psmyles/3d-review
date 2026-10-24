// The Quick Look extensions' dylib driven exactly as they drive it, without
// Finder: a thumbnail, then a preview session loaded, drawn, orbited, zoomed and
// reframed — each step written out as a PNG. Built and run by
// scripts/quicklook-check.sh, the twin of the Windows shell-thumbnail.ps1 /
// shell-preview.ps1 checks.
//
//   quicklook-check <model.fbx> <out dir> [thumbnail side] [preview WxH]

import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

let arguments = CommandLine.arguments
guard arguments.count >= 3 else {
    FileHandle.standardError.write("usage: quicklook-check <model.fbx> <out dir> [side] [WxH]\n".data(using: .utf8)!)
    exit(2)
}
let model = URL(fileURLWithPath: arguments[1])
let out = URL(fileURLWithPath: arguments[2], isDirectory: true)
let side = arguments.count > 3 ? UInt32(arguments[3]) ?? 256 : 256
let size: (UInt32, UInt32) = {
    let parts = (arguments.count > 4 ? arguments[4] : "640x400").split(separator: "x").compactMap { UInt32($0) }
    return parts.count == 2 ? (parts[0], parts[1]) : (640, 400)
}()
try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)

var failed = false

func write(_ image: CGImage, _ name: String) {
    let url = out.appendingPathComponent(name)
    guard let destination = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)
    else { failed = true; return }
    CGImageDestinationAddImage(destination, image, nil)
    if CGImageDestinationFinalize(destination) {
        print("wrote \(url.path) (\(image.width)x\(image.height))")
    } else {
        failed = true
    }
}

func step(_ what: String, _ ok: Bool) {
    print("\(ok ? "ok  " : "FAIL") \(what)")
    if !ok { failed = true }
}

// ---- Thumbnail ---------------------------------------------------------------

var started = Date()
var thumbnail = ReviewQlImage()
let drew = model.withUnsafeFileSystemRepresentation { review_ql_thumbnail($0!, side, &thumbnail) }
step(String(format: "thumbnail at %upx in %.0f ms", side, Date().timeIntervalSince(started) * 1000), drew)
if drew, let image = makeImage(thumbnail, alpha: .last) {
    write(image, "thumbnail.png")
}
review_ql_image_free(&thumbnail)
step("image freed and emptied", thumbnail.rgba == nil && thumbnail.len == 0)

// ---- Preview -----------------------------------------------------------------

let loaded = DispatchSemaphore(value: 0)
let onLoaded: ReviewQlLoaded = { context in
    Unmanaged<DispatchSemaphore>.fromOpaque(context!).takeUnretainedValue().signal()
}
started = Date()
let preview = model.withUnsafeFileSystemRepresentation {
    review_ql_preview_open($0!, onLoaded, Unmanaged.passUnretained(loaded).toOpaque())
}
step("preview opened", preview != nil)
if let text = review_ql_preview_status_text(preview) {
    print("     status while loading: \(String(cString: text))")
}
step("load called back", loaded.wait(timeout: .now() + 60) == .success)
step("load finished", review_ql_preview_finish_load(preview))
step(String(format: "loaded in %.0f ms", Date().timeIntervalSince(started) * 1000), true)
step("no status text once loaded", review_ql_preview_status_text(preview) == nil)

review_ql_preview_resize(preview, size.0, size.1)

/// Draw until the picture settles (ambient occlusion, a glide), then save it.
func settleAndSave(_ name: String) {
    review_ql_preview_draw(preview)
    var ticks = 0
    while review_ql_preview_is_settling(preview) && ticks < 200 {
        Thread.sleep(forTimeInterval: 0.016)
        review_ql_preview_tick(preview)
        ticks += 1
    }
    var frame = ReviewQlImage()
    let have = review_ql_preview_frame(preview, &frame)
    step("\(name): a \(frame.width)x\(frame.height) frame after \(ticks) ticks", have && frame.width == size.0)
    if have, let image = makeImage(frame, alpha: .noneSkipLast) {
        write(image, name)
    }
}

settleAndSave("preview-1-framed.png")

// Orbit: a left drag a quarter of the way across.
let middle = (Float(size.0) / 2, Float(size.1) / 2)
review_ql_preview_press(preview, UInt32(REVIEW_QL_BUTTON_LEFT), middle.0, middle.1, false, false)
step("dragging while the button is down", review_ql_preview_is_dragging(preview))
var moved = false
for i in 1...10 {
    moved = review_ql_preview_pointer_moved(preview, middle.0 + Float(i) * Float(size.0) / 40, middle.1) || moved
}
review_ql_preview_release(preview, UInt32(REVIEW_QL_BUTTON_LEFT))
step("an orbit drag moves the camera", moved)
step("not dragging once released", !review_ql_preview_is_dragging(preview))
settleAndSave("preview-2-orbited.png")

// Zoom: a wheel in, a trackpad scroll and a pinch.
review_ql_preview_scroll_lines(preview, 3)
review_ql_preview_scroll_pixels(preview, 120)
review_ql_preview_pinch(preview, 0.25)
settleAndSave("preview-3-zoomed.png")

// Reframe glides back to the whole model.
review_ql_preview_reframe(preview)
step("a reframe glides", review_ql_preview_is_settling(preview))
settleAndSave("preview-4-reframed.png")

// A resize draws at the new size.
review_ql_preview_resize(preview, size.0 / 2, size.1 / 2)
var stale = ReviewQlImage()
step("no frame at a new size until drawn", !review_ql_preview_frame(preview, &stale))

review_ql_preview_close(preview)
review_ql_preview_close(nil)
step("closed", true)

exit(failed ? 1 : 0)
